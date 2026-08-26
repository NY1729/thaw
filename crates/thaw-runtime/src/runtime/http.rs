#[derive(Clone, Copy)]
enum AsyncHttpState {
    Resolving,
    Connecting,
    TlsHandshaking,
    Writing,
    Reading,
}

struct AsyncHttpGet {
    fd: RawFd,
    request: Vec<u8>,
    written: usize,
    response: Vec<u8>,
    state: AsyncHttpState,
    completion: *mut ThawPromise,
    readiness: *mut ThawPromise,
    deadline: Instant,
    tls: Option<ClientConnection>,
    tls_config: Arc<ClientConfig>,
    use_tls: bool,
    host: String,
    port: u16,
    path: String,
    redirects: usize,
    resolution: Option<SharedDnsResolution>,
}

type SharedDnsResolution = Arc<Mutex<Option<Result<Vec<SocketAddr>, String>>>>;

impl Drop for AsyncHttpGet {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe { libc::close(self.fd) };
        }
    }
}

fn parse_http_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (tls, rest, default_port) = if let Some(rest) = url.strip_prefix("http://") {
        (false, rest, 80)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (true, rest, 443)
    } else {
        return Err("async fetch URL must use http:// or https://".to_string());
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return Err("HTTP URL is missing a host".to_string());
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed
            .split_once(']')
            .ok_or("malformed bracketed IPv6 host")?;
        let port = match suffix.strip_prefix(':') {
            Some(port) => port.parse().map_err(|_| "invalid HTTP port")?,
            None if suffix.is_empty() => default_port,
            None => return Err("malformed bracketed IPv6 authority".to_string()),
        };
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if port.chars().all(|ch| ch.is_ascii_digit()) {
            (
                host.to_string(),
                port.parse().map_err(|_| "invalid HTTP port")?,
            )
        } else {
            (authority.to_string(), default_port)
        }
    } else {
        (authority.to_string(), default_port)
    };
    let path = path
        .split_once('#')
        .map(|(path, _)| path.to_string())
        .unwrap_or(path);
    Ok((tls, host, port, path))
}

fn tls_client_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn open_nonblocking_socket(address: SocketAddr) -> Result<RawFd, String> {
    let domain = if address.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    let fd = unsafe {
        libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(format!(
            "creating socket: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = match address {
        SocketAddr::V4(address) => {
            let raw = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: address.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(address.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in).cast(),
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                )
            }
        }
        SocketAddr::V6(address) => {
            let raw = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: address.port().to_be(),
                sin6_flowinfo: address.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: address.ip().octets(),
                },
                sin6_scope_id: address.scope_id(),
            };
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                )
            }
        }
    };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINPROGRESS) {
        Ok(fd)
    } else {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        Err(format!("connecting HTTP socket: {error}"))
    }
}

type DnsResult = Arc<Mutex<Option<Result<Vec<SocketAddr>, String>>>>;

fn start_dns_resolution(host: String, port: u16) -> Result<(RawFd, DnsResult), String> {
    let mut pipe = [-1; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err(format!(
            "creating DNS completion pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = Arc::new(Mutex::new(None));
    let worker_result = result.clone();
    let read_fd = pipe[0];
    let write_fd = pipe[1];
    let spawn = thread::Builder::new()
        .name("thaw-dns".to_string())
        .spawn(move || {
            let resolved = (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>())
                .map_err(|error| format!("resolving {host}: {error}"))
                .and_then(|addresses| {
                    if addresses.is_empty() {
                        Err(format!("no address found for {host}"))
                    } else {
                        Ok(addresses)
                    }
                });
            *worker_result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(resolved);
            let byte = [1u8];
            unsafe {
                libc::write(write_fd, byte.as_ptr().cast(), byte.len());
                libc::close(write_fd);
            }
        });
    if let Err(error) = spawn {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(format!("starting DNS resolver: {error}"));
    }
    Ok((read_fd, result))
}

struct NonblockingSocket(RawFd);

impl Read for NonblockingSocket {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = unsafe { libc::recv(self.0, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read >= 0 {
            Ok(read as usize)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

impl Write for NonblockingSocket {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = unsafe {
            libc::send(
                self.0,
                buffer.as_ptr().cast(),
                buffer.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if written >= 0 {
            Ok(written as usize)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn flush_tls(task: *mut AsyncHttpGet) -> Result<bool, String> {
    let fd = unsafe { (*task).fd };
    let tls = unsafe { (*task).tls.as_mut().expect("TLS state is present") };
    let mut socket = NonblockingSocket(fd);
    while tls.wants_write() {
        match tls.write_tls(&mut socket) {
            Ok(0) => return Err("TLS socket closed while writing".to_string()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(format!("writing TLS records: {error}")),
        }
    }
    Ok(true)
}

fn read_tls_records(task: *mut AsyncHttpGet) -> Result<bool, String> {
    let fd = unsafe { (*task).fd };
    let tls = unsafe { (*task).tls.as_mut().expect("TLS state is present") };
    let mut socket = NonblockingSocket(fd);
    loop {
        match tls.read_tls(&mut socket) {
            Ok(0) => return Ok(true),
            Ok(_) => {
                tls.process_new_packets()
                    .map_err(|error| format!("processing TLS records: {error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(format!("reading TLS records: {error}")),
        }
    }
}

fn tls_interests(task: *mut AsyncHttpGet) -> u8 {
    let tls = unsafe { (*task).tls.as_ref().expect("TLS state is present") };
    let mut interests = 0;
    if tls.wants_read() {
        interests |= THAW_FD_READABLE;
    }
    if tls.wants_write() {
        interests |= THAW_FD_WRITABLE;
    }
    if interests == 0 {
        THAW_FD_READABLE
    } else {
        interests
    }
}

fn async_http_error(task: *mut AsyncHttpGet, message: String) {
    let task = unsafe { Box::from_raw(task) };
    let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
    thaw_promise_reject(task.completion, error);
}

fn arena_c_string(value: &str) -> Option<*const u8> {
    let value = CString::new(value).ok()?;
    let bytes = value.as_bytes_with_nul();
    let destination = thaw_arena::thaw_arena_alloc(bytes.len(), 1);
    if destination.is_null() {
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    Some(destination)
}

fn arena_pointer_slot(value: *const u8) -> Option<*const u8> {
    let slot = thaw_arena::thaw_arena_alloc(
        std::mem::size_of::<*const u8>(),
        std::mem::align_of::<*const u8>(),
    ) as *mut *const u8;
    if slot.is_null() {
        return None;
    }
    unsafe { slot.write(value) };
    Some(slot.cast())
}

fn http_authority(use_tls: bool, host: &str, port: u16) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == if use_tls { 443 } else { 80 } {
        host
    } else {
        format!("{host}:{port}")
    }
}

fn normalize_http_path(path: &str) -> String {
    let (path, suffix) = path
        .find(['?', '#'])
        .map(|index| (&path[..index], &path[index..]))
        .unwrap_or((path, ""));
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}{suffix}", parts.join("/"))
}

fn redirect_url(task: &AsyncHttpGet, location: &str) -> Result<String, String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }
    let scheme = if task.use_tls { "https" } else { "http" };
    if location.starts_with("//") {
        return Ok(format!("{scheme}:{location}"));
    }
    let authority = http_authority(task.use_tls, &task.host, task.port);
    let path = if location.starts_with('/') {
        normalize_http_path(location)
    } else if location.starts_with('?') || location.starts_with('#') {
        let base = task
            .path
            .find(['?', '#'])
            .map(|index| &task.path[..index])
            .unwrap_or(&task.path);
        format!("{base}{location}")
    } else {
        let directory = task.path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        normalize_http_path(&format!("{directory}/{location}"))
    };
    Ok(format!("{scheme}://{authority}{path}"))
}

fn restart_async_http(task: *mut AsyncHttpGet, location: &str) -> Result<(), String> {
    let task_ref = unsafe { &mut *task };
    if task_ref.redirects >= 10 {
        return Err("HTTP redirect limit exceeded (10)".to_string());
    }
    let target = redirect_url(task_ref, location)?;
    let (use_tls, host, port, path) = parse_http_url(&target)?;
    let tls = if use_tls {
        let server_name = ServerName::try_from(host.clone())
            .map_err(|_| format!("invalid TLS server name `{host}`"))?;
        Some(
            ClientConnection::new(task_ref.tls_config.clone(), server_name)
                .map_err(|error| format!("creating redirected TLS client: {error}"))?,
        )
    } else {
        None
    };
    let (fd, resolution) = start_dns_resolution(host.clone(), port)?;
    unsafe { libc::close(task_ref.fd) };
    let authority = http_authority(use_tls, &host, port);
    task_ref.fd = fd;
    task_ref.request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    )
    .into_bytes();
    task_ref.written = 0;
    task_ref.response.clear();
    task_ref.state = AsyncHttpState::Resolving;
    task_ref.tls = tls;
    task_ref.use_tls = use_tls;
    task_ref.host = host;
    task_ref.port = port;
    task_ref.path = path;
    task_ref.redirects += 1;
    task_ref.resolution = Some(resolution);
    schedule_async_http(task, THAW_FD_READABLE);
    Ok(())
}

struct ParsedHttpResponse {
    status: u16,
    body: Vec<u8>,
    location: Option<String>,
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn decode_chunked(body: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let Some(line_end) = find_bytes(&body[cursor..], b"\r\n") else {
            return Ok(None);
        };
        let line_end = cursor + line_end;
        let size_text =
            std::str::from_utf8(&body[cursor..line_end]).map_err(|_| "chunk size is not ASCII")?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| format!("invalid chunk size `{size_text}`"))?;
        cursor = line_end + 2;
        if size == 0 {
            if body.len() < cursor + 2 {
                return Ok(None);
            }
            if &body[cursor..cursor + 2] == b"\r\n" {
                return Ok(Some(decoded));
            }
            return if find_bytes(&body[cursor..], b"\r\n\r\n").is_some() {
                Ok(Some(decoded))
            } else {
                Ok(None)
            };
        }
        let chunk_end = cursor
            .checked_add(size)
            .ok_or("chunk size overflows address space")?;
        if body.len() < chunk_end + 2 {
            return Ok(None);
        }
        if &body[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err("chunk data is missing its CRLF terminator".to_string());
        }
        decoded.extend_from_slice(&body[cursor..chunk_end]);
        cursor = chunk_end + 2;
    }
}

fn parse_http_response(response: &[u8], eof: bool) -> Result<Option<ParsedHttpResponse>, String> {
    let Some(header_end) = find_bytes(response, b"\r\n\r\n") else {
        return if eof {
            Err("malformed HTTP response (incomplete headers)".to_string())
        } else {
            Ok(None)
        };
    };
    let head = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "HTTP response headers are not valid UTF-8")?;
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or("malformed HTTP status line")?;
    let mut content_length = None;
    let mut chunked = false;
    let mut location = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("malformed HTTP header `{line}`"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length header")?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
        if name.eq_ignore_ascii_case("location") {
            location = Some(value.trim().to_string());
        }
    }
    let body = &response[header_end + 4..];
    if chunked {
        return decode_chunked(body).map(|body| {
            body.map(|body| ParsedHttpResponse {
                status,
                body,
                location,
            })
        });
    }
    if let Some(length) = content_length {
        if body.len() < length {
            return if eof {
                Err(format!(
                    "HTTP body ended after {} bytes, expected {length}",
                    body.len()
                ))
            } else {
                Ok(None)
            };
        }
        return Ok(Some(ParsedHttpResponse {
            status,
            body: body[..length].to_vec(),
            location,
        }));
    }
    if eof {
        Ok(Some(ParsedHttpResponse {
            status,
            body: body.to_vec(),
            location,
        }))
    } else {
        Ok(None)
    }
}

fn async_http_finish(task: *mut AsyncHttpGet, response: ParsedHttpResponse) {
    if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        if let Some(location) = &response.location {
            if let Err(error) = restart_async_http(task, location) {
                async_http_error(task, error);
            }
            return;
        }
    }
    let task = unsafe { Box::from_raw(task) };
    let completion = task.completion;
    if !(200..=399).contains(&response.status) {
        let message = format!("HTTP request failed with status {}", response.status);
        drop(task);
        let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
        thaw_promise_reject(completion, error);
        return;
    }
    let body = std::str::from_utf8(&response.body)
        .ok()
        .and_then(arena_c_string)
        .ok_or(());
    drop(task);
    match body {
        Ok(body) => {
            if let Some(result_slot) = arena_pointer_slot(body) {
                thaw_promise_resolve(completion, result_slot);
            } else {
                thaw_promise_reject(completion, INVALID_FD_ERROR.as_ptr());
            }
        }
        Err(_) => {
            let error = arena_c_string("HTTP body contains a NUL byte")
                .unwrap_or(INVALID_FD_ERROR.as_ptr());
            thaw_promise_reject(completion, error);
        }
    }
}

fn schedule_async_http(task: *mut AsyncHttpGet, interests: u8) {
    let task_ref = unsafe { &*task };
    let remaining = task_ref.deadline.saturating_duration_since(Instant::now());
    let milliseconds = remaining.as_millis().max(1).min(u64::MAX as u128) as u64;
    let readiness = thaw_runtime_wait_fd_timeout(task_ref.fd, interests, milliseconds);
    unsafe { (*task).readiness = readiness };
    unsafe { thaw_promise_subscribe(readiness, resume_async_http, task.cast()) };
}

fn complete_async_http_if_ready(task: *mut AsyncHttpGet, eof: bool) -> bool {
    let parsed = unsafe { parse_http_response(&(*task).response, eof) };
    match parsed {
        Ok(Some(response)) => {
            async_http_finish(task, response);
            true
        }
        Ok(None) => false,
        Err(error) => {
            async_http_error(task, error);
            true
        }
    }
}

extern "C" fn resume_async_http(frame: *mut u8, _result: *const u8) {
    let task = frame.cast::<AsyncHttpGet>();
    let readiness = unsafe { (*task).readiness };
    let readiness_state = unsafe { thaw_promise_state(readiness) };
    unsafe { thaw_promise_destroy(readiness) };
    unsafe { (*task).readiness = std::ptr::null_mut() };
    if readiness_state == 2 {
        async_http_error(
            task,
            "HTTP operation timed out or descriptor failed".to_string(),
        );
        return;
    }

    loop {
        match unsafe { (*task).state } {
            AsyncHttpState::Resolving => {
                let mut byte = [0u8; 1];
                unsafe {
                    libc::read((*task).fd, byte.as_mut_ptr().cast(), byte.len());
                    libc::close((*task).fd);
                    (*task).fd = -1;
                }
                let resolution = unsafe { (*task).resolution.take() }.and_then(|result| {
                    result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                });
                let addresses = match resolution {
                    Some(Ok(addresses)) => addresses,
                    Some(Err(error)) => {
                        async_http_error(task, error);
                        return;
                    }
                    None => {
                        async_http_error(
                            task,
                            "DNS resolver completed without a result".to_string(),
                        );
                        return;
                    }
                };
                let mut last_error = None;
                let mut socket = None;
                for address in addresses {
                    match open_nonblocking_socket(address) {
                        Ok(fd) => {
                            socket = Some(fd);
                            break;
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                let Some(fd) = socket else {
                    async_http_error(
                        task,
                        last_error.unwrap_or_else(|| "DNS returned no usable address".to_string()),
                    );
                    return;
                };
                unsafe {
                    (*task).fd = fd;
                    (*task).state = AsyncHttpState::Connecting;
                }
                schedule_async_http(task, THAW_FD_WRITABLE);
                return;
            }
            AsyncHttpState::Connecting => {
                let mut error = 0;
                let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                let result = unsafe {
                    libc::getsockopt(
                        (*task).fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        (&mut error as *mut libc::c_int).cast(),
                        &mut length,
                    )
                };
                if result != 0 || error != 0 {
                    let error = if error != 0 {
                        std::io::Error::from_raw_os_error(error)
                    } else {
                        std::io::Error::last_os_error()
                    };
                    async_http_error(task, format!("connecting HTTP socket: {error}"));
                    return;
                }
                unsafe {
                    (*task).state = if (*task).tls.is_some() {
                        AsyncHttpState::TlsHandshaking
                    } else {
                        AsyncHttpState::Writing
                    }
                };
            }
            AsyncHttpState::TlsHandshaking => {
                if let Err(error) = flush_tls(task) {
                    async_http_error(task, error);
                    return;
                }
                let eof = match read_tls_records(task) {
                    Ok(eof) => eof,
                    Err(error) => {
                        async_http_error(task, error);
                        return;
                    }
                };
                if eof {
                    async_http_error(task, "TLS peer closed during handshake".to_string());
                    return;
                }
                if unsafe { !(*task).tls.as_ref().unwrap().is_handshaking() } {
                    unsafe { (*task).state = AsyncHttpState::Writing };
                    continue;
                }
                schedule_async_http(task, tls_interests(task));
                return;
            }
            AsyncHttpState::Writing => {
                if unsafe { (*task).tls.is_some() } {
                    let task_ref = unsafe { &mut *task };
                    while task_ref.written < task_ref.request.len() {
                        let remaining = &task_ref.request[task_ref.written..];
                        match task_ref.tls.as_mut().unwrap().writer().write(remaining) {
                            Ok(0) => break,
                            Ok(written) => task_ref.written += written,
                            Err(error) => {
                                async_http_error(task, format!("buffering TLS request: {error}"));
                                return;
                            }
                        }
                    }
                    let flushed = match flush_tls(task) {
                        Ok(flushed) => flushed,
                        Err(error) => {
                            async_http_error(task, error);
                            return;
                        }
                    };
                    if unsafe { (*task).written == (*task).request.len() } && flushed {
                        unsafe { (*task).state = AsyncHttpState::Reading };
                        continue;
                    }
                    schedule_async_http(task, tls_interests(task));
                    return;
                }
                let task_ref = unsafe { &mut *task };
                while task_ref.written < task_ref.request.len() {
                    let remaining = &task_ref.request[task_ref.written..];
                    let written = unsafe {
                        libc::send(
                            task_ref.fd,
                            remaining.as_ptr().cast(),
                            remaining.len(),
                            libc::MSG_NOSIGNAL,
                        )
                    };
                    if written > 0 {
                        task_ref.written += written as usize;
                        continue;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        schedule_async_http(task, THAW_FD_WRITABLE);
                    } else {
                        async_http_error(task, format!("writing HTTP request: {error}"));
                    }
                    return;
                }
                task_ref.state = AsyncHttpState::Reading;
            }
            AsyncHttpState::Reading => {
                if unsafe { (*task).tls.is_some() } {
                    if let Err(error) = flush_tls(task) {
                        async_http_error(task, error);
                        return;
                    }
                    let eof = match read_tls_records(task) {
                        Ok(eof) => eof,
                        Err(error) => {
                            async_http_error(task, error);
                            return;
                        }
                    };
                    let mut plaintext = [0u8; 8192];
                    loop {
                        let read =
                            unsafe { (*task).tls.as_mut().unwrap().reader().read(&mut plaintext) };
                        match read {
                            Ok(0) => break,
                            Ok(read) => {
                                unsafe { &mut (*task).response }
                                    .extend_from_slice(&plaintext[..read]);
                                if complete_async_http_if_ready(task, false) {
                                    return;
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                                if complete_async_http_if_ready(task, true) {
                                    return;
                                }
                                async_http_error(
                                    task,
                                    "TLS peer closed before the HTTP response completed"
                                        .to_string(),
                                );
                                return;
                            }
                            Err(error) => {
                                async_http_error(
                                    task,
                                    format!("reading decrypted HTTP response: {error}"),
                                );
                                return;
                            }
                        }
                    }
                    if complete_async_http_if_ready(task, eof) {
                        return;
                    }
                    if eof {
                        async_http_error(
                            task,
                            "TLS peer closed before the HTTP response completed".to_string(),
                        );
                        return;
                    }
                    schedule_async_http(task, tls_interests(task));
                    return;
                }
                let mut buffer = [0u8; 8192];
                loop {
                    let read = unsafe {
                        libc::recv((*task).fd, buffer.as_mut_ptr().cast(), buffer.len(), 0)
                    };
                    if read > 0 {
                        unsafe { &mut (*task).response }
                            .extend_from_slice(&buffer[..read as usize]);
                        if complete_async_http_if_ready(task, false) {
                            return;
                        }
                        continue;
                    }
                    if read == 0 {
                        if !complete_async_http_if_ready(task, true) {
                            async_http_error(
                                task,
                                "HTTP peer closed before the response completed".to_string(),
                            );
                        }
                        return;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        schedule_async_http(task, THAW_FD_READABLE);
                    } else {
                        async_http_error(task, format!("reading HTTP response: {error}"));
                    }
                    return;
                }
            }
        }
    }
}

/// Starts a non-blocking HTTP GET and returns its completion Promise. DNS,
/// connect, write, and read completion are all driven through the event loop.
#[no_mangle]
pub extern "C" fn thaw_http_get_async(url: *const c_char) -> *mut ThawPromise {
    thaw_http_get_async_timeout(url, 30_000)
}

/// Starts a non-blocking HTTP GET with a total connect/write/read timeout.
#[no_mangle]
pub extern "C" fn thaw_http_get_async_timeout(
    url: *const c_char,
    timeout_ms: u64,
) -> *mut ThawPromise {
    thaw_http_get_async_with_config(url, timeout_ms, None)
}

fn thaw_http_get_async_with_config(
    url: *const c_char,
    timeout_ms: u64,
    tls_config: Option<Arc<ClientConfig>>,
) -> *mut ThawPromise {
    let completion = thaw_promise_new();
    let start = (|| -> Result<*mut AsyncHttpGet, String> {
        let url = unsafe { CStr::from_ptr(url) }
            .to_str()
            .map_err(|_| "HTTP URL is not valid UTF-8")?;
        let (use_tls, host, port, path) = parse_http_url(url)?;
        let tls_config = tls_config.unwrap_or_else(tls_client_config);
        let tls = if use_tls {
            let server_name = ServerName::try_from(host.clone())
                .map_err(|_| format!("invalid TLS server name `{host}`"))?;
            Some(
                ClientConnection::new(tls_config.clone(), server_name)
                    .map_err(|error| format!("creating TLS client: {error}"))?,
            )
        } else {
            None
        };
        let (fd, resolution) = start_dns_resolution(host.clone(), port)?;
        let authority = http_authority(use_tls, &host, port);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
        )
        .into_bytes();
        Ok(Box::into_raw(Box::new(AsyncHttpGet {
            fd,
            request,
            written: 0,
            response: Vec::new(),
            state: AsyncHttpState::Resolving,
            completion,
            readiness: std::ptr::null_mut(),
            deadline: Instant::now() + Duration::from_millis(timeout_ms),
            tls,
            tls_config,
            use_tls,
            host,
            port,
            path,
            redirects: 0,
            resolution: Some(resolution),
        })))
    })();
    match start {
        Ok(task) => schedule_async_http(task, THAW_FD_READABLE),
        Err(message) => {
            let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
            thaw_promise_reject(completion, error);
        }
    }
    completion
}

