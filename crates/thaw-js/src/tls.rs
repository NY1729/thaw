//! `node:tls` host (client and server) over rustls: the `__thaw_tls_*` functions the registry shim calls.
//! Ported from thaw-quickjs networking.rs (same wire protocol); rustls/ring is linked exactly as
//! thaw-quickjs links it next to thaw-runtime's own copy. Tables are process-wide (see net.rs).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime,
};
use rustls::server::WebPkiClientVerifier;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, Error as RustlsError, RootCertStore,
    ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};

use crate::net::*;
use crate::zlib::{hex_decode, hex_encode};
use crate::*;

static TLS_STREAMS: Table<TlsStreamTable> = Table::new(|| Mutex::new((1, HashMap::new())));
static TLS_SERVER_STREAMS: Table<TlsServerStreamTable> =
    Table::new(|| Mutex::new((2, HashMap::new())));
static TLS_LISTENERS: Table<(u32, HashMap<u32, TlsListener>)> =
    Table::new(|| Mutex::new((1, HashMap::new())));
static TLS_CLIENT_CERTIFICATES: Table<HashMap<u32, TlsCertificates>> =
    Table::new(|| Mutex::new(HashMap::new()));
static TLS_SERVER_CERTIFICATES: Table<HashMap<u32, TlsCertificates>> =
    Table::new(|| Mutex::new(HashMap::new()));

type TlsStream = StreamOwned<ClientConnection, TcpStream>;
type TlsStreamTable = (u32, HashMap<u32, TlsStream>);
type TlsServerStream = StreamOwned<ServerConnection, TcpStream>;
type TlsServerStreamTable = (u32, HashMap<u32, TlsServerStream>);

struct TlsListener {
    socket: TcpListener,
    config: Arc<ServerConfig>,
    local_certificate: Vec<u8>,
    request_cert: bool,
}

#[derive(Default)]
struct TlsCertificates {
    peer: Option<Vec<u8>>,
    /// The whole certificate chain the peer sent (leaf first), for `getPeerCertificate(true)`.
    peer_chain: Vec<Vec<u8>>,
    local: Option<Vec<u8>>,
    authorized: bool,
    authorization_error: Option<String>,
    chain_verified: bool,
}

struct RecordingServerVerifier {
    roots: RootCertStore,
    defer_name: bool,
    allow_invalid: bool,
    result: Arc<Mutex<Option<String>>>,
    algorithms: rustls::crypto::WebPkiSupportedAlgorithms,
}

/// Node reports `socket.authorizationError` as the OpenSSL X509 verify code, not rustls's message.
fn node_authorization_error(error: &RustlsError) -> String {
    use rustls::CertificateError as C;
    let RustlsError::InvalidCertificate(reason) = error else {
        return error.to_string();
    };
    match reason {
        C::UnknownIssuer => "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
        C::Expired | C::ExpiredContext { .. } => "CERT_HAS_EXPIRED",
        C::NotValidYet | C::NotValidYetContext { .. } => "CERT_NOT_YET_VALID",
        C::Revoked => "CERT_REVOKED",
        C::BadSignature => "CERT_SIGNATURE_FAILURE",
        C::NotValidForName | C::NotValidForNameContext { .. } => "ERR_TLS_CERT_ALTNAME_INVALID",
        _ => return error.to_string(),
    }
    .to_string()
}

impl std::fmt::Debug for RecordingServerVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordingServerVerifier")
            .finish_non_exhaustive()
    }
}

impl rustls::client::danger::ServerCertVerifier for RecordingServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, RustlsError> {
        let validation = (|| {
            let parsed = rustls::server::ParsedCertificate::try_from(end_entity)?;
            rustls::client::verify_server_cert_signed_by_trust_anchor(
                &parsed,
                &self.roots,
                intermediates,
                now,
                self.algorithms.all,
            )?;
            if !self.defer_name {
                rustls::client::verify_server_name(&parsed, server_name)?;
            }
            Ok(())
        })();
        let error = validation.as_ref().err().map(node_authorization_error);
        if let Ok(mut result) = self.result.lock() {
            *result = error.clone();
        }
        match validation {
            Ok(()) => Ok(rustls::client::danger::ServerCertVerified::assertion()),
            Err(_error) if self.allow_invalid => {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            Err(error) => Err(error),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
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

/// Protocol versions allowed by `minVersion`/`maxVersion` (rustls implements TLS 1.2 and 1.3 only).
fn tls_protocol_versions(
    spec: &str,
) -> Result<Vec<&'static rustls::SupportedProtocolVersion>, String> {
    let (minimum, maximum) = spec.split_once(',').unwrap_or(("", ""));
    let rank = |name: &str, default: usize| match name {
        "TLSv1" => 0,
        "TLSv1.1" => 1,
        "TLSv1.2" => 2,
        "TLSv1.3" => 3,
        _ => default,
    };
    let (minimum, maximum) = (rank(minimum, 0), rank(maximum, 3));
    let mut versions = Vec::new();
    if minimum <= 2 && maximum >= 2 {
        versions.push(&rustls::version::TLS12);
    }
    if minimum <= 3 && maximum >= 3 {
        versions.push(&rustls::version::TLS13);
    }
    if versions.is_empty() {
        return Err("error:0A0000BF:SSL routines::no protocols available".to_string());
    }
    Ok(versions)
}

fn decode_alpn_protocols(spec: &str) -> Vec<Vec<u8>> {
    spec.split(',')
        .filter(|value| !value.is_empty())
        .map(hex_decode)
        .collect()
}

pub(crate) struct TlsClientOptions<'a> {
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) server_name: &'a str,
    pub(crate) ca_spec: Option<&'a str>,
    pub(crate) cert_spec: &'a str,
    pub(crate) key_spec: &'a str,
    pub(crate) alpn_spec: &'a str,
    /// `minVersion,maxVersion` (TLSv1 .. TLSv1.3), either side may be empty.
    pub(crate) versions_spec: &'a str,
    pub(crate) report_alpn: bool,
    pub(crate) reject_unauthorized: bool,
    pub(crate) defer_name_verification: bool,
}

fn server_client_auth_status(
    request_cert: bool,
    peer_present: bool,
    verifier_accepted: bool,
) -> (bool, Option<&'static str>) {
    if !request_cert {
        return (false, Some("TLS client authentication was not requested"));
    }
    if !peer_present {
        return (false, Some("CERT_NOT_PROVIDED"));
    }
    if verifier_accepted {
        (true, None)
    } else {
        (false, Some("CERT_REJECTED"))
    }
}

pub(crate) fn take_tls_stream_handle(next: &mut u32) -> Option<u32> {
    let handle = *next;
    *next = handle.checked_add(2)?;
    Some(handle)
}

fn tls_client_roots(ca_spec: Option<&str>) -> Result<RootCertStore, String> {
    match ca_spec {
        None => Ok(RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        )),
        Some(spec) => tls_ca_roots(Some(spec)),
    }
}

fn tls_ca_roots(ca_spec: Option<&str>) -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    let Some(spec) = ca_spec else {
        return Ok(roots);
    };
    if spec.is_empty() {
        return Ok(roots);
    }
    for encoded in spec.split(',') {
        if encoded.is_empty()
            || encoded.len() % 2 != 0
            || !encoded.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("invalid CA encoding".to_string());
        }
        let bytes = hex_decode(encoded);
        let certificates = decode_ca_certificates(&bytes)?;
        for certificate in certificates {
            roots.add(certificate).map_err(|error| error.to_string())?;
        }
    }
    Ok(roots)
}

fn decode_ca_certificates(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    if bytes.is_empty() {
        return Err("empty CA certificate".to_string());
    }
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
            let Some(body_start) = rest.strip_prefix(prefix) else {
                return Err("invalid CA PEM block".to_string());
            };
            let Some((body, tail)) = body_start.split_once("-----END CERTIFICATE-----") else {
                return Err("unterminated CA PEM block".to_string());
            };
            let encoded = body
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect::<String>();
            let der = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| error.to_string())?;
            if der.is_empty() {
                return Err("empty CA certificate".to_string());
            }
            certs.push(CertificateDer::from(der));
            rest = tail.trim_matches(|character: char| character.is_ascii_whitespace());
        }
        return if certs.is_empty() {
            Err("no CA certificates found".to_string())
        } else {
            Ok(certs)
        };
    }
    Ok(vec![CertificateDer::from(bytes.to_vec())])
}

/// Everything a TLS client connection needs after the (cheap) configuration step; `connect` is the blocking part (TCP connect and
/// handshake) and may run on a worker thread, `register` publishes the finished connection on the JavaScript thread.
struct TlsClientJob {
    config: Arc<ClientConfig>,
    name: ServerName<'static>,
    host: String,
    port: u16,
    verifier_result: Arc<Mutex<Option<String>>>,
    local_certificate: Option<Vec<u8>>,
    report_alpn: bool,
    /// DER of the configured trust anchors, to complete the chain `getPeerCertificate(true)` reports.
    ca_certificates: Vec<Vec<u8>>,
    /// STARTTLS: an already connected stream (taken from a `net.Socket`) to run the handshake over.
    stream: Option<TcpStream>,
}

struct TlsClientDone {
    connection: ClientConnection,
    socket: TcpStream,
    verifier_result: Arc<Mutex<Option<String>>>,
    local_certificate: Option<Vec<u8>>,
    report_alpn: bool,
    ca_certificates: Vec<Vec<u8>>,
}

/// Appends the trust anchor that issued the last certificate of `chain` (OpenSSL reports the verified path, not only what the peer sent).
fn tls_extend_chain_with_anchor(chain: &mut Vec<Vec<u8>>, anchors: &[Vec<u8>]) {
    let Some(last) = chain.last() else { return };
    let Ok((_, last_certificate)) = x509_parser::parse_x509_certificate(last) else {
        return;
    };
    if last_certificate.subject().as_raw() == last_certificate.issuer().as_raw() {
        return;
    }
    let wanted = last_certificate.issuer().as_raw().to_vec();
    for anchor in anchors {
        if let Ok((_, certificate)) = x509_parser::parse_x509_certificate(anchor) {
            if certificate.subject().as_raw() == wanted.as_slice() {
                chain.push(anchor.clone());
                return;
            }
        }
    }
}

fn tls_client_prepare(options: &TlsClientOptions<'_>) -> Result<TlsClientJob, String> {
    let TlsClientOptions {
        host,
        port,
        server_name,
        ca_spec,
        cert_spec,
        key_spec,
        alpn_spec,
        versions_spec,
        report_alpn,
        reject_unauthorized,
        defer_name_verification,
    } = *options;
    let roots = tls_client_roots(ca_spec).map_err(|error| format!("err:{error}"))?;
    let verifier_result = Arc::new(Mutex::new(None));
    let versions = tls_protocol_versions(versions_spec).map_err(|error| format!("err:{error}"))?;
    let builder = ClientConfig::builder_with_protocol_versions(&versions)
        .with_root_certificates(roots.clone());
    let mut local_certificate = None;
    let config = if cert_spec.is_empty() && key_spec.is_empty() {
        builder.with_no_client_auth()
    } else if cert_spec.is_empty() || key_spec.is_empty() {
        return Err("err:client cert and key must be provided together".to_string());
    } else {
        let certificates =
            decode_certificates(cert_spec).map_err(|error| format!("err:{error}"))?;
        local_certificate = certificates
            .first()
            .map(|certificate| certificate.as_ref().to_vec());
        let key = decode_private_key(key_spec).map_err(|error| format!("err:{error}"))?;
        builder
            .with_client_auth_cert(certificates, key)
            .map_err(|error| format!("err:{error}"))?
    };
    let mut config = config;
    config.alpn_protocols = decode_alpn_protocols(alpn_spec);
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(RecordingServerVerifier {
            roots,
            defer_name: defer_name_verification,
            allow_invalid: !reject_unauthorized,
            result: Arc::clone(&verifier_result),
            algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }));
    let name =
        ServerName::try_from(server_name.to_string()).map_err(|error| format!("err:{error}"))?;
    let mut ca_certificates = Vec::new();
    for encoded in ca_spec
        .unwrap_or("")
        .split(',')
        .filter(|encoded| !encoded.is_empty())
    {
        let certificates = decode_ca_certificates(&hex_decode(encoded))
            .map_err(|error| format!("err:invalid CA certificate: {error}"))?;
        ca_certificates.extend(
            certificates
                .into_iter()
                .map(|certificate| certificate.as_ref().to_vec()),
        );
    }
    Ok(TlsClientJob {
        config: Arc::new(config),
        name,
        host: host.to_string(),
        port,
        verifier_result,
        local_certificate,
        report_alpn,
        ca_certificates,
        stream: None,
    })
}

impl TlsClientJob {
    fn connect(self) -> Result<TlsClientDone, String> {
        let mut socket = match self.stream {
            Some(stream) => stream,
            None => tcp_connect_structured(&self.host, self.port)?,
        };
        let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
        let mut connection = ClientConnection::new(self.config, self.name)
            .map_err(|error| format!("err:{error}"))?;
        while connection.is_handshaking() {
            connection
                .complete_io(&mut socket)
                .map_err(|error| format!("err:{error}"))?;
        }
        Ok(TlsClientDone {
            connection,
            socket,
            verifier_result: self.verifier_result,
            local_certificate: self.local_certificate,
            report_alpn: self.report_alpn,
            ca_certificates: self.ca_certificates,
        })
    }
}

impl TlsClientDone {
    fn register(self) -> String {
        let TlsClientDone {
            connection,
            socket,
            verifier_result,
            local_certificate,
            report_alpn,
            ca_certificates,
        } = self;
        let peer_certificate = connection
            .peer_certificates()
            .and_then(|certificates| certificates.first())
            .map(|certificate| certificate.as_ref().to_vec());
        let mut peer_chain: Vec<Vec<u8>> = connection
            .peer_certificates()
            .map(|certificates| {
                certificates
                    .iter()
                    .map(|certificate| certificate.as_ref().to_vec())
                    .collect()
            })
            .unwrap_or_default();
        tls_extend_chain_with_anchor(&mut peer_chain, &ca_certificates);
        TLS_STREAMS.with(|streams| {
            let mut streams = streams.borrow_mut();
            let Some(handle) = take_tls_stream_handle(&mut streams.0) else {
                return "err:TLS socket handle exhausted".to_string();
            };
            let verification_error = verifier_result
                .lock()
                .ok()
                .and_then(|result| result.clone());
            let chain_verified = verification_error.is_none();
            TLS_CLIENT_CERTIFICATES.with(|certificates| {
                certificates.borrow_mut().insert(
                    handle,
                    TlsCertificates {
                        peer: peer_certificate,
                        peer_chain,
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
}

pub(crate) fn tls_connect(options: TlsClientOptions<'_>) -> String {
    match tls_client_prepare(&options).and_then(TlsClientJob::connect) {
        Ok(done) => done.register(),
        Err(wire) => wire,
    }
}

/// Like `tls_connect`, but the TCP connect and the handshake run on a worker thread (see `async_op_start`).
pub(crate) fn tls_connect_start(options: TlsClientOptions<'_>) -> u32 {
    match tls_client_prepare(&options) {
        Err(wire) => async_op_ready(wire),
        Ok(job) => async_op_start(move || {
            let connected = job.connect();
            Box::new(move || match connected {
                Ok(done) => done.register(),
                Err(wire) => wire,
            })
        }),
    }
}

/// STARTTLS: the TLS client handshake over the connection a `net.Socket` already owns. The stream leaves the
/// socket tables (the `net.Socket` no longer uses its handle); queued writes are flushed by the writer thread first.
pub(crate) fn tls_wrap_start(options: TlsClientOptions<'_>, net_handle: u32) -> u32 {
    let Some(stream) = net_take_stream(net_handle) else {
        return async_op_ready("err:socket is closed".to_string());
    };
    match tls_client_prepare(&options) {
        Err(wire) => async_op_ready(wire),
        Ok(mut job) => {
            job.stream = Some(stream);
            async_op_start(move || {
                let connected = job.connect();
                Box::new(move || match connected {
                    Ok(done) => done.register(),
                    Err(wire) => wire,
                })
            })
        }
    }
}

pub(crate) struct TlsServerOptions<'a> {
    pub(crate) host: &'a str,
    pub(crate) port: u16,
    pub(crate) cert_spec: &'a str,
    pub(crate) key_spec: &'a str,
    pub(crate) ca_spec: Option<&'a str>,
    pub(crate) request_cert: bool,
    pub(crate) reject_unauthorized: bool,
    pub(crate) alpn_spec: &'a str,
    pub(crate) versions_spec: &'a str,
}

pub(crate) fn tls_server_listen(options: TlsServerOptions<'_>) -> String {
    let TlsServerOptions {
        host,
        port,
        cert_spec,
        key_spec,
        ca_spec,
        request_cert,
        reject_unauthorized,
        alpn_spec,
        versions_spec,
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
    let roots = match tls_ca_roots(ca_spec) {
        Ok(roots) => roots,
        Err(error) => return format!("err:{error}"),
    };
    let versions = match tls_protocol_versions(versions_spec) {
        Ok(versions) => versions,
        Err(error) => return format!("err:{error}"),
    };
    let builder = ServerConfig::builder_with_protocol_versions(&versions);
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

type TlsAcceptedParts = (
    TcpStream,
    std::net::SocketAddr,
    Arc<ServerConfig>,
    Vec<u8>,
    bool,
);

fn tls_server_accept_tcp(handle: u32, nonblocking: bool) -> Result<TlsAcceptedParts, String> {
    TLS_LISTENERS.with(|listeners| {
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
    })
}

/// The blocking part of accepting a TLS client: the handshake. A client that connects and then says nothing stalls only its own
/// worker thread (bounded by the read timeout), never the accept loop.
fn tls_server_handshake(
    mut socket: TcpStream,
    config: Arc<ServerConfig>,
) -> Result<(ServerConnection, TcpStream), String> {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut connection = ServerConnection::new(config).map_err(|error| format!("err:{error}"))?;
    while connection.is_handshaking() {
        connection
            .complete_io(&mut socket)
            .map_err(|error| format!("err:{error}"))?;
    }
    Ok((connection, socket))
}

fn tls_server_register(
    connection: ServerConnection,
    socket: TcpStream,
    peer: std::net::SocketAddr,
    local_certificate: Vec<u8>,
    request_cert: bool,
) -> String {
    let peer_certificate = connection
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .map(|certificate| certificate.as_ref().to_vec());
    let peer_chain: Vec<Vec<u8>> = connection
        .peer_certificates()
        .map(|certificates| {
            certificates
                .iter()
                .map(|certificate| certificate.as_ref().to_vec())
                .collect()
        })
        .unwrap_or_default();
    TLS_SERVER_STREAMS.with(|streams| {
        let mut streams = streams.borrow_mut();
        let Some(stream_handle) = take_tls_stream_handle(&mut streams.0) else {
            return "err:TLS socket handle exhausted".to_string();
        };
        TLS_SERVER_CERTIFICATES.with(|certificates| {
            certificates.borrow_mut().insert(stream_handle, {
                let peer_present = peer_certificate
                    .as_ref()
                    .is_some_and(|certificate| !certificate.is_empty());
                let (authorized, authorization_error) =
                    server_client_auth_status(request_cert, peer_present, true);
                TlsCertificates {
                    peer: peer_certificate.clone(),
                    peer_chain,
                    local: Some(local_certificate),
                    authorized,
                    authorization_error: authorization_error.map(str::to_owned),
                    chain_verified: authorized,
                }
            });
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

static TLS_ACCEPT_PENDING: Table<HashMap<u32, Vec<u32>>> =
    Table::new(|| Mutex::new(HashMap::new()));

fn tls_server_accept_impl(handle: u32, nonblocking: bool) -> String {
    if nonblocking {
        // A finished handshake first; then start handshakes for newly connected clients on worker threads.
        let ready = TLS_ACCEPT_PENDING.with(|pending| {
            let mut pending = pending.borrow_mut();
            let operations = pending.entry(handle).or_default();
            let mut index = 0;
            while index < operations.len() {
                let outcome = async_op_poll(operations[index]);
                if outcome != "pending" {
                    operations.remove(index);
                    return Some(outcome);
                }
                index += 1;
            }
            None
        });
        if let Some(outcome) = ready {
            return outcome;
        }
        loop {
            // Bounded: further clients wait in the kernel backlog while this many handshakes are in flight.
            if TLS_ACCEPT_PENDING.with(|pending| {
                pending
                    .borrow()
                    .get(&handle)
                    .is_some_and(|operations| operations.len() >= 64)
            }) {
                return "err:pending".to_string();
            }
            match tls_server_accept_tcp(handle, true) {
                Ok((socket, peer, config, local_certificate, request_cert)) => {
                    let operation = async_op_start(move || {
                        let handshaken = tls_server_handshake(socket, config);
                        Box::new(move || match handshaken {
                            Ok((connection, socket)) => tls_server_register(
                                connection,
                                socket,
                                peer,
                                local_certificate,
                                request_cert,
                            ),
                            Err(wire) => wire,
                        })
                    });
                    TLS_ACCEPT_PENDING.with(|pending| {
                        pending
                            .borrow_mut()
                            .entry(handle)
                            .or_default()
                            .push(operation)
                    });
                }
                Err(error) if error == "pending" => return "err:pending".to_string(),
                Err(error) => return format!("err:{error}"),
            }
        }
    }
    let (socket, peer, config, local_certificate, request_cert) =
        match tls_server_accept_tcp(handle, false) {
            Ok(accepted) => accepted,
            Err(error) => return format!("err:{error}"),
        };
    match tls_server_handshake(socket, config) {
        Ok((connection, socket)) => {
            tls_server_register(connection, socket, peer, local_certificate, request_cert)
        }
        Err(wire) => wire,
    }
}

pub(crate) fn tls_server_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, false)
}

pub(crate) fn tls_server_poll_accept(handle: u32) -> String {
    tls_server_accept_impl(handle, true)
}

pub(crate) fn tls_server_read(handle: u32) -> String {
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

pub(crate) fn tls_server_close_listener(handle: u32) {
    TLS_LISTENERS.with(|listeners| {
        listeners.borrow_mut().1.remove(&handle);
    });
    for operation in TLS_ACCEPT_PENDING
        .with(|pending| pending.borrow_mut().remove(&handle))
        .unwrap_or_default()
    {
        async_op_cancel(operation);
    }
}

pub(crate) fn tls_write(handle: u32, value: &[u8]) -> String {
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

pub(crate) fn tls_finish(handle: u32) -> String {
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

pub(crate) fn tls_shutdown_write(handle: u32) -> String {
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

pub(crate) fn tls_poll_read(handle: u32) -> String {
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

pub(crate) fn tls_destroy(handle: u32) {
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

pub(crate) fn tls_alpn(handle: u32) -> String {
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
pub(crate) fn tls_session_info(handle: u32) -> String {
    fn describe(
        version: Option<rustls::ProtocolVersion>,
        suite: Option<rustls::SupportedCipherSuite>,
    ) -> String {
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
        streams.borrow().1.get(&handle).map(|stream| {
            describe(
                stream.conn.protocol_version(),
                stream.conn.negotiated_cipher_suite(),
            )
        })
    });
    client.unwrap_or_else(|| {
        TLS_SERVER_STREAMS.with(|streams| {
            streams
                .borrow()
                .1
                .get(&handle)
                .map(|stream| {
                    describe(
                        stream.conn.protocol_version(),
                        stream.conn.negotiated_cipher_suite(),
                    )
                })
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

pub(crate) fn tls_certificate(handle: u32, peer: bool) -> String {
    tls_certificate_bytes(handle, peer)
        .as_deref()
        .map(hex_encode)
        .unwrap_or_default()
}

pub(crate) fn tls_authorization(handle: u32) -> String {
    let read = |certificates: &HashMap<u32, TlsCertificates>| {
        certificates.get(&handle).map(|entry| {
            serde_json::json!({
                "authorized": entry.authorized,
                "authorizationError": entry.authorization_error,
                "chainVerified": entry.chain_verified,
            })
        })
    };
    let value = TLS_CLIENT_CERTIFICATES
        .with(|certificates| read(&certificates.borrow()))
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
                    if let Ok(octets) = <[u8; 4]>::try_from(*bytes) {
                        ip_addresses.push(std::net::Ipv4Addr::from(octets).to_string());
                    }
                }
                x509_parser::extensions::GeneralName::IPAddress(bytes) if bytes.len() == 16 => {
                    if let Ok(octets) = <[u8; 16]>::try_from(*bytes) {
                        ip_addresses.push(std::net::Ipv6Addr::from(octets).to_string());
                    }
                }
                _ => {}
            }
        }
    }
    let mut san_tokens = dns_names
        .iter()
        .map(|name| {
            if name.chars().any(|character| {
                matches!(character, ',' | '"' | '\\') || character.is_ascii_whitespace()
            }) {
                format!(
                    "DNS:{}",
                    serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string())
                )
            } else {
                format!("DNS:{name}")
            }
        })
        .collect::<Vec<_>>();
    san_tokens.extend(
        ip_addresses
            .iter()
            .map(|address| format!("IP Address:{address}")),
    );
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

/// The peer's whole certificate chain as a JSON array of hex strings (leaf first); empty when unknown.
pub(crate) fn tls_peer_chain(handle: u32) -> String {
    let read = |certificates: &HashMap<u32, TlsCertificates>| {
        certificates
            .get(&handle)
            .map(|entry| entry.peer_chain.clone())
    };
    let chain = TLS_CLIENT_CERTIFICATES
        .with(|certificates| read(&certificates.borrow()))
        .or_else(|| TLS_SERVER_CERTIFICATES.with(|certificates| read(&certificates.borrow())))
        .unwrap_or_default();
    serde_json::Value::Array(
        chain
            .iter()
            .map(|certificate| serde_json::Value::String(hex_encode(certificate)))
            .collect(),
    )
    .to_string()
}

/// Metadata (subject, issuer, validity, ...) of one DER certificate given as hex.
pub(crate) fn tls_certificate_metadata_hex(hex: &str) -> String {
    certificate_metadata(&hex_decode(hex))
}

pub(crate) fn tls_certificate_metadata(handle: u32, peer: bool) -> String {
    tls_certificate_bytes(handle, peer)
        .as_deref()
        .map(certificate_metadata)
        .unwrap_or_else(|| "{}".to_string())
}

/// `tls` arm of `__thaw_net_set_option`: TCP_NODELAY / SO_KEEPALIVE on the socket under a TLS connection.
pub(crate) fn socket_option(
    handle: u32,
    option: &str,
    enable: bool,
    delay_ms: u32,
) -> Option<io::Result<()>> {
    TLS_STREAMS
        .with(|streams| {
            streams
                .borrow()
                .1
                .get(&handle)
                .map(|stream| tcp_socket_option(&stream.sock, option, enable, delay_ms))
        })
        .or_else(|| {
            TLS_SERVER_STREAMS.with(|streams| {
                streams
                    .borrow()
                    .1
                    .get(&handle)
                    .map(|stream| tcp_socket_option(&stream.sock, option, enable, delay_ms))
            })
        })
}

fn ca_spec(it: &mut Interp, v: &Value) -> Result<Option<String>, String> {
    match v {
        Value::Undefined => Ok(None),
        Value::Str(_) => Ok(Some(it.to_str(v).to_string())),
        _ => Err("CA must be a string".to_string()),
    }
}

/// `verification|alpn|deferName|versions` (the options string of connect_with_options / connect_start).
fn client_options<'a>(
    host: &'a str,
    port: u16,
    server_name: &'a str,
    ca: Option<&'a str>,
    cert: &'a str,
    key: &'a str,
    options: &'a str,
) -> TlsClientOptions<'a> {
    let mut parts = options.splitn(4, '|');
    let verification = parts.next().unwrap_or("1");
    let alpn = parts.next().unwrap_or("");
    let defer_name_verification = parts.next() == Some("1");
    let versions = parts.next().unwrap_or("");
    TlsClientOptions {
        host,
        port,
        server_name,
        ca_spec: ca,
        cert_spec: cert,
        key_spec: key,
        alpn_spec: alpn,
        versions_spec: versions,
        report_alpn: true,
        reject_unauthorized: verification != "0",
        defer_name_verification,
    }
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    macro_rules! def {
        ($name:expr, $f:expr) => {{
            let f = it.native($f);
            g.declare($name, f, true);
        }};
    }
    // (host, port, serverName, ca[, cert, key[, options]])
    for (name, identity, with_options) in [
        ("__thaw_tls_connect", false, false),
        ("__thaw_tls_connect_with_identity", true, false),
        ("__thaw_tls_connect_with_options", true, true),
    ] {
        def!(name, move |it, _, a| {
            let (host, port, server_name) = (s(it, a, 0), n(it, a, 1), s(it, a, 2));
            let ca = match ca_spec(it, &arg(a, 3)) {
                Ok(v) => v,
                Err(e) => return Ok(Value::str(&format!("err:{e}"))),
            };
            let (cert, key) = if identity {
                (s(it, a, 4), s(it, a, 5))
            } else {
                (String::new(), String::new())
            };
            let port = match checked_port(port) {
                Ok(p) => p,
                Err(e) => return Ok(Value::str(&e)),
            };
            let opts = if with_options {
                s(it, a, 6)
            } else {
                "1".to_string()
            };
            let mut o =
                client_options(&host, port, &server_name, ca.as_deref(), &cert, &key, &opts);
            if !with_options {
                o.report_alpn = false;
            }
            Ok(Value::str(&tls_connect(o)))
        });
    }
    def!("__thaw_tls_connect_start", |it, _, a| {
        let (host, port, server_name) = (s(it, a, 0), n(it, a, 1), s(it, a, 2));
        let ca = match ca_spec(it, &arg(a, 3)) {
            Ok(v) => v,
            Err(e) => return Ok(Value::Number(f64::from(async_op_ready(format!("err:{e}"))))),
        };
        let (cert, key, options) = (s(it, a, 4), s(it, a, 5), s(it, a, 6));
        let port = match checked_port(port) {
            Ok(p) => p,
            Err(e) => return Ok(Value::Number(f64::from(async_op_ready(e)))),
        };
        let o = client_options(
            &host,
            port,
            &server_name,
            ca.as_deref(),
            &cert,
            &key,
            &options,
        );
        Ok(Value::Number(f64::from(tls_connect_start(o))))
    });
    def!("__thaw_tls_wrap_start", |it, _, a| {
        let (net_handle, server_name) = (n(it, a, 0), s(it, a, 1));
        let ca = match ca_spec(it, &arg(a, 2)) {
            Ok(v) => v,
            Err(e) => return Ok(Value::Number(f64::from(async_op_ready(format!("err:{e}"))))),
        };
        let (cert, key, options) = (s(it, a, 3), s(it, a, 4), s(it, a, 5));
        let o = client_options("", 0, &server_name, ca.as_deref(), &cert, &key, &options);
        Ok(Value::Number(f64::from(tls_wrap_start(o, net_handle))))
    });
    def!("__thaw_tls_write", |it, _, a| {
        let h = n(it, a, 0);
        Ok(Value::str(&tls_write(h, &hex_decode(&s(it, a, 1)))))
    });
    def!("__thaw_tls_finish", |it, _, a| Ok(Value::str(&tls_finish(
        n(it, a, 0)
    ))));
    def!("__thaw_tls_shutdown", |it, _, a| Ok(Value::str(
        &tls_shutdown_write(n(it, a, 0))
    )));
    def!("__thaw_tls_poll_read", |it, _, a| Ok(Value::str(
        &tls_poll_read(n(it, a, 0))
    )));
    def!("__thaw_tls_destroy", |it, _, a| {
        tls_destroy(n(it, a, 0));
        Ok(Value::Undefined)
    });
    def!("__thaw_tls_alpn", |it, _, a| Ok(Value::str(&tls_alpn(n(
        it, a, 0
    )))));
    def!("__thaw_tls_session_info", |it, _, a| Ok(Value::str(
        &tls_session_info(n(it, a, 0))
    )));
    def!("__thaw_tls_peer_certificate", |it, _, a| Ok(Value::str(
        &tls_certificate(n(it, a, 0), true)
    )));
    def!("__thaw_tls_local_certificate", |it, _, a| Ok(Value::str(
        &tls_certificate(n(it, a, 0), false)
    )));
    def!("__thaw_tls_peer_chain", |it, _, a| Ok(Value::str(
        &tls_peer_chain(n(it, a, 0))
    )));
    def!("__thaw_tls_certificate_metadata_hex", |it, _, a| Ok(
        Value::str(&tls_certificate_metadata_hex(&s(it, a, 0)))
    ));
    def!("__thaw_tls_peer_certificate_metadata", |it, _, a| Ok(
        Value::str(&tls_certificate_metadata(n(it, a, 0), true))
    ));
    def!("__thaw_tls_local_certificate_metadata", |it, _, a| Ok(
        Value::str(&tls_certificate_metadata(n(it, a, 0), false))
    ));
    def!("__thaw_tls_authorization", |it, _, a| Ok(Value::str(
        &tls_authorization(n(it, a, 0))
    )));
    def!("__thaw_tls_canonical_ip", |it, _, a| {
        Ok(Value::str(
            &s(it, a, 0)
                .parse::<std::net::IpAddr>()
                .map(|ip| ip.to_string())
                .unwrap_or_default(),
        ))
    });
    // (host, port, cert, key) / (+ ca, rejectUnauthorized) / (+ ca, flags, "alpn|min,max")
    for (name, kind) in [
        ("__thaw_tls_server_listen", 0),
        ("__thaw_tls_server_listen_with_ca", 1),
        ("__thaw_tls_server_listen_with_options", 2),
    ] {
        def!(name, move |it, _, a| {
            let (host, port, cert, key) = (s(it, a, 0), n(it, a, 1), s(it, a, 2), s(it, a, 3));
            let ca = if kind == 0 {
                None
            } else {
                match ca_spec(it, &arg(a, 4)) {
                    Ok(v) => v,
                    Err(e) => return Ok(Value::str(&format!("err:{e}"))),
                }
            };
            let port = match checked_port(port) {
                Ok(p) => p,
                Err(e) => return Ok(Value::str(&e)),
            };
            let (request_cert, reject_unauthorized, alpn, versions) = match kind {
                0 => (false, true, String::new(), String::new()),
                1 => (true, arg(a, 5).truthy(), String::new(), String::new()),
                _ => {
                    let flags = n(it, a, 5);
                    let av = s(it, a, 6);
                    let (alpn, versions) = av.split_once('|').unwrap_or((av.as_str(), ""));
                    (
                        flags & 1 != 0,
                        flags & 2 != 0,
                        alpn.to_string(),
                        versions.to_string(),
                    )
                }
            };
            Ok(Value::str(&tls_server_listen(TlsServerOptions {
                host: &host,
                port,
                cert_spec: &cert,
                key_spec: &key,
                ca_spec: ca.as_deref(),
                request_cert,
                reject_unauthorized,
                alpn_spec: &alpn,
                versions_spec: &versions,
            })))
        });
    }
    def!("__thaw_tls_server_accept", |it, _, a| Ok(Value::str(
        &tls_server_accept(n(it, a, 0))
    )));
    def!("__thaw_tls_server_poll_accept", |it, _, a| Ok(Value::str(
        &tls_server_poll_accept(n(it, a, 0))
    )));
    def!("__thaw_tls_server_read", |it, _, a| Ok(Value::str(
        &tls_server_read(n(it, a, 0))
    )));
    def!("__thaw_tls_server_close", |it, _, a| {
        tls_server_close_listener(n(it, a, 0));
        Ok(Value::Undefined)
    });
}
