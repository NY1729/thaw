//! `node:net` host (plain TCP): the `__thaw_net_*` / `__thaw_async_*` functions the registry shim calls, plus
//! the small hosts the http stack needs (HPACK huffman, hash, timer refs).
//! Same wire protocol and logic as crates/thaw-quickjs networking.rs; sockets are tables owned by the
//! interpreter thread, slow work (connect) runs on a worker thread and is collected by `__thaw_async_poll`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::zlib::{hex_decode, hex_encode};
use crate::*;

/// Process-wide tables behind a mutex. Compiled async continuations can run on a different thread than the one that
/// started an operation, so thread-locals (the QuickJS host's choice, one JS thread) would lose sockets and jobs.
/// `with` keeps the QuickJS host's `Table.with(|t| t.borrow_mut()...)` shape.
pub(crate) struct Slot<'a, T>(&'a Mutex<T>);

impl<T> Slot<'_, T> {
    pub(crate) fn borrow(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn borrow_mut(&self) -> std::sync::MutexGuard<'_, T> {
        self.borrow()
    }
}

pub(crate) trait With<T> {
    fn with<R>(&self, f: impl FnOnce(&Slot<'_, T>) -> R) -> R;
}

impl<T> With<T> for std::sync::LazyLock<Mutex<T>> {
    fn with<R>(&self, f: impl FnOnce(&Slot<'_, T>) -> R) -> R {
        f(&Slot(self))
    }
}

pub(crate) type Table<T> = std::sync::LazyLock<Mutex<T>>;

pub(crate) static NET_STREAMS: Table<(u32, HashMap<u32, TcpStream>)> =
    Table::new(|| Mutex::new((1, HashMap::new())));
pub(crate) static NET_LISTENERS: Table<(u32, HashMap<u32, TcpListener>)> =
    Table::new(|| Mutex::new((1, HashMap::new())));

/// Ports arrive from JavaScript as 32-bit numbers; narrowing without a range
/// check would silently turn 65536 into 0 ("any free port").
pub(crate) fn checked_port(port: u32) -> Result<u16, String> {
    u16::try_from(port).map_err(|_| format!("err:port should be >= 0 and < 65536. Received {port}"))
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
pub(crate) fn connect_failure_wire(error: &io::Error, address: &str) -> String {
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
pub(crate) fn tcp_connect_structured(host: &str, port: u16) -> Result<TcpStream, String> {
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
    Err(last.unwrap_or_else(|| {
        format!("err:ENOTFOUND|-3008|getaddrinfo|{host}|no addresses for {host}")
    }))
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

static ASYNC_OPS: Table<(u32, HashMap<u32, AsyncOp>)> =
    Table::new(|| Mutex::new((1, HashMap::new())));

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
pub(crate) fn async_op_start<W>(work: W) -> u32
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
pub(crate) fn async_op_ready(outcome: String) -> u32 {
    let (sender, receiver) = std::sync::mpsc::channel();
    let _ = sender.send(Box::new(move || outcome) as AsyncFinisher);
    async_op_insert(receiver)
}

/// `"pending"` until the worker finished; then the finisher's wire string (the operation is removed).
pub(crate) fn async_op_poll(id: u32) -> String {
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
pub(crate) fn async_op_cancel(id: u32) {
    ASYNC_OPS.with(|ops| {
        ops.borrow_mut().1.remove(&id);
    });
}

pub(crate) fn net_connect_start(host: &str, port: u32) -> u32 {
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

static NET_WRITERS: Table<HashMap<u32, NetWriter>> = Table::new(|| Mutex::new(HashMap::new()));

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
                    .spawn(move || {
                        net_writer_loop(stream, receiver, thread_pending, thread_failure)
                    })
                {
                    return format!("err:{error}");
                }
                entry.insert(NetWriter {
                    sender,
                    pending,
                    failure,
                    shutdown_requested: false,
                })
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
            .map(|writer| {
                writer
                    .pending
                    .load(std::sync::atomic::Ordering::Acquire)
                    .min(u32::MAX as usize) as u32
            })
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
        return if accepted {
            "ok".to_string()
        } else {
            "err:socket is closed".to_string()
        };
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
pub(crate) fn tcp_socket_option(
    stream: &TcpStream,
    option: &str,
    enable: bool,
    delay_ms: u32,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    match option {
        "nodelay" => stream.set_nodelay(enable),
        "keepalive" => {
            let fd = stream.as_raw_fd();
            let set =
                |level: libc::c_int, name: libc::c_int, value: libc::c_int| -> io::Result<()> {
                    let result = unsafe {
                        libc::setsockopt(
                            fd,
                            level,
                            name,
                            (&value as *const libc::c_int).cast(),
                            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                        )
                    };
                    if result == 0 {
                        Ok(())
                    } else {
                        Err(io::Error::last_os_error())
                    }
                };
            set(
                libc::SOL_SOCKET,
                libc::SO_KEEPALIVE,
                libc::c_int::from(enable),
            )?;
            if enable && delay_ms > 0 {
                // Node passes the initial delay in milliseconds; the kernel takes whole seconds (at least one).
                let seconds =
                    libc::c_int::try_from((delay_ms / 1000).max(1)).unwrap_or(libc::c_int::MAX);
                #[cfg(any(target_os = "linux", target_os = "android"))]
                set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, seconds)?;
                #[cfg(target_vendor = "apple")]
                set(libc::IPPROTO_TCP, libc::TCP_KEEPALIVE, seconds)?;
                #[cfg(not(any(
                    target_os = "linux",
                    target_os = "android",
                    target_vendor = "apple"
                )))]
                let _ = seconds;
            }
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown socket option",
        )),
    }
}

/// `kind` is `tcp`, `tls` (client or server connection, whichever owns the handle).
fn net_set_option(kind: &str, handle: u32, option: &str, enable: bool, delay_ms: u32) -> String {
    let outcome: Option<io::Result<()>> = match kind {
        "tcp" => NET_STREAMS.with(|streams| {
            streams
                .borrow()
                .1
                .get(&handle)
                .map(|stream| tcp_socket_option(stream, option, enable, delay_ms))
        }),
        "tls" => crate::tls::socket_option(handle, option, enable, delay_ms),
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
pub(crate) fn net_take_stream(handle: u32) -> Option<TcpStream> {
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
                let linger = libc::linger {
                    l_onoff: 1,
                    l_linger: 0,
                };
                unsafe {
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        (&linger as *const libc::linger).cast(),
                        std::mem::size_of::<libc::linger>() as libc::socklen_t,
                    );
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

pub(crate) fn s(it: &mut Interp, a: &[Value], i: usize) -> String {
    it.to_str(&arg(a, i)).to_string()
}

pub(crate) fn n(it: &mut Interp, a: &[Value], i: usize) -> u32 {
    it.num(&arg(a, i)) as u32
}

pub(crate) fn install(it: &mut Interp) {
    let g = it.global.clone();
    macro_rules! def {
        ($name:expr, $f:expr) => {{
            let f = it.native($f);
            g.declare($name, f, true);
        }};
    }
    def!("__thaw_net_connect", |it, _, a| {
        let (host, port) = (s(it, a, 0), n(it, a, 1));
        Ok(Value::str(&match checked_port(port) {
            Ok(port) => net_connect(&host, port),
            Err(e) => e,
        }))
    });
    def!("__thaw_net_connect_start", |it, _, a| {
        let (host, port) = (s(it, a, 0), n(it, a, 1));
        Ok(Value::Number(f64::from(net_connect_start(&host, port))))
    });
    def!("__thaw_async_poll", |it, _, a| Ok(Value::str(
        &async_op_poll(n(it, a, 0))
    )));
    def!("__thaw_async_cancel", |it, _, a| {
        async_op_cancel(n(it, a, 0));
        Ok(Value::Undefined)
    });
    def!("__thaw_net_write", |it, _, a| {
        let h = n(it, a, 0);
        let bytes = hex_decode(&s(it, a, 1));
        Ok(Value::str(&net_write(h, &bytes)))
    });
    def!("__thaw_net_set_option", |it, _, a| {
        let (kind, h, option) = (s(it, a, 0), n(it, a, 1), s(it, a, 2));
        let (enable, delay) = (arg(a, 3).truthy(), n(it, a, 4));
        Ok(Value::str(&net_set_option(
            &kind, h, &option, enable, delay,
        )))
    });
    def!("__thaw_net_pending", |it, _, a| Ok(Value::Number(
        f64::from(net_pending_bytes(n(it, a, 0)))
    )));
    def!("__thaw_net_finish", |it, _, a| Ok(Value::str(&net_finish(
        n(it, a, 0)
    ))));
    def!("__thaw_net_shutdown", |it, _, a| Ok(Value::str(
        &net_shutdown_write(n(it, a, 0))
    )));
    def!("__thaw_net_poll_read", |it, _, a| {
        let h = n(it, a, 0);
        let retain = arg(a, 1).truthy();
        Ok(Value::str(&if retain {
            net_poll_read_impl(h, true)
        } else {
            net_poll_read(h)
        }))
    });
    def!("__thaw_net_destroy", |it, _, a| {
        net_destroy_with(n(it, a, 0), arg(a, 1).truthy());
        Ok(Value::Undefined)
    });
    def!("__thaw_net_listen", |it, _, a| {
        let (host, port) = (s(it, a, 0), n(it, a, 1));
        Ok(Value::str(&match checked_port(port) {
            Ok(port) => net_listen(&host, port),
            Err(e) => e,
        }))
    });
    def!("__thaw_net_accept", |it, _, a| Ok(Value::str(&net_accept(
        n(it, a, 0)
    ))));
    def!("__thaw_net_poll_accept", |it, _, a| Ok(Value::str(
        &net_poll_accept(n(it, a, 0))
    )));
    def!("__thaw_net_read", |it, _, a| Ok(Value::str(&net_read_all(
        n(it, a, 0)
    ))));
    def!("__thaw_net_close_listener", |it, _, a| {
        net_close_listener(n(it, a, 0));
        Ok(Value::Undefined)
    });

    // ----- HPACK huffman (node:http2), hash, timer refs -----
    def!("__thaw_hpack_huffman_encode", |it, _, a| {
        let mut out = Vec::new();
        match httlib_huffman::encode(&hex_decode(&s(it, a, 0)), &mut out) {
            Ok(()) => Ok(Value::str(&hex_encode(&out))),
            Err(e) => it.throw_error("Error", &e.to_string()),
        }
    });
    def!("__thaw_hpack_huffman_decode", |it, _, a| {
        let mut out = Vec::new();
        match httlib_huffman::decode(
            &hex_decode(&s(it, a, 0)),
            &mut out,
            httlib_huffman::DecoderSpeed::FourBits,
        ) {
            Ok(()) => Ok(Value::str(&hex_encode(&out))),
            Err(e) => it.throw_error("Error", &e.to_string()),
        }
    });
    def!("__thaw_crypto_hash_hex", |it, _, a| {
        use sha2::Digest;
        let (alg, data) = (
            s(it, a, 0).to_ascii_lowercase().replace('-', ""),
            hex_decode(&s(it, a, 1)),
        );
        let sum = match alg.as_str() {
            "md5" => md5::Md5::digest(&data).to_vec(),
            "sha1" => sha1::Sha1::digest(&data).to_vec(),
            "sha256" => sha2::Sha256::digest(&data).to_vec(),
            "sha384" => sha2::Sha384::digest(&data).to_vec(),
            "sha512" => sha2::Sha512::digest(&data).to_vec(),
            _ => return it.throw_error("Error", &format!("Digest method not supported: {alg}")),
        };
        Ok(Value::str(&hex_encode(&sum)))
    });
    let _ = it.run(
        "(function () {\n\
         Object.defineProperty(process, Symbol.toStringTag, { value: 'process', configurable: true });\n\
         globalThis.__thaw_set_timeout_ref = function (cb, delay, refed) { var t = setTimeout(cb, delay); if (!refed) t.unref(); return t; };\n\
         globalThis.__thaw_set_timer_ref = function (t, refed) { if (t) { if (refed) t.ref(); else t.unref(); } };\n\
         })();",
    );
}
