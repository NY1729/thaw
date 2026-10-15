// Behaviour contracts for the asynchronous host operations behind node:net / node:dns / node:tls.
// Expected JSON was recorded by running the identical body under real Node.js 22, with the same saturated listener
// (`__PORT__` is substituted by the test).

/// A loopback listener whose accept queue is full: the next SYN is dropped, so a client `connect` neither succeeds nor
/// fails for a long time. Returns the port and the sockets that keep the listener saturated.
#[cfg(target_os = "linux")]
fn k_saturated_listener() -> (u16, Vec<std::net::TcpStream>, i32) {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        assert!(fd >= 0);
        let mut address: libc::sockaddr_in = std::mem::zeroed();
        address.sin_family = libc::AF_INET as libc::sa_family_t;
        address.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
        assert_eq!(libc::bind(fd, &address as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as u32), 0);
        assert_eq!(libc::listen(fd, 0), 0);
        let mut length = std::mem::size_of::<libc::sockaddr_in>() as u32;
        assert_eq!(libc::getsockname(fd, &mut address as *mut _ as *mut libc::sockaddr, &mut length), 0);
        let port = u16::from_be(address.sin_port);
        let fillers = vec![std::net::TcpStream::connect(("127.0.0.1", port)).unwrap()];
        // The second connection overflows the queue; its SYN is dropped, so it only times out here.
        let _ = std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().unwrap(), std::time::Duration::from_millis(150));
        (port, fillers, fd)
    }
}

#[cfg(target_os = "linux")]
#[test]
fn k_net_connect_does_not_block_the_event_loop() {
    // expected: node22
    let (port, fillers, fd) = k_saturated_listener();
    let body = r#####"const net = require('node:net');
const out = {};
let ticks = 0; const timer = setInterval(() => ticks++, 20);
const socket = net.connect({ port: __PORT__, host: '127.0.0.1' });
out.connecting = socket.connecting;
await new Promise(r => setTimeout(r, 400));
out.stillConnecting = socket.connecting;
out.timersRan = ticks >= 10;
out.pendingProp = socket.pending;
const closed = new Promise(r => socket.once('close', () => r('closed')));
socket.destroy();
out.close = await closed;
out.destroyed = socket.destroyed;
clearInterval(timer);
return out;
"#####
    .replace("__PORT__", &port.to_string());
    let actual = p2c_run("k_net_connect_nonblocking", &body);
    drop(fillers);
    unsafe { libc::close(fd) };
    assert_eq!(
        actual,
        r#####"{"connecting":true,"stillConnecting":true,"timersRan":true,"pendingProp":true,"close":"closed","destroyed":true}"#####
    );
}

#[test]
fn k_net_writes_and_end_before_connect_are_queued_in_order() {
    // expected: node22
    assert_eq!(
        p2c_run("k_net_writes_and_end_before_connect_are_queued_in_order", r#####"const net = require('node:net');
const out = {};
const received = [];
const server = net.createServer((sock) => {
  let data = '';
  sock.on('data', (d) => { data += d; });
  sock.on('end', () => { received.push(data); sock.end('echo:' + data); });
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const events = [];
const c = net.connect(port, '127.0.0.1');
out.connectingAtStart = c.connecting;
out.writeReturn = c.write('a');
c.write('b', () => events.push('write-cb'));
let reply = '';
c.on('data', (d) => { reply += d; });
c.on('connect', () => events.push('connect'));
c.end('c');
await new Promise((r) => c.on('close', r));
events.push('close');
out.events = events;
out.reply = reply;
out.received = received;
out.bytesWritten = c.bytesWritten;
await new Promise((r) => server.close(r));
return out;
"#####),
        r#####"{"connectingAtStart":true,"writeReturn":true,"events":["connect","write-cb","close"],"reply":"echo:abc","received":["abc"],"bytesWritten":3}"#####
    );
}

#[test]
fn k_net_connect_failure_and_early_destroy() {
    // expected: node22
    assert_eq!(
        p2c_run("k_net_connect_failure_and_early_destroy", r#####"const net = require('node:net');
const out = {};
const probe = net.createServer(); await new Promise((r) => probe.listen(0, '127.0.0.1', r));
const closedPort = probe.address().port; await new Promise((r) => probe.close(r));
const c = net.connect({ port: closedPort, host: '127.0.0.1' });
const events = [];
c.on('connect', () => events.push('connect'));
out.connecting = c.connecting;
const closed = new Promise((r) => c.on('close', r));
const err = await new Promise((r) => c.on('error', r));
events.push('error');
await closed;
events.push('close');
out.events = events;
out.code = err.code; out.syscall = err.syscall; out.address = err.address; out.port = err.port === closedPort;
out.connectingAfter = c.connecting;
// destroy before connect completes: no connect event, one close
const server = net.createServer(() => {}); await new Promise((r) => server.listen(0, '127.0.0.1', r));
const d = net.connect(server.address().port, '127.0.0.1');
const e2 = [];
d.on('connect', () => e2.push('connect')); d.on('close', () => e2.push('close')); d.on('error', () => e2.push('error'));
d.destroy();
await new Promise((r) => setTimeout(r, 100));
out.destroyedEarly = e2;
await new Promise((r) => server.close(r));
return out;
"#####),
        r#####"{"connecting":true,"events":["error","close"],"code":"ECONNREFUSED","syscall":"connect","address":"127.0.0.1","port":true,"connectingAfter":false,"destroyedEarly":["close"]}"#####
    );
}

#[test]
fn k_dns_lookup_completes_asynchronously() {
    // expected: node22
    assert_eq!(
        p2c_run("k_dns_lookup_completes_asynchronously", r#####"const dns = require('node:dns');
const out = {};
let sync = true;
out.cb = await new Promise((r) => { dns.lookup('localhost', { family: 4 }, (e, a, f) => r({ sync, a, f })); sync = false; });
out.all = await dns.promises.lookup('127.0.0.1', { all: true });
let order = [];
const p = dns.promises.lookup('localhost', { family: 4 }).then(() => order.push('lookup'));
setTimeout(() => order.push('timer'), 50);
await p; await new Promise((r) => setTimeout(r, 80));
out.order = order;
return out;
"#####),
        r#####"{"cb":{"sync":false,"a":"127.0.0.1","f":4},"all":[{"address":"127.0.0.1","family":4}],"order":["lookup","timer"]}"#####
    );
}

#[test]
fn k_net_writes_to_a_stalled_peer_do_not_block_the_event_loop() {
    // expected: node22
    assert_eq!(
        p2c_run("k_net_stalled_peer", r#####"const net = require('node:net');
const out = {};
// a peer that stops reading must not block the event loop; draining resumes when it reads again
let resume; const gate = new Promise((r) => { resume = r; });
let total = 0;
const server = net.createServer((sock) => {
  sock.pause();
  gate.then(() => { sock.resume(); });
  sock.on('data', (d) => { total += d.length; });
  sock.on('end', () => sock.end());
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const c = net.connect(server.address().port, '127.0.0.1');
await new Promise((r) => c.once('connect', r));
let ticks = 0; const timer = setInterval(() => ticks++, 10);
const chunk = Buffer.alloc(1024 * 1024, 7);
let backpressure = false; let writes = 0;
for (let i = 0; i < 64; i++) { writes++; if (c.write(chunk) === false) { backpressure = true; break; } }
out.backpressure = backpressure;
const drained = new Promise((r) => c.once('drain', () => r('drain')));
await new Promise((r) => setTimeout(r, 150));
out.timersRanWhileBlocked = ticks >= 5;
resume();
out.drain = await drained;
c.end();
await new Promise((r) => c.on('close', r));
clearInterval(timer);
out.allReceived = total === writes * chunk.length;
await new Promise((r) => server.close(r));
return out;
"#####),
        r#####"{"backpressure":true,"timersRanWhileBlocked":true,"drain":"drain","allReceived":true}"#####
    );
}

/// A TCP server that accepts connections and never answers: a TLS client stays in its handshake.
#[test]
fn k_tls_handshake_does_not_block_the_event_loop() {
    // expected: node22
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = std::thread::spawn(move || {
        let mut streams = Vec::new();
        listener.set_nonblocking(true).unwrap();
        for _ in 0..300 {
            if let Ok((stream, _)) = listener.accept() {
                streams.push(stream);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        streams
    });
    let body = r#####"const tls = require('node:tls');
const out = {};
let ticks = 0; const timer = setInterval(() => ticks++, 20);
const socket = tls.connect({ port: __PORT__, host: '127.0.0.1', rejectUnauthorized: false });
socket.on('error', () => {});
out.connecting = socket.connecting;
await new Promise((r) => setTimeout(r, 400));
out.timersRan = ticks >= 10;
const closed = new Promise((r) => socket.once('close', () => r('closed')));
socket.write('queued before the handshake');
socket.destroy();
out.close = await closed;
out.destroyed = socket.destroyed;
clearInterval(timer);
return out;
"#####
    .replace("__PORT__", &port.to_string());
    let actual = p2c_run("k_tls_handshake_nonblocking", &body);
    let _ = held.join();
    assert_eq!(
        actual,
        r#####"{"connecting":true,"timersRan":true,"close":"closed","destroyed":true}"#####
    );
}

/// A client that connects and stays silent must not stall a TLS server's accept loop for other clients.
#[test]
fn k_tls_server_handshake_does_not_stall_accept() {
    // expected: node22
    use std::process::{Command, Stdio};
    let dir = temp_registry("k_tls_server_accept");
    let openssl = |args: &[&str]| {
        assert!(Command::new("openssl").args(args).current_dir(&dir).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success());
    };
    openssl(&["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca2.key", "-out", "ca2.pem", "-days", "2", "-subj", "/CN=K CA", "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign"]);
    openssl(&["req", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=localhost"]);
    fs::write(dir.join("ext.cnf"), "subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n").unwrap();
    openssl(&["x509", "-req", "-in", "server.csr", "-CA", "ca2.pem", "-CAkey", "ca2.key", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-extfile", "ext.cnf"]);
    let body = r#####"const tls = require('node:tls'); const net = require('node:net'); const fs = require('node:fs');
const d = '__TLS_DIR__';
const out = {};
const server = tls.createServer({ key: fs.readFileSync(d + '/server.key'), cert: fs.readFileSync(d + '/server.pem') }, (s) => { s.end('hello'); });
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
// a client that connects and stays silent must not stall the server for other clients
const idle = net.connect(port, '127.0.0.1');
await new Promise((r) => idle.once('connect', r));
const started = Date.now();
const reply = await new Promise((res, rej) => { const c = tls.connect({ port, host: '127.0.0.1', ca: fs.readFileSync(d + '/ca2.pem'), servername: 'localhost' }); out.optionsBeforeConnect = [c.setNoDelay(true) === c, c.setKeepAlive(true, 1000) === c]; let data = ''; c.on('data', (x) => { data += x; }); c.on('end', () => res(data)); c.on('error', rej); });
out.reply = reply;
out.fast = Date.now() - started < 2000;
idle.destroy();
await new Promise((r) => server.close(r));
return out;
"#####
    .replace("__TLS_DIR__", dir.to_str().unwrap());
    let actual = p2c_run("k_tls_server_accept", &body);
    let _ = fs::remove_dir_all(&dir);
    assert_eq!(actual, r#####"{"optionsBeforeConnect":[true,true],"reply":"hello","fast":true}"#####);
}

#[test]
fn k_net_tcp_socket_options_are_accepted_before_and_after_connect() {
    // expected: node22
    assert_eq!(
        p2c_run("k_net_tcp_socket_options_are_accepted_before_and_after_connect", r#####"const net = require('node:net');
const out = {};
const server = net.createServer((sock) => { sock.on('data', (d) => sock.end('re:' + d)); });
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const c = new net.Socket();
out.preConnect = [c.setNoDelay() === c, c.setNoDelay(false) === c, c.setKeepAlive(true, 2000) === c, c.setKeepAlive(false) === c];
c.setNoDelay(true); c.setKeepAlive(true, 1500);
await new Promise((r) => c.connect(server.address().port, '127.0.0.1', r));
out.postConnect = [c.setNoDelay(true) === c, c.setKeepAlive(true, 3000) === c];
let reply = ''; c.on('data', (d) => { reply += d; });
c.write('hi');
await new Promise((r) => c.on('close', r));
out.reply = reply;
await new Promise((r) => server.close(r));
return out;
"#####),
        r#####"{"preConnect":[true,true,true,true],"postConnect":[true,true],"reply":"re:hi"}"#####
    );
}

#[test]
fn k_dgram_ttl_and_multicast_option_validation() {
    // expected: node22
    assert_eq!(
        p2c_run("k_dgram_ttl_and_multicast_option_validation", r#####"const dgram = require('node:dgram');
const out = {};
const r = (f) => { try { return f(); } catch (e) { return e.code; } };
const s = dgram.createSocket({ type: 'udp4', reuseAddr: true });
out.beforeBind = [r(() => s.setTTL(64)), r(() => s.setMulticastTTL(2))];
await new Promise((res) => s.bind(0, '0.0.0.0', res));
out.ttl = [r(() => s.setTTL(64)), r(() => s.setMulticastTTL(2)), r(() => s.setMulticastLoopback(true)), r(() => s.setMulticastLoopback(false))];
out.badTtl = [r(() => s.setTTL('x')), r(() => s.setTTL(0)), r(() => s.setTTL(300)), r(() => s.setMulticastTTL(300))];
out.missing = r(() => s.addMembership());
out.invalid = r(() => s.addMembership('not-an-ip'));
out.notMulticast = r(() => s.addMembership('10.0.0.1'));
out.iface = r(() => s.setMulticastInterface('127.0.0.1'));
s.close();
return out;
"#####),
        r#####"{"beforeBind":["EBADF","EBADF"],"ttl":[64,2,true,false],"badTtl":["ERR_INVALID_ARG_TYPE","EINVAL","EINVAL","EINVAL"],"missing":"ERR_MISSING_ARGS","invalid":"EINVAL","notMulticast":"EINVAL"}"#####
    );
}

/// Loopback multicast round trip; skipped on hosts whose loopback cannot join a group.
#[test]
fn k_dgram_multicast_membership_delivers_group_datagrams() {
    // expected: node22
    let actual = p2c_run("k_dgram_multicast", r#####"const dgram = require('node:dgram');
const out = {};
const group = '239.255.77.77';
const recv = dgram.createSocket({ type: 'udp4', reuseAddr: true });
await new Promise((r) => recv.bind(0, '0.0.0.0', r));
const port = recv.address().port;
try { recv.addMembership(group, '127.0.0.1'); out.joined = true; } catch (e) { out.joined = e.code; }
const got = []; recv.on('message', (m) => got.push(String(m)));
const send = dgram.createSocket('udp4');
await new Promise((r) => send.bind(0, '127.0.0.1', r));
try { send.setMulticastInterface('127.0.0.1'); send.setMulticastLoopback(true); } catch (e) { out.sendSetup = e.code; }
await new Promise((r) => send.send('hello-group', port, group, (e) => { out.sendErr = e ? e.code : null; r(); }));
await new Promise((r) => setTimeout(r, 200));
out.got = got;
try { recv.dropMembership(group, '127.0.0.1'); out.dropped = true; } catch (e) { out.dropped = e.code; }
recv.close(); send.close();
return out;
"#####);
    if !actual.contains(r#""joined":true"#) {
        eprintln!("skipping: this host cannot join a loopback multicast group: {actual}");
        return;
    }
    assert_eq!(actual, r#####"{"joined":true,"sendErr":null,"got":["hello-group"],"dropped":true}"#####);
}

/// A UDP DNS server that replays recorded answers: `dns_responses.json` maps `name|TYPE` to the response (after the 2-byte id) a
/// reference server produced; the id of each query is put back in front. Serves until dropped.
struct KDnsServer {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl KDnsServer {
    fn start() -> KDnsServer {
        let responses: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("dns_responses.json")).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_millis(50))).unwrap();
        let port = socket.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let names = [(1u16, "A"), (2, "NS"), (5, "CNAME"), (6, "SOA"), (12, "PTR"), (15, "MX"), (16, "TXT"), (28, "AAAA"), (33, "SRV"), (35, "NAPTR"), (255, "ANY"), (257, "CAA")];
            let mut buffer = [0u8; 512];
            while !thread_stop.load(std::sync::atomic::Ordering::Relaxed) {
                let Ok((length, client)) = socket.recv_from(&mut buffer) else { continue };
                let query = &buffer[..length];
                let mut position = 12;
                let mut labels = Vec::new();
                while position < length && query[position] != 0 {
                    let size = query[position] as usize;
                    labels.push(String::from_utf8_lossy(&query[position + 1..position + 1 + size]).to_lowercase());
                    position += 1 + size;
                }
                let code = u16::from_be_bytes([query[position + 1], query[position + 2]]);
                let type_name = names.iter().find(|(value, _)| *value == code).map(|(_, label)| *label).unwrap_or("?");
                let Some(hex) = responses.get(&format!("{}|{}", labels.join("."), type_name)) else { continue };
                let mut reply = query[..2].to_vec();
                reply.extend((0..hex.len() / 2).map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap()));
                let _ = socket.send_to(&reply, client);
            }
        });
        KDnsServer { port, stop, handle: Some(handle) }
    }
}

impl Drop for KDnsServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn k_dns_resolver_record_types_and_errors() {
    // expected: node22 (c-ares) against the same recorded answers
    let server = KDnsServer::start();
    let body = r#####"const dns = require('node:dns');
const r = new dns.promises.Resolver();
r.setServers(['127.0.0.1:__DNS_PORT__']);
const out = {};
const t = async (key, fn) => { try { out[key] = await fn(); } catch (e) { out[key] = { code: e.code, syscall: e.syscall, hostname: e.hostname, message: e.message }; } };
await t('a', () => r.resolve4('a.test'));
await t('aTtl', () => r.resolve4('a.test', { ttl: true }));
await t('aaaa', () => r.resolve6('a.test'));
await t('aaaaTtl', () => r.resolve6('a.test', { ttl: true }));
await t('mx', () => r.resolveMx('mx.test'));
await t('txt', () => r.resolveTxt('txt.test'));
await t('srv', () => r.resolveSrv('srv.test'));
await t('ns', () => r.resolveNs('ns.test'));
await t('cname', () => r.resolveCname('cname.test'));
await t('soa', () => r.resolveSoa('soa.test'));
await t('naptr', () => r.resolveNaptr('naptr.test'));
await t('caa', () => r.resolveCaa('caa.test'));
await t('any', () => r.resolveAny('any.test'));
await t('resolveMxGeneric', () => r.resolve('mx.test', 'MX'));
await t('reverse4', () => r.reverse('192.0.2.1'));
await t('nodata', () => r.resolveMx('a.test'));
await t('nxdomain', () => r.resolve4('missing.nx.test'));
await t('servfail', () => r.resolveTxt('servfail.test'));
await t('refused', () => r.resolveNs('refused.test'));
await t('badType', () => r.resolve('a.test', 'BOGUS'));
out.servers = r.getServers().map((s) => s.replace(':__DNS_PORT__', ':PORT'));
return out;
"#####.replace("__DNS_PORT__", &server.port.to_string());
    assert_eq!(
        p2c_run("k_dns_records", &body),
        r#####"{"a":["192.0.2.1","192.0.2.2"],"aTtl":[{"address":"192.0.2.1","ttl":60},{"address":"192.0.2.2","ttl":120}],"aaaa":["2001:db8::1"],"aaaaTtl":[{"address":"2001:db8::1","ttl":30}],"mx":[{"exchange":"mail1.mx.test","priority":10},{"exchange":"mail2.mx.test","priority":20}],"txt":[["v=spf1 -all"],["part-one","part-two"]],"srv":[{"name":"sip.srv.test","port":5060,"priority":5,"weight":10}],"ns":["ns1.ns.test","ns2.ns.test"],"cname":["target.cname.test"],"soa":{"nsname":"ns1.soa.test","hostmaster":"hostmaster.soa.test","serial":2024010101,"refresh":7200,"retry":3600,"expire":1209600,"minttl":300},"naptr":[{"flags":"s","service":"SIP+D2U","regexp":"","replacement":"_sip._udp.naptr.test","order":100,"preference":50}],"caa":[{"critical":0,"issue":"ca.example.net"},{"critical":128,"iodef":"mailto:sec@caa.test"}],"any":[{"address":"192.0.2.50","ttl":60,"type":"A"},{"exchange":"mx.any.test","priority":5,"type":"MX"},{"entries":["any-text"],"type":"TXT"}],"resolveMxGeneric":[{"exchange":"mail1.mx.test","priority":10},{"exchange":"mail2.mx.test","priority":20}],"reverse4":["host.ptr.test"],"nodata":{"code":"ENODATA","syscall":"queryMx","hostname":"a.test","message":"queryMx ENODATA a.test"},"nxdomain":{"code":"ENOTFOUND","syscall":"queryA","hostname":"missing.nx.test","message":"queryA ENOTFOUND missing.nx.test"},"servfail":{"code":"ESERVFAIL","syscall":"queryTxt","hostname":"servfail.test","message":"queryTxt ESERVFAIL servfail.test"},"refused":{"code":"EREFUSED","syscall":"queryNs","hostname":"refused.test","message":"queryNs EREFUSED refused.test"},"badType":{"code":"ERR_INVALID_ARG_VALUE","message":"The argument 'rrtype' is invalid. Received 'BOGUS'"},"servers":["127.0.0.1:PORT"]}"#####
    );
}

#[test]
fn k_dns_resolver_servers_reverse_and_cancel() {
    // expected: node22 (c-ares) against the same recorded answers
    let server = KDnsServer::start();
    let body = r#####"const dns = require('node:dns');
const out = {};
const r = new dns.Resolver();
out.defaultsAreArray = Array.isArray(r.getServers());
r.setServers(['127.0.0.1:__DNS_PORT__', '[::1]:5353', '8.8.8.8', '2001:db8::1', '2001:db8::2:5300']);
out.servers = r.getServers().map((s) => s.replace('__DNS_PORT__', 'PORT'));
const bad = (v) => { try { r.setServers(v); return 'no throw'; } catch (e) { return e.code; } };
out.badServers = [bad(['not an ip']), bad('x'), bad([1]), bad(['1.2.3.4:abc'])];
out.afterBad = r.getServers().length;
r.setServers(['127.0.0.1:__DNS_PORT__']);
const q = (fn) => new Promise((res) => fn((e, v) => res(e ? { code: e.code, syscall: e.syscall } : v)));
out.cbMx = await q((cb) => r.resolveMx('mx.test', cb));
out.cbResolveTxt = await q((cb) => r.resolve('txt.test', 'TXT', cb));
out.cbReverse = await q((cb) => r.reverse('192.0.2.1', cb));
out.reverse6 = await q((cb) => r.reverse('2001:db8::1', cb));
out.reverseBad = await q((cb) => { try { r.reverse('nope', cb); } catch (e) { cb(e); } });
out.anyTypes = (await q((cb) => r.resolveAny('any.test', cb))).map((x) => x.type);
// cancel: a query to a silent server is cancelled
const silent = new dns.Resolver({ timeout: 4000, tries: 1 }); silent.setServers(['127.0.0.1:9']);
const pending = q((cb) => silent.resolve4('a.test', cb));
silent.cancel();
out.cancelled = await pending;
return out;
"#####.replace("__DNS_PORT__", &server.port.to_string());
    assert_eq!(
        p2c_run("k_dns_servers", &body),
        r#####"{"defaultsAreArray":true,"servers":["127.0.0.1:PORT","[::1]:5353","8.8.8.8","2001:db8::1","2001:db8::2:5300"],"badServers":["ERR_INVALID_IP_ADDRESS","ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE","ERR_INVALID_IP_ADDRESS"],"afterBad":5,"cbMx":[{"exchange":"mail1.mx.test","priority":10},{"exchange":"mail2.mx.test","priority":20}],"cbResolveTxt":[["v=spf1 -all"],["part-one","part-two"]],"cbReverse":["host.ptr.test"],"reverse6":["v6host.ptr.test"],"reverseBad":{"code":"EINVAL","syscall":"getHostByAddr"},"anyTypes":["A","MX","TXT"],"cancelled":{"code":"ECANCELLED","syscall":"queryA"}}"#####
    );
}
