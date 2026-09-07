fn net_connect(host: &str, port: u16) -> String {
    match TcpStream::connect((host, port)) {
        Ok(stream) => {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            NET_STREAMS.with(|streams| {
                let mut streams = streams.borrow_mut();
                let handle = streams.0;
                streams.0 = streams.0.wrapping_add(1).max(1);
                streams.1.insert(handle, stream);
                format!("ok:{handle}")
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
    NET_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(stream) = streams.1.get_mut(&handle) else {
            return "err:socket is closed".to_string();
        };
        if let Err(error) = stream.set_nonblocking(true) {
            return format!("err:{error}");
        }
        let mut value = vec![0u8; 16 * 1024];
        let result = stream.read(&mut value);
        let _ = stream.set_nonblocking(false);
        match result {
            Ok(0) => {
                streams.1.remove(&handle);
                "eof".to_string()
            }
            Ok(length) => {
                value.truncate(length);
                format!("ok:{}", hex_encode(&value))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => "pending".to_string(),
            Err(error) => format!("err:{error}"),
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

fn udp_bind(host: &str, port: u16) -> String {
    match UdpSocket::bind((host, port)) {
        Ok(socket) => {
            let address = match socket.local_addr() {
                Ok(address) => address,
                Err(error) => return format!("err|{error}"),
            };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
            UDP_SOCKETS.with(|sockets| {
                let mut sockets = sockets.borrow_mut();
                let handle = sockets.0;
                sockets.0 = sockets.0.wrapping_add(1).max(1);
                sockets.1.insert(handle, socket);
                format!("ok|{handle}|{}|{}", address.ip(), address.port())
            })
        }
        Err(error) => format!("err|{error}"),
    }
}

fn udp_send(handle: u32, value: &[u8], host: &str, port: u16) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return "err|socket is closed".to_string();
        };
        socket
            .send_to(value, (host, port))
            .map(|written| format!("ok|{written}"))
            .unwrap_or_else(|error| format!("err|{error}"))
    })
}

fn udp_receive(handle: u32) -> String {
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else {
            return "err|socket is closed".to_string();
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
            Err(error) => format!("err|{error}"),
        }
    })
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
    pub(super) ca_spec: &'a str,
    pub(super) cert_spec: &'a str,
    pub(super) key_spec: &'a str,
    pub(super) alpn_spec: &'a str,
    pub(super) report_alpn: bool,
    pub(super) reject_unauthorized: bool,
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
    } = options;
    let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate_spec in ca_spec.split(',').filter(|value| !value.is_empty()) {
        let certificates = match decode_certificates(certificate_spec) {
            Ok(certificates) => certificates,
            Err(error) => return format!("err:{error}"),
        };
        for certificate in certificates {
            if let Err(error) = roots.add(certificate) {
                return format!("err:{error}");
            }
        }
    }
    let builder = ClientConfig::builder().with_root_certificates(roots);
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
    if !reject_unauthorized {
        config
            .dangerous()
            .set_certificate_verifier(Arc::new(InsecureServerVerifier));
    }
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
        let handle = streams.0;
        streams.0 = streams.0.wrapping_add(1).max(1);
        TLS_CLIENT_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().insert(
                handle,
                TlsCertificates {
                    peer: peer_certificate,
                    local: local_certificate,
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
    pub(super) ca_spec: &'a str,
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
    let builder = ServerConfig::builder();
    let builder = if request_cert {
        let mut roots = RootCertStore::empty();
        for certificate_spec in ca_spec.split(',').filter(|value| !value.is_empty()) {
            let ca_certificates = match decode_certificates(certificate_spec) {
                Ok(certificates) => certificates,
                Err(error) => return format!("err:{error}"),
            };
            for certificate in ca_certificates {
                if let Err(error) = roots.add(certificate) {
                    return format!("err:{error}");
                }
            }
        }
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
    let (mut socket, peer, config, local_certificate) = match accepted {
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
        let stream_handle = streams.0;
        streams.0 = streams.0.wrapping_add(1).max(1);
        TLS_SERVER_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().insert(
                stream_handle,
                TlsCertificates {
                    peer: peer_certificate,
                    local: Some(local_certificate),
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
                Ok(0) => {
                    streams.1.remove(&handle);
                    TLS_SERVER_CERTIFICATES.with(|certificates| {
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

pub(super) fn tls_certificate_metadata(handle: u32, peer: bool) -> String {
    let Some(bytes) = tls_certificate_bytes(handle, peer) else {
        return "{}".to_string();
    };
    let Ok((_, certificate)) = x509_parser::parse_x509_certificate(&bytes) else {
        return "{}".to_string();
    };
    let common_name = |name: &x509_parser::x509::X509Name<'_>| {
        name.iter_common_name()
            .next()
            .and_then(|attribute| attribute.as_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    let validity = certificate.validity();
    serde_json::json!({
        "subject": { "CN": common_name(certificate.subject()) },
        "issuer": { "CN": common_name(certificate.issuer()) },
        "valid_from": validity.not_before.to_rfc2822().unwrap_or_else(|_| validity.not_before.to_string()),
        "valid_to": validity.not_after.to_rfc2822().unwrap_or_else(|_| validity.not_after.to_string()),
        "serialNumber": certificate.raw_serial_as_string().replace(':', "").to_uppercase(),
    })
    .to_string()
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
    ca_spec: &'a str,
    cert_spec: &'a str,
    key_spec: &'a str,
    alpn_spec: &'a str,
    report_alpn: bool,
    reject_unauthorized: bool,
}

#[cfg(not(feature = "tls"))]
#[allow(dead_code)]
struct TlsServerOptions<'a> {
    host: &'a str,
    port: u16,
    cert_spec: &'a str,
    key_spec: &'a str,
    ca_spec: &'a str,
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
fn tls_certificate(_: u32, _: bool) -> String { String::new() }
#[cfg(not(feature = "tls"))]
fn tls_certificate_metadata(_: u32, _: bool) -> String { "{}".to_string() }
