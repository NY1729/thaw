fn net_connect(host: &str, port: u16) -> String {
    match TcpStream::connect((host, port)) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            // The connected peer, not the requested host name: `ok:<handle>:<ip>:<port>` (IPv6 contains colons, so the port is the last field).
            let peer = stream
                .peer_addr()
                .map(|peer| (peer.ip().to_string(), peer.port()))
                .unwrap_or_else(|_| (host.to_string(), port));
            NET_STREAMS.with(|streams| {
                let mut streams = streams.borrow_mut();
                let handle = streams.0;
                streams.0 = streams.0.wrapping_add(1).max(1);
                streams.1.insert(handle, stream);
                format!("ok:{handle}:{}:{}", peer.0, peer.1)
            })
        }
        Err(error) => format!("err:{error}"),
    }
}

fn net_write(handle: u32, value: &[u8]) -> String {
    NET_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        match streams.1.get_mut(&handle) {
            Some(stream) => stream
                .write_all(value)
                .map(|_| "ok".to_string())
                .unwrap_or_else(|error| format!("err:{error}")),
            None => "err:socket is closed".to_string(),
        }
    })
}

fn net_finish(handle: u32) -> String {
    NET_STREAMS.with(|streams| {
        let Some(mut stream) = streams.borrow_mut().1.remove(&handle) else {
            return "err:socket is closed".to_string();
        };
        if let Err(error) = stream.shutdown(Shutdown::Write) {
            return format!("err:{error}");
        }
        let mut value = Vec::new();
        match stream.read_to_end(&mut value) {
            Ok(_) => format!("ok:{}", hex_encode(&value)),
            // Bytes received before the failure still belong to the reader.
            Err(error) if !value.is_empty() => format!("data-err:{}:{error}", hex_encode(&value)),
            Err(error) => format!("err:{error}"),
        }
    })
}

fn net_shutdown_write(handle: u32) -> String {
    NET_STREAMS.with(|streams| {
        let streams = streams.borrow();
        let Some(stream) = streams.1.get(&handle) else {
            return "err:socket is closed".to_string();
        };
        stream
            .shutdown(Shutdown::Write)
            .map(|_| "ok".to_string())
            .unwrap_or_else(|error| format!("err:{error}"))
    })
}

fn net_poll_read(handle: u32) -> String {
    net_poll_read_impl(handle, false)
}

fn net_poll_read_impl(handle: u32, retain_on_eof: bool) -> String {
    NET_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let mut value = vec![0u8; 16 * 1024];
        let outcome = {
            let Some(stream) = streams.1.get_mut(&handle) else {
                return "err:socket is closed".to_string();
            };
            match stream.set_nonblocking(true) {
                Err(error) => Err(error),
                Ok(()) => {
                    let read = stream.read(&mut value);
                    let restore = stream.set_nonblocking(false);
                    match read {
                        Ok(length) => restore.map(|_| Some(length)),
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            restore.map(|_| None)
                        }
                        Err(error) => Err(error),
                    }
                }
            }
        };
        match outcome {
            Ok(Some(0)) => {
                if !retain_on_eof {
                    streams.1.remove(&handle);
                }
                "eof".to_string()
            }
            Ok(Some(length)) => {
                value.truncate(length);
                format!("ok:{}", hex_encode(&value))
            }
            Ok(None) => "pending".to_string(),
            Err(error) => {
                streams.1.remove(&handle);
                format!("err:{error}")
            }
        }
    })
}

fn net_destroy(handle: u32) {
    NET_STREAMS.with(|streams| {
        if let Some(stream) = streams.borrow_mut().1.remove(&handle) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    });
}

fn net_listen(host: &str, port: u16) -> String {
    match TcpListener::bind((host, port)) {
        Ok(listener) => {
            let actual_port = listener
                .local_addr()
                .map(|value| value.port())
                .unwrap_or(port);
            NET_LISTENERS.with(|listeners| {
                let mut listeners = listeners.borrow_mut();
                let handle = listeners.0;
                listeners.0 = listeners.0.wrapping_add(1).max(1);
                listeners.1.insert(handle, listener);
                format!("ok:{handle}:{actual_port}")
            })
        }
        Err(error) => format!("err:{error}"),
    }
}

fn net_accept_impl(handle: u32, nonblocking: bool) -> String {
    NET_LISTENERS.with(|listeners| {
        let listeners = listeners.borrow();
        let Some(listener) = listeners.1.get(&handle) else {
            return "err:server is closed".to_string();
        };
        if let Err(error) = listener.set_nonblocking(nonblocking) {
            return format!("err:{error}");
        }
        let accepted = listener.accept();
        if nonblocking {
            let _ = listener.set_nonblocking(false);
        }
        match accepted {
            Ok((stream, peer)) => NET_STREAMS.with(|streams| {
                let mut streams = streams.borrow_mut();
                let stream_handle = streams.0;
                streams.0 = streams.0.wrapping_add(1).max(1);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                streams.1.insert(stream_handle, stream);
                format!("ok:{stream_handle}:{}:{}", peer.ip(), peer.port())
            }),
            Err(error) if nonblocking && error.kind() == io::ErrorKind::WouldBlock => {
                "err:pending".to_string()
            }
            Err(error) => format!("err:{error}"),
        }
    })
}

fn net_accept(handle: u32) -> String {
    net_accept_impl(handle, false)
}

fn net_poll_accept(handle: u32) -> String {
    net_accept_impl(handle, true)
}

fn net_read_all(handle: u32) -> String {
    NET_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(stream) = streams.1.get_mut(&handle) else {
            return "err:socket is closed".to_string();
        };
        let mut value = vec![0; 16 * 1024];
        match stream.read(&mut value) {
            Ok(length) => {
                value.truncate(length);
                format!("ok:{}", hex_encode(&value))
            }
            Err(error) => format!("err:{error}"),
        }
    })
}

fn net_close_listener(handle: u32) {
    NET_LISTENERS.with(|listeners| {
        listeners.borrow_mut().1.remove(&handle);
    });
}

fn udp_error(error: io::Error) -> String {
    let code = match error.kind() {
        io::ErrorKind::AddrInUse => "EADDRINUSE",
        io::ErrorKind::AddrNotAvailable => "EADDRNOTAVAIL",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::NotFound => "ENOENT",
        _ => "EUNKNOWN",
    };
    #[cfg(unix)]
    let code = match error.raw_os_error() {
        Some(libc::ENETUNREACH) => "ENETUNREACH",
        Some(libc::EHOSTUNREACH) => "EHOSTUNREACH",
        Some(libc::EAFNOSUPPORT) => "EAFNOSUPPORT",
        Some(libc::EBADF) => "EBADF",
        _ => code,
    };
    format!("err|{code}|{error}")
}

fn udp_bind(host: &str, port: u16) -> String {
    match UdpSocket::bind((host, port)) {
        Ok(socket) => {
            let address = match socket.local_addr() {
                Ok(address) => address,
                Err(error) => return udp_error(error),
            };
            if let Err(error) = socket.set_nonblocking(true) { return udp_error(error); }
            UDP_SOCKETS.with(|sockets| {
                let mut sockets = sockets.borrow_mut();
                let handle = sockets.0;
                sockets.0 = sockets.0.wrapping_add(1).max(1);
                sockets.1.insert(handle, socket);
                format!("ok|{handle}|{}|{}", address.ip(), address.port())
            })
        }
        Err(error) => udp_error(error),
    }
}

fn udp_send(handle: u32, value: &[u8], host: &str, port: u16) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return "err|EBADF|socket is closed".to_string();
        };
        (if socket.peer_addr().is_ok() { socket.send(value) } else { socket.send_to(value, (host, port)) })
            .map(|written| format!("ok|{written}"))
            .unwrap_or_else(udp_error)
    })
}

fn udp_receive(handle: u32) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return "err|EBADF|socket is closed".to_string();
        };
        let mut value = vec![0u8; 65_536];
        match socket.recv_from(&mut value) {
            Ok((length, peer)) => {
                value.truncate(length);
                format!(
                    "ok|{}|{}|{}|{}",
                    hex_encode(&value),
                    peer.ip(),
                    peer.port(),
                    length
                )
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => "pending".to_string(),
            Err(error) => udp_error(error),
        }
    })
}

fn udp_connect(handle: u32, host: &str, port: u16) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else { return "err|EBADF|socket is closed".to_string(); };
        socket.connect((host, port))
            .map(|()| "ok".to_string())
            .unwrap_or_else(udp_error)
    })
}

fn udp_set_broadcast(handle: u32, enabled: bool) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else { return "err|EBADF|socket is closed".to_string(); };
        socket.set_broadcast(enabled)
            .map(|()| "ok".to_string())
            .unwrap_or_else(udp_error)
    })
}

#[cfg(unix)]
fn udp_disconnect(handle: u32) -> String {
    use std::os::fd::AsRawFd;
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else { return "err|EBADF|socket is closed".to_string(); };
        let mut address: libc::sockaddr = unsafe { std::mem::zeroed() };
        address.sa_family = libc::AF_UNSPEC as libc::sa_family_t;
        let result = unsafe { libc::connect(socket.as_raw_fd(), &address, std::mem::size_of::<libc::sockaddr>() as libc::socklen_t) };
        if result == 0 { "ok".to_string() } else { udp_error(io::Error::last_os_error()) }
    })
}

#[cfg(not(unix))]
fn udp_disconnect(_handle: u32) -> String {
    "err|ENOTSUP|UDP disconnect is unsupported on this platform".to_string()
}

fn udp_close(handle: u32) {
    UDP_SOCKETS.with(|sockets| {
        sockets.borrow_mut().1.remove(&handle);
    });
}

#[cfg(feature = "tls")]
mod tls_host {
use super::*;

fn decode_pem_blocks(bytes: &[u8], label: &str) -> Result<Vec<Vec<u8>>, String> {
    let begin = format!("-----BEGIN {label}-----");
    let end_marker = format!("-----END {label}-----");
    let text = String::from_utf8_lossy(bytes);
    let mut remaining = text.as_ref();
    let mut decoded = Vec::new();
    while let Some(start) = remaining.find(&begin) {
        remaining = &remaining[start + begin.len()..];
        let Some(end) = remaining.find(&end_marker) else {
            return Err(format!("unterminated PEM {label}"));
        };
        let encoded = remaining[..end]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        decoded.push(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| error.to_string())?,
        );
        remaining = &remaining[end + end_marker.len()..];
    }
    Ok(decoded)
}

fn decode_certificates(spec: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    let bytes = hex_decode(spec);
    let values = if bytes.starts_with(b"-----BEGIN CERTIFICATE-----") {
        decode_pem_blocks(&bytes, "CERTIFICATE")?
    } else {
        vec![bytes]
    };
    Ok(values.into_iter().map(CertificateDer::from).collect())
}

fn decode_private_key(spec: &str) -> Result<PrivateKeyDer<'static>, String> {
    let bytes = hex_decode(spec);
    if bytes.starts_with(b"-----BEGIN PRIVATE KEY-----") {
        let value = decode_pem_blocks(&bytes, "PRIVATE KEY")?
            .into_iter()
            .next()
            .ok_or_else(|| "missing PEM private key".to_string())?;
        Ok(PrivatePkcs8KeyDer::from(value).into())
    } else if bytes.starts_with(b"-----BEGIN RSA PRIVATE KEY-----") {
        let value = decode_pem_blocks(&bytes, "RSA PRIVATE KEY")?
            .into_iter()
            .next()
            .ok_or_else(|| "missing PEM RSA private key".to_string())?;
        Ok(PrivatePkcs1KeyDer::from(value).into())
    } else {
        Ok(PrivatePkcs8KeyDer::from(bytes).into())
    }
}

fn decode_alpn_protocols(spec: &str) -> Vec<Vec<u8>> {
    spec.split(',')
        .filter(|value| !value.is_empty())
        .map(hex_decode)
        .collect()
}

pub(super) struct TlsClientOptions<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) server_name: &'a str,
    pub(super) ca_spec: Option<&'a str>,
    pub(super) cert_spec: &'a str,
    pub(super) key_spec: &'a str,
    pub(super) alpn_spec: &'a str,
    pub(super) report_alpn: bool,
    pub(super) reject_unauthorized: bool,
    pub(super) defer_name_verification: bool,
}

fn server_client_auth_status(request_cert: bool, peer_present: bool, verifier_accepted: bool) -> (bool, Option<&'static str>) {
    if !request_cert { return (false, Some("TLS client authentication was not requested")); }
    if !peer_present { return (false, Some("CERT_NOT_PROVIDED")); }
    if verifier_accepted { (true, None) } else { (false, Some("CERT_REJECTED")) }
}

pub(super) fn take_tls_stream_handle(next: &mut u32) -> Option<u32> {
    let handle = *next;
    *next = handle.checked_add(2)?;
    Some(handle)
}

fn tls_client_roots(ca_spec: Option<&str>) -> Result<RootCertStore, String> {
    match ca_spec {
        None => Ok(RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned())),
        Some(spec) => tls_ca_roots(Some(spec)),
    }
}

fn tls_ca_roots(ca_spec: Option<&str>) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    let Some(spec) = ca_spec else { return Ok(roots) };
    if spec.is_empty() { return Ok(roots) }
    for encoded in spec.split(',') {
        if encoded.is_empty() || encoded.len() % 2 != 0 || !encoded.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid CA encoding".to_string());
        }
        let bytes = hex_decode(encoded);
        let certificates = decode_ca_certificates(&bytes)?;
        for certificate in certificates { roots.add(certificate).map_err(|error| error.to_string())?; }
    }
    Ok(roots)
}

fn decode_ca_certificates(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    if bytes.is_empty() { return Err("empty CA certificate".to_string()); }
    let pem = std::str::from_utf8(bytes)
        .ok()
        .map(|text| text.trim_start_matches(|character: char| character.is_ascii_whitespace()))
        .is_some_and(|text| text.starts_with("-----BEGIN "));
    if pem {
        let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
        let mut rest = text.trim_matches(|character: char| character.is_ascii_whitespace());
        let mut certs = Vec::new();
        while !rest.is_empty() {
            let prefix = "-----BEGIN CERTIFICATE-----";
            let Some(body_start) = rest.strip_prefix(prefix) else { return Err("invalid CA PEM block".to_string()) };
            let Some((body, tail)) = body_start.split_once("-----END CERTIFICATE-----") else { return Err("unterminated CA PEM block".to_string()) };
            let encoded = body
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect::<String>();
            let der = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| error.to_string())?;
            if der.is_empty() { return Err("empty CA certificate".to_string()); }
            certs.push(CertificateDer::from(der));
            rest = tail.trim_matches(|character: char| character.is_ascii_whitespace());
        }
        return if certs.is_empty() { Err("no CA certificates found".to_string()) } else { Ok(certs) };
    }
    Ok(vec![CertificateDer::from(bytes.to_vec())])
}

pub(super) fn tls_connect(options: TlsClientOptions<'_>) -> String {
    let TlsClientOptions {
        host,
        port,
        server_name,
        ca_spec,
        cert_spec,
        key_spec,
        alpn_spec,
        report_alpn,
        reject_unauthorized,
        defer_name_verification,
    } = options;
    let roots = match tls_client_roots(ca_spec) { Ok(roots) => roots, Err(error) => return format!("err:{error}") };
    let verifier_result = Arc::new(Mutex::new(None));
    let builder = ClientConfig::builder().with_root_certificates(roots.clone());
    let mut local_certificate = None;
    let config = if cert_spec.is_empty() && key_spec.is_empty() {
        builder.with_no_client_auth()
    } else if cert_spec.is_empty() || key_spec.is_empty() {
        return "err:client cert and key must be provided together".to_string();
    } else {
        let certificates = match decode_certificates(cert_spec) {
            Ok(certificates) => certificates,
            Err(error) => return format!("err:{error}"),
        };
        local_certificate = certificates
            .first()
            .map(|certificate| certificate.as_ref().to_vec());
        let key = match decode_private_key(key_spec) {
            Ok(key) => key,
            Err(error) => return format!("err:{error}"),
        };
        match builder.with_client_auth_cert(certificates, key) {
            Ok(config) => config,
            Err(error) => return format!("err:{error}"),
        }
    };
    let mut config = config;
    config.alpn_protocols = decode_alpn_protocols(alpn_spec);
    config.dangerous().set_certificate_verifier(Arc::new(RecordingServerVerifier {
        roots,
        defer_name: defer_name_verification,
        allow_invalid: !reject_unauthorized,
        result: Arc::clone(&verifier_result),
        algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
    }));
    let config = Arc::new(config);
    let name = match ServerName::try_from(server_name.to_string()) {
        Ok(name) => name,
        Err(error) => return format!("err:{error}"),
    };
    let mut socket = match TcpStream::connect((host, port)) {
        Ok(socket) => socket,
        Err(error) => return format!("err:{error}"),
    };
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut connection = match ClientConnection::new(config, name) {
        Ok(connection) => connection,
        Err(error) => return format!("err:{error}"),
    };
    while connection.is_handshaking() {
        if let Err(error) = connection.complete_io(&mut socket) {
            return format!("err:{error}");
        }
    }
    let peer_certificate = connection
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| certificate.as_ref().to_vec());
    TLS_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(handle) = take_tls_stream_handle(&mut streams.0) else {
            return "err:TLS socket handle exhausted".to_string();
        };
        let verification_error = verifier_result.lock().ok().and_then(|result| result.clone());
        let chain_verified = verification_error.is_none();
        TLS_CLIENT_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().insert(
                handle,
                TlsCertificates {
                    peer: peer_certificate,
                    local: local_certificate,
                    authorized: chain_verified,
                    authorization_error: verification_error,
                    chain_verified,
                },
            );
        });
        streams
            .1
            .insert(handle, StreamOwned::new(connection, socket));
        if report_alpn {
            let protocol = streams
                .1
                .get(&handle)
                .and_then(|stream| stream.conn.alpn_protocol())
                .map(hex_encode)
                .unwrap_or_default();
            format!("ok:{handle}:{protocol}")
        } else {
            format!("ok:{handle}")
        }
    })
}

pub(super) struct TlsServerOptions<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) cert_spec: &'a str,
    pub(super) key_spec: &'a str,
    pub(super) ca_spec: Option<&'a str>,
    pub(super) request_cert: bool,
    pub(super) reject_unauthorized: bool,
    pub(super) alpn_spec: &'a str,
}

pub(super) fn tls_server_listen(options: TlsServerOptions<'_>) -> String {
    let TlsServerOptions {
        host,
        port,
        cert_spec,
        key_spec,
        ca_spec,
        request_cert,
        reject_unauthorized,
        alpn_spec,
    } = options;
    let certificates = match decode_certificates(cert_spec) {
        Ok(certificates) => certificates,
        Err(error) => return format!("err:{error}"),
    };
    let local_certificate = certificates
        .first()
        .map(|certificate| certificate.as_ref().to_vec())
        .unwrap_or_default();
    let private_key = match decode_private_key(key_spec) {
        Ok(private_key) => private_key,
        Err(error) => return format!("err:{error}"),
    };
    let roots = match tls_ca_roots(ca_spec) { Ok(roots) => roots, Err(error) => return format!("err:{error}") };
    let builder = ServerConfig::builder();
    let builder = if request_cert {
        let verifier = if reject_unauthorized {
            WebPkiClientVerifier::builder(Arc::new(roots)).build()
        } else {
            WebPkiClientVerifier::builder(Arc::new(roots))
                .allow_unauthenticated()
                .build()
        };
        let verifier = match verifier {
            Ok(verifier) => verifier,
            Err(error) => return format!("err:{error}"),
        };
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    let config = match builder.with_single_cert(certificates, private_key) {
        Ok(mut config) => {
            config.alpn_protocols = decode_alpn_protocols(alpn_spec);
            Arc::new(config)
        }
        Err(error) => return format!("err:{error}"),
    };
    let socket = match TcpListener::bind((host, port)) {
        Ok(socket) => socket,
        Err(error) => return format!("err:{error}"),
    };
    let actual_port = match socket.local_addr() {
        Ok(address) => address.port(),
        Err(error) => return format!("err:{error}"),
    };
    TLS_LISTENERS.with(|listeners| {
        let mut listeners = listeners.borrow_mut();
        let handle = listeners.0;
        listeners.0 = listeners.0.wrapping_add(1).max(1);
        listeners.1.insert(
            handle,
            TlsListener {
                socket,
                config,
                local_certificate,
                request_cert,
            },
        );
        format!("ok:{handle}:{actual_port}")
    })
}

fn tls_server_accept_impl(handle: u32, nonblocking: bool) -> String {
    let accepted = TLS_LISTENERS.with(|listeners| {
        let listeners = listeners.borrow();
        let Some(listener) = listeners.1.get(&handle) else {
            return Err("listener is closed".to_string());
        };
        if let Err(error) = listener.socket.set_nonblocking(nonblocking) {
            return Err(error.to_string());
        }
        let accepted = listener.socket.accept();
        if nonblocking {
            let _ = listener.socket.set_nonblocking(false);
        }
        accepted
            .map(|(socket, peer)| {
                (
                    socket,
                    peer,
                    Arc::clone(&listener.config),
                    listener.local_certificate.clone(),
                    listener.request_cert,
                )
            })
            .map_err(|error| {
                if nonblocking && error.kind() == io::ErrorKind::WouldBlock {
                    "pending".to_string()
                } else {
                    error.to_string()
                }
            })
    });
    let (mut socket, peer, config, local_certificate, request_cert) = match accepted {
        Ok(accepted) => accepted,
        Err(error) => return format!("err:{error}"),
    };
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut connection = match ServerConnection::new(config) {
        Ok(connection) => connection,
        Err(error) => return format!("err:{error}"),
    };
    while connection.is_handshaking() {
        if let Err(error) = connection.complete_io(&mut socket) {
            return format!("err:{error}");
        }
    }
    let peer_certificate = connection
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| certificate.as_ref().to_vec());
    TLS_SERVER_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(stream_handle) = take_tls_stream_handle(&mut streams.0) else {
            return "err:TLS socket handle exhausted".to_string();
        };
        TLS_SERVER_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().insert(
                stream_handle,
                {
                    let peer_present = peer_certificate.as_ref().is_some_and(|certificate| !certificate.is_empty());
                    let (authorized, authorization_error) = server_client_auth_status(request_cert, peer_present, true);
                    TlsCertificates {
                        peer: peer_certificate.clone(),
                        local: Some(local_certificate),
                        authorized,
                        authorization_error: authorization_error.map(str::to_owned),
                        chain_verified: authorized,
                    }
                },
            );
        });
        streams
            .1
            .insert(stream_handle, StreamOwned::new(connection, socket));
        let protocol = streams
            .1
            .get(&stream_handle)
            .and_then(|stream| stream.conn.alpn_protocol())
            .map(hex_encode)
            .unwrap_or_default();
        format!(
            "ok:{stream_handle}:{}:{}:{protocol}",
            peer.ip(),
            peer.port()
        )
    })
}

pub(super) fn tls_server_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, false)
}

pub(super) fn tls_server_poll_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, true)
}

pub(super) fn tls_server_read(handle: u32) -> String {
    TLS_SERVER_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(stream) = streams.1.get_mut(&handle) else {
            return "err:socket is closed".to_string();
        };
        let mut value = Vec::new();
        stream
            .read_to_end(&mut value)
            .map(|_| format!("ok:{}", hex_encode(&value)))
            .unwrap_or_else(|error| format!("err:{error}"))
    })
}

pub(super) fn tls_server_close_listener(handle: u32) {
    TLS_LISTENERS.with(|listeners| {
        listeners.borrow_mut().1.remove(&handle);
    });
}

pub(super) fn tls_write(handle: u32, value: &[u8]) -> String {
    let client_result = TLS_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let stream = streams.1.get_mut(&handle)?;
        Some(
            stream
                .write_all(value)
                .and_then(|_| stream.flush())
                .map(|_| "ok".to_string())
                .unwrap_or_else(|error| format!("err:{error}")),
        )
    });
    client_result.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            let mut streams = streams.borrow_mut();
            let Some(stream) = streams.1.get_mut(&handle) else {
                return "err:socket is closed".to_string();
            };
            stream
                .write_all(value)
                .and_then(|_| stream.flush())
                .map(|_| "ok".to_string())
                .unwrap_or_else(|error| format!("err:{error}"))
        })
    })
}

pub(super) fn tls_finish(handle: u32) -> String {
    let client_result = TLS_STREAMS.with(|streams| {
        let mut stream = streams.borrow_mut().1.remove(&handle)?;
        TLS_CLIENT_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().remove(&handle);
        });
        stream.conn.send_close_notify();
        if let Err(error) = stream.flush() {
            return Some(format!("err:{error}"));
        }
        let mut value = Vec::new();
        Some(match stream.read_to_end(&mut value) {
            Ok(_) => format!("ok:{}", hex_encode(&value)),
            // A peer that closes its TCP connection without first sending
            // a TLS `close_notify` alert is extremely common in the wild
            // (plenty of real HTTP servers do this after answering a
            // `Connection: close` request) -- rustls deliberately surfaces
            // it as `UnexpectedEof` rather than treating it as a clean EOF,
            // to guard against a truncation attack. But by this point the
            // caller already framed the response itself (`Content-Length`/
            // chunked terminator), so whatever bytes were read before the
            // error is exactly the complete response; only a genuine mid-
            // stream failure should still be reported as an error.
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                format!("ok:{}", hex_encode(&value))
            }
            Err(error) => format!("err:{error}"),
        })
    });
    client_result.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            let Some(mut stream) = streams.borrow_mut().1.remove(&handle) else {
                return "err:socket is closed".to_string();
            };
            TLS_SERVER_CERTIFICATES.with(|certificates| {
                certificates.borrow_mut().remove(&handle);
            });
            stream.conn.send_close_notify();
            if let Err(error) = stream.flush() {
                return format!("err:{error}");
            }
            let mut value = Vec::new();
            match stream.read_to_end(&mut value) {
                Ok(_) => format!("ok:{}", hex_encode(&value)),
                // See the matching client-side comment above.
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    format!("ok:{}", hex_encode(&value))
                }
                Err(error) => format!("err:{error}"),
            }
        })
    })
}

pub(super) fn tls_shutdown_write(handle: u32) -> String {
    let client = TLS_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let stream = streams.1.get_mut(&handle)?;
        stream.conn.send_close_notify();
        Some(
            stream
                .flush()
                .map(|_| "ok".to_string())
                .unwrap_or_else(|error| format!("err:{error}")),
        )
    });
    client.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            let mut streams = streams.borrow_mut();
            let Some(stream) = streams.1.get_mut(&handle) else {
                return "err:socket is closed".to_string();
            };
            stream.conn.send_close_notify();
            stream
                .flush()
                .map(|_| "ok".to_string())
                .unwrap_or_else(|error| format!("err:{error}"))
        })
    })
}

pub(super) fn tls_poll_read(handle: u32) -> String {
    let client = TLS_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let stream = streams.1.get_mut(&handle)?;
        let _ = stream.sock.set_nonblocking(true);
        let mut value = vec![0u8; 16 * 1024];
        let result = stream.read(&mut value);
        let _ = stream.sock.set_nonblocking(false);
        Some(match result {
            Ok(0) => {
                streams.1.remove(&handle);
                TLS_CLIENT_CERTIFICATES.with(|certificates| {
                    certificates.borrow_mut().remove(&handle);
                });
                "eof".to_string()
            }
            Ok(length) => {
                value.truncate(length);
                format!("ok:{}", hex_encode(&value))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => "pending".to_string(),
            Err(error) => format!("err:{error}"),
        })
    });
    client.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            let mut streams = streams.borrow_mut();
            let Some(stream) = streams.1.get_mut(&handle) else {
                return "err:socket is closed".to_string();
            };
            let _ = stream.sock.set_nonblocking(true);
            let mut value = vec![0u8; 16 * 1024];
            let result = stream.read(&mut value);
            let _ = stream.sock.set_nonblocking(false);
            match result {
                // Preserve the write half: an HTTP handler may reply after request EOF.
                // The JS socket retires this handle on end() or destroy().
                Ok(0) => "eof".to_string(),
                Ok(length) => {
                    value.truncate(length);
                    format!("ok:{}", hex_encode(&value))
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => "pending".to_string(),
                Err(error) => format!("err:{error}"),
            }
        })
    })
}

pub(super) fn tls_destroy(handle: u32) {
    TLS_STREAMS.with(|streams| {
        streams.borrow_mut().1.remove(&handle);
    });
    TLS_SERVER_STREAMS.with(|streams| {
        streams.borrow_mut().1.remove(&handle);
    });
    TLS_CLIENT_CERTIFICATES.with(|certificates| {
        certificates.borrow_mut().remove(&handle);
    });
    TLS_SERVER_CERTIFICATES.with(|certificates| {
        certificates.borrow_mut().remove(&handle);
    });
}

pub(super) fn tls_alpn(handle: u32) -> String {
    let client = TLS_STREAMS.with(|streams| {
        streams
            .borrow()
            .1
            .get(&handle)
            .and_then(|stream| stream.conn.alpn_protocol().map(hex_encode))
    });
    client.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            streams
                .borrow()
                .1
                .get(&handle)
                .and_then(|stream| stream.conn.alpn_protocol().map(hex_encode))
                .unwrap_or_default()
        })
    })
}

/// Negotiated protocol version and cipher suite of a connected TLS stream, as JSON, in the names Node reports
/// (`getProtocol()` / `getCipher()`); empty when the handle is unknown or the handshake has not finished.
pub(super) fn tls_session_info(handle: u32) -> String {
    fn describe(version: Option<rustls::ProtocolVersion>, suite: Option<rustls::SupportedCipherSuite>) -> String {
        let (Some(version), Some(suite)) = (version, suite) else {
            return String::new();
        };
        let protocol = match version {
            rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3".to_string(),
            rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2".to_string(),
            other => format!("{other:?}"),
        };
        // rustls spells TLS 1.3 suites `TLS13_*`; the IANA/Node standard name is `TLS_*`.
        let standard = format!("{:?}", suite.suite()).replacen("TLS13_", "TLS_", 1);
        // OpenSSL's cipher name: `TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256` -> `ECDHE-RSA-AES128-GCM-SHA256`
        // (TLS 1.3 suites keep their standard name).
        let name = if protocol == "TLSv1.3" {
            standard.clone()
        } else {
            let trimmed = standard.trim_start_matches("TLS_").replacen("WITH_", "", 1);
            trimmed
                .replacen("CHACHA20_POLY1305_SHA256", "CHACHA20_POLY1305", 1)
                .replace('_', "-")
                .replace("AES-128", "AES128")
                .replace("AES-256", "AES256")
        };
        format!(r#"{{"protocol":"{protocol}","name":"{name}","standardName":"{standard}"}}"#)
    }
    let client = TLS_STREAMS.with(|streams| {
        streams
            .borrow()
            .1
            .get(&handle)
            .map(|stream| describe(stream.conn.protocol_version(), stream.conn.negotiated_cipher_suite()))
    });
    client.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            streams
                .borrow()
                .1
                .get(&handle)
                .map(|stream| describe(stream.conn.protocol_version(), stream.conn.negotiated_cipher_suite()))
                .unwrap_or_default()
        })
    })
}

fn tls_certificate_bytes(handle: u32, peer: bool) -> Option<Vec<u8>> {
    let read = |certificates: &HashMap<u32, TlsCertificates>| {
        certificates
            .get(&handle)
            .and_then(|pair| {
                if peer {
                    pair.peer.as_deref()
                } else {
                    pair.local.as_deref()
                }
            })
            .map(ToOwned::to_owned)
    };
    let client = TLS_CLIENT_CERTIFICATES.with(|certificates| read(&certificates.borrow()));
    client.or_else(|| TLS_SERVER_CERTIFICATES.with(|certificates| read(&certificates.borrow())))
}

pub(super) fn tls_certificate(handle: u32, peer: bool) -> String {
    tls_certificate_bytes(handle, peer)
        .as_deref()
        .map(hex_encode)
        .unwrap_or_default()
}

pub(super) fn tls_authorization(handle: u32) -> String {
    let read = |certificates: &HashMap<u32, TlsCertificates>| certificates.get(&handle).map(|entry| serde_json::json!({
        "authorized": entry.authorized,
        "authorizationError": entry.authorization_error,
        "chainVerified": entry.chain_verified,
    }));
    let value = TLS_CLIENT_CERTIFICATES.with(|certificates| read(&certificates.borrow()))
        .or_else(|| TLS_SERVER_CERTIFICATES.with(|certificates| read(&certificates.borrow())));
    value.unwrap_or_else(|| serde_json::json!({"authorized": false, "authorizationError": "TLS socket is closed", "chainVerified": false})).to_string()
}

fn certificate_metadata(bytes: &[u8]) -> String {
    let Ok((_, certificate)) = x509_parser::parse_x509_certificate(bytes) else {
        return "{}".to_string();
    };
    let common_names = |name: &x509_parser::x509::X509Name<'_>| {
        name.iter_common_name()
            .filter_map(|attribute| attribute.as_str().ok().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let validity = certificate.validity();
    let san_result = certificate.subject_alternative_name();
    let mut dns_names = Vec::new();
    let mut ip_addresses = Vec::new();
    let mut has_dns_san = false;
    if let Ok(Some(extension)) = &san_result {
        for name in &extension.value.general_names {
            match name {
                x509_parser::extensions::GeneralName::DNSName(name) => {
                    has_dns_san = true;
                    dns_names.push((*name).to_owned())
                }
                x509_parser::extensions::GeneralName::IPAddress(bytes) if bytes.len() == 4 => {
                    if let Ok(octets) = <[u8; 4]>::try_from(*bytes) { ip_addresses.push(std::net::Ipv4Addr::from(octets).to_string()); }
                }
                x509_parser::extensions::GeneralName::IPAddress(bytes) if bytes.len() == 16 => {
                    if let Ok(octets) = <[u8; 16]>::try_from(*bytes) { ip_addresses.push(std::net::Ipv6Addr::from(octets).to_string()); }
                }
                _ => {}
            }
        }
    }
    let mut san_tokens = dns_names.iter().map(|name| {
        if name.chars().any(|character| matches!(character, ',' | '"' | '\\') || character.is_ascii_whitespace()) {
            format!("DNS:{}", serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string()))
        } else {
            format!("DNS:{name}")
        }
    }).collect::<Vec<_>>();
    san_tokens.extend(ip_addresses.iter().map(|address| format!("IP Address:{address}")));
    let subject_alt_name = san_tokens.join(", ");
    serde_json::json!({
        "subject": { "CN": common_names(certificate.subject()) },
        "issuer": { "CN": common_names(certificate.issuer()) },
        "valid_from": validity.not_before.to_rfc2822().unwrap_or_else(|_| validity.not_before.to_string()),
        "valid_to": validity.not_after.to_rfc2822().unwrap_or_else(|_| validity.not_after.to_string()),
        "serialNumber": certificate.raw_serial_as_string().replace(':', "").to_uppercase(),
        "dnsNames": dns_names,
        "hasDnsSan": has_dns_san,
        "ipAddresses": ip_addresses,
        "subjectaltname": subject_alt_name,
        "sanError": san_result.is_err(),
    })
    .to_string()
}

pub(super) fn tls_certificate_metadata(handle: u32, peer: bool) -> String {
    tls_certificate_bytes(handle, peer).as_deref().map(certificate_metadata).unwrap_or_else(|| "{}".to_string())
}

#[cfg(test)]
mod explicit_ca_tests {
    use super::*;

    const ROOT: &[u8] = include_bytes!("fixtures/tubitak-root.der");

    fn spec(bytes: &[u8]) -> String { hex_encode(bytes) }

    #[test]
    fn explicit_ca_replaces_defaults_and_preserves_all_pem_certificates() {
        let defaults = tls_client_roots(None).unwrap();
        assert_eq!(defaults.roots.as_slice(), webpki_roots::TLS_SERVER_ROOTS);
        assert!(defaults.len() > 1);
        assert_eq!(tls_ca_roots(None).unwrap().len(), 0);
        assert_eq!(tls_client_roots(Some("")).unwrap().len(), 0);
        let root = spec(ROOT);
        let explicit = tls_client_roots(Some(&root)).unwrap();
        let mut expected = RootCertStore::empty();
        expected.add(CertificateDer::from(ROOT.to_vec())).unwrap();
        assert_eq!(explicit.roots.as_slice(), expected.roots.as_slice());
        assert_eq!(explicit.len(), 1);
        let comma = tls_ca_roots(Some(&format!("{root},{root}"))).unwrap();
        assert_eq!(comma.roots.as_slice(), [expected.roots[0].clone(), expected.roots[0].clone()]);
        let pem = format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----", base64::engine::general_purpose::STANDARD.encode(ROOT), base64::engine::general_purpose::STANDARD.encode(ROOT));
        let pem_roots = tls_ca_roots(Some(&spec(pem.as_bytes()))).unwrap();
        assert_eq!(pem_roots.roots.as_slice(), [expected.roots[0].clone(), expected.roots[0].clone()]);
    }

    #[test]
    fn public_root_metadata_exposes_safe_subject_and_san_shapes() {
        let metadata: serde_json::Value = serde_json::from_str(&certificate_metadata(ROOT)).unwrap();
        assert!(metadata["subject"]["CN"].is_array());
        assert!(metadata["issuer"]["CN"].is_array());
        assert!(metadata["dnsNames"].is_array());
        assert!(metadata["hasDnsSan"].is_boolean());
        assert!(metadata["ipAddresses"].is_array());
        assert!(metadata["subjectaltname"].is_string());
        assert!(metadata["sanError"].is_boolean());
        assert_eq!(tls_canonical_ip("2001:0db8:0:0:0:0:0:1"), "2001:db8::1");
        assert_eq!(tls_canonical_ip("name.example"), "");
    }

    #[test]
    fn invalid_server_certificate_chain_is_recorded_before_strict_or_permissive_result() {
        let name = ServerName::try_from("identity.example").unwrap();
        let certificate = CertificateDer::from(b"not an X.509 certificate".to_vec());
        let timestamp = UnixTime::since_unix_epoch(Duration::from_secs(1));

        let strict_result = Arc::new(Mutex::new(None));
        let strict = RecordingServerVerifier {
            roots: RootCertStore::empty(),
            defer_name: true,
            allow_invalid: false,
            result: Arc::clone(&strict_result),
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        };
        assert!(rustls::client::danger::ServerCertVerifier::verify_server_cert(
            &strict, &certificate, &[], &name, &[], timestamp,
        ).is_err());
        assert!(strict_result.lock().unwrap().as_ref().is_some());

        let permissive_result = Arc::new(Mutex::new(None));
        let permissive = RecordingServerVerifier {
            roots: RootCertStore::empty(),
            defer_name: true,
            allow_invalid: true,
            result: Arc::clone(&permissive_result),
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        };
        assert!(rustls::client::danger::ServerCertVerifier::verify_server_cert(
            &permissive, &certificate, &[], &name, &[], timestamp,
        ).is_ok());
        assert!(permissive_result.lock().unwrap().as_ref().is_some());
    }

    #[test]
    fn server_authorization_requires_requested_and_verified_client_identity() {
        assert_eq!(server_client_auth_status(false, false, true), (false, Some("TLS client authentication was not requested")));
        assert_eq!(server_client_auth_status(true, false, true), (false, Some("CERT_NOT_PROVIDED")));
        assert_eq!(server_client_auth_status(true, true, false), (false, Some("CERT_REJECTED")));
        assert_eq!(server_client_auth_status(true, true, true), (true, None));
    }

    #[test]
    fn explicit_ca_rejects_malformed_or_partial_inputs() {
        for invalid in ["z", "abc", "00", "zz", "00,"] {
            assert!(tls_ca_roots(Some(invalid)).is_err(), "accepted {invalid:?}");
        }
        for pem in [
            b"-----BEGIN CERTIFICATE-----bad".as_slice(),
            b"-----BEGIN CERTIFICATE-----\n\n-----END CERTIFICATE-----".as_slice(),
            b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----".as_slice(),
        ] {
            assert!(tls_ca_roots(Some(&spec(pem))).is_err());
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(ROOT);
        let valid_pem = format!("-----BEGIN CERTIFICATE-----\n{encoded}\n-----END CERTIFICATE-----");
        for invalid_bundle in [
            format!("{valid_pem}\n-----BEGIN CERTIFICATE-----\nbad"),
            format!("{valid_pem}\n-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----"),
            format!("{valid_pem}\ntrailing"),
            format!("\u{2003}{valid_pem}"),
            format!("{valid_pem}\n\u{2003}{valid_pem}"),
            format!("-----BEGIN CERTIFICATE-----\n{encoded}\u{2003}\n-----END CERTIFICATE-----"),
        ] {
            assert!(tls_ca_roots(Some(&spec(invalid_bundle.as_bytes()))).is_err());
        }
        let mixed_bad_hex = format!("{},zz", spec(ROOT));
        assert!(tls_ca_roots(Some(&mixed_bad_hex)).is_err());
        let mixed_bad_der = format!("{},00", spec(ROOT));
        assert!(tls_ca_roots(Some(&mixed_bad_der)).is_err());
        let mixed_bad_pem = format!("{},{}", spec(ROOT), spec(b"-----BEGIN CERTIFICATE-----bad"));
        assert!(tls_ca_roots(Some(&mixed_bad_pem)).is_err());
    }
}

}

#[cfg(feature = "tls")]
use tls_host::*;

#[cfg(not(feature = "tls"))]
#[allow(dead_code)]
struct TlsClientOptions<'a> {
    host: &'a str,
    port: u16,
    server_name: &'a str,
    ca_spec: Option<&'a str>,
    cert_spec: &'a str,
    key_spec: &'a str,
    alpn_spec: &'a str,
    report_alpn: bool,
    reject_unauthorized: bool,
    defer_name_verification: bool,
}

#[cfg(not(feature = "tls"))]
#[allow(dead_code)]
struct TlsServerOptions<'a> {
    host: &'a str,
    port: u16,
    cert_spec: &'a str,
    key_spec: &'a str,
    ca_spec: Option<&'a str>,
    request_cert: bool,
    reject_unauthorized: bool,
    alpn_spec: &'a str,
}

#[cfg(not(feature = "tls"))]
fn tls_unavailable() -> String { "err:TLS support is not linked".to_string() }
#[cfg(not(feature = "tls"))]
fn tls_connect(_: TlsClientOptions<'_>) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_server_listen(_: TlsServerOptions<'_>) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_server_accept(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_server_poll_accept(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_server_read(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_server_close_listener(_: u32) {}
#[cfg(not(feature = "tls"))]
fn tls_write(_: u32, _: &[u8]) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_finish(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_shutdown_write(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_poll_read(_: u32) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_destroy(_: u32) {}
#[cfg(not(feature = "tls"))]
fn tls_alpn(_: u32) -> String { String::new() }
#[cfg(not(feature = "tls"))]
fn tls_session_info(_: u32) -> String { String::new() }
#[cfg(not(feature = "tls"))]
fn tls_certificate(_: u32, _: bool) -> String { String::new() }
#[cfg(not(feature = "tls"))]
fn tls_certificate_metadata(_: u32, _: bool) -> String { "{}".to_string() }
#[cfg(not(feature = "tls"))]
fn tls_authorization(_: u32) -> String { r#"{"authorized":false,"authorizationError":"TLS support is unavailable","chainVerified":false}"#.to_string() }

fn tls_canonical_ip(host: &str) -> String {
    host.parse::<std::net::IpAddr>().map(|address| address.to_string()).unwrap_or_default()
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn net_poll_read_retires_stream_after_peer_reset() {
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connected = net_connect("127.0.0.1", port);
    let handle: u32 = connected.strip_prefix("ok:").unwrap().split(':').next().unwrap().parse().unwrap();
    let (peer, _) = listener.accept().unwrap();
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    let configured = unsafe {
        libc::setsockopt(
            peer.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&linger as *const libc::linger).cast(),
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    assert_eq!(configured, 0);
    drop(peer);

    let deadline = Instant::now() + Duration::from_secs(1);
    let outcome = loop {
        let outcome = net_poll_read(handle);
        if outcome != "pending" || Instant::now() >= deadline {
            break outcome;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(outcome.starts_with("err:"), "{outcome}");
    assert_eq!(net_poll_read(handle), "err:socket is closed");
    NET_STREAMS.with(|streams| assert!(!streams.borrow().1.contains_key(&handle)));
}
