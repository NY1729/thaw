/// Ports arrive from JavaScript as 32-bit numbers; narrowing without a range
/// check would silently turn 65536 into 0 ("any free port").
fn checked_port(port: u32) -> Result<u16, String> {
    u16::try_from(port)
        .map_err(|_| format!("err:port should be >= 0 and < 65536. Received {port}"))
}

/// libuv/Node error names for the `errno` values a TCP connect can fail with.
fn errno_name(errno: i32) -> Option<&'static str> {
    Some(match errno {
        1 => "EPERM",
        13 => "EACCES",
        24 => "EMFILE",
        98 => "EADDRINUSE",
        99 => "EADDRNOTAVAIL",
        101 => "ENETUNREACH",
        103 => "ECONNABORTED",
        104 => "ECONNRESET",
        110 => "ETIMEDOUT",
        111 => "ECONNREFUSED",
        113 => "EHOSTUNREACH",
        _ => return None,
    })
}

/// `err:<CODE>|<errno>|<syscall>|<address>|<message>`: the structured failure the JS sockets turn into a Node-shaped error
/// (`code`, `errno`, `syscall`, `address`, `port` / `hostname`).
fn connect_failure_wire(error: &io::Error, address: &str) -> String {
    let errno = error.raw_os_error();
    let name = errno.and_then(errno_name).unwrap_or(match error.kind() {
        io::ErrorKind::TimedOut => "ETIMEDOUT",
        io::ErrorKind::ConnectionReset => "ECONNRESET",
        io::ErrorKind::PermissionDenied => "EACCES",
        _ => "ECONNREFUSED",
    });
    let number = errno.map(|errno| -errno).unwrap_or(match name {
        "ETIMEDOUT" => -110,
        "ECONNRESET" => -104,
        "EACCES" => -13,
        _ => -111,
    });
    format!("err:{name}|{number}|connect|{address}|{error}")
}

/// Resolves and connects like Node: a name that does not resolve is `ENOTFOUND` (`getaddrinfo`), a refused or unreachable
/// address keeps the OS error.
fn tcp_connect_structured(host: &str, port: u16) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addresses: Vec<std::net::SocketAddr> = match (host, port).to_socket_addrs() {
        Ok(addresses) => addresses.collect(),
        Err(error) => return Err(format!("err:ENOTFOUND|-3008|getaddrinfo|{host}|{error}")),
    };
    let mut last = None;
    for address in addresses {
        match TcpStream::connect(address) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(connect_failure_wire(&error, &address.ip().to_string())),
        }
    }
    Err(last.unwrap_or_else(|| format!("err:ENOTFOUND|-3008|getaddrinfo|{host}|no addresses for {host}")))
}

/// Registers a connected stream in the thread-local table and builds the `ok:<handle>:<ip>:<port>` wire.
fn net_register_stream(stream: TcpStream, host: &str, port: u16) -> String {
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

fn net_connect(host: &str, port: u16) -> String {
    match tcp_connect_structured(host, port) {
        Ok(stream) => net_register_stream(stream, host, port),
        Err(wire) => wire,
    }
}

/// Asynchronous host operations. Slow work (name resolution, connect, TLS handshake) runs on a worker thread and hands a
/// "finisher" back; the finisher runs on the JavaScript thread, so thread-local socket tables stay single-threaded and the
/// finisher's result is the same wire string the synchronous host function would have returned. JavaScript polls from a
/// (ref'd) timer, which also keeps the event loop alive while an operation is pending.
type AsyncFinisher = Box<dyn FnOnce() -> String + Send>;

struct AsyncOp {
    receiver: std::sync::mpsc::Receiver<AsyncFinisher>,
}

thread_local! {
    static ASYNC_OPS: RefCell<(u32, HashMap<u32, AsyncOp>)> = RefCell::new((1, HashMap::new()));
}

fn async_op_insert(receiver: std::sync::mpsc::Receiver<AsyncFinisher>) -> u32 {
    ASYNC_OPS.with(|ops| {
        let mut ops = ops.borrow_mut();
        let id = ops.0;
        ops.0 = ops.0.wrapping_add(1).max(1);
        ops.1.insert(id, AsyncOp { receiver });
        id
    })
}

/// Runs `work` on a worker thread; `work` returns the finisher that completes the operation on the JavaScript thread.
fn async_op_start<W>(work: W) -> u32
where
    W: FnOnce() -> AsyncFinisher + Send + 'static,
{
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("thaw-async-op".to_string())
        .spawn(move || {
            // A cancelled operation dropped its receiver: the finisher (and the socket it owns) is dropped here.
            let _ = sender.send(work());
        });
    if let Err(error) = spawned {
        let (failed_sender, failed_receiver) = std::sync::mpsc::channel();
        let message = format!("err:failed to start an asynchronous operation: {error}");
        let _ = failed_sender.send(Box::new(move || message) as AsyncFinisher);
        return async_op_insert(failed_receiver);
    }
    async_op_insert(receiver)
}

/// An operation whose outcome is already known (for example an argument error); it completes on the first poll.
fn async_op_ready(outcome: String) -> u32 {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = sender.send(Box::new(move || outcome) as AsyncFinisher);
    async_op_insert(receiver)
}

/// `"pending"` until the worker finished; then the finisher's wire string (the operation is removed).
fn async_op_poll(id: u32) -> String {
    let outcome = ASYNC_OPS.with(|ops| {
        let mut ops = ops.borrow_mut();
        let Some(op) = ops.1.get(&id) else {
            return Some(Err("err:unknown asynchronous operation".to_string()));
        };
        match op.receiver.try_recv() {
            Ok(finisher) => {
                ops.1.remove(&id);
                Some(Ok(finisher))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                ops.1.remove(&id);
                Some(Err("err:asynchronous operation failed".to_string()))
            }
        }
    });
    match outcome {
        None => "pending".to_string(),
        Some(Ok(finisher)) => finisher(),
        Some(Err(wire)) => wire,
    }
}

/// Forgets a pending operation; a socket the worker connects afterwards is dropped by the worker.
fn async_op_cancel(id: u32) {
    ASYNC_OPS.with(|ops| {
        ops.borrow_mut().1.remove(&id);
    });
}

fn net_connect_start(host: &str, port: u32) -> u32 {
    let port = match checked_port(port) {
        Ok(port) => port,
        Err(wire) => return async_op_ready(wire),
    };
    let host = host.to_string();
    async_op_start(move || {
        let connected = tcp_connect_structured(&host, port);
        Box::new(move || match connected {
            Ok(stream) => net_register_stream(stream, &host, port),
            Err(wire) => wire,
        })
    })
}

fn dns_lookup_start(hostname: String) -> u32 {
    async_op_start(move || {
        let json = dns_lookup_json(hostname);
        Box::new(move || json)
    })
}

/// Outbound bytes of one TCP socket. A dedicated thread owns a duplicate of the descriptor and writes in order, so a peer that
/// stops reading blocks only that thread (the event loop keeps running); JavaScript sees the queued size for backpressure and any
/// write failure on its next call.
enum NetWriterMessage {
    Data(Vec<u8>),
    Shutdown,
}

struct NetWriter {
    sender: std::sync::mpsc::Sender<NetWriterMessage>,
    pending: Arc<std::sync::atomic::AtomicUsize>,
    failure: Arc<Mutex<Option<String>>>,
    /// `end()` already queued the write-side shutdown: a later `destroy()` must let the queued bytes drain first.
    shutdown_requested: bool,
}

thread_local! {
    static NET_WRITERS: RefCell<HashMap<u32, NetWriter>> = RefCell::new(HashMap::new());
}

fn net_writer_loop(
    mut stream: TcpStream,
    receiver: std::sync::mpsc::Receiver<NetWriterMessage>,
    pending: Arc<std::sync::atomic::AtomicUsize>,
    failure: Arc<Mutex<Option<String>>>,
) {
    use std::sync::atomic::Ordering;
    for message in receiver {
        match message {
            NetWriterMessage::Data(bytes) => {
                let mut offset = 0;
                while offset < bytes.len() {
                    match stream.write(&bytes[offset..]) {
                        Ok(0) => {
                            if let Ok(mut slot) = failure.lock() {
                                slot.get_or_insert_with(|| "write returned zero bytes".to_string());
                            }
                            return;
                        }
                        Ok(written) => {
                            offset += written;
                            pending.fetch_sub(written, Ordering::AcqRel);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        // The reader toggles O_NONBLOCK on the shared descriptor while it polls.
                        // ponytail: sleep-retry, a poll(POLLOUT) wait is the upgrade if this ever shows up in profiles.
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => {
                            if let Ok(mut slot) = failure.lock() {
                                slot.get_or_insert_with(|| error.to_string());
                            }
                            return;
                        }
                    }
                }
            }
            NetWriterMessage::Shutdown => {
                if let Err(error) = stream.shutdown(Shutdown::Write) {
                    if let Ok(mut slot) = failure.lock() {
                        slot.get_or_insert_with(|| error.to_string());
                    }
                }
                return;
            }
        }
    }
}

fn net_writer_failure(handle: u32) -> Option<String> {
    NET_WRITERS.with(|writers| {
        writers
            .borrow()
            .get(&handle)
            .and_then(|writer| writer.failure.lock().ok().and_then(|slot| slot.clone()))
    })
}

/// Queues `value` for the socket's writer thread, creating the thread on the first write.
fn net_write(handle: u32, value: &[u8]) -> String {
    use std::sync::atomic::Ordering;
    let clone = NET_STREAMS.with(|streams| {
        streams
            .borrow()
            .1
            .get(&handle)
            .map(|stream| stream.try_clone())
    });
    NET_WRITERS.with(|writers| {
        let mut writers = writers.borrow_mut();
        let writer = match writers.entry(handle) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let stream = match clone {
                    Some(Ok(stream)) => stream,
                    Some(Err(error)) => return format!("err:{error}"),
                    None => return "err:socket is closed".to_string(),
                };
                let (sender, receiver) = std::sync::mpsc::channel();
                let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let failure = Arc::new(Mutex::new(None));
                let thread_pending = Arc::clone(&pending);
                let thread_failure = Arc::clone(&failure);
                if let Err(error) = std::thread::Builder::new()
                    .name("thaw-net-writer".to_string())
                    .spawn(move || net_writer_loop(stream, receiver, thread_pending, thread_failure))
                {
                    return format!("err:{error}");
                }
                entry.insert(NetWriter { sender, pending, failure, shutdown_requested: false })
            }
        };
        if let Some(error) = writer.failure.lock().ok().and_then(|slot| slot.clone()) {
            return format!("err:{error}");
        }
        writer.pending.fetch_add(value.len(), Ordering::AcqRel);
        match writer.sender.send(NetWriterMessage::Data(value.to_vec())) {
            Ok(()) => "ok".to_string(),
            Err(_) => "err:socket is closed".to_string(),
        }
    })
}

/// Bytes accepted by `net_write` that the writer thread has not handed to the kernel yet.
fn net_pending_bytes(handle: u32) -> u32 {
    NET_WRITERS.with(|writers| {
        writers
            .borrow()
            .get(&handle)
            .map(|writer| writer.pending.load(std::sync::atomic::Ordering::Acquire).min(u32::MAX as usize) as u32)
            .unwrap_or(0)
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
    let queued = NET_WRITERS.with(|writers| {
        writers.borrow_mut().get_mut(&handle).map(|writer| {
            writer.shutdown_requested = true;
            writer.sender.send(NetWriterMessage::Shutdown).is_ok()
        })
    });
    if let Some(accepted) = queued {
        return if accepted { "ok".to_string() } else { "err:socket is closed".to_string() };
    }
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

/// TCP_NODELAY / SO_KEEPALIVE (+ idle time) on an already connected socket.
fn tcp_socket_option(stream: &TcpStream, option: &str, enable: bool, delay_ms: u32) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    match option {
        "nodelay" => stream.set_nodelay(enable),
        "keepalive" => {
            let fd = stream.as_raw_fd();
            let set = |level: libc::c_int, name: libc::c_int, value: libc::c_int| -> io::Result<()> {
                let result = unsafe {
                    libc::setsockopt(fd, level, name, (&value as *const libc::c_int).cast(), std::mem::size_of::<libc::c_int>() as libc::socklen_t)
                };
                if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
            };
            set(libc::SOL_SOCKET, libc::SO_KEEPALIVE, libc::c_int::from(enable))?;
            if enable && delay_ms > 0 {
                // Node passes the initial delay in milliseconds; the kernel takes whole seconds (at least one).
                let seconds = libc::c_int::try_from((delay_ms / 1000).max(1)).unwrap_or(libc::c_int::MAX);
                #[cfg(any(target_os = "linux", target_os = "android"))]
                set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, seconds)?;
                #[cfg(target_vendor = "apple")]
                set(libc::IPPROTO_TCP, libc::TCP_KEEPALIVE, seconds)?;
                #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
                let _ = seconds;
            }
            Ok(())
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown socket option")),
    }
}

/// `kind` is `tcp`, `tls` (client or server connection, whichever owns the handle).
fn net_set_option(kind: &str, handle: u32, option: &str, enable: bool, delay_ms: u32) -> String {
    let outcome: Option<io::Result<()>> = match kind {
        "tcp" => NET_STREAMS.with(|streams| streams.borrow().1.get(&handle).map(|stream| tcp_socket_option(stream, option, enable, delay_ms))),
        #[cfg(feature = "tls")]
        "tls" => TLS_STREAMS
            .with(|streams| streams.borrow().1.get(&handle).map(|stream| tcp_socket_option(&stream.sock, option, enable, delay_ms)))
            .or_else(|| {
                TLS_SERVER_STREAMS.with(|streams| streams.borrow().1.get(&handle).map(|stream| tcp_socket_option(&stream.sock, option, enable, delay_ms)))
            }),
        _ => None,
    };
    match outcome {
        Some(Ok(())) => "ok".to_string(),
        Some(Err(error)) => format!("err:{error}"),
        None => "err:socket is closed".to_string(),
    }
}

fn net_poll_read(handle: u32) -> String {
    net_poll_read_impl(handle, false)
}

fn net_poll_read_impl(handle: u32, retain_on_eof: bool) -> String {
    let result = net_poll_read_stream(handle, retain_on_eof);
    if result == "pending" {
        if let Some(error) = net_writer_failure(handle) {
            net_destroy(handle);
            return format!("err:{error}");
        }
    }
    if result.starts_with("err:") || (result == "eof" && !retain_on_eof) {
        NET_WRITERS.with(|writers| {
            writers.borrow_mut().remove(&handle);
        });
    }
    result
}

fn net_poll_read_stream(handle: u32, retain_on_eof: bool) -> String {
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

/// Hands the connected stream behind `handle` to the caller (STARTTLS). The socket's writer thread
/// drains what is already queued and exits when its channel closes.
fn net_take_stream(handle: u32) -> Option<TcpStream> {
    NET_WRITERS.with(|writers| writers.borrow_mut().remove(&handle));
    NET_STREAMS.with(|streams| streams.borrow_mut().1.remove(&handle))
}

fn net_destroy(handle: u32) {
    net_destroy_with(handle, false);
}

/// `reset` closes with `SO_LINGER{on, 0}` and no FIN, so the peer sees an RST (`socket.resetAndDestroy()`).
fn net_destroy_with(handle: u32, reset: bool) {
    // After `end()` the queued bytes still belong to the peer: the writer thread drains them and closes its descriptor.
    let draining = NET_WRITERS.with(|writers| {
        writers
            .borrow_mut()
            .remove(&handle)
            .is_some_and(|writer| writer.shutdown_requested)
    });
    NET_STREAMS.with(|streams| {
        if let Some(stream) = streams.borrow_mut().1.remove(&handle) {
            if reset {
                use std::os::fd::AsRawFd;
                let linger = libc::linger { l_onoff: 1, l_linger: 0 };
                unsafe {
                    libc::setsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_LINGER, (&linger as *const libc::linger).cast(), std::mem::size_of::<libc::linger>() as libc::socklen_t);
                }
            } else if !draining {
                let _ = stream.shutdown(Shutdown::Both);
            }
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

/// UDP socket options Node exposes beyond broadcast: TTLs, multicast loopback / interface / membership.
/// `name` selects the option; `a`, `b`, `c` are its textual arguments (group, interface, source address or a number).
#[cfg(unix)]
fn udp_option(handle: u32, name: &str, a: &str, b: &str, c: &str) -> String {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::os::fd::AsRawFd;
    UDP_SOCKETS.with(|sockets| {
        let sockets = sockets.borrow();
        let Some(socket) = sockets.1.get(&handle) else { return "err|EBADF|socket is closed".to_string(); };
        let fd = socket.as_raw_fd();
        let v6 = socket.local_addr().map(|address| address.is_ipv6()).unwrap_or(false);
        let invalid = |what: &str| format!("err|EINVAL|{what} EINVAL");
        fn set<T>(fd: libc::c_int, level: libc::c_int, option: libc::c_int, value: &T) -> Result<(), String> {
            let result = unsafe { libc::setsockopt(fd, level, option, (value as *const T).cast(), std::mem::size_of::<T>() as libc::socklen_t) };
            if result == 0 { Ok(()) } else { Err(udp_error(io::Error::last_os_error())) }
        }
        let number = |text: &str| text.parse::<libc::c_int>().ok();
        let outcome: Result<(), String> = match name {
            "ttl" => match number(a) {
                Some(value) if v6 => set(fd, libc::IPPROTO_IPV6, libc::IPV6_UNICAST_HOPS, &value),
                Some(value) => set(fd, libc::IPPROTO_IP, libc::IP_TTL, &value),
                None => return invalid("setTTL"),
            },
            "multicast_ttl" => match number(a) {
                Some(value) if v6 => set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_HOPS, &value),
                Some(value) => set(fd, libc::IPPROTO_IP, libc::IP_MULTICAST_TTL, &value),
                None => return invalid("setMulticastTTL"),
            },
            "multicast_loop" => {
                let value = libc::c_int::from(a == "1");
                if v6 { set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_LOOP, &value) } else { set(fd, libc::IPPROTO_IP, libc::IP_MULTICAST_LOOP, &value) }
            }
            "multicast_if" => {
                if v6 {
                    let index = number(a).map(|value| value as libc::c_uint).unwrap_or(0);
                    set(fd, libc::IPPROTO_IPV6, libc::IPV6_MULTICAST_IF, &index)
                } else {
                    match a.parse::<Ipv4Addr>() {
                        Ok(address) => set(fd, libc::IPPROTO_IP, libc::IP_MULTICAST_IF, &libc::in_addr { s_addr: u32::from_ne_bytes(address.octets()) }),
                        Err(_) => return invalid("setMulticastInterface"),
                    }
                }
            }
            "add_membership" | "drop_membership" => {
                let add = name == "add_membership";
                if v6 {
                    let Ok(group) = a.parse::<Ipv6Addr>() else { return invalid(if add { "addMembership" } else { "dropMembership" }) };
                    let request = libc::ipv6_mreq {
                        ipv6mr_multiaddr: libc::in6_addr { s6_addr: group.octets() },
                        ipv6mr_interface: number(b).map(|value| value as libc::c_uint).unwrap_or(0),
                    };
                    set(fd, libc::IPPROTO_IPV6, if add { libc::IPV6_ADD_MEMBERSHIP } else { libc::IPV6_DROP_MEMBERSHIP }, &request)
                } else {
                    let Ok(group) = a.parse::<Ipv4Addr>() else { return invalid(if add { "addMembership" } else { "dropMembership" }) };
                    let interface = if b.is_empty() { Some(Ipv4Addr::UNSPECIFIED) } else { b.parse::<Ipv4Addr>().ok() };
                    let Some(interface) = interface else { return invalid(if add { "addMembership" } else { "dropMembership" }) };
                    let request = libc::ip_mreq {
                        imr_multiaddr: libc::in_addr { s_addr: u32::from_ne_bytes(group.octets()) },
                        imr_interface: libc::in_addr { s_addr: u32::from_ne_bytes(interface.octets()) },
                    };
                    set(fd, libc::IPPROTO_IP, if add { libc::IP_ADD_MEMBERSHIP } else { libc::IP_DROP_MEMBERSHIP }, &request)
                }
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            "add_source_membership" | "drop_source_membership" => {
                let add = name == "add_source_membership";
                if v6 {
                    return "err|ENOTSUP|source-specific IPv6 membership is not supported".to_string();
                }
                // a = source, b = group, c = interface
                let (Ok(source), Ok(group)) = (a.parse::<Ipv4Addr>(), b.parse::<Ipv4Addr>()) else { return invalid("addSourceSpecificMembership") };
                let interface = if c.is_empty() { Some(Ipv4Addr::UNSPECIFIED) } else { c.parse::<Ipv4Addr>().ok() };
                let Some(interface) = interface else { return invalid("addSourceSpecificMembership") };
                let request = libc::ip_mreq_source {
                    imr_multiaddr: libc::in_addr { s_addr: u32::from_ne_bytes(group.octets()) },
                    imr_interface: libc::in_addr { s_addr: u32::from_ne_bytes(interface.octets()) },
                    imr_sourceaddr: libc::in_addr { s_addr: u32::from_ne_bytes(source.octets()) },
                };
                set(fd, libc::IPPROTO_IP, if add { libc::IP_ADD_SOURCE_MEMBERSHIP } else { libc::IP_DROP_SOURCE_MEMBERSHIP }, &request)
            }
            _ => return invalid("socket option"),
        };
        match outcome {
            Ok(()) => "ok".to_string(),
            Err(wire) => wire,
        }
    })
}

#[cfg(not(unix))]
fn udp_option(_: u32, _: &str, _: &str, _: &str, _: &str) -> String {
    "err|ENOTSUP|UDP socket options are not supported on this platform".to_string()
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

/// Protocol versions allowed by `minVersion`/`maxVersion` (rustls implements TLS 1.2 and 1.3 only).
fn tls_protocol_versions(spec: &str) -> Result<Vec<&'static rustls::SupportedProtocolVersion>, String> {
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

pub(super) struct TlsClientOptions<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) server_name: &'a str,
    pub(super) ca_spec: Option<&'a str>,
    pub(super) cert_spec: &'a str,
    pub(super) key_spec: &'a str,
    pub(super) alpn_spec: &'a str,
    /// `minVersion,maxVersion` (TLSv1 .. TLSv1.3), either side may be empty.
    pub(super) versions_spec: &'a str,
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
    let Ok((_, last_certificate)) = x509_parser::parse_x509_certificate(last) else { return };
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
    let builder = ClientConfig::builder_with_protocol_versions(&versions).with_root_certificates(roots.clone());
    let mut local_certificate = None;
    let config = if cert_spec.is_empty() && key_spec.is_empty() {
        builder.with_no_client_auth()
    } else if cert_spec.is_empty() || key_spec.is_empty() {
        return Err("err:client cert and key must be provided together".to_string());
    } else {
        let certificates = decode_certificates(cert_spec).map_err(|error| format!("err:{error}"))?;
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
    config.dangerous().set_certificate_verifier(Arc::new(RecordingServerVerifier {
        roots,
        defer_name: defer_name_verification,
        allow_invalid: !reject_unauthorized,
        result: Arc::clone(&verifier_result),
        algorithms: rustls::crypto::ring::default_provider().signature_verification_algorithms,
    }));
    let name = ServerName::try_from(server_name.to_string()).map_err(|error| format!("err:{error}"))?;
    let mut ca_certificates = Vec::new();
    for encoded in ca_spec.unwrap_or("").split(',').filter(|encoded| !encoded.is_empty()) {
        let certificates = decode_ca_certificates(&hex_decode(encoded))
            .map_err(|error| format!("err:invalid CA certificate: {error}"))?;
        ca_certificates.extend(certificates.into_iter().map(|certificate| certificate.as_ref().to_vec()));
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
        let mut connection = ClientConnection::new(self.config, self.name).map_err(|error| format!("err:{error}"))?;
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
            .map(|certificates| certificates.iter().map(|certificate| certificate.as_ref().to_vec()).collect())
            .unwrap_or_default();
        tls_extend_chain_with_anchor(&mut peer_chain, &ca_certificates);
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

pub(super) fn tls_connect(options: TlsClientOptions<'_>) -> String {
    match tls_client_prepare(&options).and_then(TlsClientJob::connect) {
        Ok(done) => done.register(),
        Err(wire) => wire,
    }
}

/// Like `tls_connect`, but the TCP connect and the handshake run on a worker thread (see `async_op_start`).
pub(super) fn tls_connect_start(options: TlsClientOptions<'_>) -> u32 {
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
pub(super) fn tls_wrap_start(options: TlsClientOptions<'_>, net_handle: u32) -> u32 {
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

pub(super) struct TlsServerOptions<'a> {
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) cert_spec: &'a str,
    pub(super) key_spec: &'a str,
    pub(super) ca_spec: Option<&'a str>,
    pub(super) request_cert: bool,
    pub(super) reject_unauthorized: bool,
    pub(super) alpn_spec: &'a str,
    pub(super) versions_spec: &'a str,
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
    let roots = match tls_ca_roots(ca_spec) { Ok(roots) => roots, Err(error) => return format!("err:{error}") };
    let versions = match tls_protocol_versions(versions_spec) { Ok(versions) => versions, Err(error) => return format!("err:{error}") };
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

type TlsAcceptedParts = (TcpStream, std::net::SocketAddr, Arc<ServerConfig>, Vec<u8>, bool);

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
fn tls_server_handshake(mut socket: TcpStream, config: Arc<ServerConfig>) -> Result<(ServerConnection, TcpStream), String> {
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
        .map(|certificates| certificates.iter().map(|certificate| certificate.as_ref().to_vec()).collect())
        .unwrap_or_default();
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
                        peer_chain,
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

thread_local! {
    /// Handshakes in flight, per TLS listener (operation ids in the asynchronous operation table).
    static TLS_ACCEPT_PENDING: RefCell<HashMap<u32, Vec<u32>>> = RefCell::new(HashMap::new());
}

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
            if TLS_ACCEPT_PENDING.with(|pending| pending.borrow().get(&handle).is_some_and(|operations| operations.len() >= 64)) {
                return "err:pending".to_string();
            }
            match tls_server_accept_tcp(handle, true) {
                Ok((socket, peer, config, local_certificate, request_cert)) => {
                    let operation = async_op_start(move || {
                        let handshaken = tls_server_handshake(socket, config);
                        Box::new(move || match handshaken {
                            Ok((connection, socket)) => tls_server_register(connection, socket, peer, local_certificate, request_cert),
                            Err(wire) => wire,
                        })
                    });
                    TLS_ACCEPT_PENDING.with(|pending| pending.borrow_mut().entry(handle).or_default().push(operation));
                }
                Err(error) if error == "pending" => return "err:pending".to_string(),
                Err(error) => return format!("err:{error}"),
            }
        }
    }
    let (socket, peer, config, local_certificate, request_cert) = match tls_server_accept_tcp(handle, false) {
        Ok(accepted) => accepted,
        Err(error) => return format!("err:{error}"),
    };
    match tls_server_handshake(socket, config) {
        Ok((connection, socket)) => tls_server_register(connection, socket, peer, local_certificate, request_cert),
        Err(wire) => wire,
    }
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
    for operation in TLS_ACCEPT_PENDING.with(|pending| pending.borrow_mut().remove(&handle)).unwrap_or_default() {
        async_op_cancel(operation);
    }
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

/// The peer's whole certificate chain as a JSON array of hex strings (leaf first); empty when unknown.
pub(super) fn tls_peer_chain(handle: u32) -> String {
    let read = |certificates: &HashMap<u32, TlsCertificates>| certificates.get(&handle).map(|entry| entry.peer_chain.clone());
    let chain = TLS_CLIENT_CERTIFICATES
        .with(|certificates| read(&certificates.borrow()))
        .or_else(|| TLS_SERVER_CERTIFICATES.with(|certificates| read(&certificates.borrow())))
        .unwrap_or_default();
    serde_json::Value::Array(chain.iter().map(|certificate| serde_json::Value::String(hex_encode(certificate))).collect()).to_string()
}

/// Metadata (subject, issuer, validity, ...) of one DER certificate given as hex.
pub(super) fn tls_certificate_metadata_hex(hex: &str) -> String {
    certificate_metadata(&hex_decode(hex))
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
    versions_spec: &'a str,
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
    versions_spec: &'a str,
}

#[cfg(not(feature = "tls"))]
fn tls_unavailable() -> String { "err:TLS support is not linked".to_string() }
#[cfg(not(feature = "tls"))]
fn tls_connect(_: TlsClientOptions<'_>) -> String { tls_unavailable() }
#[cfg(not(feature = "tls"))]
fn tls_connect_start(_: TlsClientOptions<'_>) -> u32 { async_op_ready(tls_unavailable()) }
#[cfg(not(feature = "tls"))]
fn tls_wrap_start(_: TlsClientOptions<'_>, _: u32) -> u32 { async_op_ready(tls_unavailable()) }
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
fn tls_peer_chain(_: u32) -> String { "[]".to_string() }
#[cfg(not(feature = "tls"))]
fn tls_certificate_metadata_hex(_: &str) -> String { "{}".to_string() }
#[cfg(not(feature = "tls"))]
fn tls_authorization(_: u32) -> String { r#"{"authorized":false,"authorizationError":"TLS support is unavailable","chainVerified":false}"#.to_string() }

fn tls_canonical_ip(host: &str) -> String {
    host.parse::<std::net::IpAddr>().map(|address| address.to_string()).unwrap_or_default()
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn net_set_option_applies_nodelay_and_keepalive_to_the_socket() {
    use std::os::fd::AsRawFd;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let connected = net_connect("127.0.0.1", port);
    let handle: u32 = connected.strip_prefix("ok:").unwrap().split(':').next().unwrap().parse().unwrap();
    let (_peer, _) = listener.accept().unwrap();
    let read = |level: libc::c_int, name: libc::c_int| -> libc::c_int {
        NET_STREAMS.with(|streams| {
            let streams = streams.borrow();
            let stream = streams.1.get(&handle).unwrap();
            let mut value: libc::c_int = -1;
            let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let result = unsafe { libc::getsockopt(stream.as_raw_fd(), level, name, (&mut value as *mut libc::c_int).cast(), &mut length) };
            assert_eq!(result, 0);
            value
        })
    };
    assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 0);
    assert_eq!(net_set_option("tcp", handle, "keepalive", true, 4500), "ok");
    assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 1);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE), 4);
    assert_eq!(net_set_option("tcp", handle, "keepalive", false, 0), "ok");
    assert_eq!(read(libc::SOL_SOCKET, libc::SO_KEEPALIVE), 0);
    assert_eq!(net_set_option("tcp", handle, "nodelay", true, 0), "ok");
    assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_NODELAY), 1);
    assert_eq!(net_set_option("tcp", handle, "nodelay", false, 0), "ok");
    assert_eq!(read(libc::IPPROTO_TCP, libc::TCP_NODELAY), 0);
    assert_eq!(net_set_option("tcp", 4_000_000, "nodelay", true, 0), "err:socket is closed");
    net_destroy(handle);
}

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
