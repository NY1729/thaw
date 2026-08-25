//! QuickJS-NG integration for the Fallback path (docs/design/bridge.md
//! section 7): runs arbitrary JS (e.g. an npm package's actual source, once
//! thaw-registry can fetch one) in an embedded engine, exchanging values
//! with compiled Thaw code as JSON text.
//!
//! `rquickjs` (which bundles QuickJS-NG -- the bnoordhuis/saghul fork the
//! design doc names, not Bellard's original) does the real work; this
//! crate is a thin C ABI shim around it, in the same spirit as
//! thaw-std/thaw-runtime.
//!
//! Argument/result marshaling goes through JSON both at the Rust/QuickJS
//! boundary (`thaw_js_call`'s `args_json`/return) and, one level up, at the
//! HIR/codegen boundary (`callDynamic`, see hir_codegen.rs), which
//! composes this crate's `thaw_js_call` with thaw-std's
//! `thaw_json_stringify`/`thaw_json_parse` so a `Json` value flows in and
//! out without this crate needing to know thaw-std's internal
//! representation. A Promise-returning call is driven to completion by
//! polling QuickJS's job queue (`Promise::finish`) rather than true
//! non-blocking integration -- the same "poll until resolved" shortcut V1
//! async/await already takes (docs/design/async-await.md), consistent
//! rather than a special case.
//!
//! Generated code uses `thaw_js_call_result` to route unknown functions,
//! thrown JS exceptions, malformed arguments, and Promise rejections through
//! Thaw's `try`/`catch`. The original `thaw_js_call` JSON-error-object API is
//! retained for C ABI compatibility with older callers.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::os::raw::c_char;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;
use rquickjs::function::Args;
use rquickjs::{Array, ArrayBuffer, Context, Ctx, Function, Object, Runtime, Value};
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error as RustlsError, RootCertStore,
    ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};
use sha2::{Digest, Sha256, Sha512};
use std::sync::Arc;

type TlsStream = StreamOwned<ClientConnection, TcpStream>;
type TlsStreamTable = (u32, HashMap<u32, TlsStream>);
type TlsServerStream = StreamOwned<ServerConnection, TcpStream>;
type TlsServerStreamTable = (u32, HashMap<u32, TlsServerStream>);

struct TlsListener {
    socket: TcpListener,
    config: Arc<ServerConfig>,
    local_certificate: Vec<u8>,
}

#[derive(Default)]
struct TlsCertificates {
    peer: Option<Vec<u8>>,
    local: Option<Vec<u8>>,
}

#[derive(Debug)]
struct InsecureServerVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureServerVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, RustlsError> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn compress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::write::{DeflateEncoder, GzEncoder, ZlibEncoder};
    use flate2::Compression;
    let compression = Compression::default();
    match format {
        "gzip" => {
            let mut encoder = GzEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
        "deflateRaw" => {
            let mut encoder = DeflateEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
        _ => {
            let mut encoder = ZlibEncoder::new(Vec::new(), compression);
            encoder.write_all(value)?;
            encoder.finish()
        }
    }
}

fn decompress_bytes(format: &str, value: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    let mut output = Vec::new();
    match format {
        "gzip" => GzDecoder::new(value).read_to_end(&mut output)?,
        "deflateRaw" => DeflateDecoder::new(value).read_to_end(&mut output)?,
        _ => ZlibDecoder::new(value).read_to_end(&mut output)?,
    };
    Ok(output)
}

thread_local! {
    static JS: RefCell<Option<(Runtime, Context)>> = const { RefCell::new(None) };
    static NET_STREAMS: RefCell<(u32, HashMap<u32, TcpStream>)> = RefCell::new((1, HashMap::new()));
    static NET_LISTENERS: RefCell<(u32, HashMap<u32, TcpListener>)> = RefCell::new((1, HashMap::new()));
    static UDP_SOCKETS: RefCell<(u32, HashMap<u32, UdpSocket>)> = RefCell::new((1, HashMap::new()));
    static TLS_STREAMS: RefCell<TlsStreamTable> = RefCell::new((1, HashMap::new()));
    static TLS_SERVER_STREAMS: RefCell<TlsServerStreamTable> = RefCell::new((1, HashMap::new()));
    static TLS_LISTENERS: RefCell<(u32, HashMap<u32, TlsListener>)> = RefCell::new((1, HashMap::new()));
    static TLS_CLIENT_CERTIFICATES: RefCell<HashMap<u32, TlsCertificates>> = RefCell::new(HashMap::new());
    static TLS_SERVER_CERTIFICATES: RefCell<HashMap<u32, TlsCertificates>> = RefCell::new(HashMap::new());
    static HOST_WORKERS: RefCell<HostWorkerTable> = RefCell::new(HostWorkerTable { next_handle: 1, workers: HashMap::new() });
}

enum HostWorkerCommand {
    Message(String),
    Terminate,
}

enum HostWorkerEvent {
    Online,
    Message(String),
    Error(String),
    Exit(i32),
}

struct HostWorker {
    commands: Sender<HostWorkerCommand>,
    events: Receiver<HostWorkerEvent>,
    thread: Option<JoinHandle<()>>,
}

struct HostWorkerTable {
    next_handle: u32,
    workers: HashMap<u32, HostWorker>,
}

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
        let mut value = Vec::new();
        match stream.read_to_end(&mut value) {
            Ok(_) => format!("ok:{}", hex_encode(&value)),
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

fn tls_connect(options: TlsClientOptions<'_>) -> String {
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

fn tls_server_listen(options: TlsServerOptions<'_>) -> String {
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

fn tls_server_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, false)
}

fn tls_server_poll_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, true)
}

fn tls_server_read(handle: u32) -> String {
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

fn tls_server_close_listener(handle: u32) {
    TLS_LISTENERS.with(|listeners| {
        listeners.borrow_mut().1.remove(&handle);
    });
}

fn tls_write(handle: u32, value: &[u8]) -> String {
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

fn tls_finish(handle: u32) -> String {
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

fn tls_destroy(handle: u32) {
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

fn tls_alpn(handle: u32) -> String {
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

fn tls_certificate(handle: u32, peer: bool) -> String {
    tls_certificate_bytes(handle, peer)
        .as_deref()
        .map(hex_encode)
        .unwrap_or_default()
}

fn tls_certificate_metadata(handle: u32, peer: bool) -> String {
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

fn to_str(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn hex_decode(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap_or("00");
            u8::from_str_radix(text, 16).unwrap_or(0)
        })
        .collect()
}

fn hex_encode(value: &[u8]) -> String {
    value.iter().fold(String::new(), |mut output, byte| {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
        output
    })
}

fn digest_bytes(algorithm: &str, value: &[u8]) -> Vec<u8> {
    match algorithm {
        "sha256" => Sha256::digest(value).to_vec(),
        "sha512" => Sha512::digest(value).to_vec(),
        _ => Vec::new(),
    }
}

fn hmac_bytes(algorithm: &str, key: &[u8], value: &[u8]) -> Vec<u8> {
    let block_size = if algorithm == "sha512" { 128 } else { 64 };
    let mut normalized = if key.len() > block_size {
        digest_bytes(algorithm, key)
    } else {
        key.to_vec()
    };
    normalized.resize(block_size, 0);
    let inner_key = normalized
        .iter()
        .map(|byte| byte ^ 0x36)
        .collect::<Vec<_>>();
    let outer_key = normalized
        .iter()
        .map(|byte| byte ^ 0x5c)
        .collect::<Vec<_>>();
    let mut inner = inner_key;
    inner.extend_from_slice(value);
    let inner_digest = digest_bytes(algorithm, &inner);
    let mut outer = outer_key;
    outer.extend_from_slice(&inner_digest);
    digest_bytes(algorithm, &outer)
}

fn os_info_json() -> String {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        value => value,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        value => value,
    };
    let mut hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "localhost".to_string())
        .trim()
        .to_string();
    if hostname.is_empty() {
        hostname = "localhost".to_string();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let tmp = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let cpu_info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("model name\t: "))
        .unwrap_or(arch)
        .to_string();
    let speed = cpu_info
        .lines()
        .find_map(|line| line.strip_prefix("cpu MHz\t\t: "))
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .round() as u64;
    let cpu_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let cpus = (0..cpu_count)
        .map(|_| serde_json::json!({ "model": model, "speed": speed, "times": { "user": 0, "nice": 0, "sys": 0, "idle": 0, "irq": 0 } }))
        .collect::<Vec<_>>();
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let memory_value = |name: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    let uptime = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0);
    let loadavg = std::fs::read_to_string("/proc/loadavg")
        .unwrap_or_default()
        .split_whitespace()
        .take(3)
        .filter_map(|value| value.parse::<f64>().ok())
        .collect::<Vec<_>>();
    serde_json::json!({
        "platform": platform, "arch": arch, "type": if platform == "linux" { "Linux" } else { platform },
        "hostname": hostname, "homedir": home, "tmpdir": tmp, "release": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
        "version": std::fs::read_to_string("/proc/sys/kernel/version").unwrap_or_default().trim(), "machine": std::env::consts::ARCH,
        "endianness": if cfg!(target_endian = "little") { "LE" } else { "BE" }, "cpus": cpus,
        "totalmem": memory_value("MemTotal:"), "freemem": memory_value("MemAvailable:"), "uptime": uptime, "loadavg": loadavg,
        "userInfo": { "username": std::env::var("USER").unwrap_or_default(), "homedir": home, "shell": std::env::var("SHELL").unwrap_or_default(), "uid": 0, "gid": 0 }
    }).to_string()
}

fn host_worker_bootstrap(worker_data_json: &str, config_json: &str, thread_id: u32) -> String {
    let worker_data = serde_json::to_string(worker_data_json).unwrap_or_else(|_| "\"null\"".into());
    let config = serde_json::to_string(config_json).unwrap_or_else(|_| "\"{}\"".into());
    format!(
        r#"
        globalThis.__thaw_host_worker_events = [];
        globalThis.__thaw_host_worker_closed = false;
        const __thaw_host_worker_listeners = new Map();
        function __thaw_host_worker_on(name, listener) {{
          const key = String(name), list = __thaw_host_worker_listeners.get(key) || [];
          list.push(listener); __thaw_host_worker_listeners.set(key, list); return parentPort;
        }}
        function __thaw_host_worker_off(name, listener) {{
          const key = String(name), list = __thaw_host_worker_listeners.get(key) || [];
          __thaw_host_worker_listeners.set(key, list.filter(item => item !== listener && item.listener !== listener)); return parentPort;
        }}
        const parentPort = {{
          postMessage(value) {{
            const payload = JSON.stringify(value);
            if (payload === undefined) throw new DOMException('value cannot be cloned', 'DataCloneError');
            __thaw_host_worker_events.push(payload);
          }},
          on: __thaw_host_worker_on,
          addListener: __thaw_host_worker_on,
          once(name, listener) {{ const wrapped = (...args) => {{ __thaw_host_worker_off(name, wrapped); listener(...args); }}; wrapped.listener = listener; return __thaw_host_worker_on(name, wrapped); }},
          off: __thaw_host_worker_off,
          removeListener: __thaw_host_worker_off,
          close() {{ globalThis.__thaw_host_worker_closed = true; }},
          start() {{}}, ref() {{ return this; }}, unref() {{ return this; }}, hasRef() {{ return true; }}
        }};
        const workerData = JSON.parse({worker_data});
        const __thaw_host_worker_config = JSON.parse({config});
        globalThis.module = {{ exports: {{}} }};
        globalThis.exports = globalThis.module.exports;
        process.env = Object.assign({{}}, __thaw_host_worker_config.env || {{}});
        process.argv = Array.from(__thaw_host_worker_config.argv || []);
        process.execArgv = Array.from(__thaw_host_worker_config.execArgv || []);
        globalThis.__thaw_worker_module = {{ isMainThread: false, threadId: {thread_id}, threadName: String(__thaw_host_worker_config.threadName || ''), workerData, parentPort, resourceLimits: Object.assign({{}}, __thaw_host_worker_config.resourceLimits || {{}}), MessageChannel, MessagePort, BroadcastChannel, receiveMessageOnPort(port) {{ const record = port && port.__thawQueue && port.__thawQueue.shift(); return record ? {{ message: record.data }} : undefined; }} }};
        globalThis.require = function(name) {{
          if (name === 'worker_threads' || name === 'node:worker_threads') return globalThis.__thaw_worker_module;
          if (typeof globalThis.__thaw_bundle_create_require === 'function') return globalThis.__thaw_bundle_create_require('')(name);
          throw new Error("require('" + name + "') is not available in this Worker");
        }};
        globalThis.__thaw_host_worker_deliver = function(payload) {{
          const value = JSON.parse(payload);
          for (const listener of (__thaw_host_worker_listeners.get('message') || []).slice()) listener(value);
        }};
        globalThis.__thaw_host_worker_drain = function() {{ return JSON.stringify(__thaw_host_worker_events.splice(0)); }};
        globalThis.__thaw_host_worker_should_exit = function() {{
          return globalThis.__thaw_host_worker_closed || (((__thaw_host_worker_listeners.get('message') || []).length === 0) && __thaw_next_timer_delay() < 0);
        }};
        "#
    )
}

fn drain_host_worker_events(ctx: &Ctx<'_>, events: &Sender<HostWorkerEvent>) -> Result<(), String> {
    let drain: Function = ctx
        .globals()
        .get("__thaw_host_worker_drain")
        .map_err(|error| error.to_string())?;
    let payloads: String = drain.call(()).map_err(|error| error.to_string())?;
    let payloads: Vec<String> =
        serde_json::from_str(&payloads).map_err(|error| error.to_string())?;
    for payload in payloads {
        let _ = events.send(HostWorkerEvent::Message(payload));
    }
    Ok(())
}

fn run_host_worker(
    bundle_source: String,
    source: String,
    worker_data_json: String,
    config_json: String,
    thread_id: u32,
    commands: Receiver<HostWorkerCommand>,
    events: Sender<HostWorkerEvent>,
) {
    let result = with_context(|ctx| -> Result<i32, String> {
        if !bundle_source.is_empty() {
            load_impl(ctx.clone(), &bundle_source)?;
        }
        load_impl(
            ctx.clone(),
            &host_worker_bootstrap(&worker_data_json, &config_json, thread_id),
        )?;
        let _ = events.send(HostWorkerEvent::Online);
        load_impl(ctx.clone(), &source)?;
        loop {
            while ctx.execute_pending_job() {}
            let run_due: Function = ctx
                .globals()
                .get("__thaw_run_due_timers")
                .map_err(|error| error.to_string())?;
            run_due
                .call::<_, usize>(())
                .map_err(|error| error.to_string())?;
            while ctx.execute_pending_job() {}
            drain_host_worker_events(&ctx, &events)?;

            let should_exit: Function = ctx
                .globals()
                .get("__thaw_host_worker_should_exit")
                .map_err(|error| error.to_string())?;
            if should_exit
                .call::<_, bool>(())
                .map_err(|error| error.to_string())?
            {
                return Ok(0);
            }

            match commands.recv_timeout(Duration::from_millis(1)) {
                Ok(HostWorkerCommand::Message(payload)) => {
                    let deliver: Function = ctx
                        .globals()
                        .get("__thaw_host_worker_deliver")
                        .map_err(|error| error.to_string())?;
                    deliver
                        .call::<_, ()>((payload,))
                        .map_err(|error| match error {
                            rquickjs::Error::Exception => describe_exception(&ctx),
                            error => error.to_string(),
                        })?;
                }
                Ok(HostWorkerCommand::Terminate) => return Ok(1),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(1),
            }
        }
    });
    match result {
        Ok(code) => {
            let _ = events.send(HostWorkerEvent::Exit(code));
        }
        Err(error) => {
            let _ = events.send(HostWorkerEvent::Error(error));
            let _ = events.send(HostWorkerEvent::Exit(1));
        }
    }
}

fn spawn_host_worker(
    bundle_source: String,
    source: String,
    worker_data_json: String,
    config_json: String,
    thread_id: u32,
) -> u32 {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        run_host_worker(
            bundle_source,
            source,
            worker_data_json,
            config_json,
            thread_id,
            command_receiver,
            event_sender,
        );
    });
    HOST_WORKERS.with(|table| {
        let mut table = table.borrow_mut();
        let handle = table.next_handle;
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        table.workers.insert(
            handle,
            HostWorker {
                commands: command_sender,
                events: event_receiver,
                thread: Some(thread),
            },
        );
        handle
    })
}

fn send_host_worker(handle: u32, payload: String) -> bool {
    HOST_WORKERS.with(|table| {
        table.borrow().workers.get(&handle).is_some_and(|worker| {
            worker
                .commands
                .send(HostWorkerCommand::Message(payload))
                .is_ok()
        })
    })
}

fn terminate_host_worker(handle: u32) -> bool {
    HOST_WORKERS.with(|table| {
        table
            .borrow()
            .workers
            .get(&handle)
            .is_some_and(|worker| worker.commands.send(HostWorkerCommand::Terminate).is_ok())
    })
}

fn host_workers_active() -> bool {
    HOST_WORKERS.with(|table| !table.borrow().workers.is_empty())
}

fn poll_host_workers() -> String {
    HOST_WORKERS.with(|table| {
        let mut table = table.borrow_mut();
        let handles = table.workers.keys().copied().collect::<Vec<_>>();
        let mut output = Vec::new();
        let mut finished = Vec::new();
        for handle in handles {
            let Some(worker) = table.workers.get(&handle) else {
                continue;
            };
            loop {
                match worker.events.try_recv() {
                    Ok(HostWorkerEvent::Online) => {
                        output.push(serde_json::json!({ "handle": handle, "type": "online" }));
                    }
                    Ok(HostWorkerEvent::Message(payload)) => {
                        let value = serde_json::from_str::<serde_json::Value>(&payload)
                            .unwrap_or(serde_json::Value::Null);
                        output.push(
                        serde_json::json!({ "handle": handle, "type": "message", "value": value }),
                    );
                    }
                    Ok(HostWorkerEvent::Error(error)) => {
                        output.push(
                        serde_json::json!({ "handle": handle, "type": "error", "error": error }),
                    );
                    }
                    Ok(HostWorkerEvent::Exit(code)) => {
                        output.push(
                            serde_json::json!({ "handle": handle, "type": "exit", "code": code }),
                        );
                        finished.push(handle);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        finished.push(handle);
                        break;
                    }
                }
            }
        }
        finished.sort_unstable();
        finished.dedup();
        for handle in finished {
            if let Some(mut worker) = table.workers.remove(&handle) {
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
            }
        }
        serde_json::to_string(&output).unwrap_or_else(|_| "[]".into())
    })
}

fn with_context<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
    JS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let (_, context) = slot.get_or_insert_with(|| {
            let runtime = Runtime::new().expect("failed to create a QuickJS runtime");
            let context = Context::full(&runtime).expect("failed to create a QuickJS context");
            context.with(|ctx| {
                ctx.globals()
                    .set(
                        "__thaw_os_thread_token",
                        format!("{:?}", std::thread::current().id()),
                    )
                    .expect("failed to install OS thread identity");
                let stdout = Function::new(ctx.clone(), |text: String| {
                    print!("{text}");
                    let _ = io::stdout().flush();
                })
                .expect("failed to create JavaScript stdout writer");
                let stderr = Function::new(ctx.clone(), |text: String| {
                    eprint!("{text}");
                    let _ = io::stderr().flush();
                })
                .expect("failed to create JavaScript stderr writer");
                ctx.globals()
                    .set("__thaw_console_stdout", stdout)
                    .expect("failed to install JavaScript stdout writer");
                ctx.globals()
                    .set("__thaw_console_stderr", stderr)
                    .expect("failed to install JavaScript stderr writer");
                let detach_array_buffer =
                    Function::new(ctx.clone(), |mut value: ArrayBuffer<'_>| {
                        value.detach();
                    })
                    .expect("failed to create ArrayBuffer detacher");
                ctx.globals()
                    .set("__thaw_detach_array_buffer", detach_array_buffer)
                    .expect("failed to install ArrayBuffer detacher");
                let worker_spawn = Function::new(
                    ctx.clone(),
                    |bundle_source: String,
                     source: String,
                     worker_data_json: String,
                     config_json: String,
                     thread_id: u32| {
                        spawn_host_worker(
                            bundle_source,
                            source,
                            worker_data_json,
                            config_json,
                            thread_id,
                        )
                    },
                )
                .expect("failed to create Worker spawner");
                let worker_send = Function::new(ctx.clone(), |handle: u32, payload: String| {
                    send_host_worker(handle, payload)
                })
                .expect("failed to create Worker sender");
                let worker_terminate =
                    Function::new(ctx.clone(), |handle: u32| terminate_host_worker(handle))
                        .expect("failed to create Worker terminator");
                let worker_poll = Function::new(ctx.clone(), poll_host_workers)
                    .expect("failed to create Worker event poller");
                let worker_active = Function::new(ctx.clone(), host_workers_active)
                    .expect("failed to create Worker activity probe");
                ctx.globals()
                    .set("__thaw_worker_spawn", worker_spawn)
                    .expect("failed to install Worker spawner");
                ctx.globals()
                    .set("__thaw_worker_send", worker_send)
                    .expect("failed to install Worker sender");
                ctx.globals()
                    .set("__thaw_worker_terminate", worker_terminate)
                    .expect("failed to install Worker terminator");
                ctx.globals()
                    .set("__thaw_worker_poll", worker_poll)
                    .expect("failed to install Worker event poller");
                ctx.globals()
                    .set("__thaw_worker_active", worker_active)
                    .expect("failed to install Worker activity probe");
                let random_hex = Function::new(ctx.clone(), |size: u32| {
                    let mut bytes = vec![0u8; size as usize];
                    getrandom::getrandom(&mut bytes).expect("OS random source failed");
                    hex_encode(&bytes)
                })
                .expect("failed to create JavaScript random source");
                let hash_hex = Function::new(ctx.clone(), |algorithm: String, value: String| {
                    hex_encode(&digest_bytes(&algorithm, &hex_decode(&value)))
                })
                .expect("failed to create JavaScript hash function");
                let hmac_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String, key: String, value: String| {
                        hex_encode(&hmac_bytes(
                            &algorithm,
                            &hex_decode(&key),
                            &hex_decode(&value),
                        ))
                    },
                )
                .expect("failed to create JavaScript HMAC function");
                let zlib_hex = Function::new(
                    ctx.clone(),
                    |operation: String,
                     format: String,
                     value: String|
                     -> rquickjs::Result<String> {
                        let input = hex_decode(&value);
                        let result = if operation == "compress" {
                            compress_bytes(&format, &input)
                        } else {
                            decompress_bytes(&format, &input)
                        };
                        result.map(|bytes| hex_encode(&bytes)).map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "zlib",
                                "Buffer",
                                error.to_string(),
                            )
                        })
                    },
                )
                .expect("failed to create JavaScript compression function");
                let tcp_connect = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_connect(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP connector");
                let tcp_write = Function::new(ctx.clone(), |handle: u32, value: String| {
                    net_write(handle, &hex_decode(&value))
                })
                .expect("failed to create JavaScript TCP writer");
                let tcp_finish = Function::new(ctx.clone(), |handle: u32| net_finish(handle))
                    .expect("failed to create JavaScript TCP finisher");
                let tcp_destroy = Function::new(ctx.clone(), |handle: u32| net_destroy(handle))
                    .expect("failed to create JavaScript TCP closer");
                let tcp_listen = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_listen(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP listener");
                let tcp_accept = Function::new(ctx.clone(), |handle: u32| net_accept(handle))
                    .expect("failed to create JavaScript TCP acceptor");
                let tcp_poll_accept =
                    Function::new(ctx.clone(), |handle: u32| net_poll_accept(handle))
                        .expect("failed to create JavaScript TCP polling acceptor");
                let tcp_read = Function::new(ctx.clone(), |handle: u32| net_read_all(handle))
                    .expect("failed to create JavaScript TCP reader");
                let tcp_close_listener =
                    Function::new(ctx.clone(), |handle: u32| net_close_listener(handle))
                        .expect("failed to create JavaScript TCP listener closer");
                let udp_bind_function = Function::new(ctx.clone(), |host: String, port: u32| {
                    udp_bind(&host, port as u16)
                })
                .expect("failed to create JavaScript UDP binder");
                let udp_send_function = Function::new(
                    ctx.clone(),
                    |handle: u32, value: String, host: String, port: u32| {
                        udp_send(handle, &hex_decode(&value), &host, port as u16)
                    },
                )
                .expect("failed to create JavaScript UDP sender");
                let udp_receive_function =
                    Function::new(ctx.clone(), |handle: u32| udp_receive(handle))
                        .expect("failed to create JavaScript UDP receiver");
                let udp_close_function =
                    Function::new(ctx.clone(), |handle: u32| udp_close(handle))
                        .expect("failed to create JavaScript UDP closer");
                let tls_connect_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, server_name: String, ca: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: "",
                            key_spec: "",
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS connector");
                let tls_connect_with_identity_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS connector");
                let tls_connect_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String,
                     options: String| {
                        let (verification, alpn) =
                            options.split_once('|').unwrap_or(("1", options.as_str()));
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: alpn,
                            report_alpn: true,
                            reject_unauthorized: verification != "0",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options connector");
                let tls_write_function =
                    Function::new(ctx.clone(), |handle: u32, value: String| {
                        tls_write(handle, &hex_decode(&value))
                    })
                    .expect("failed to create JavaScript TLS writer");
                let tls_finish_function =
                    Function::new(ctx.clone(), |handle: u32| tls_finish(handle))
                        .expect("failed to create JavaScript TLS finisher");
                let tls_destroy_function =
                    Function::new(ctx.clone(), |handle: u32| tls_destroy(handle))
                        .expect("failed to create JavaScript TLS closer");
                let tls_alpn_function = Function::new(ctx.clone(), |handle: u32| tls_alpn(handle))
                    .expect("failed to create JavaScript TLS ALPN reader");
                let tls_peer_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, true))
                        .expect("failed to create JavaScript TLS peer certificate reader");
                let tls_local_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, false))
                        .expect("failed to create JavaScript TLS local certificate reader");
                let tls_peer_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, true)
                    })
                    .expect("failed to create JavaScript TLS peer certificate metadata reader");
                let tls_local_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, false)
                    })
                    .expect("failed to create JavaScript TLS local certificate metadata reader");
                let tls_server_listen_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, cert: String, key: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: "",
                            request_cert: false,
                            reject_unauthorized: true,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS listener");
                let tls_server_listen_with_ca_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     reject_unauthorized: bool| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: true,
                            reject_unauthorized,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS listener");
                let tls_server_listen_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     flags: u32,
                     alpn: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: flags & 1 != 0,
                            reject_unauthorized: flags & 2 != 0,
                            alpn_spec: &alpn,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options listener");
                let tls_server_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_accept(handle))
                        .expect("failed to create JavaScript TLS acceptor");
                let tls_server_poll_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_poll_accept(handle))
                        .expect("failed to create JavaScript TLS polling acceptor");
                let tls_server_read_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_read(handle))
                        .expect("failed to create JavaScript TLS reader");
                let tls_server_close_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_close_listener(handle))
                        .expect("failed to create JavaScript TLS listener closer");
                let os_info_function = Function::new(ctx.clone(), os_info_json)
                    .expect("failed to create JavaScript OS information source");
                ctx.globals()
                    .set("__thaw_crypto_random_hex", random_hex)
                    .expect("failed to install JavaScript random source");
                ctx.globals()
                    .set("__thaw_crypto_hash_hex", hash_hex)
                    .expect("failed to install JavaScript hash function");
                ctx.globals()
                    .set("__thaw_crypto_hmac_hex", hmac_hex)
                    .expect("failed to install JavaScript HMAC function");
                ctx.globals()
                    .set("__thaw_zlib_hex", zlib_hex)
                    .expect("failed to install JavaScript compression function");
                ctx.globals()
                    .set("__thaw_net_connect", tcp_connect)
                    .expect("failed to install JavaScript TCP connector");
                ctx.globals()
                    .set("__thaw_net_write", tcp_write)
                    .expect("failed to install JavaScript TCP writer");
                ctx.globals()
                    .set("__thaw_net_finish", tcp_finish)
                    .expect("failed to install JavaScript TCP finisher");
                ctx.globals()
                    .set("__thaw_net_destroy", tcp_destroy)
                    .expect("failed to install JavaScript TCP closer");
                ctx.globals()
                    .set("__thaw_net_listen", tcp_listen)
                    .expect("failed to install JavaScript TCP listener");
                ctx.globals()
                    .set("__thaw_net_accept", tcp_accept)
                    .expect("failed to install JavaScript TCP acceptor");
                ctx.globals()
                    .set("__thaw_net_poll_accept", tcp_poll_accept)
                    .expect("failed to install JavaScript TCP polling acceptor");
                ctx.globals()
                    .set("__thaw_net_read", tcp_read)
                    .expect("failed to install JavaScript TCP reader");
                ctx.globals()
                    .set("__thaw_net_close_listener", tcp_close_listener)
                    .expect("failed to install JavaScript TCP listener closer");
                ctx.globals()
                    .set("__thaw_udp_bind", udp_bind_function)
                    .expect("failed to install JavaScript UDP binder");
                ctx.globals()
                    .set("__thaw_udp_send", udp_send_function)
                    .expect("failed to install JavaScript UDP sender");
                ctx.globals()
                    .set("__thaw_udp_receive", udp_receive_function)
                    .expect("failed to install JavaScript UDP receiver");
                ctx.globals()
                    .set("__thaw_udp_close", udp_close_function)
                    .expect("failed to install JavaScript UDP closer");
                ctx.globals()
                    .set("__thaw_tls_connect", tls_connect_function)
                    .expect("failed to install JavaScript TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_identity",
                        tls_connect_with_identity_function,
                    )
                    .expect("failed to install JavaScript mutual TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_options",
                        tls_connect_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options connector");
                ctx.globals()
                    .set("__thaw_tls_write", tls_write_function)
                    .expect("failed to install JavaScript TLS writer");
                ctx.globals()
                    .set("__thaw_tls_finish", tls_finish_function)
                    .expect("failed to install JavaScript TLS finisher");
                ctx.globals()
                    .set("__thaw_tls_destroy", tls_destroy_function)
                    .expect("failed to install JavaScript TLS closer");
                ctx.globals()
                    .set("__thaw_tls_alpn", tls_alpn_function)
                    .expect("failed to install JavaScript TLS ALPN reader");
                ctx.globals()
                    .set("__thaw_tls_peer_certificate", tls_peer_certificate_function)
                    .expect("failed to install JavaScript TLS peer certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate",
                        tls_local_certificate_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_peer_certificate_metadata",
                        tls_peer_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS peer certificate metadata reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate_metadata",
                        tls_local_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate metadata reader");
                ctx.globals()
                    .set("__thaw_tls_server_listen", tls_server_listen_function)
                    .expect("failed to install JavaScript TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_ca",
                        tls_server_listen_with_ca_function,
                    )
                    .expect("failed to install JavaScript mutual TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_options",
                        tls_server_listen_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options listener");
                ctx.globals()
                    .set("__thaw_tls_server_accept", tls_server_accept_function)
                    .expect("failed to install JavaScript TLS acceptor");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_poll_accept",
                        tls_server_poll_accept_function,
                    )
                    .expect("failed to install JavaScript TLS polling acceptor");
                ctx.globals()
                    .set("__thaw_tls_server_read", tls_server_read_function)
                    .expect("failed to install JavaScript TLS reader");
                ctx.globals()
                    .set("__thaw_tls_server_close", tls_server_close_function)
                    .expect("failed to install JavaScript TLS listener closer");
                ctx.globals()
                    .set("__thaw_os_info", os_info_function)
                    .expect("failed to install JavaScript OS information source");
                ctx.eval::<(), _>(PLATFORM_GLOBALS)
                    .expect("failed to install JavaScript platform globals");
            });
            (runtime, context)
        });
        context.with(f)
    })
}

const PLATFORM_GLOBALS: &str = r#"
(() => {
  let nextTimerId = 1;
  const timers = new Map();
  const normalizeDelay = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) return 0;
    return Math.min(Math.trunc(number), 2147483647);
  };
  const schedule = (callback, delay, repeat, args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('timer callback must be a function');
    }
    const id = nextTimerId++;
    const milliseconds = normalizeDelay(delay);
    timers.set(id, { callback, args, repeat, milliseconds,
                     due: Date.now() + milliseconds });
    return id;
  };
  globalThis.setTimeout = (callback, delay = 0, ...args) =>
    schedule(callback, delay, false, args);
  globalThis.clearTimeout = id => { timers.delete(Number(id)); };
  globalThis.setInterval = (callback, delay = 0, ...args) =>
    schedule(callback, delay, true, args);
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = (callback, ...args) =>
    schedule(callback, 0, false, args);
  globalThis.clearImmediate = globalThis.clearTimeout;
  globalThis.queueMicrotask = callback => {
    if (typeof callback !== 'function') {
      throw new TypeError('microtask callback must be a function');
    }
    Promise.resolve().then(callback);
  };
  const consoleInspect = value => {
    if (typeof value === 'string') return value;
    if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);
    try { const encoded = JSON.stringify(value); return encoded === undefined ? String(value) : encoded; }
    catch (_) { return '[Circular]'; }
  };
  const consoleFormat = (...args) => {
    if (args.length === 0) return '';
    if (typeof args[0] !== 'string') return args.map(consoleInspect).join(' ');
    let index = 1;
    let output = args[0].replace(/%[sdifjoOc%]/g, token => {
      if (token === '%%') return '%';
      if (index >= args.length) return token;
      const value = args[index++];
      if (token === '%s') return String(value);
      if (token === '%d' || token === '%f') return String(Number(value));
      if (token === '%i') return String(parseInt(value, 10));
      if (token === '%c') return '';
      return consoleInspect(value);
    });
    while (index < args.length) output += ' ' + consoleInspect(args[index++]);
    return output;
  };
  globalThis.Console = class Console {
    constructor(stdout, stderr) {
      const options = stdout && stdout.stdout ? stdout : { stdout, stderr };
      this._stdout = options.stdout;
      this._stderr = options.stderr || options.stdout;
      this._counts = new Map();
      this._timers = new Map();
      this._indent = '';
    }
    _write(stream, args) {
      const text = this._indent + consoleFormat(...args) + '\n';
      if (typeof stream === 'function') stream(text);
      else if (stream && typeof stream.write === 'function') stream.write(text);
    }
    log(...args) { this._write(this._stdout, args); }
    info(...args) { this.log(...args); }
    debug(...args) { this.log(...args); }
    warn(...args) { this._write(this._stderr, args); }
    error(...args) { this.warn(...args); }
    dir(value, options) { this.log(consoleInspect(value)); }
    dirxml(...args) { this.log(...args); }
    table(value) { this.log(consoleInspect(value)); }
    assert(condition, ...args) {
      if (!condition) this.error('Assertion failed' + (args.length ? ': ' + consoleFormat(...args) : ''));
    }
    count(label = 'default') {
      const name = String(label); const value = (this._counts.get(name) || 0) + 1;
      this._counts.set(name, value); this.log(`${name}: ${value}`);
    }
    countReset(label = 'default') { this._counts.delete(String(label)); }
    time(label = 'default') { this._timers.set(String(label), Date.now()); }
    timeLog(label = 'default', ...args) {
      const name = String(label); const started = this._timers.get(name);
      if (started !== undefined) this.log(`${name}: ${Date.now() - started}ms`, ...args);
    }
    timeEnd(label = 'default') { const name = String(label); this.timeLog(name); this._timers.delete(name); }
    group(...args) { if (args.length) this.log(...args); this._indent += '  '; }
    groupCollapsed(...args) { this.group(...args); }
    groupEnd() { this._indent = this._indent.substring(0, Math.max(0, this._indent.length - 2)); }
    trace(...args) {
      const error = new Error(consoleFormat(...args));
      this.error(`Trace: ${error.message}` + (error.stack ? `\n${error.stack}` : ''));
    }
    clear() {}
    profile() {}
    profileEnd() {}
    timeStamp() {}
  };
  if (typeof globalThis.console === 'undefined') {
    globalThis.console = new Console(globalThis.__thaw_console_stdout,
                                     globalThis.__thaw_console_stderr);
  }
  const nextTick = (callback, ...args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('process.nextTick callback must be a function');
    }
    queueMicrotask(() => callback(...args));
  };
  if (typeof globalThis.process === 'undefined') globalThis.process = {};
  const processStart = Date.now();
  let processCwd = typeof globalThis.process.cwd === 'function'
    ? globalThis.process.cwd() : '/';
  const processListeners = new Map();
  const processOn = (event, listener, once = false) => {
    if (typeof listener !== 'function') throw new TypeError('listener must be a function');
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    listeners.push({ listener, once });
    processListeners.set(name, listeners);
    return globalThis.process;
  };
  const processOff = (event, listener) => {
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    processListeners.set(name, listeners.filter(entry => entry.listener !== listener));
    return globalThis.process;
  };
  const processEmit = (event, ...args) => {
    const name = String(event);
    const listeners = (processListeners.get(name) || []).slice();
    for (const entry of listeners) {
      if (entry.once) processOff(name, entry.listener);
      entry.listener(...args);
    }
    return listeners.length !== 0;
  };
  const hrtime = previous => {
    const nanoseconds = BigInt(Date.now() - processStart) * 1000000n;
    let seconds = Number(nanoseconds / 1000000000n);
    let remainder = Number(nanoseconds % 1000000000n);
    if (previous !== undefined) {
      seconds -= Number(previous[0]);
      remainder -= Number(previous[1]);
      if (remainder < 0) { seconds--; remainder += 1000000000; }
    }
    return [seconds, remainder];
  };
  hrtime.bigint = () => BigInt(Date.now() - processStart) * 1000000n;
  Object.assign(globalThis.process, {
    argv: globalThis.process.argv || [],
    env: globalThis.process.env || {},
    platform: globalThis.process.platform || 'linux',
    version: globalThis.process.version || '',
    execPath: globalThis.process.execPath || '/usr/bin/node',
    config: globalThis.process.config || { variables: {} },
    versions: Object.assign({ node: '', modules: '', uv: '' },
                            globalThis.process.versions || {}),
    cwd: () => processCwd,
    chdir: directory => {
      const value = String(directory);
      processCwd = value.charAt(0) === '/' ? value : processCwd.replace(/\/$/, '') + '/' + value;
    },
    nextTick,
    uptime: () => (Date.now() - processStart) / 1000,
    hrtime,
    memoryUsage: () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 }),
    cpuUsage: () => ({ user: 0, system: 0 }),
    on: (event, listener) => processOn(event, listener),
    once: (event, listener) => processOn(event, listener, true),
    off: processOff,
    removeListener: processOff,
    emit: processEmit,
    listenerCount: event => (processListeners.get(String(event)) || []).length,
    emitWarning: (warning, options) => {
      const value = warning instanceof Error ? warning : new Error(String(warning));
      value.name = options && options.type ? String(options.type) : 'Warning';
      if (options && options.code) value.code = String(options.code);
      if (!processEmit('warning', value) && globalThis.console
          && typeof globalThis.console.warn === 'function') globalThis.console.warn(value);
    },
    exitCode: globalThis.process.exitCode,
    title: globalThis.process.title || 'thaw'
  });

  if (typeof globalThis.DOMException !== 'function') {
    const legacyCodes = {
      IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4,
      InvalidCharacterError: 5, NoModificationAllowedError: 7,
      NotFoundError: 8, NotSupportedError: 9, InUseAttributeError: 10,
      InvalidStateError: 11, SyntaxError: 12, InvalidModificationError: 13,
      NamespaceError: 14, InvalidAccessError: 15, TypeMismatchError: 17,
      SecurityError: 18, NetworkError: 19, AbortError: 20,
      URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23,
      InvalidNodeTypeError: 24, DataCloneError: 25
    };
    class DOMException extends Error {
      constructor(message = '', name = 'Error') {
        super(String(message));
        this.name = String(name);
        this.code = legacyCodes[this.name] || 0;
      }
    }
    for (const [name, code] of Object.entries(legacyCodes)) {
      const constant = name.replace(/Error$/, '')
        .replace(/([a-z])([A-Z])/g, '$1_$2').toUpperCase() + '_ERR';
      Object.defineProperty(DOMException, constant, { value: code });
      Object.defineProperty(DOMException.prototype, constant, { value: code });
    }
    globalThis.DOMException = DOMException;
  }

  const base64Alphabet =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = input => {
    const text = String(input);
    let output = '';
    for (let offset = 0; offset < text.length; offset += 3) {
      const first = text.charCodeAt(offset);
      const second = offset + 1 < text.length ? text.charCodeAt(offset + 1) : 0;
      const third = offset + 2 < text.length ? text.charCodeAt(offset + 2) : 0;
      if (first > 255 || second > 255 || third > 255) {
        throw new DOMException('btoa input must contain only Latin-1 characters',
                               'InvalidCharacterError');
      }
      const bits = first << 16 | second << 8 | third;
      output += base64Alphabet[bits >> 18 & 63];
      output += base64Alphabet[bits >> 12 & 63];
      output += offset + 1 < text.length ? base64Alphabet[bits >> 6 & 63] : '=';
      output += offset + 2 < text.length ? base64Alphabet[bits & 63] : '=';
    }
    return output;
  };
  globalThis.atob = input => {
    const encoded = String(input).replace(/[\t\n\f\r ]/g, '');
    if (encoded.length % 4 === 1 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) {
      throw new DOMException('invalid base64 input', 'InvalidCharacterError');
    }
    const unpadded = encoded.replace(/=+$/, '');
    if (encoded.includes('=') && encoded.length % 4 !== 0) {
      throw new DOMException('invalid base64 padding', 'InvalidCharacterError');
    }
    let output = '';
    let bits = 0;
    let bitCount = 0;
    for (const character of unpadded) {
      bits = bits << 6 | base64Alphabet.indexOf(character);
      bitCount += 6;
      if (bitCount >= 8) {
        bitCount -= 8;
        output += String.fromCharCode(bits >> bitCount & 255);
      }
    }
    return output;
  };
  const performanceTimeOrigin = Date.now();
  let lastPerformanceNow = 0;
  const performanceNow = () => {
    lastPerformanceNow = Math.max(lastPerformanceNow,
                                  Date.now() - performanceTimeOrigin);
    return lastPerformanceNow;
  };
  if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = { timeOrigin: performanceTimeOrigin,
                               now: performanceNow };
  } else {
    if (typeof globalThis.performance.now !== 'function') {
      globalThis.performance.now = performanceNow;
    }
    if (typeof globalThis.performance.timeOrigin !== 'number') {
      globalThis.performance.timeOrigin = performanceTimeOrigin;
    }
  }
  const performanceEntries = [];
  const performanceObservers = new Set();
  class PerformanceEntry {
    constructor(name, entryType, startTime, duration = 0, detail = null) {
      this.name = String(name); this.entryType = String(entryType);
      this.startTime = Number(startTime); this.duration = Number(duration); this.detail = detail;
    }
    toJSON() { return { name: this.name, entryType: this.entryType, startTime: this.startTime, duration: this.duration, detail: this.detail }; }
  }
  class PerformanceMark extends PerformanceEntry {
    constructor(name, options = {}) { super(name, 'mark', options.startTime === undefined ? performanceNow() : options.startTime, 0, options.detail); }
  }
  class PerformanceMeasure extends PerformanceEntry {
    constructor(name, startTime, duration, detail) { super(name, 'measure', startTime, duration, detail); }
  }
  class PerformanceObserverEntryList {
    constructor(entries) { this._entries = entries; }
    getEntries() { return this._entries.slice().sort((left, right) => left.startTime - right.startTime); }
    getEntriesByType(type) { return this.getEntries().filter(entry => entry.entryType === String(type)); }
    getEntriesByName(name, type) { return this.getEntries().filter(entry => entry.name === String(name) && (type === undefined || entry.entryType === String(type))); }
  }
  const queuePerformanceEntry = entry => {
    performanceEntries.push(entry);
    for (const observer of performanceObservers) {
      if (!observer._types.has(entry.entryType)) continue;
      observer._records.push(entry);
      if (!observer._queued) {
        observer._queued = true;
        queueMicrotask(() => {
          observer._queued = false;
          const records = observer.takeRecords();
          if (records.length) observer._callback(new PerformanceObserverEntryList(records), observer);
        });
      }
    }
    return entry;
  };
  class PerformanceObserver {
    constructor(callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); this._callback = callback; this._types = new Set(); this._records = []; this._queued = false; }
    observe(options = {}) {
      if (options.type) this._types.add(String(options.type));
      if (options.entryTypes) for (const type of options.entryTypes) this._types.add(String(type));
      performanceObservers.add(this);
      if (options.buffered && options.type) this._records.push(...performanceEntries.filter(entry => entry.entryType === String(options.type)));
      if (this._records.length && !this._queued) {
        this._queued = true;
        queueMicrotask(() => { this._queued = false; const records = this.takeRecords(); if (records.length) this._callback(new PerformanceObserverEntryList(records), this); });
      }
    }
    disconnect() { performanceObservers.delete(this); this._records = []; this._types.clear(); }
    takeRecords() { const records = this._records; this._records = []; return records; }
  }
  PerformanceObserver.supportedEntryTypes = ['function', 'mark', 'measure'];
  const entryTime = (value, fallback) => {
    if (value === undefined) return fallback;
    if (typeof value === 'number') return value;
    const entry = performanceEntries.slice().reverse().find(candidate => candidate.name === String(value));
    if (!entry) throw new SyntaxError(`Unknown performance mark: ${value}`);
    return entry.startTime;
  };
  Object.assign(globalThis.performance, {
    mark(name, options = {}) { return queuePerformanceEntry(new PerformanceMark(name, options)); },
    measure(name, startOrOptions, endMark) {
      let start, end, detail;
      if (startOrOptions && typeof startOrOptions === 'object') {
        start = entryTime(startOrOptions.start, 0); detail = startOrOptions.detail;
        end = startOrOptions.duration === undefined ? entryTime(startOrOptions.end, performanceNow()) : start + Number(startOrOptions.duration);
      } else { start = entryTime(startOrOptions, 0); end = entryTime(endMark, performanceNow()); }
      return queuePerformanceEntry(new PerformanceMeasure(name, start, end - start, detail));
    },
    clearMarks(name) { for (let index = performanceEntries.length - 1; index >= 0; index--) if (performanceEntries[index].entryType === 'mark' && (name === undefined || performanceEntries[index].name === String(name))) performanceEntries.splice(index, 1); },
    clearMeasures(name) { for (let index = performanceEntries.length - 1; index >= 0; index--) if (performanceEntries[index].entryType === 'measure' && (name === undefined || performanceEntries[index].name === String(name))) performanceEntries.splice(index, 1); },
    getEntries() { return new PerformanceObserverEntryList(performanceEntries).getEntries(); },
    getEntriesByType(type) { return this.getEntries().filter(entry => entry.entryType === String(type)); },
    getEntriesByName(name, type) { return this.getEntries().filter(entry => entry.name === String(name) && (type === undefined || entry.entryType === String(type))); },
    timerify(callback) {
      return function(...args) { const start = performanceNow(); try { const result = callback.apply(this, args); if (result && typeof result.then === 'function') return Promise.resolve(result).finally(() => queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start))); queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start)); return result; } catch (error) { queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start)); throw error; } };
    },
    toJSON() { return { timeOrigin: this.timeOrigin }; }
  });
  globalThis.PerformanceEntry = PerformanceEntry;
  globalThis.PerformanceMark = PerformanceMark;
  globalThis.PerformanceMeasure = PerformanceMeasure;
  globalThis.PerformanceObserver = PerformanceObserver;
  globalThis.PerformanceObserverEntryList = PerformanceObserverEntryList;
  if (typeof globalThis.structuredClone !== 'function') {
    globalThis.structuredClone = (value, options = {}) => {
      const seen = new Map();
      const transfer = new Set();
      for (const item of options.transfer || []) {
        if (transfer.has(item)) {
          throw new DOMException('transfer list contains duplicate values',
                                 'DataCloneError');
        }
        if (!(item instanceof ArrayBuffer)
            && !(typeof globalThis.MessagePort === 'function'
                 && item instanceof globalThis.MessagePort)) {
          throw new DOMException('value is not transferable', 'DataCloneError');
        }
        transfer.add(item);
      }
      const clone = input => {
        if (input === null || typeof input !== 'object') {
          if (typeof input === 'function' || typeof input === 'symbol') {
            throw new DOMException('value cannot be structured-cloned',
                                   'DataCloneError');
          }
          return input;
        }
        if (seen.has(input)) return seen.get(input);
        if (typeof globalThis.MessagePort === 'function'
            && input instanceof globalThis.MessagePort) {
          if (!transfer.has(input)) {
            throw new DOMException('MessagePort requires a transfer list',
                                   'DataCloneError');
          }
          const output = input.__thawTransfer();
          seen.set(input, output);
          return output;
        }
        let output;
        if (input instanceof Date) {
          output = new Date(input.getTime());
        } else if (input instanceof RegExp) {
          output = new RegExp(input.source, input.flags);
          output.lastIndex = input.lastIndex;
        } else if (input instanceof Map) {
          output = new Map();
          seen.set(input, output);
          for (const [key, entry] of input) output.set(clone(key), clone(entry));
          return output;
        } else if (input instanceof Set) {
          output = new Set();
          seen.set(input, output);
          for (const entry of input) output.add(clone(entry));
          return output;
        } else if (input instanceof ArrayBuffer) {
          output = input.slice(0);
        } else if (ArrayBuffer.isView(input)) {
          if (input instanceof DataView) {
            output = new DataView(input.buffer.slice(input.byteOffset,
              input.byteOffset + input.byteLength));
          } else {
            output = new input.constructor(input);
          }
        } else {
          output = Array.isArray(input) ? [] : {};
        }
        seen.set(input, output);
        if (!(input instanceof Date) && !(input instanceof RegExp)
            && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input)) {
          for (const key of Object.keys(input)) output[key] = clone(input[key]);
        }
        return output;
      };
      const output = clone(value);
      for (const item of transfer) {
        if (item instanceof ArrayBuffer) __thaw_detach_array_buffer(item);
      }
      return output;
    };
  }
  if (typeof globalThis.URLSearchParams !== 'function') {
    const encodeFormPart = value => encodeURIComponent(String(value)).replace(/%20/g, '+');
    const decodeFormPart = value => decodeURIComponent(String(value).replace(/\+/g, ' '));
    globalThis.URLSearchParams = class URLSearchParams {
      constructor(init = '') {
        this.__thawEntries = [];
        this.__thawUpdate = null;
        if (typeof init === 'string') {
          const source = init.charAt(0) === '?' ? init.substring(1) : init;
          if (source !== '') for (const field of source.split('&')) {
            const separator = field.indexOf('=');
            const name = separator < 0 ? field : field.substring(0, separator);
            const value = separator < 0 ? '' : field.substring(separator + 1);
            this.append(decodeFormPart(name), decodeFormPart(value));
          }
        } else if (init != null && typeof init[Symbol.iterator] === 'function') {
          for (const pair of init) {
            if (pair == null || typeof pair[Symbol.iterator] !== 'function') throw new TypeError('URLSearchParams entry must be a pair');
            const values = [...pair];
            if (values.length !== 2) throw new TypeError('URLSearchParams entry must contain two values');
            this.append(values[0], values[1]);
          }
        } else if (init != null && typeof init === 'object') {
          for (const name of Object.keys(init)) this.append(name, init[name]);
        }
      }
      get size() { return this.__thawEntries.length; }
      __thawChanged() { if (this.__thawUpdate) this.__thawUpdate(this.toString()); }
      append(name, value) {
        this.__thawEntries.push([String(name), String(value)]);
        this.__thawChanged();
      }
      delete(name, value) {
        const key = String(name);
        if (arguments.length < 2) this.__thawEntries = this.__thawEntries.filter(entry => entry[0] !== key);
        else {
          const expected = String(value);
          this.__thawEntries = this.__thawEntries.filter(entry => entry[0] !== key || entry[1] !== expected);
        }
        this.__thawChanged();
      }
      get(name) {
        const key = String(name);
        const entry = this.__thawEntries.find(candidate => candidate[0] === key);
        return entry ? entry[1] : null;
      }
      getAll(name) {
        const key = String(name);
        return this.__thawEntries.filter(entry => entry[0] === key).map(entry => entry[1]);
      }
      has(name, value) {
        const key = String(name);
        if (arguments.length < 2) return this.__thawEntries.some(entry => entry[0] === key);
        const expected = String(value);
        return this.__thawEntries.some(entry => entry[0] === key && entry[1] === expected);
      }
      set(name, value) {
        const key = String(name);
        const replacement = String(value);
        const first = this.__thawEntries.findIndex(entry => entry[0] === key);
        if (first < 0) this.__thawEntries.push([key, replacement]);
        else {
          this.__thawEntries[first][1] = replacement;
          this.__thawEntries = this.__thawEntries.filter((entry, index) => entry[0] !== key || index === first);
        }
        this.__thawChanged();
      }
      sort() {
        this.__thawEntries = this.__thawEntries.map((entry, index) => [entry, index])
          .sort((left, right) => left[0][0] < right[0][0] ? -1 : left[0][0] > right[0][0] ? 1 : left[1] - right[1])
          .map(item => item[0]);
        this.__thawChanged();
      }
      entries() { return this.__thawEntries.map(entry => entry.slice())[Symbol.iterator](); }
      keys() { return this.__thawEntries.map(entry => entry[0])[Symbol.iterator](); }
      values() { return this.__thawEntries.map(entry => entry[1])[Symbol.iterator](); }
      forEach(callback, thisArg) {
        for (const entry of this.__thawEntries.slice()) callback.call(thisArg, entry[1], entry[0], this);
      }
      toString() { return this.__thawEntries.map(entry => encodeFormPart(entry[0]) + '=' + encodeFormPart(entry[1])).join('&'); }
      [Symbol.iterator]() { return this.entries(); }
    };
  }
  if (typeof globalThis.URL !== 'function') {
    const normalizePath = path => {
      const absolute = path.charAt(0) === '/';
      const trailing = path.endsWith('/') || path.endsWith('/.') || path.endsWith('/..');
      const output = [];
      for (const part of path.split('/')) {
        if (part === '' || part === '.') continue;
        if (part === '..') output.pop(); else output.push(part);
      }
      let result = (absolute ? '/' : '') + output.join('/');
      if (trailing && result !== '/') result += '/';
      return result || (absolute ? '/' : '');
    };
    const parseAuthority = authority => {
      let userinfo = '', host = authority;
      const at = authority.lastIndexOf('@');
      if (at >= 0) { userinfo = authority.substring(0, at); host = authority.substring(at + 1); }
      let username = '', password = '';
      const colon = userinfo.indexOf(':');
      if (colon < 0) username = userinfo;
      else { username = userinfo.substring(0, colon); password = userinfo.substring(colon + 1); }
      let hostname = host, port = '';
      if (host.charAt(0) === '[') {
        const close = host.indexOf(']');
        if (close < 0) throw new TypeError('Invalid URL');
        hostname = host.substring(0, close + 1);
        if (host.charAt(close + 1) === ':') port = host.substring(close + 2);
      } else {
        const hostColon = host.lastIndexOf(':');
        if (hostColon >= 0) { hostname = host.substring(0, hostColon); port = host.substring(hostColon + 1); }
      }
      if (port !== '' && !/^\d+$/.test(port)) throw new TypeError('Invalid URL');
      return { username, password, hostname: hostname.toLowerCase(), port };
    };
    globalThis.URL = class URL {
      constructor(input, base) {
        const value = String(input);
        const absolute = value.match(/^([A-Za-z][A-Za-z\d+.-]*:)(?:\/\/([^\/?#]*))?([^?#]*)(\?[^#]*)?(#.*)?$/);
        if (absolute) {
          this.__thawProtocol = absolute[1].toLowerCase();
          const authority = absolute[2] === undefined ? '' : absolute[2];
          const parsed = parseAuthority(authority);
          this.__thawUsername = parsed.username;
          this.__thawPassword = parsed.password;
          this.__thawHostname = parsed.hostname;
          this.__thawPort = parsed.port;
          this.__thawPathname = normalizePath(absolute[3] || (authority !== '' ? '/' : ''));
          this.__thawSearch = absolute[4] || '';
          this.__thawHash = absolute[5] || '';
        } else {
          if (base === undefined) throw new TypeError('Invalid URL');
          const parent = base instanceof URL ? base : new URL(base);
          this.__thawProtocol = parent.protocol;
          this.__thawUsername = parent.username;
          this.__thawPassword = parent.password;
          this.__thawHostname = parent.hostname;
          this.__thawPort = parent.port;
          const match = value.match(/^([^?#]*)(\?[^#]*)?(#.*)?$/);
          const relativePath = match[1];
          if (relativePath === '') this.__thawPathname = parent.pathname;
          else if (relativePath.charAt(0) === '/') this.__thawPathname = normalizePath(relativePath);
          else this.__thawPathname = normalizePath(parent.pathname.substring(0, parent.pathname.lastIndexOf('/') + 1) + relativePath);
          this.__thawSearch = match[2] !== undefined ? match[2] : (relativePath === '' ? parent.search : '');
          this.__thawHash = match[3] || '';
        }
        this.__thawNormalizePort();
        this.__thawRefreshParams();
      }
      __thawNormalizePort() {
        if ((this.__thawProtocol === 'http:' && this.__thawPort === '80')
            || (this.__thawProtocol === 'https:' && this.__thawPort === '443')) this.__thawPort = '';
      }
      __thawRefreshParams() {
        const params = new URLSearchParams(this.__thawSearch);
        params.__thawUpdate = value => { this.__thawSearch = value === '' ? '' : '?' + value; };
        this.__thawSearchParams = params;
      }
      get protocol() { return this.__thawProtocol; }
      set protocol(value) { this.__thawProtocol = String(value).replace(/:$/, '').toLowerCase() + ':'; this.__thawNormalizePort(); }
      get username() { return this.__thawUsername; }
      set username(value) { this.__thawUsername = String(value); }
      get password() { return this.__thawPassword; }
      set password(value) { this.__thawPassword = String(value); }
      get hostname() { return this.__thawHostname; }
      set hostname(value) { this.__thawHostname = String(value).toLowerCase(); }
      get port() { return this.__thawPort; }
      set port(value) { const port = String(value); if (port !== '' && !/^\d+$/.test(port)) return; this.__thawPort = port; this.__thawNormalizePort(); }
      get host() { return this.hostname + (this.port ? ':' + this.port : ''); }
      set host(value) { const parsed = parseAuthority(String(value)); this.__thawHostname = parsed.hostname; this.__thawPort = parsed.port; this.__thawNormalizePort(); }
      get pathname() { return this.__thawPathname; }
      set pathname(value) { this.__thawPathname = normalizePath(String(value).charAt(0) === '/' ? String(value) : '/' + String(value)); }
      get search() { return this.__thawSearch; }
      set search(value) { const search = String(value); this.__thawSearch = search === '' ? '' : (search.charAt(0) === '?' ? search : '?' + search); this.__thawRefreshParams(); }
      get searchParams() { return this.__thawSearchParams; }
      get hash() { return this.__thawHash; }
      set hash(value) { const hash = String(value); this.__thawHash = hash === '' ? '' : (hash.charAt(0) === '#' ? hash : '#' + hash); }
      get origin() { return this.__thawHostname === '' || this.__thawProtocol === 'file:' ? 'null' : this.protocol + '//' + this.host; }
      get href() {
        const credentials = this.username || this.password ? this.username + (this.password ? ':' + this.password : '') + '@' : '';
        const authority = this.hostname !== '' || this.protocol === 'file:' ? '//' + credentials + this.host : '';
        return this.protocol + authority + this.pathname + this.search + this.hash;
      }
      set href(value) { const replacement = new URL(value); Object.assign(this, replacement); this.__thawRefreshParams(); }
      toString() { return this.href; }
      toJSON() { return this.href; }
      static canParse(input, base) { try { new URL(input, base); return true; } catch (_) { return false; } }
      static parse(input, base) { try { return new URL(input, base); } catch (_) { return null; } }
    };
  }
  if (typeof globalThis.Blob !== 'function') {
    globalThis.Blob = class Blob {
      constructor(parts = [], options = {}) {
        const chunks = Array.from(parts, part => {
          if (part instanceof Blob) return part.__thawBytes;
          if (part instanceof ArrayBuffer) return new Uint8Array(part);
          if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
          return new TextEncoder().encode(String(part));
        });
        const size = chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
        this.__thawBytes = new Uint8Array(size);
        let offset = 0;
        for (const chunk of chunks) { this.__thawBytes.set(chunk, offset); offset += chunk.byteLength; }
        this.size = size;
        this.type = String(options.type || '').toLowerCase();
      }
      arrayBuffer() { const copy = this.__thawBytes.slice(); return Promise.resolve(copy.buffer); }
      bytes() { return Promise.resolve(this.__thawBytes.slice()); }
      text() { return Promise.resolve(new TextDecoder().decode(this.__thawBytes)); }
      slice(start = 0, end = this.size, type = '') {
        const from = start < 0 ? Math.max(this.size + Number(start), 0) : Math.min(Number(start), this.size);
        const to = end < 0 ? Math.max(this.size + Number(end), 0) : Math.min(Number(end), this.size);
        return new Blob([this.__thawBytes.slice(from, Math.max(from, to))], { type });
      }
      stream() {
        const bytes = this.__thawBytes.slice(); let done = false;
        return { getReader() { return { read() { if (done) return Promise.resolve({ value: undefined, done: true }); done = true; return Promise.resolve({ value: bytes, done: false }); }, releaseLock() {} }; }, [Symbol.asyncIterator]() { return { next() { if (done) return Promise.resolve({ value: undefined, done: true }); done = true; return Promise.resolve({ value: bytes, done: false }); }, [Symbol.asyncIterator]() { return this; } }; } };
      }
    };
    globalThis.File = class File extends Blob {
      constructor(parts, name, options = {}) { super(parts, options); this.name = String(name); this.lastModified = options.lastModified === undefined ? Date.now() : Number(options.lastModified); this.webkitRelativePath = ''; }
    };
  }
  if (typeof globalThis.Event !== 'function') {
    globalThis.Event = class Event {
      constructor(type, options = {}) {
        this.type = String(type);
        this.bubbles = Boolean(options.bubbles);
        this.cancelable = Boolean(options.cancelable);
        this.composed = Boolean(options.composed);
        this.target = null;
        this.currentTarget = null;
        this.defaultPrevented = false;
        this.__thawImmediateStopped = false;
      }
      preventDefault() {
        if (this.cancelable) this.defaultPrevented = true;
      }
      stopPropagation() {}
      stopImmediatePropagation() { this.__thawImmediateStopped = true; }
    };
  }
  if (typeof globalThis.EventTarget !== 'function') {
    globalThis.EventTarget = class EventTarget {
      constructor() { this.__thawListeners = new Map(); }
      addEventListener(type, callback, options = {}) {
        if (callback == null) return;
        if (typeof callback !== 'function'
            && typeof callback.handleEvent !== 'function') {
          throw new TypeError('event listener must be callable');
        }
        const name = String(type);
        const listeners = this.__thawListeners.get(name) || [];
        if (!listeners.some(entry => entry.callback === callback)) {
          listeners.push({ callback, once: Boolean(options && options.once) });
          this.__thawListeners.set(name, listeners);
        }
      }
      removeEventListener(type, callback) {
        const name = String(type);
        const listeners = this.__thawListeners.get(name);
        if (listeners) {
          this.__thawListeners.set(name,
            listeners.filter(entry => entry.callback !== callback));
        }
      }
      dispatchEvent(event) {
        if (!(event instanceof Event)) throw new TypeError('expected an Event');
        event.target = this;
        event.currentTarget = this;
        event.__thawImmediateStopped = false;
        const listeners = (this.__thawListeners.get(event.type) || []).slice();
        for (const entry of listeners) {
          if (entry.once) this.removeEventListener(event.type, entry.callback);
          if (typeof entry.callback === 'function') entry.callback.call(this, event);
          else entry.callback.handleEvent(event);
          if (event.__thawImmediateStopped) break;
        }
        event.currentTarget = null;
        return !event.defaultPrevented;
      }
    };
  }
  if (typeof globalThis.MessageEvent !== 'function') {
    globalThis.MessageEvent = class MessageEvent extends Event {
      constructor(type, options = {}) {
        super(type, options);
        this.data = options.data === undefined ? null : options.data;
        this.origin = options.origin === undefined ? '' : String(options.origin);
        this.lastEventId = options.lastEventId === undefined ? '' : String(options.lastEventId);
        this.source = options.source === undefined ? null : options.source;
        this.ports = options.ports === undefined ? [] : Array.from(options.ports);
      }
    };
  }
  if (typeof globalThis.ReadableStream !== 'function') {
    class ReadableStreamDefaultController {
      constructor(stream) { this._stream = stream; }
      enqueue(chunk) {
        const stream = this._stream;
        if (stream._state !== 'readable') throw new TypeError('stream is not readable');
        if (stream._reads.length) stream._reads.shift().resolve({ value: chunk, done: false });
        else stream._queue.push(chunk);
      }
      close() {
        const stream = this._stream; if (stream._state !== 'readable') return;
        stream._state = 'closed'; while (stream._reads.length) stream._reads.shift().resolve({ value: undefined, done: true }); stream._resolveClosed();
      }
      error(error) {
        const stream = this._stream; if (stream._state !== 'readable') return;
        stream._state = 'errored'; stream._error = error; while (stream._reads.length) stream._reads.shift().reject(error); stream._rejectClosed(error);
      }
      get desiredSize() { return this._stream._state === 'readable' ? 1 - this._stream._queue.length : null; }
    }
    class ReadableStreamDefaultReader {
      constructor(stream) { if (stream.locked) throw new TypeError('stream is locked'); this._stream = stream; stream._reader = this; this.closed = stream._closed; }
      read() {
        const stream = this._stream; if (!stream) return Promise.reject(new TypeError('reader is released'));
        if (stream._queue.length) return Promise.resolve({ value: stream._queue.shift(), done: false });
        if (stream._state === 'closed') return Promise.resolve({ value: undefined, done: true });
        if (stream._state === 'errored') return Promise.reject(stream._error);
        const result = new Promise((resolve, reject) => stream._reads.push({ resolve, reject }));
        if (typeof stream._source.pull === 'function') Promise.resolve(stream._source.pull(stream._controller)).catch(error => stream._controller.error(error));
        return result;
      }
      cancel(reason) { return this._stream.cancel(reason); }
      releaseLock() { if (this._stream) this._stream._reader = null; this._stream = null; }
    }
    globalThis.ReadableStream = class ReadableStream {
      constructor(source = {}) {
        this._source = source; this._queue = []; this._reads = []; this._state = 'readable'; this._error = undefined; this._reader = null;
        this._closed = new Promise((resolve, reject) => { this._resolveClosed = resolve; this._rejectClosed = reject; });
        this._closed.catch(() => {}); this._controller = new ReadableStreamDefaultController(this);
        try { if (typeof source.start === 'function') Promise.resolve(source.start(this._controller)).catch(error => this._controller.error(error)); } catch (error) { this._controller.error(error); }
      }
      get locked() { return this._reader !== null; }
      getReader() { return new ReadableStreamDefaultReader(this); }
      cancel(reason) { if (this.locked) return Promise.reject(new TypeError('stream is locked')); if (this._state === 'closed') return Promise.resolve(); if (typeof this._source.cancel === 'function') return Promise.resolve(this._source.cancel(reason)).then(() => this._controller.close()); this._controller.close(); return Promise.resolve(); }
      async pipeTo(destination, options = {}) { const reader = this.getReader(), writer = destination.getWriter(); try { while (true) { const result = await reader.read(); if (result.done) break; await writer.write(result.value); } if (!options.preventClose) await writer.close(); } catch (error) { if (!options.preventAbort) await writer.abort(error); throw error; } finally { reader.releaseLock(); writer.releaseLock(); } }
      pipeThrough(transform, options) { this.pipeTo(transform.writable, options).catch(error => transform.readable._controller.error(error)); return transform.readable; }
      values(options = {}) { const reader = this.getReader(); return { next: () => reader.read(), return: async () => { if (!options.preventCancel) await reader.cancel(); reader.releaseLock(); return { value: undefined, done: true }; }, [Symbol.asyncIterator]() { return this; } }; }
      [Symbol.asyncIterator]() { return this.values(); }
    };
    globalThis.ReadableStreamDefaultController = ReadableStreamDefaultController;
    globalThis.ReadableStreamDefaultReader = ReadableStreamDefaultReader;
    class WritableStreamDefaultWriter {
      constructor(stream) { if (stream.locked) throw new TypeError('stream is locked'); this._stream = stream; stream._writer = this; this.ready = Promise.resolve(); this.closed = stream._closed; }
      write(chunk) { return this._stream._write(chunk); }
      close() { return this._stream.close(); }
      abort(reason) { return this._stream.abort(reason); }
      releaseLock() { if (this._stream) this._stream._writer = null; this._stream = null; }
      get desiredSize() { return this._stream && this._stream._state === 'writable' ? 1 : null; }
    }
    globalThis.WritableStream = class WritableStream {
      constructor(sink = {}) { this._sink = sink; this._state = 'writable'; this._writer = null; this._chain = Promise.resolve(); this._closed = new Promise((resolve, reject) => { this._resolveClosed = resolve; this._rejectClosed = reject; }); this._closed.catch(() => {}); if (typeof sink.start === 'function') this._chain = this._chain.then(() => sink.start(this)); }
      get locked() { return this._writer !== null; }
      getWriter() { return new WritableStreamDefaultWriter(this); }
      _write(chunk) { if (this._state !== 'writable') return Promise.reject(new TypeError('stream is not writable')); this._chain = this._chain.then(() => typeof this._sink.write === 'function' ? this._sink.write(chunk, this) : undefined); return this._chain; }
      close() { if (this._state !== 'writable') return this._closed; this._state = 'closed'; this._chain = this._chain.then(() => typeof this._sink.close === 'function' ? this._sink.close() : undefined).then(() => this._resolveClosed(), error => { this._rejectClosed(error); throw error; }); return this._chain; }
      abort(reason) { this._state = 'errored'; this._chain = this._chain.then(() => typeof this._sink.abort === 'function' ? this._sink.abort(reason) : undefined).then(() => this._rejectClosed(reason)); return this._chain; }
    };
    globalThis.WritableStreamDefaultWriter = WritableStreamDefaultWriter;
    globalThis.TransformStream = class TransformStream {
      constructor(transformer = {}) { let controller; this.readable = new ReadableStream({ start(value) { controller = value; } }); this.writable = new WritableStream({ write(chunk) { return typeof transformer.transform === 'function' ? transformer.transform(chunk, controller) : controller.enqueue(chunk); }, close() { return Promise.resolve(typeof transformer.flush === 'function' ? transformer.flush(controller) : undefined).then(() => controller.close()); }, abort(error) { controller.error(error); } }); }
    };
    const hexFromBytes = value => Array.from(value, byte => byte.toString(16).padStart(2, '0')).join('');
    const bytesFromHex = value => new Uint8Array(String(value).match(/../g)?.map(pair => parseInt(pair, 16)) || []);
    const compressionTransform = (operation, format) => {
      const normalized = String(format);
      if (normalized !== 'gzip' && normalized !== 'deflate' && normalized !== 'deflate-raw') throw new TypeError('Unsupported compression format');
      const chunks = [];
      return new TransformStream({
        transform(chunk) { chunks.push(chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk)); },
        flush(controller) {
          const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0), input = new Uint8Array(length); let offset = 0;
          for (const chunk of chunks) { input.set(chunk, offset); offset += chunk.byteLength; }
          const nativeFormat = normalized === 'deflate-raw' ? 'deflateRaw' : normalized;
          controller.enqueue(bytesFromHex(__thaw_zlib_hex(operation, nativeFormat, hexFromBytes(input))));
        }
      });
    };
    globalThis.CompressionStream = class CompressionStream { constructor(format) { const stream = compressionTransform('compress', format); this.readable = stream.readable; this.writable = stream.writable; } };
    globalThis.DecompressionStream = class DecompressionStream { constructor(format) { const stream = compressionTransform('decompress', format); this.readable = stream.readable; this.writable = stream.writable; } };
    globalThis.ByteLengthQueuingStrategy = class ByteLengthQueuingStrategy { constructor(options) { this.highWaterMark = Number(options.highWaterMark); } size(chunk) { return chunk.byteLength; } };
    globalThis.CountQueuingStrategy = class CountQueuingStrategy { constructor(options) { this.highWaterMark = Number(options.highWaterMark); } size() { return 1; } };
    globalThis.TextEncoderStream = class TextEncoderStream { constructor() { const encoder = new TextEncoder(); const transform = new TransformStream({ transform(chunk, controller) { controller.enqueue(encoder.encode(String(chunk))); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = 'utf-8'; } };
    globalThis.TextDecoderStream = class TextDecoderStream { constructor(label = 'utf-8', options = {}) { const decoder = new TextDecoder(label, options); const chunks = []; const transform = new TransformStream({ transform(chunk) { chunks.push(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength)); }, flush(controller) { const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0); const value = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { value.set(chunk, offset); offset += chunk.byteLength; } const text = decoder.decode(value); if (text) controller.enqueue(text); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = decoder.encoding || String(label).toLowerCase(); this.fatal = Boolean(options.fatal); this.ignoreBOM = Boolean(options.ignoreBOM); } };
  }
  if (typeof globalThis.MessageChannel !== 'function') {
    class MessagePort extends EventTarget {
      constructor() {
        super();
        this.__thawPeer = null;
        this.__thawQueue = [];
        this.__thawScheduled = false;
        this.__thawClosed = false;
        this.__thawRefed = true;
        this.__thawNodeListeners = new Map();
        this.__thawOnMessage = null;
        this.__thawOnMessageError = null;
      }
      postMessage(value, transfer = []) {
        if (this.__thawClosed || !this.__thawPeer || this.__thawPeer.__thawClosed) return;
        let copy;
        try { copy = structuredClone(value, { transfer }); }
        catch (error) {
          const peer = this.__thawPeer;
          queueMicrotask(() => peer.__thawDispatchError(error));
          return;
        }
        const ports = transfer.filter(item => item instanceof MessagePort)
          .map(item => item.__thawTransferredPort || item);
        this.__thawPeer.__thawQueue.push({ data: copy, ports });
        this.__thawPeer.__thawSchedule();
      }
      __thawTransfer() {
        if (this.__thawClosed || !this.__thawPeer) {
          throw new DOMException('MessagePort is already detached', 'DataCloneError');
        }
        const transferred = new MessagePort();
        transferred.__thawPeer = this.__thawPeer;
        transferred.__thawQueue = this.__thawQueue;
        this.__thawPeer.__thawPeer = transferred;
        this.__thawQueue = [];
        this.__thawPeer = null;
        this.__thawClosed = true;
        this.__thawTransferredPort = transferred;
        return transferred;
      }
      __thawSchedule() {
        if (this.__thawScheduled || this.__thawClosed) return;
        this.__thawScheduled = true;
        queueMicrotask(() => {
          this.__thawScheduled = false;
          while (!this.__thawClosed && this.__thawQueue.length) {
            const record = this.__thawQueue.shift();
            const event = new MessageEvent('message', { data: record.data, ports: record.ports });
            this.dispatchEvent(event);
            if (typeof this.__thawOnMessage === 'function') this.__thawOnMessage.call(this, event);
            for (const listener of (this.__thawNodeListeners.get('message') || []).slice()) listener.call(this, record.data);
          }
        });
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
        if (typeof this.__thawOnMessageError === 'function') this.__thawOnMessageError.call(this, event);
        for (const listener of (this.__thawNodeListeners.get('messageerror') || []).slice()) listener.call(this, error);
      }
      start() { this.__thawSchedule(); }
      close() {
        if (this.__thawClosed) return;
        this.__thawClosed = true;
        this.__thawQueue.length = 0;
        this.dispatchEvent(new Event('close'));
        for (const listener of (this.__thawNodeListeners.get('close') || []).slice()) listener.call(this);
      }
      ref() { this.__thawRefed = true; return this; }
      unref() { this.__thawRefed = false; return this; }
      hasRef() { return this.__thawRefed; }
      on(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; list.push(listener); this.__thawNodeListeners.set(key, list); if (key === 'message') this.start(); return this; }
      once(name, listener) { const wrapped = (...args) => { this.off(name, wrapped); listener.apply(this, args); }; wrapped.listener = listener; return this.on(name, wrapped); }
      off(name, listener) { const key = String(name); const list = this.__thawNodeListeners.get(key) || []; this.__thawNodeListeners.set(key, list.filter(entry => entry !== listener && entry.listener !== listener)); return this; }
      addListener(name, listener) { return this.on(name, listener); }
      removeListener(name, listener) { return this.off(name, listener); }
      removeAllListeners(name) { if (name === undefined) this.__thawNodeListeners.clear(); else this.__thawNodeListeners.delete(String(name)); return this; }
      set onmessage(listener) { this.__thawOnMessage = listener; if (listener) this.start(); }
      get onmessage() { return this.__thawOnMessage; }
      set onmessageerror(listener) { this.__thawOnMessageError = listener; }
      get onmessageerror() { return this.__thawOnMessageError; }
    }
    globalThis.MessagePort = MessagePort;
    globalThis.MessageChannel = class MessageChannel {
      constructor() {
        this.port1 = new MessagePort();
        this.port2 = new MessagePort();
        this.port1.__thawPeer = this.port2;
        this.port2.__thawPeer = this.port1;
      }
    };
  }
  if (typeof globalThis.BroadcastChannel !== 'function') {
    const broadcastChannels = new Map();
    globalThis.BroadcastChannel = class BroadcastChannel extends EventTarget {
      constructor(name) {
        super();
        this.name = String(name);
        this.__thawClosed = false;
        this.__thawOnMessage = null;
        this.__thawOnMessageError = null;
        this.__thawQueue = [];
        const channels = broadcastChannels.get(this.name) || new Set();
        channels.add(this);
        broadcastChannels.set(this.name, channels);
      }
      postMessage(value) {
        if (this.__thawClosed) throw new DOMException('BroadcastChannel is closed', 'InvalidStateError');
        const channels = broadcastChannels.get(this.name) || [];
        for (const channel of channels) {
          if (channel === this || channel.__thawClosed) continue;
          let copy;
          try { copy = structuredClone(value); }
          catch (error) {
            queueMicrotask(() => channel.__thawDispatchError(error));
            continue;
          }
          channel.__thawQueue.push({ data: copy });
          queueMicrotask(() => {
            if (channel.__thawClosed) return;
            const record = channel.__thawQueue.shift();
            if (!record) return;
            const event = new MessageEvent('message', { data: record.data });
            channel.dispatchEvent(event);
            if (typeof channel.__thawOnMessage === 'function') channel.__thawOnMessage.call(channel, event);
          });
        }
      }
      __thawDispatchError(error) {
        const event = new MessageEvent('messageerror', { data: error });
        this.dispatchEvent(event);
        if (typeof this.__thawOnMessageError === 'function') this.__thawOnMessageError.call(this, event);
      }
      close() {
        if (this.__thawClosed) return;
        this.__thawClosed = true;
        const channels = broadcastChannels.get(this.name);
        if (channels) {
          channels.delete(this);
          if (channels.size === 0) broadcastChannels.delete(this.name);
        }
      }
      ref() { return this; }
      unref() { return this; }
      set onmessage(listener) { this.__thawOnMessage = listener; }
      get onmessage() { return this.__thawOnMessage; }
      set onmessageerror(listener) { this.__thawOnMessageError = listener; }
      get onmessageerror() { return this.__thawOnMessageError; }
    };
  }
  if (typeof globalThis.AbortController !== 'function') {
    const abortError = message => {
      return new DOMException(message || 'This operation was aborted', 'AbortError');
    };
    const timeoutError = () => {
      return new DOMException('The operation timed out', 'TimeoutError');
    };
    class AbortSignal extends EventTarget {
      constructor() {
        super();
        this.aborted = false;
        this.reason = undefined;
        this.onabort = null;
      }
      throwIfAborted() {
        if (this.aborted) throw this.reason;
      }
      __thawAbort(reason) {
        if (this.aborted) return;
        this.aborted = true;
        this.reason = reason === undefined ? abortError() : reason;
        const event = new Event('abort');
        if (typeof this.onabort === 'function') this.onabort.call(this, event);
        this.dispatchEvent(event);
      }
      static abort(reason) {
        const signal = new AbortSignal();
        signal.__thawAbort(reason);
        return signal;
      }
      static timeout(milliseconds) {
        const signal = new AbortSignal();
        setTimeout(() => signal.__thawAbort(timeoutError()), milliseconds);
        return signal;
      }
      static any(signals) {
        const combined = new AbortSignal();
        const subscriptions = [];
        const sources = [...signals];
        for (const signal of sources) {
          if (!signal || typeof signal.addEventListener !== 'function') {
            throw new TypeError('AbortSignal.any expects AbortSignal values');
          }
        }
        const finish = signal => {
          if (combined.aborted) return;
          combined.__thawAbort(signal.reason);
          for (const [source, listener] of subscriptions) {
            source.removeEventListener('abort', listener);
          }
        };
        for (const signal of sources) {
          if (signal.aborted) {
            finish(signal);
            break;
          }
          const listener = () => finish(signal);
          subscriptions.push([signal, listener]);
          signal.addEventListener('abort', listener, { once: true });
        }
        return combined;
      }
    }
    class AbortController {
      constructor() { this.signal = new AbortSignal(); }
      abort(reason) { this.signal.__thawAbort(reason); }
    }
    globalThis.AbortSignal = AbortSignal;
    globalThis.AbortController = AbortController;
  }
  globalThis.__thaw_next_timer_delay = () => {
    let due = Infinity;
    for (const timer of timers.values()) due = Math.min(due, timer.due);
    return due === Infinity ? -1 : Math.max(0, due - Date.now());
  };
  globalThis.__thaw_run_due_timers = () => {
    const now = Date.now();
    const due = [...timers.entries()]
      .filter(([, timer]) => timer.due <= now)
      .sort((a, b) => a[1].due - b[1].due || a[0] - b[0]);
    for (const [id, timer] of due) {
      if (!timers.has(id)) continue;
      if (timer.repeat) timer.due = Date.now() + timer.milliseconds;
      else timers.delete(id);
      timer.callback(...timer.args);
    }
    return due.length;
  };

  const encodeUtf8 = input => {
    const bytes = [];
    for (const character of String(input)) {
      const code = character.codePointAt(0);
      if (code <= 0x7f) bytes.push(code);
      else if (code <= 0x7ff) bytes.push(0xc0 | code >> 6, 0x80 | code & 0x3f);
      else if (code <= 0xffff) bytes.push(0xe0 | code >> 12,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
      else bytes.push(0xf0 | code >> 18, 0x80 | code >> 12 & 0x3f,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
    }
    return bytes;
  };
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') { return Uint8Array.from(encodeUtf8(input)); }
    encodeInto(input, destination) {
      if (!(destination instanceof Uint8Array)) {
        throw new TypeError('destination must be a Uint8Array');
      }
      let read = 0;
      let written = 0;
      for (const character of String(input)) {
        const bytes = encodeUtf8(character);
        if (written + bytes.length > destination.length) break;
        destination.set(bytes, written);
        written += bytes.length;
        read += character.length;
      }
      return { read, written };
    }
  };

  const replacement = '\ufffd';
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = 'utf-8', options = {}) {
      const normalized = String(label).trim().toLowerCase().replace(/[_\s]/g, '-');
      if (!['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(normalized)) {
        throw new RangeError(`unsupported encoding: ${label}`);
      }
      this.fatal = Boolean(options.fatal);
      this.ignoreBOM = Boolean(options.ignoreBOM);
      this.encoding = 'utf-8';
    }
    decode(input = new Uint8Array(), options = {}) {
      if (options.stream) throw new TypeError('streaming TextDecoder is not supported');
      const bytes = input instanceof ArrayBuffer
        ? new Uint8Array(input)
        : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      let output = '';
      let index = 0;
      const invalid = () => {
        if (this.fatal) throw new TypeError('invalid UTF-8 data');
        output += replacement;
      };
      while (index < bytes.length) {
        const first = bytes[index++];
        if (first <= 0x7f) { output += String.fromCodePoint(first); continue; }
        let length, code, minimum;
        if (first >= 0xc2 && first <= 0xdf) { length = 1; code = first & 0x1f; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { length = 2; code = first & 0x0f; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { length = 3; code = first & 7; minimum = 0x10000; }
        else { invalid(); continue; }
        if (index + length > bytes.length) { invalid(); break; }
        let valid = true;
        for (let offset = 0; offset < length; offset++) {
          const continuation = bytes[index + offset];
          if ((continuation & 0xc0) !== 0x80) { valid = false; break; }
          code = code << 6 | continuation & 0x3f;
        }
        if (!valid || code < minimum || code > 0x10ffff ||
            (code >= 0xd800 && code <= 0xdfff)) { invalid(); continue; }
        index += length;
        output += String.fromCodePoint(code);
      }
      if (!this.ignoreBOM && output.charCodeAt(0) === 0xfeff) output = output.slice(1);
      return output;
    }
  };

  const normalizeBufferEncoding = encoding => String(encoding || 'utf8')
    .toLowerCase().replace(/[-_]/g, '');
  const bufferBytes = (value, encoding = 'utf8') => {
    const normalized = normalizeBufferEncoding(encoding);
    if (typeof value !== 'string') return Array.from(value);
    if (normalized === 'utf8' || normalized === 'utf') return encodeUtf8(value);
    if (normalized === 'hex') {
      const bytes = [];
      for (let index = 0; index + 1 < value.length && /^[0-9a-f]{2}$/i.test(value.substring(index, index + 2)); index += 2) {
        bytes.push(parseInt(value.substring(index, index + 2), 16));
      }
      return bytes;
    }
    if (normalized === 'base64' || normalized === 'base64url') {
      let encoded = value.replace(/-/g, '+').replace(/_/g, '/');
      while (encoded.length % 4) encoded += '=';
      return Array.from(atob(encoded), character => character.charCodeAt(0));
    }
    if (normalized === 'latin1' || normalized === 'binary' || normalized === 'ascii') {
      return Array.from(value, character => character.charCodeAt(0) & (normalized === 'ascii' ? 0x7f : 0xff));
    }
    if (normalized === 'utf16le' || normalized === 'ucs2') {
      const bytes = [];
      for (let index = 0; index < value.length; index++) {
        const code = value.charCodeAt(index); bytes.push(code & 255, code >> 8);
      }
      return bytes;
    }
    throw new TypeError(`Unknown encoding: ${encoding}`);
  };
  globalThis.Buffer = class Buffer extends Uint8Array {
    static from(value, encodingOrOffset, length) {
      if (typeof value === 'string') return new Buffer(bufferBytes(value, encodingOrOffset));
      if (value instanceof ArrayBuffer) {
        const offset = Number(encodingOrOffset || 0);
        return new Buffer(value, offset, length === undefined ? value.byteLength - offset : Number(length));
      }
      if (ArrayBuffer.isView(value)) return new Buffer(Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength)));
      if (value && value.type === 'Buffer' && Array.isArray(value.data)) return new Buffer(value.data);
      return new Buffer(Array.from(value || []));
    }
    static alloc(size, fill = 0, encoding) {
      const buffer = new Buffer(Number(size));
      if (fill !== 0) buffer.fill(fill, 0, buffer.length, encoding);
      return buffer;
    }
    static allocUnsafe(size) { return new Buffer(Number(size)); }
    static allocUnsafeSlow(size) { return new Buffer(Number(size)); }
    static isBuffer(value) { return value instanceof Buffer; }
    static isEncoding(value) {
      return ['utf8', 'utf', 'hex', 'base64', 'base64url', 'latin1', 'binary', 'ascii', 'utf16le', 'ucs2']
        .includes(normalizeBufferEncoding(value));
    }
    static byteLength(value, encoding) {
      if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) return value.byteLength;
      return bufferBytes(String(value), encoding).length;
    }
    static compare(left, right) {
      const length = Math.min(left.length, right.length);
      for (let index = 0; index < length; index++) if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
      return left.length === right.length ? 0 : left.length < right.length ? -1 : 1;
    }
    static concat(list, totalLength) {
      if (!Array.isArray(list)) throw new TypeError('list must be an Array');
      const length = totalLength === undefined ? list.reduce((sum, item) => sum + item.length, 0) : Number(totalLength);
      const output = Buffer.alloc(length); let offset = 0;
      for (const item of list) { offset += Buffer.from(item).copy(output, offset); if (offset >= length) break; }
      return output;
    }
    toString(encoding = 'utf8', start = 0, end = this.length) {
      const bytes = this.subarray(Math.max(0, Number(start)), Math.min(this.length, Number(end)));
      const normalized = normalizeBufferEncoding(encoding);
      if (normalized === 'utf8' || normalized === 'utf') return new TextDecoder().decode(bytes);
      if (normalized === 'hex') return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
      if (normalized === 'base64' || normalized === 'base64url') {
        let value = btoa(Array.from(bytes, byte => String.fromCharCode(byte)).join(''));
        if (normalized === 'base64url') value = value.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        return value;
      }
      if (normalized === 'latin1' || normalized === 'binary') return Array.from(bytes, byte => String.fromCharCode(byte)).join('');
      if (normalized === 'ascii') return Array.from(bytes, byte => String.fromCharCode(byte & 0x7f)).join('');
      if (normalized === 'utf16le' || normalized === 'ucs2') {
        let value = ''; for (let index = 0; index + 1 < bytes.length; index += 2) value += String.fromCharCode(bytes[index] | bytes[index + 1] << 8); return value;
      }
      throw new TypeError(`Unknown encoding: ${encoding}`);
    }
    toJSON() { return { type: 'Buffer', data: Array.from(this) }; }
    equals(other) { return Buffer.compare(this, other) === 0; }
    compare(other) { return Buffer.compare(this, other); }
    copy(target, targetStart = 0, sourceStart = 0, sourceEnd = this.length) {
      const bytes = this.subarray(Number(sourceStart), Number(sourceEnd));
      const count = Math.min(bytes.length, target.length - Number(targetStart));
      target.set(bytes.subarray(0, count), Number(targetStart)); return Math.max(0, count);
    }
    fill(value, start = 0, end = this.length, encoding) {
      const pattern = typeof value === 'string' ? bufferBytes(value, encoding) : [Number(value) & 255];
      if (pattern.length === 0) return this;
      for (let index = Number(start); index < Number(end); index++) this[index] = pattern[(index - Number(start)) % pattern.length];
      return this;
    }
    indexOf(value, byteOffset = 0, encoding) {
      const needle = bufferBytes(typeof value === 'number' ? [value & 255] : value, encoding);
      outer: for (let index = Math.max(0, Number(byteOffset)); index <= this.length - needle.length; index++) {
        for (let part = 0; part < needle.length; part++) if (this[index + part] !== needle[part]) continue outer;
        return index;
      }
      return -1;
    }
    lastIndexOf(value, byteOffset = this.length, encoding) {
      const needle = bufferBytes(typeof value === 'number' ? [value & 255] : value, encoding);
      outer: for (let index = Math.min(Number(byteOffset), this.length - needle.length); index >= 0; index--) {
        for (let part = 0; part < needle.length; part++) if (this[index + part] !== needle[part]) continue outer;
        return index;
      }
      return -1;
    }
    includes(value, byteOffset, encoding) { return this.indexOf(value, byteOffset, encoding) !== -1; }
    slice(start, end) { return this.subarray(start, end); }
    write(value, offset = 0, length, encoding = 'utf8') {
      if (typeof length === 'string') { encoding = length; length = undefined; }
      const bytes = bufferBytes(value, encoding); const count = Math.min(bytes.length, length === undefined ? this.length - Number(offset) : Number(length));
      this.set(bytes.slice(0, count), Number(offset)); return count;
    }
    readUInt8(offset = 0) { return this[Number(offset)]; }
    writeUInt8(value, offset = 0) { this[Number(offset)] = Number(value) & 255; return Number(offset) + 1; }
    readUInt16LE(offset = 0) { const index = Number(offset); return this[index] | this[index + 1] << 8; }
    readUInt16BE(offset = 0) { const index = Number(offset); return this[index] << 8 | this[index + 1]; }
    writeUInt16LE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) & 255; this[index + 1] = Number(value) >> 8 & 255; return index + 2; }
    writeUInt16BE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) >> 8 & 255; this[index + 1] = Number(value) & 255; return index + 2; }
  };
  Buffer.poolSize = 8192;
  globalThis.SlowBuffer = size => Buffer.alloc(Number(size));
  const normalizeHashAlgorithm = algorithm => {
    const name = String(algorithm).toLowerCase().replace(/[-_]/g, '');
    if (name !== 'sha256' && name !== 'sha512') throw new TypeError(`Unsupported digest: ${algorithm}`);
    return name;
  };
  const randomBytesSync = size => Buffer.from(__thaw_crypto_random_hex(Number(size)), 'hex');
  class Hash {
    constructor(algorithm) { this.algorithm = normalizeHashAlgorithm(algorithm); this._chunks = []; this._digested = false; }
    update(data, encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._chunks.push(Buffer.from(data, encoding)); return this;
    }
    digest(encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._digested = true;
      const value = Buffer.from(__thaw_crypto_hash_hex(this.algorithm, Buffer.concat(this._chunks).toString('hex')), 'hex');
      return encoding === undefined ? value : value.toString(encoding);
    }
    copy() { const copied = new Hash(this.algorithm); copied._chunks = this._chunks.map(chunk => Buffer.from(chunk)); return copied; }
  }
  class Hmac extends Hash {
    constructor(algorithm, key) { super(algorithm); this._key = Buffer.from(key); }
    digest(encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._digested = true;
      const value = Buffer.from(__thaw_crypto_hmac_hex(this.algorithm, this._key.toString('hex'), Buffer.concat(this._chunks).toString('hex')), 'hex');
      return encoding === undefined ? value : value.toString(encoding);
    }
  }
  const randomFillSync = (buffer, offset = 0, size = buffer.byteLength - Number(offset)) => {
    if (!ArrayBuffer.isView(buffer)) throw new TypeError('buffer must be an ArrayBuffer view');
    const bytes = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
    bytes.set(randomBytesSync(Number(size)), Number(offset)); return buffer;
  };
  const randomFill = (buffer, offset, size, callback) => {
    if (typeof offset === 'function') { callback = offset; offset = 0; size = buffer.byteLength; }
    else if (typeof size === 'function') { callback = size; size = buffer.byteLength - Number(offset || 0); }
    if (typeof callback !== 'function') throw new TypeError('callback must be a function');
    try { randomFillSync(buffer, offset || 0, size); queueMicrotask(() => callback(null, buffer)); }
    catch (error) { queueMicrotask(() => callback(error)); }
  };
  const randomBytes = (size, callback) => {
    const value = randomBytesSync(size);
    if (callback === undefined) return value;
    if (typeof callback !== 'function') throw new TypeError('callback must be a function');
    queueMicrotask(() => callback(null, value));
  };
  const randomInt = (min, max, callback) => {
    if (max === undefined || typeof max === 'function') { callback = typeof max === 'function' ? max : callback; max = min; min = 0; }
    min = Math.ceil(Number(min)); max = Math.floor(Number(max));
    if (!Number.isSafeInteger(min) || !Number.isSafeInteger(max) || max <= min) throw new RangeError('invalid random integer range');
    const range = max - min; const bytes = randomBytesSync(4);
    const value = min + ((bytes[0] * 0x1000000 + bytes[1] * 0x10000 + bytes[2] * 0x100 + bytes[3]) % range);
    if (callback === undefined) return value;
    queueMicrotask(() => callback(null, value));
  };
  const randomUUID = () => {
    const bytes = randomBytesSync(16); bytes[6] = bytes[6] & 0x0f | 0x40; bytes[8] = bytes[8] & 0x3f | 0x80;
    const hex = bytes.toString('hex');
    return `${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}`;
  };
  const timingSafeEqual = (left, right) => {
    const a = Buffer.from(left), b = Buffer.from(right);
    if (a.length !== b.length) throw new RangeError('input buffers must have the same length');
    let difference = 0; for (let index = 0; index < a.length; index++) difference |= a[index] ^ b[index]; return difference === 0;
  };
  const cryptoModule = {
    createHash: algorithm => new Hash(algorithm),
    createHmac: (algorithm, key) => new Hmac(algorithm, key),
    Hash, Hmac, randomBytes, randomFill, randomFillSync, randomInt, randomUUID,
    timingSafeEqual, getHashes: () => ['sha256', 'sha512']
  };
  const subtle = {
    digest(algorithm, data) {
      const name = typeof algorithm === 'string' ? algorithm : algorithm.name;
      const value = cryptoModule.createHash(name).update(Buffer.from(data.buffer || data,
        data.byteOffset || 0, data.byteLength)).digest();
      return Promise.resolve(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    }
  };
  const webCrypto = globalThis.crypto || {};
  webCrypto.getRandomValues = value => {
    if (!ArrayBuffer.isView(value) || value.byteLength > 65536) throw new DOMException('invalid random value target', 'QuotaExceededError');
    return randomFillSync(value);
  };
  webCrypto.randomUUID = randomUUID;
  webCrypto.subtle = webCrypto.subtle || subtle;
  globalThis.crypto = webCrypto;
  cryptoModule.webcrypto = webCrypto;
  cryptoModule.subtle = webCrypto.subtle;
  globalThis.__thaw_crypto_module = cryptoModule;
})();
"#;

/// Evaluates `source` in the (per-thread) global QuickJS context. Top-level
/// function declarations become callable afterwards via `thaw_js_call`.
/// Returns `1` on success, `0` on failure (syntax error, thrown exception).
#[no_mangle]
pub extern "C" fn thaw_js_load(source: *const c_char) -> u8 {
    let source = to_str(source);
    with_context(|ctx| match load_impl(ctx, &source) {
        Ok(()) => 1,
        Err(reason) => {
            eprintln!("thaw-quickjs: failed to load script: {reason}");
            0
        }
    })
}

/// Real error message for a failed `eval`, extracted the same way
/// `call_impl` already does for a thrown exception -- without this,
/// `rquickjs::Error::Exception`'s own `Display` is just a generic
/// placeholder ("Exception generated by QuickJS"), which made a real
/// npm package's load-time failure (e.g. a top-level `require(...)` call
/// this crate's stub rejects, see thaw-bridge's `wrap_as_commonjs_module`)
/// undiagnosable.
fn load_impl(ctx: Ctx<'_>, source: &str) -> Result<(), String> {
    ctx.eval::<(), _>(source).map_err(|e| match e {
        rquickjs::Error::Exception => describe_exception(&ctx),
        e => e.to_string(),
    })?;
    if let Ok(ready) = ctx
        .globals()
        .get::<_, rquickjs::Promise>("__thaw_module_ready")
    {
        finish_with_platform_events(&ctx, &ready).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => format!("module initialization failed: {error}"),
        })?;
        while ctx.execute_pending_job() {}
        let _ = ctx
            .globals()
            .set("__thaw_module_ready", Value::new_undefined(ctx.clone()));
    }
    Ok(())
}

/// Evaluates a script in the shared embedded context and returns its result as
/// JSON. JavaScript values for which `JSON.stringify` returns `undefined`
/// produce `None`; exceptions retain their JavaScript message.
pub fn eval_json(source: &str) -> Result<Option<String>, String> {
    with_context(|ctx| {
        let value = ctx
            .eval::<Value<'_>, _>(source)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })?;
        ctx.json_stringify(value)
            .map(|value| value.map(|value| value.to_string().unwrap_or_default()))
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })
    })
}

/// Calls a top-level function (previously loaded via `thaw_js_load`) named
/// `func_name`, with `args_json` a JSON-encoded array of arguments.
/// Returns the JSON-encoded result (or an error object -- see the module
/// doc comment).
#[no_mangle]
pub extern "C" fn thaw_js_call(
    func_name: *const c_char,
    args_json: *const c_char,
) -> *const c_char {
    let func_name = to_str(func_name);
    let args_json = to_str(args_json);

    let text = with_context(|ctx| match call_impl(ctx, &func_name, &args_json) {
        Ok(text) => text,
        Err(reason) => format!("{{\"__thaw_error__\":{}}}", json_escape_string(&reason)),
    });

    CString::new(text).unwrap_or_default().into_raw() as *const c_char
}

/// Result-ABI companion to [`thaw_js_call`]. Unlike the legacy JSON error
/// object API, failures occupy the error channel so generated Thaw code can
/// route JavaScript throws and Promise rejections through `try`/`catch`.
#[no_mangle]
pub extern "C" fn thaw_js_call_result(
    func_name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    let func_name = to_str(func_name);
    let args_json = to_str(args_json);
    match with_context(|ctx| call_impl(ctx, &func_name, &args_json)) {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(reason).unwrap_or_default().into_raw(),
        },
    }
}

fn call_impl(ctx: Ctx<'_>, func_name: &str, args_json: &str) -> Result<String, String> {
    let target: Function = ctx
        .globals()
        .get(func_name)
        .map_err(|_| format!("no such function `{func_name}` (was it loaded via loadScript?)"))?;
    invoke_impl(ctx, target, func_name, args_json)
}

fn invoke_impl<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    label: &str,
    args_json: &str,
) -> Result<String, String> {
    let to_string_err = |e: rquickjs::Error| e.to_string();

    let json: Object = ctx.globals().get("JSON").map_err(to_string_err)?;
    let parse: Function = json.get("parse").map_err(to_string_err)?;

    let args_array: Array = parse
        .call((args_json,))
        .map_err(|e| format!("args_json is not a valid JSON array: {e}"))?;

    let mut call_args = Args::new_unsized(ctx.clone());
    for i in 0..args_array.len() {
        let arg: Value = args_array.get(i).map_err(to_string_err)?;
        call_args.push_arg(arg).map_err(to_string_err)?;
    }

    let result: Value = target.call_arg(call_args).map_err(|e| match e {
        rquickjs::Error::Exception => format!("`{label}` threw: {}", describe_exception(&ctx)),
        e => format!("`{label}` threw: {e}"),
    })?;

    resolve_value_impl(ctx, result, label)
}

fn resolve_value_impl<'js>(
    ctx: Ctx<'js>,
    result: Value<'js>,
    label: &str,
) -> Result<String, String> {
    let to_string_err = |e: rquickjs::Error| e.to_string();
    let json: Object = ctx.globals().get("JSON").map_err(to_string_err)?;
    let stringify: Function = json.get("stringify").map_err(to_string_err)?;

    // Always pass the result through the realm's Promise resolution
    // procedure. `Value::as_promise` only recognizes native Promise objects;
    // `Promise.resolve` also assimilates arbitrary foreign thenables, handles
    // throwing `then` accessors/calls, and obeys the first-settlement-wins
    // rule required by JavaScript.
    let assimilate: Function = ctx
        .eval("(value) => Promise.resolve(value)")
        .map_err(to_string_err)?;
    let promise: rquickjs::Promise = assimilate.call((result,)).map_err(|e| match e {
        rquickjs::Error::Exception => {
            format!(
                "`{label}` could not resolve its result: {}",
                describe_exception(&ctx)
            )
        }
        e => format!("`{label}` could not resolve its result: {e}"),
    })?;
    let result = finish_with_platform_events(&ctx, &promise).map_err(|e| match e {
        rquickjs::Error::Exception => {
            format!("`{label}`'s promise rejected: {}", describe_exception(&ctx))
        }
        e => format!("`{label}`'s promise rejected or stalled: {e}"),
    })?;

    stringify
        .call((result,))
        .map_err(|e| format!("failed to JSON-encode the result: {e}"))
}

fn finish_with_platform_events<'js>(
    ctx: &Ctx<'js>,
    promise: &rquickjs::Promise<'js>,
) -> rquickjs::Result<Value<'js>> {
    loop {
        if let Ok(poll_platform_events) = ctx
            .globals()
            .get::<_, Function>("__thaw_poll_platform_events")
        {
            poll_platform_events.call::<_, ()>(())?;
        }
        if let Some(result) = promise.result() {
            return result;
        }
        if ctx.execute_pending_job() {
            continue;
        }

        let next_delay: Function = ctx.globals().get("__thaw_next_timer_delay")?;
        let delay: i64 = next_delay.call(())?;
        if delay < 0 {
            let active = ctx
                .globals()
                .get::<_, Function>("__thaw_worker_active")
                .ok()
                .and_then(|probe| probe.call::<_, bool>(()).ok())
                .unwrap_or(false);
            if active {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            return Err(rquickjs::Error::WouldBlock);
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay as u64));
        }
        let run_due: Function = ctx.globals().get("__thaw_run_due_timers")?;
        run_due.call::<_, usize>(())?;
    }
}

/// Retains a global JavaScript value in the realm and returns a stable opaque
/// handle. Zero denotes failure or a missing value.
#[no_mangle]
pub extern "C" fn thaw_js_get_global(name: *const c_char) -> u64 {
    let name = to_str(name);
    with_context(|ctx| {
        let Ok(value) = ctx.globals().get::<_, Value>(name.as_str()) else {
            return 0;
        };
        let globals = ctx.globals();
        let handles: Array = match globals.get("__thaw_value_handles") {
            Ok(handles) => handles,
            Err(_) => {
                let Ok(handles) = Array::new(ctx.clone()) else {
                    return 0;
                };
                if globals
                    .set("__thaw_value_handles", handles.clone())
                    .is_err()
                {
                    return 0;
                }
                let Ok(live) = Array::new(ctx.clone()) else {
                    return 0;
                };
                if globals.set("__thaw_value_handle_live", live).is_err() {
                    return 0;
                }
                handles
            }
        };
        let index = handles.len();
        if handles.set(index, value).is_err() {
            return 0;
        }
        let Ok(live) = ctx.globals().get::<_, Array>("__thaw_value_handle_live") else {
            return 0;
        };
        if live.set(index, true).is_err() {
            return 0;
        }
        index as u64 + 1
    })
}

fn handle_array<'js>(ctx: &Ctx<'js>) -> Result<Array<'js>, String> {
    ctx.globals()
        .get("__thaw_value_handles")
        .map_err(|_| "JavaScript value handle registry is empty".to_string())
}

fn value_for_handle<'js>(ctx: &Ctx<'js>, handle: u64) -> Result<Value<'js>, String> {
    if handle == 0 {
        return Err("invalid JavaScript value handle 0".to_string());
    }
    let live: Array = ctx
        .globals()
        .get("__thaw_value_handle_live")
        .map_err(|_| "JavaScript value handle liveness registry is empty".to_string())?;
    if !live.get::<bool>((handle - 1) as usize).unwrap_or(false) {
        return Err(format!("released JavaScript value handle {handle}"));
    }
    let value: Value = handle_array(ctx)?
        .get((handle - 1) as usize)
        .map_err(|_| format!("invalid JavaScript value handle {handle}"))?;
    Ok(value)
}

fn retain_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<u64, String> {
    let handles = handle_array(ctx)?;
    let index = handles.len();
    handles
        .set(index, value)
        .map_err(|error| error.to_string())?;
    let live: Array = ctx
        .globals()
        .get("__thaw_value_handle_live")
        .map_err(|_| "JavaScript value handle liveness registry is empty".to_string())?;
    live.set(index, true).map_err(|error| error.to_string())?;
    Ok(index as u64 + 1)
}

fn invoke_raw<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    args_json: &str,
) -> Result<Value<'js>, String> {
    let json: Object = ctx
        .globals()
        .get("JSON")
        .map_err(|error| error.to_string())?;
    let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
    let args_array: Array = parse
        .call((args_json,))
        .map_err(|error| format!("args_json is not a valid JSON array: {error}"))?;
    let mut call_args = Args::new_unsized(ctx.clone());
    for index in 0..args_array.len() {
        let arg: Value = args_array.get(index).map_err(|error| error.to_string())?;
        call_args.push_arg(arg).map_err(|error| error.to_string())?;
    }
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_exception(&ctx),
        error => error.to_string(),
    })
}

unsafe fn native_handle_slice<'a>(array: *const u8) -> Result<&'a [u64], String> {
    if array.is_null() {
        return Err("null JsValue array".into());
    }
    let length = unsafe { *(array.cast::<u64>()) } as usize;
    Ok(unsafe { std::slice::from_raw_parts(array.add(8).cast::<u64>(), length) })
}

unsafe fn invoke_mixed<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    args_json: &str,
    handles: *const u8,
) -> Result<Value<'js>, String> {
    let json: Object = ctx
        .globals()
        .get("JSON")
        .map_err(|error| error.to_string())?;
    let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
    let arguments: Array = parse
        .call((args_json,))
        .map_err(|error| error.to_string())?;
    let mut call_args = Args::new_unsized(ctx.clone());
    for index in 0..arguments.len() {
        call_args
            .push_arg(
                arguments
                    .get::<Value>(index)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    for handle in unsafe { native_handle_slice(handles)? } {
        call_args
            .push_arg(value_for_handle(&ctx, *handle)?)
            .map_err(|error| error.to_string())?;
    }
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_exception(&ctx),
        error => error.to_string(),
    })
}

#[no_mangle]
/// Calls a retained function with JSON arguments followed by retained values.
///
/// # Safety
///
/// `handles` must point to a live Thaw array buffer containing an initial
/// `u64` length followed by that many aligned `u64` handle identifiers.
pub unsafe extern "C" fn thaw_js_call_handle_mixed_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let value = unsafe { invoke_mixed(ctx.clone(), target, &args_json, handles)? };
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_construct_handle_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    match with_context(|ctx| {
        let constructor = value_for_handle(&ctx, handle)?
            .into_constructor()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not a constructor"))?;
        let json: Object = ctx
            .globals()
            .get("JSON")
            .map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let arguments: Array = parse
            .call((args_json,))
            .map_err(|error| error.to_string())?;
        let mut args = Args::new_unsized(ctx.clone());
        for index in 0..arguments.len() {
            args.push_arg(
                arguments
                    .get::<Value>(index)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        let value: Value = constructor
            .construct_args(args)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })?;
        retain_value(&ctx, value)
    }) {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_handle_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    let result: Result<u64, String> = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let result = invoke_raw(ctx.clone(), target, &args_json)?;
        retain_value(&ctx, result)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_value_result(handle: u64, argument: u64) -> ThawResult {
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let argument = value_for_handle(&ctx, argument)?;
        let value: Value = target.call((argument,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_release_handle(handle: u64) -> u8 {
    with_context(|ctx| {
        if handle == 0 {
            return 0;
        }
        let Ok(handles) = handle_array(&ctx) else {
            return 0;
        };
        let index = (handle - 1) as usize;
        let Ok(live) = ctx.globals().get::<_, Array>("__thaw_value_handle_live") else {
            return 0;
        };
        if !live.get::<bool>(index).unwrap_or(false) {
            return 0;
        }
        if live.set(index, false).is_err() {
            return 0;
        }
        u8::from(handles.set(index, Value::new_undefined(ctx)).is_ok())
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_release_all_handles() -> u64 {
    with_context(|ctx| {
        let count = handle_array(&ctx).map(|handles| handles.len()).unwrap_or(0);
        let Ok(handles) = Array::new(ctx.clone()) else {
            return 0;
        };
        let Ok(live) = Array::new(ctx.clone()) else {
            return 0;
        };
        if ctx.globals().set("__thaw_value_handles", handles).is_err()
            || ctx.globals().set("__thaw_value_handle_live", live).is_err()
        {
            return 0;
        }
        count as u64
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_get_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let value = object.get(name.as_str()).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_result(
    handle: u64,
    name: *const c_char,
    value_handle: u64,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let value = value_for_handle(&ctx, value_handle)?;
        object
            .set(name.as_str(), value)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })?;
        Ok(1)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_method_result(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let method: Function = object.get(name.as_str()).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        let json: Object = ctx
            .globals()
            .get("JSON")
            .map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let arguments: Array = parse
            .call((args_json.as_str(),))
            .map_err(|error| error.to_string())?;
        let mut call_args = Args::new_unsized(ctx.clone());
        call_args.this(object).map_err(|error| error.to_string())?;
        for index in 0..arguments.len() {
            let argument: Value = arguments.get(index).map_err(|error| error.to_string())?;
            call_args
                .push_arg(argument)
                .map_err(|error| error.to_string())?;
        }
        let value: Value = method.call_arg(call_args).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_impl(ctx, value, &format!("JavaScript method `{name}`"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_resolve_handle_result(handle: u64) -> ThawResult {
    let result = with_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

/// Calls a callable value retained by [`thaw_js_get_global`]. Arguments and
/// results use the existing JSON bridge while the callable itself preserves
/// identity and closures inside QuickJS.
#[no_mangle]
pub extern "C" fn thaw_js_call_handle_result(handle: u64, args_json: *const c_char) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        invoke_impl(
            ctx,
            target,
            &format!("JavaScript value #{handle}"),
            &args_json,
        )
    });
    match result {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(reason).unwrap_or_default().into_raw(),
        },
    }
}

/// `rquickjs::Error::Exception` doesn't carry the thrown value itself
/// (just a generic placeholder message) -- the real value has to be
/// fetched separately via `Ctx::catch`.
fn describe_exception(ctx: &Ctx<'_>) -> String {
    let exc = ctx.catch();
    if let Some(obj) = exc.as_object() {
        if let Ok(msg) = obj.get::<_, String>("message") {
            return msg;
        }
    }
    if let Some(s) = exc.as_string() {
        if let Ok(s) = s.to_string() {
            return s;
        }
    }
    format!("{exc:?}")
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn call(func_name: &str, args_json: &str) -> String {
        let func_name = CString::new(func_name).unwrap();
        let args_json = CString::new(args_json).unwrap();
        let result_ptr = thaw_js_call(func_name.as_ptr(), args_json.as_ptr());
        unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn load(source: &str) -> u8 {
        let source = CString::new(source).unwrap();
        thaw_js_load(source.as_ptr())
    }

    #[test]
    fn loads_and_calls_a_simple_function() {
        assert_eq!(load("function add(a, b) { return a + b; }"), 1);
        assert_eq!(call("add", "[2, 3]"), "5");
    }

    #[test]
    fn round_trips_objects_and_arrays() {
        assert_eq!(load("function identity(x) { return x; }"), 1);
        assert_eq!(
            call("identity", r#"[{"a": 1, "b": [true, "x"]}]"#),
            r#"{"a":1,"b":[true,"x"]}"#
        );
    }

    #[test]
    fn resolves_a_returned_promise() {
        assert_eq!(
            load("function later(x) { return Promise.resolve(x * 2); }"),
            1
        );
        assert_eq!(call("later", "[21]"), "42");
    }

    #[test]
    fn timeout_resolves_promises_and_forwards_arguments() {
        assert_eq!(
            load(
                "function timed(value) { return new Promise(resolve => {\n\
                   setTimeout((left, right) => resolve(left + right), 2, value, 2);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("timed", "[40]"), "42");
    }

    #[test]
    fn timeout_can_be_cancelled() {
        assert_eq!(
            load(
                "function cancelled() { return new Promise(resolve => {\n\
                   const id = setTimeout(() => resolve('wrong'), 0);\n\
                   clearTimeout(id);\n\
                   setTimeout(() => resolve('right'), 1);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("cancelled", "[]"), r#""right""#);
    }

    #[test]
    fn immediate_forwards_arguments_and_can_be_cancelled() {
        assert_eq!(
            load(
                "function immediate(value) { return new Promise(resolve => {\n\
                   const cancelled = setImmediate(() => resolve('wrong'));\n\
                   clearImmediate(cancelled);\n\
                   setImmediate((left, right) => resolve(left + right), value, 2);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("immediate", "[40]"), "42");
    }

    #[test]
    fn interval_repeats_and_can_cancel_itself() {
        assert_eq!(
            load(
                "function countIntervals() { return new Promise(resolve => {\n\
                   let count = 0;\n\
                   const id = setInterval(() => {\n\
                     if (++count === 3) { clearInterval(id); resolve(count); }\n\
                   }, 1);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("countIntervals", "[]"), "3");
    }

    #[test]
    fn microtasks_run_before_zero_delay_timers() {
        assert_eq!(
            load(
                "function eventOrder() { return new Promise(resolve => {\n\
                   const events = [];\n\
                   setTimeout(() => { events.push('timer'); resolve(events); }, 0);\n\
                   queueMicrotask(() => events.push('microtask'));\n\
                   events.push('sync');\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("eventOrder", "[]"), r#"["sync","microtask","timer"]"#);
    }

    #[test]
    fn process_next_tick_is_async_and_forwards_arguments() {
        assert_eq!(
            load(
                "function tickOrder() { return new Promise(resolve => {\n\
                   const events = ['sync'];\n\
                   process.nextTick((left, right) => { events.push(left + right); resolve(events); }, 'next', 'Tick');\n\
                   events.push('after');\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("tickOrder", "[]"), r#"["sync","after","nextTick"]"#);
    }

    #[test]
    fn process_exposes_time_cwd_and_event_helpers() {
        assert_eq!(
            load(
                "function processHelpers() {\n\
                   const original = process.cwd(); process.chdir('/tmp/app'); const changed = process.cwd(); process.chdir(original);\n\
                   const warnings = []; const removed = () => warnings.push('removed');\n\
                   process.on('warning', removed); process.off('warning', removed);\n\
                   process.once('warning', warning => warnings.push(warning.name + ':' + warning.code + ':' + warning.message));\n\
                   process.emitWarning('careful', { type: 'ThawWarning', code: 'THAW001' });\n\
                   process.emitWarning('ignored by once listener');\n\
                   const start = process.hrtime(); const elapsed = process.hrtime(start); const memory = process.memoryUsage();\n\
                   return [changed, process.cwd() === original, warnings.join(','), process.listenerCount('warning'), process.uptime() >= 0, start.length, elapsed[0] >= 0, elapsed[1] >= 0, typeof process.hrtime.bigint() === 'bigint', memory.rss, process.cpuUsage().user, process.title];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("processHelpers", "[]"),
            r#"["/tmp/app",true,"ThawWarning:THAW001:careful",0,true,2,true,true,true,0,0,"thaw"]"#
        );
    }

    #[test]
    fn console_formats_groups_counts_and_writes_to_streams() {
        assert_eq!(
            load(
                "function consoleHelpers() {\n\
                   const stdout = []; const stderr = []; const instance = new Console({ write(value) { stdout.push(value); } }, { write(value) { stderr.push(value); } });\n\
                   instance.log('%s:%d:%j:%%', 'value', 4, { ok: true }); instance.group('group'); instance.log('child'); instance.groupEnd();\n\
                   instance.count('item'); instance.count('item'); instance.countReset('item'); instance.count('item'); instance.assert(false, 'bad %s', 'value'); instance.trace('trace-value');\n\
                   return [stdout.join(''), stderr[0], stderr[1].startsWith('Trace: trace-value\\n'), typeof console.log, console instanceof Console];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("consoleHelpers", "[]"),
            r#"["value:4:{\"ok\":true}:%\ngroup\n  child\nitem: 1\nitem: 2\nitem: 1\n","Assertion failed: bad value\n",true,"function",true]"#
        );
    }

    #[test]
    fn base64_globals_round_trip_latin1_and_validate_input() {
        assert_eq!(
            load(
                "function base64() {\n\
                   let invalid = false;\n\
                   try { atob('%%%'); } catch (error) { invalid = error instanceof DOMException && error.name === 'InvalidCharacterError' && error.code === DOMException.INVALID_CHARACTER_ERR; }\n\
                   let invalidUnicode = false;\n\
                   try { btoa('雪'); } catch (error) { invalidUnicode = error instanceof DOMException && error.name === 'InvalidCharacterError'; }\n\
                   return [btoa('hello\\u00ff'), atob('aGVs bG//\\n'), invalid, invalidUnicode];\n\
                 }"
            ),
            1
        );
        assert_eq!(call("base64", "[]"), r#"["aGVsbG//","helloÿ",true,true]"#);
    }

    #[test]
    fn performance_exposes_time_origin_and_monotonic_elapsed_time() {
        assert_eq!(
            load(
                "function timing() {\n\
                   const first = performance.now();\n\
                   const second = performance.now();\n\
                   return [typeof performance.timeOrigin, first >= 0, second >= first];\n\
                 }"
            ),
            1
        );
        assert_eq!(call("timing", "[]"), r#"["number",true,true]"#);
    }

    #[test]
    fn performance_timeline_marks_measures_and_observes_entries() {
        assert_eq!(
            load(
                "async function performanceTimeline() {\n\
                   performance.clearMarks(); performance.clearMeasures();\n\
                   performance.mark('start', { startTime: 10, detail: { id: 1 } });\n\
                   const observed = []; const observer = new PerformanceObserver(list => observed.push(...list.getEntries().map(entry => entry.name)));\n\
                   observer.observe({ type: 'mark', buffered: true }); performance.mark('end', { startTime: 25 }); await Promise.resolve();\n\
                   const measure = performance.measure('work', { start: 'start', end: 'end', detail: 'detail' });\n\
                   const wrapped = performance.timerify(function double(value) { return value * 2; }); const result = wrapped(4);\n\
                   const asyncWrapped = performance.timerify(async function later(value) { await Promise.resolve(); return value + 1; }); const asyncResult = await asyncWrapped(4);\n\
                   observer.disconnect(); const marks = performance.getEntriesByType('mark'); const functions = performance.getEntriesByType('function');\n\
                   const beforeClear = performance.getEntriesByName('work', 'measure').length; performance.clearMarks('start'); performance.clearMeasures();\n\
                   return [observed.join(','), marks.map(entry => entry.name).join(','), marks[0].detail.id, measure.startTime, measure.duration, measure.detail, result, asyncResult, functions.map(entry => entry.name).sort().join(','), beforeClear, performance.getEntriesByName('start').length, PerformanceObserver.supportedEntryTypes.join(',')];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("performanceTimeline", "[]"),
            r#"["start,end","start,end",1,10,15,"detail",8,5,"double,later",1,0,"function,mark,measure"]"#
        );
    }

    #[test]
    fn structured_clone_copies_cycles_and_builtins() {
        assert_eq!(
            load(
                "function cloneValues() {\n\
                   const source = { nested: { value: 1 }, bytes: new Uint8Array([2, 3]), map: new Map([['key', 4]]), set: new Set([5]), date: new Date(6) };\n\
                   source.self = source;\n\
                   const copy = structuredClone(source);\n\
                   copy.nested.value = 7; copy.bytes[0] = 8;\n\
                   let rejected = false;\n\
                   try { structuredClone(() => 1); } catch (error) { rejected = error instanceof DOMException && error.name === 'DataCloneError' && error.code === DOMException.DATA_CLONE_ERR; }\n\
                   return [copy !== source, copy.self === copy, source.nested.value, source.bytes[0], copy.map.get('key'), copy.set.has(5), copy.date.getTime(), rejected];\n\
                 }"
            ),
            1
        );
        assert_eq!(call("cloneValues", "[]"), "[true,true,1,2,4,true,6,true]");
    }

    #[test]
    fn structured_clone_transfer_detaches_array_buffers() {
        assert_eq!(
            load(
                "function transferBuffer() {\n\
                   const source = new ArrayBuffer(4); new Uint8Array(source).set([1, 2, 3, 4]);\n\
                   const copy = structuredClone(source, { transfer: [source] });\n\
                   let duplicate = false, invalid = false; const other = new ArrayBuffer(1);\n\
                   try { structuredClone(other, { transfer: [other, other] }); } catch (error) { duplicate = error.name === 'DataCloneError'; }\n\
                   try { structuredClone({}, { transfer: [{}] }); } catch (error) { invalid = error.name === 'DataCloneError'; }\n\
                   return [source.byteLength, copy.byteLength, Array.from(new Uint8Array(copy)), duplicate, invalid, other.byteLength];\n\
                 }"
            ),
            1
        );
        assert_eq!(call("transferBuffer", "[]"), "[0,4,[1,2,3,4],true,true,1]");
    }

    #[test]
    fn message_port_transfer_detaches_the_source_port() {
        assert_eq!(
            load(
                "async function transferPort() {\n\
                   const channel = new MessageChannel(), carrier = new MessageChannel();\n\
                   const received = new Promise(resolve => { carrier.port2.onmessage = event => resolve([event.data.port, event.ports]); });\n\
                   carrier.port1.postMessage({ port: channel.port1 }, [channel.port1]);\n\
                   const [port, ports] = await received;\n\
                   const delivered = new Promise(resolve => { channel.port2.onmessage = event => resolve(event.data); });\n\
                   channel.port1.postMessage('ignored'); port.postMessage('through');\n\
                   const value = await delivered; carrier.port1.close(); carrier.port2.close(); port.close(); channel.port2.close();\n\
                   return [channel.port1.__thawClosed, value, ports[0] === port];\n\
                 }"
            ),
            1
        );
        assert_eq!(call("transferPort", "[]"), r#"[true,"through",true]"#);
    }

    #[test]
    fn url_search_params_preserves_duplicates_and_iterates() {
        assert_eq!(
            load(
                "function queryParams() {\n\
                   const params = new URLSearchParams('?b=two+words&a=1&a=2');\n\
                   params.append('snow', '雪');\n\
                   const before = [params.get('a'), params.getAll('a').join(','), params.has('a', '2'), params.size];\n\
                   params.set('a', 3); params.delete('b', 'wrong'); params.sort();\n\
                   const visited = []; params.forEach((value, key, owner) => visited.push(key + ':' + value + ':' + (owner === params)));\n\
                   const record = new URLSearchParams({ x: 1, y: false });\n\
                   const pairs = new URLSearchParams([['z', 4], ['z', 5]]); pairs.delete('z', 4);\n\
                   return [before, params.toString(), [...params.keys()].join(','), visited.join('|'), record.toString(), pairs.toString()];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("queryParams", "[]"),
            r#"[["1","1,2",true,4],"a=3&b=two+words&snow=%E9%9B%AA","a,b,snow","a:3:true|b:two words:true|snow:雪:true","x=1&y=false","z=5"]"#
        );
    }

    #[test]
    fn url_resolves_relative_paths_and_synchronizes_search_params() {
        assert_eq!(
            load(
                "function urls() {\n\
                   const url = new URL('../next?x=1#part', 'https://User:Pass@Example.COM:443/a/b/file');\n\
                   url.searchParams.append('x', 2);\n\
                   url.searchParams.set('space', 'two words');\n\
                   const first = [url.href, url.origin, url.protocol, url.username, url.password, url.host, url.hostname, url.port, url.pathname, url.search, url.hash, url.toJSON()];\n\
                   url.search = '?fresh=yes';\n\
                   const oldParamsDetached = url.searchParams.get('x') === null && url.searchParams.get('fresh') === 'yes';\n\
                   url.host = 'Other.test:8080'; url.pathname = 'root/./child/../end'; url.hash = 'done';\n\
                   return [first, oldParamsDetached, url.href, URL.canParse('/ok', 'https://example.test'), URL.canParse('/bad'), URL.parse('bad')];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("urls", "[]"),
            r##"[["https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part","https://example.com","https:","User","Pass","example.com","example.com","","/a/next","?x=1&x=2&space=two+words","#part","https://User:Pass@example.com/a/next?x=1&x=2&space=two+words#part"],true,"https://User:Pass@other.test:8080/root/end?fresh=yes#done",true,false,null]"##
        );
    }

    #[test]
    fn event_target_dispatches_listeners_and_cancellation() {
        assert_eq!(
            load(
                "function dispatchEvents() {\n\
                   const target = new EventTarget();\n\
                   const events = [];\n\
                   const removed = () => events.push('removed');\n\
                   target.addEventListener('work', removed);\n\
                   target.removeEventListener('work', removed);\n\
                   target.addEventListener('work', event => { events.push(event.target === target && event.currentTarget === target); event.preventDefault(); }, { once: true });\n\
                   target.addEventListener('work', { handleEvent(event) { events.push('object'); event.stopImmediatePropagation(); } });\n\
                   target.addEventListener('work', () => events.push('late'));\n\
                   const first = target.dispatchEvent(new Event('work', { cancelable: true }));\n\
                   const second = target.dispatchEvent(new Event('work', { cancelable: true }));\n\
                   return [events.join(','), first, second];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("dispatchEvents", "[]"),
            r#"["true,object,object",false,true]"#
        );
    }

    #[test]
    fn abort_controller_dispatches_once_and_timeout_aborts() {
        assert_eq!(
            load(
                "async function abortSignals() {\n\
                   const controller = new AbortController();\n\
                   const events = [];\n\
                   controller.signal.onabort = event => events.push(event instanceof Event ? 'property' : 'wrong');\n\
                   controller.signal.addEventListener('abort', () => events.push('once'), { once: true });\n\
                   controller.abort('reason'); controller.abort('ignored');\n\
                   let thrown = '';\n\
                   try { controller.signal.throwIfAborted(); } catch (error) { thrown = error; }\n\
                   const timed = AbortSignal.timeout(0);\n\
                   await new Promise(resolve => timed.addEventListener('abort', resolve));\n\
                   const first = new AbortController();\n\
                   const second = new AbortController();\n\
                   const combined = AbortSignal.any([first.signal, second.signal]);\n\
                   second.abort('combined');\n\
                   const preAborted = AbortSignal.any([AbortSignal.abort('pre')]);\n\
                   const defaultAbort = AbortSignal.abort();\n\
                   let invalidAny = false;\n\
                   try { AbortSignal.any([first.signal, {}]); } catch (error) { invalidAny = error instanceof TypeError; }\n\
                   return [events.join(','), controller.signal instanceof EventTarget, controller.signal.reason, thrown, timed.aborted, timed.reason.name, timed.reason.code, combined.aborted, combined.reason, preAborted.reason, defaultAbort.reason.name, defaultAbort.reason.code, invalidAny];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("abortSignals", "[]"),
            r#"["property,once",true,"reason","reason",true,"TimeoutError",23,true,"combined","pre","AbortError",20,true]"#
        );
    }

    #[test]
    fn text_encoder_and_decoder_support_utf8() {
        assert_eq!(
            load(
                "function utf8RoundTrip(value) {\n\
                   const encoder = new TextEncoder();\n\
                   const encoded = encoder.encode(value);\n\
                   return { bytes: Array.from(encoded), text: new TextDecoder().decode(encoded) };\n\
                 }\n\
                 function encodeInto() {\n\
                   const output = new Uint8Array(5);\n\
                   const result = new TextEncoder().encodeInto('A😀B', output);\n\
                   return { result, bytes: Array.from(output) };\n\
                 }\n\
                 function decodeInvalid() {\n\
                   return new TextDecoder().decode(Uint8Array.from([0x61, 0xff, 0x62]));\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("utf8RoundTrip", r#"["A😀"]"#),
            r#"{"bytes":[65,240,159,152,128],"text":"A😀"}"#
        );
        assert_eq!(
            call("encodeInto", "[]"),
            r#"{"result":{"read":3,"written":5},"bytes":[65,240,159,152,128]}"#
        );
        assert_eq!(call("decodeInvalid", "[]"), r#""a�b""#);
    }

    #[test]
    fn buffer_supports_encodings_views_search_and_numeric_access() {
        assert_eq!(
            load(
                "function buffers() {\n\
                   const utf8 = Buffer.from('雪ab'); const hex = Buffer.from('00ff10', 'hex'); const base64 = Buffer.from('aGk=', 'base64');\n\
                   const shared = utf8.slice(3); shared[0] = 65;\n\
                   const allocated = Buffer.alloc(5, 'xy'); allocated.write('Z', 2);\n\
                   const numeric = Buffer.alloc(4); numeric.writeUInt16LE(0x1234, 0); numeric.writeUInt16BE(0x5678, 2);\n\
                   const joined = Buffer.concat([base64, Buffer.from('!')]);\n\
                   return [Buffer.isBuffer(utf8), Buffer.isEncoding('base64url'), Buffer.byteLength('雪'), utf8.toString(), hex.toString('base64url'), joined.toString(), allocated.toString(), allocated.indexOf('y'), allocated.lastIndexOf('x'), allocated.includes('Z'), numeric.readUInt16LE(0), numeric.readUInt16BE(2), Buffer.compare(Buffer.from('a'), Buffer.from('b')), Buffer.from(utf8.toJSON()).equals(utf8), shared.buffer === utf8.buffer];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("buffers", "[]"),
            r#"[true,true,3,"雪Ab","AP8Q","hi!","xyZyx",1,4,true,4660,22136,-1,true,true]"#
        );
    }

    #[test]
    fn crypto_hash_hmac_random_and_webcrypto_are_available() {
        assert_eq!(
            load(
                "async function cryptoHelpers() {\n\
                   const sha256 = __thaw_crypto_module.createHash('sha-256').update('a').copy().update('bc').digest('hex');\n\
                   const sha512 = __thaw_crypto_module.createHash('sha512').update('abc').digest('hex');\n\
                   const hmac = __thaw_crypto_module.createHmac('sha256', 'key').update('The quick brown fox jumps over the lazy dog').digest('hex');\n\
                   const random = __thaw_crypto_module.randomBytes(12); const filled = new Uint8Array(8); const same = crypto.getRandomValues(filled) === filled;\n\
                   const callbackValue = await new Promise((resolve, reject) => __thaw_crypto_module.randomBytes(5, (error, value) => error ? reject(error) : resolve(value.length)));\n\
                   const integer = await new Promise((resolve, reject) => __thaw_crypto_module.randomInt(10, 20, (error, value) => error ? reject(error) : resolve(value)));\n\
                   const webDigest = Buffer.from(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('abc'))).toString('hex');\n\
                   const uuid = crypto.randomUUID();\n\
                   return [sha256, sha512, hmac, random.length, same, filled.length, callbackValue, integer >= 10 && integer < 20, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid), __thaw_crypto_module.timingSafeEqual(Buffer.from('same'), Buffer.from('same')), webDigest, __thaw_crypto_module.getHashes()];\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("cryptoHelpers", "[]"),
            r#"["ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f","f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",12,true,8,5,true,true,true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",["sha256","sha512"]]"#
        );
    }

    #[test]
    fn retains_and_calls_a_callable_javascript_value() {
        assert_eq!(
            load("globalThis.times = factor => value => Promise.resolve(value * factor); globalThis.twice = times(2);"),
            1
        );
        let name = CString::new("twice").unwrap();
        let handle = thaw_js_get_global(name.as_ptr());
        assert_ne!(handle, 0);
        let args = CString::new("[21]").unwrap();
        let result = thaw_js_call_handle_result(handle, args.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
            "42"
        );
    }

    #[test]
    fn assimilates_foreign_thenables_once_and_reports_then_errors() {
        assert_eq!(
            load(
                "function thenable() {\n\
                   return { then(resolve, reject) { resolve(42); reject('late'); resolve(99); } };\n\
                 }\n\
                 function throwingThen() {\n\
                   return Object.create(null, { then: { get() { throw new Error('bad then'); } } });\n\
                 }"
            ),
            1
        );
        assert_eq!(call("thenable", "[]"), "42");
        let failed = call("throwingThen", "[]");
        assert!(failed.contains("__thaw_error__"), "{failed}");
        assert!(failed.contains("bad then"), "{failed}");
    }

    #[test]
    fn unknown_function_reports_an_error_object_instead_of_crashing() {
        let result = call("doesNotExist", "[]");
        assert!(
            result.contains("__thaw_error__"),
            "unexpected result: {result}"
        );
    }

    #[test]
    fn thrown_exception_reports_an_error_object_instead_of_crashing() {
        assert_eq!(load("function boom() { throw new Error('kaboom'); }"), 1);
        let result = call("boom", "[]");
        assert!(
            result.contains("__thaw_error__"),
            "unexpected result: {result}"
        );
        assert!(result.contains("kaboom"), "unexpected result: {result}");
    }

    #[test]
    fn result_abi_separates_success_from_javascript_exceptions() {
        assert_eq!(
            load("function ok() { return 42; } function boom() { throw new Error('kaboom'); }"),
            1
        );
        let ok_name = CString::new("ok").unwrap();
        let boom_name = CString::new("boom").unwrap();
        let args = CString::new("[]").unwrap();
        let ok = thaw_js_call_result(ok_name.as_ptr(), args.as_ptr());
        assert!(ok.error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(ok.value) }.to_str().unwrap(), "42");

        let failed = thaw_js_call_result(boom_name.as_ptr(), args.as_ptr());
        assert!(failed.value.is_null());
        let error = unsafe { CStr::from_ptr(failed.error) }.to_string_lossy();
        assert!(error.contains("kaboom"), "{error}");
    }

    #[test]
    fn syntax_error_fails_to_load_instead_of_crashing() {
        assert_eq!(load("function( this is not valid js"), 0);
    }

    /// Regression test for the pre-fix behavior: a thrown exception during
    /// `loadScript` used to report only `rquickjs::Error::Exception`'s
    /// generic placeholder message on stderr, not the real thrown message
    /// -- undiagnosable for e.g. a real npm package's top-level
    /// `require(...)` call. `load_impl` is the testable inner function
    /// (`thaw_js_load` itself only returns 0/1, with the message going to
    /// stderr).
    #[test]
    fn load_failure_reports_the_real_thrown_message() {
        with_context(|ctx| {
            let err = load_impl(ctx, "throw new Error('boom');").unwrap_err();
            assert_eq!(err, "boom");
        });
    }
}
/// Common C ABI result for external operations that can either return a
/// pointer-shaped value or throw. Both pointers are owned by the callee for
/// the current request; exactly one is non-null.
#[repr(C)]
pub struct ThawResult {
    pub value: *const c_char,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawHandleResult {
    pub value: u64,
    pub error: *const c_char,
}
