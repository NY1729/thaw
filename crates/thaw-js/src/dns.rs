//! `node:dns` host: the `__thaw_dns_*` functions the registry shim calls. The resolver client (`dns_client` below, UDP with
//! TCP retry, Node's record shapes) is a verbatim copy of crates/thaw-quickjs/src/quickjs/dns_client.rs; queries run on
//! worker threads through `async_op_start` and are collected by `__thaw_async_poll`.

use std::io::{self, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::time::Duration;

use crate::net::{async_op_start, n, s};
use crate::Interp;
use crate::Value;

/// `dns.lookup` through the OS resolver (`getaddrinfo`); blocking, like the rest of the synchronous host calls.
fn dns_lookup_json(hostname: String) -> String {
    use std::net::ToSocketAddrs;

    match (hostname.as_str(), 0u16).to_socket_addrs() {
        Ok(addrs) => {
            let mut addresses: Vec<serde_json::Value> = addrs
                .map(|addr| {
                    let ip = addr.ip();
                    serde_json::json!({ "address": ip.to_string(), "family": if ip.is_ipv4() { 4 } else { 6 } })
                })
                .collect();
            addresses.dedup();
            serde_json::json!({ "ok": true, "addresses": addresses }).to_string()
        }
        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
    }
}

fn dns_lookup_start(hostname: String) -> u32 {
    async_op_start(move || {
        let json = dns_lookup_json(hostname);
        Box::new(move || json)
    })
}

// A small DNS client for `node:dns` resolve* / reverse: UDP with a TCP retry for truncated answers, Node's record shapes, c-ares error codes.
// Queries run on worker threads (see `async_op_start`); nothing here touches JavaScript or thread-local socket tables.

const DNS_TYPES: &[(&str, u16)] = &[
    ("A", 1),
    ("NS", 2),
    ("CNAME", 5),
    ("SOA", 6),
    ("PTR", 12),
    ("MX", 15),
    ("TXT", 16),
    ("AAAA", 28),
    ("SRV", 33),
    ("NAPTR", 35),
    ("CAA", 257),
    ("ANY", 255),
];

fn dns_type_code(name: &str) -> Option<u16> {
    DNS_TYPES
        .iter()
        .find(|(label, _)| *label == name)
        .map(|(_, code)| *code)
}

fn dns_type_name(code: u16) -> Option<&'static str> {
    DNS_TYPES
        .iter()
        .find(|(_, value)| *value == code)
        .map(|(label, _)| *label)
}

/// c-ares error code Node reports for a failed query.
struct DnsFailure {
    code: &'static str,
}

fn dns_encode_name(name: &str, packet: &mut Vec<u8>) -> Result<(), DnsFailure> {
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    if trimmed.len() > 253 {
        return Err(DnsFailure { code: "EBADNAME" });
    }
    if !trimmed.is_empty() {
        for label in trimmed.split('.') {
            if label.is_empty() || label.len() > 63 {
                return Err(DnsFailure { code: "EBADNAME" });
            }
            packet.push(label.len() as u8);
            packet.extend_from_slice(label.as_bytes());
        }
    }
    packet.push(0);
    Ok(())
}

fn dns_build_query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>, DnsFailure> {
    let mut packet = Vec::with_capacity(32 + name.len());
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]); // recursion desired
    packet.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    dns_encode_name(name, &mut packet)?;
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&1u16.to_be_bytes());
    Ok(packet)
}

/// Reads a (possibly compressed) name starting at `position`; returns the dotted name and the offset after the name's first form.
fn dns_read_name(packet: &[u8], mut position: usize) -> Result<(String, usize), DnsFailure> {
    let bad = || DnsFailure { code: "EBADRESP" };
    let mut name = String::new();
    let mut after = None;
    let mut jumps = 0;
    loop {
        let length = *packet.get(position).ok_or_else(bad)? as usize;
        if length == 0 {
            position += 1;
            break;
        }
        if length & 0xC0 == 0xC0 {
            let low = *packet.get(position + 1).ok_or_else(bad)? as usize;
            if after.is_none() {
                after = Some(position + 2);
            }
            position = ((length & 0x3F) << 8) | low;
            jumps += 1;
            if jumps > 32 {
                return Err(bad());
            }
            continue;
        }
        let label = packet
            .get(position + 1..position + 1 + length)
            .ok_or_else(bad)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label));
        position += 1 + length;
    }
    Ok((name, after.unwrap_or(position)))
}

fn dns_u16(packet: &[u8], position: usize) -> Result<u16, DnsFailure> {
    packet
        .get(position..position + 2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        .ok_or(DnsFailure { code: "EBADRESP" })
}

fn dns_u32(packet: &[u8], position: usize) -> Result<u32, DnsFailure> {
    packet
        .get(position..position + 4)
        .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or(DnsFailure { code: "EBADRESP" })
}

fn dns_character_string(
    packet: &[u8],
    position: usize,
    end: usize,
) -> Result<(String, usize), DnsFailure> {
    let length = *packet
        .get(position)
        .ok_or(DnsFailure { code: "EBADRESP" })? as usize;
    if position + 1 + length > end {
        return Err(DnsFailure { code: "EBADRESP" });
    }
    Ok((
        String::from_utf8_lossy(&packet[position + 1..position + 1 + length]).into_owned(),
        position + 1 + length,
    ))
}

/// One resource record in Node's `resolveAny` shape (`type` first); the callers pick the fields each rrtype's resolver exposes.
fn dns_record(
    packet: &[u8],
    code: u16,
    start: usize,
    end: usize,
    ttl: u32,
) -> Result<Option<serde_json::Value>, DnsFailure> {
    use serde_json::json;
    let Some(label) = dns_type_name(code) else {
        return Ok(None);
    };
    let value = match code {
        1 => {
            let octets: [u8; 4] = packet
                .get(start..end)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(DnsFailure { code: "EBADRESP" })?;
            json!({ "type": label, "address": std::net::Ipv4Addr::from(octets).to_string(), "ttl": ttl })
        }
        28 => {
            let octets: [u8; 16] = packet
                .get(start..end)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(DnsFailure { code: "EBADRESP" })?;
            json!({ "type": label, "address": std::net::Ipv6Addr::from(octets).to_string(), "ttl": ttl })
        }
        2 | 5 | 12 => {
            let (name, _) = dns_read_name(packet, start)?;
            json!({ "type": label, "value": name })
        }
        15 => {
            let priority = dns_u16(packet, start)?;
            let (exchange, _) = dns_read_name(packet, start + 2)?;
            json!({ "type": label, "priority": priority, "exchange": exchange })
        }
        16 => {
            let mut entries = Vec::new();
            let mut position = start;
            while position < end {
                let (text, next) = dns_character_string(packet, position, end)?;
                entries.push(text);
                position = next;
            }
            json!({ "type": label, "entries": entries })
        }
        33 => {
            let priority = dns_u16(packet, start)?;
            let weight = dns_u16(packet, start + 2)?;
            let port = dns_u16(packet, start + 4)?;
            let (name, _) = dns_read_name(packet, start + 6)?;
            json!({ "type": label, "priority": priority, "weight": weight, "port": port, "name": name })
        }
        6 => {
            let (nsname, next) = dns_read_name(packet, start)?;
            let (hostmaster, next) = dns_read_name(packet, next)?;
            json!({
                "type": label,
                "nsname": nsname,
                "hostmaster": hostmaster,
                "serial": dns_u32(packet, next)?,
                "refresh": dns_u32(packet, next + 4)?,
                "retry": dns_u32(packet, next + 8)?,
                "expire": dns_u32(packet, next + 12)?,
                "minttl": dns_u32(packet, next + 16)?,
            })
        }
        35 => {
            let order = dns_u16(packet, start)?;
            let preference = dns_u16(packet, start + 2)?;
            let (flags, next) = dns_character_string(packet, start + 4, end)?;
            let (service, next) = dns_character_string(packet, next, end)?;
            let (regexp, next) = dns_character_string(packet, next, end)?;
            let (replacement, _) = dns_read_name(packet, next)?;
            json!({ "type": label, "flags": flags, "service": service, "regexp": regexp, "replacement": replacement, "order": order, "preference": preference })
        }
        257 => {
            let critical = *packet.get(start).ok_or(DnsFailure { code: "EBADRESP" })?;
            let tag_length = *packet
                .get(start + 1)
                .ok_or(DnsFailure { code: "EBADRESP" })? as usize;
            let tag = String::from_utf8_lossy(
                packet
                    .get(start + 2..start + 2 + tag_length)
                    .ok_or(DnsFailure { code: "EBADRESP" })?,
            )
            .into_owned();
            let text = String::from_utf8_lossy(
                packet
                    .get(start + 2 + tag_length..end)
                    .ok_or(DnsFailure { code: "EBADRESP" })?,
            )
            .into_owned();
            let mut object = serde_json::Map::new();
            object.insert("type".to_string(), json!(label));
            object.insert("critical".to_string(), json!(critical));
            object.insert(tag, json!(text));
            serde_json::Value::Object(object)
        }
        _ => return Ok(None),
    };
    Ok(Some(value))
}

/// Parses a response; `Ok` carries the answer records, `Err` the c-ares code Node would report.
fn dns_parse_response(packet: &[u8], id: u16) -> Result<Vec<serde_json::Value>, DnsFailure> {
    if packet.len() < 12 || u16::from_be_bytes([packet[0], packet[1]]) != id {
        return Err(DnsFailure { code: "EBADRESP" });
    }
    let flags = u16::from_be_bytes([packet[2], packet[3]]);
    if flags & 0x8000 == 0 {
        return Err(DnsFailure { code: "EBADRESP" });
    }
    match flags & 0x000F {
        0 => {}
        1 => return Err(DnsFailure { code: "EFORMERR" }),
        2 => return Err(DnsFailure { code: "ESERVFAIL" }),
        3 => return Err(DnsFailure { code: "ENOTFOUND" }),
        4 => return Err(DnsFailure { code: "ENOTIMP" }),
        5 => return Err(DnsFailure { code: "EREFUSED" }),
        _ => return Err(DnsFailure { code: "EBADRESP" }),
    }
    let questions = dns_u16(packet, 4)? as usize;
    let answers = dns_u16(packet, 6)? as usize;
    let mut position = 12;
    for _ in 0..questions {
        let (_, next) = dns_read_name(packet, position)?;
        position = next + 4;
    }
    let mut records = Vec::new();
    for _ in 0..answers {
        let (_, next) = dns_read_name(packet, position)?;
        let code = dns_u16(packet, next)?;
        let ttl = dns_u32(packet, next + 4)?;
        let length = dns_u16(packet, next + 8)? as usize;
        let start = next + 10;
        let end = start + length;
        if end > packet.len() {
            return Err(DnsFailure { code: "EBADRESP" });
        }
        if let Some(record) = dns_record(packet, code, start, end, ttl)? {
            records.push(record);
        }
        position = end;
    }
    Ok(records)
}

fn dns_exchange(
    server: std::net::SocketAddr,
    query: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, DnsFailure> {
    let timed_out = |error: &io::Error| {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        )
    };
    let bind: std::net::SocketAddr = if server.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let socket = UdpSocket::bind(bind).map_err(|_| DnsFailure {
        code: "ECONNREFUSED",
    })?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|_| DnsFailure {
            code: "ECONNREFUSED",
        })?;
    socket.connect(server).map_err(|_| DnsFailure {
        code: "ECONNREFUSED",
    })?;
    socket.send(query).map_err(|_| DnsFailure {
        code: "ECONNREFUSED",
    })?;
    let mut buffer = vec![0u8; 4096];
    let received = match socket.recv(&mut buffer) {
        Ok(length) => length,
        Err(error) if timed_out(&error) => return Err(DnsFailure { code: "ETIMEOUT" }),
        Err(_) => {
            return Err(DnsFailure {
                code: "ECONNREFUSED",
            })
        }
    };
    buffer.truncate(received);
    if buffer.len() >= 4 && buffer[2] & 0x02 != 0 {
        // Truncated: ask again over TCP (length-prefixed).
        let mut stream = TcpStream::connect_timeout(&server, timeout).map_err(|_| DnsFailure {
            code: "ECONNREFUSED",
        })?;
        let _ = stream.set_read_timeout(Some(timeout));
        let _ = stream.set_write_timeout(Some(timeout));
        let mut framed = (query.len() as u16).to_be_bytes().to_vec();
        framed.extend_from_slice(query);
        stream.write_all(&framed).map_err(|_| DnsFailure {
            code: "ECONNREFUSED",
        })?;
        let mut length = [0u8; 2];
        stream.read_exact(&mut length).map_err(|error| DnsFailure {
            code: if timed_out(&error) {
                "ETIMEOUT"
            } else {
                "ECONNREFUSED"
            },
        })?;
        let mut body = vec![0u8; u16::from_be_bytes(length) as usize];
        stream.read_exact(&mut body).map_err(|error| DnsFailure {
            code: if timed_out(&error) {
                "ETIMEOUT"
            } else {
                "ECONNREFUSED"
            },
        })?;
        return Ok(body);
    }
    Ok(buffer)
}

fn dns_query_id() -> u16 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as u16
}

/// Resolves `name` for `rrtype` through the configured servers (tried in order, `tries` attempts each); the JSON result is
/// `{"ok":true,"records":[...]}` or `{"ok":false,"code":"ENODATA"}`.
fn dns_query_json(
    name: &str,
    rrtype: &str,
    servers: &[std::net::SocketAddr],
    timeout_ms: u64,
    tries: u32,
) -> String {
    let outcome = (|| -> Result<Vec<serde_json::Value>, DnsFailure> {
        let qtype = dns_type_code(rrtype).ok_or(DnsFailure { code: "EBADQUERY" })?;
        if servers.is_empty() {
            return Err(DnsFailure {
                code: "ECONNREFUSED",
            });
        }
        let timeout = Duration::from_millis(timeout_ms.max(1));
        let mut last = DnsFailure { code: "ETIMEOUT" };
        for _ in 0..tries.max(1) {
            for server in servers {
                let id = dns_query_id();
                let query = dns_build_query(id, name, qtype)?;
                match dns_exchange(*server, &query, timeout) {
                    Ok(packet) => {
                        let records = dns_parse_response(&packet, id);
                        // A definitive answer (data or an authoritative error) ends the search; transport failures try the next server.
                        match &records {
                            Err(failure)
                                if failure.code == "ESERVFAIL" || failure.code == "EBADRESP" =>
                            {
                                last = DnsFailure { code: failure.code }
                            }
                            _ => return records,
                        }
                    }
                    Err(failure) => last = failure,
                }
            }
        }
        Err(last)
    })();
    match outcome {
        Ok(records) if records.is_empty() => {
            serde_json::json!({ "ok": false, "code": "ENODATA" }).to_string()
        }
        Ok(records) => serde_json::json!({ "ok": true, "records": records }).to_string(),
        Err(failure) => serde_json::json!({ "ok": false, "code": failure.code }).to_string(),
    }
}

/// `host`, `host:port`, `[v6]` or `[v6]:port` -> socket address (default port 53).
fn dns_parse_server(text: &str) -> Option<std::net::SocketAddr> {
    if let Ok(address) = text.parse::<std::net::SocketAddr>() {
        return Some(address);
    }
    text.parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| std::net::SocketAddr::new(ip, 53))
}

fn dns_query_start(
    name: String,
    rrtype: String,
    servers_json: String,
    timeout_ms: u32,
    tries: u32,
) -> u32 {
    let servers: Vec<std::net::SocketAddr> = serde_json::from_str::<Vec<String>>(&servers_json)
        .unwrap_or_default()
        .iter()
        .filter_map(|text| dns_parse_server(text))
        .collect();
    async_op_start(move || {
        let json = dns_query_json(&name, &rrtype, &servers, u64::from(timeout_ms), tries);
        Box::new(move || json)
    })
}

/// The nameservers from /etc/resolv.conf as Node prints them (`ip` or `ip:port`, IPv6 bracketed with a port).
fn dns_default_servers() -> String {
    let mut servers = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/etc/resolv.conf") {
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            if fields.next() == Some("nameserver") {
                if let Some(address) = fields.next() {
                    let address = address.split('%').next().unwrap_or(address);
                    if address.parse::<std::net::IpAddr>().is_ok() {
                        servers.push(address.to_string());
                    }
                }
            }
        }
    }
    if servers.is_empty() {
        servers.push("127.0.0.1".to_string());
    }
    serde_json::Value::Array(servers.into_iter().map(serde_json::Value::String).collect())
        .to_string()
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    macro_rules! def {
        ($name:expr, $f:expr) => {{
            let f = it.native($f);
            g.declare($name, f, true);
        }};
    }
    def!("__thaw_dns_lookup", |it, _, a| Ok(Value::str(
        &dns_lookup_json(s(it, a, 0))
    )));
    def!("__thaw_dns_lookup_start", |it, _, a| Ok(Value::Number(
        f64::from(dns_lookup_start(s(it, a, 0)))
    )));
    def!("__thaw_dns_query_start", |it, _, a| Ok(Value::Number(
        f64::from(dns_query_start(
            s(it, a, 0),
            s(it, a, 1),
            s(it, a, 2),
            n(it, a, 3),
            n(it, a, 4),
        ))
    )));
    def!("__thaw_dns_default_servers", |_, _, _| Ok(Value::str(
        &dns_default_servers()
    )));
}

#[cfg(test)]
mod dns_client_tests {
    use super::*;

    fn response(id: u16, name: &str, code: u16, rdata: &[u8], ttl: u32) -> Vec<u8> {
        let mut packet = id.to_be_bytes().to_vec();
        packet.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
        dns_encode_name(name, &mut packet).ok().unwrap();
        packet.extend_from_slice(&code.to_be_bytes());
        packet.extend_from_slice(&1u16.to_be_bytes());
        packet.extend_from_slice(&[0xC0, 0x0C]);
        packet.extend_from_slice(&code.to_be_bytes());
        packet.extend_from_slice(&1u16.to_be_bytes());
        packet.extend_from_slice(&ttl.to_be_bytes());
        packet.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        packet.extend_from_slice(rdata);
        packet
    }

    #[test]
    fn parses_mx_txt_and_a_records_and_error_codes() {
        let mut mx = 10u16.to_be_bytes().to_vec();
        dns_encode_name("mail.example.test", &mut mx).ok().unwrap();
        let records = dns_parse_response(&response(7, "example.test", 15, &mx, 60), 7)
            .ok()
            .unwrap();
        assert_eq!(
            records,
            vec![
                serde_json::json!({ "type": "MX", "priority": 10, "exchange": "mail.example.test" })
            ]
        );
        let txt = [3, b'a', b'b', b'c', 2, b'd', b'e'];
        let records = dns_parse_response(&response(8, "example.test", 16, &txt, 60), 8)
            .ok()
            .unwrap();
        assert_eq!(
            records,
            vec![serde_json::json!({ "type": "TXT", "entries": ["abc", "de"] })]
        );
        let records = dns_parse_response(&response(9, "example.test", 1, &[192, 0, 2, 7], 30), 9)
            .ok()
            .unwrap();
        assert_eq!(
            records,
            vec![serde_json::json!({ "type": "A", "address": "192.0.2.7", "ttl": 30 })]
        );
        let mut nxdomain = response(10, "example.test", 1, &[1, 2, 3, 4], 1);
        nxdomain[3] = 0x83;
        assert_eq!(
            dns_parse_response(&nxdomain, 10)
                .err()
                .map(|failure| failure.code),
            Some("ENOTFOUND")
        );
        assert_eq!(
            dns_parse_response(&response(1, "x.test", 1, &[1, 2, 3, 4], 1), 2)
                .err()
                .map(|failure| failure.code),
            Some("EBADRESP")
        );
    }

    #[test]
    fn queries_a_udp_server_and_reports_no_data() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut buffer = [0u8; 512];
            let (length, client) = server.recv_from(&mut buffer).unwrap();
            let id = u16::from_be_bytes([buffer[0], buffer[1]]);
            let mut reply = response(id, "example.test", 1, &[192, 0, 2, 9], 5);
            let _ = length;
            server.send_to(&reply, client).unwrap();
            let (_, client) = server.recv_from(&mut buffer).unwrap();
            let id = u16::from_be_bytes([buffer[0], buffer[1]]);
            // NOERROR with no answers
            reply = id.to_be_bytes().to_vec();
            reply.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0]);
            dns_encode_name("example.test", &mut reply).ok().unwrap();
            reply.extend_from_slice(&[0, 1, 0, 1]);
            server.send_to(&reply, client).unwrap();
        });
        let parsed = |text: String| serde_json::from_str::<serde_json::Value>(&text).unwrap();
        assert_eq!(
            parsed(dns_query_json("example.test", "A", &[address], 1000, 1)),
            serde_json::json!({ "ok": true, "records": [{ "type": "A", "address": "192.0.2.9", "ttl": 5 }] })
        );
        assert_eq!(
            parsed(dns_query_json("example.test", "MX", &[address], 1000, 1)),
            serde_json::json!({ "ok": false, "code": "ENODATA" })
        );
        handle.join().unwrap();
        assert_eq!(
            dns_query_json("bad..name", "A", &[address], 100, 1),
            serde_json::json!({ "ok": false, "code": "EBADNAME" }).to_string()
        );
    }
}
