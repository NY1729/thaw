//! `node:child_process` host: the `__thaw_child_process_*` functions the registry shim calls, plus a thread-local
//! process table the event loop polls (`__thaw_poll_platform_events` -> `__thaw_child_process_poll`).
//! Plain std/libc; the JSON event shapes match the QuickJS host so the one JavaScript shim serves both.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::builtins::json_quote;
use crate::host::Native;
use crate::json::parse_native;
use crate::*;

enum Ev {
    Out(Vec<u8>),
    Err(Vec<u8>),
    Eof,
}

struct Kid {
    child: std::process::Child,
    rx: Receiver<Ev>,
    stdin: Option<Sender<Vec<u8>>>,
    refd: bool,
    open_pipes: u8,
    exited: bool,
    /// Events produced at spawn time (the `spawn` record, or `error` + `exit` for a failed spawn).
    pending: Vec<String>,
    /// The `exit` (close) record, held until the output pipes reach EOF.
    last: Option<String>,
}

#[derive(Default)]
struct Table {
    next: u32,
    kids: BTreeMap<u32, Kid>,
    failed: BTreeMap<u32, Vec<String>>,
}

thread_local! {
    static TABLE: RefCell<Table> = RefCell::new(Table::default());
}

fn q(s: &str) -> String {
    let mut out = String::new();
    json_quote(s, &mut out);
    out
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

fn opt<'a>(o: &'a Native, key: &str) -> Option<&'a Native> {
    match o {
        Native::Object(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn opt_str<'a>(o: &'a Native, key: &str) -> Option<&'a str> {
    match opt(o, key) {
        Some(Native::Str(s)) => Some(s),
        _ => None,
    }
}

fn opt_num(o: &Native, key: &str) -> Option<f64> {
    match opt(o, key) {
        Some(Native::Number(n)) => Some(*n),
        _ => None,
    }
}

fn error_code(e: &std::io::Error) -> &'static str {
    use std::io::ErrorKind::*;
    match (e.raw_os_error(), e.kind()) {
        (Some(libc::EPERM), _) => "EPERM",
        (Some(libc::EACCES), _) | (_, PermissionDenied) => "EACCES",
        (_, NotFound) => "ENOENT",
        (_, InvalidInput) => "EINVAL",
        (_, Unsupported) => "ENOTSUP",
        _ => "UNKNOWN",
    }
}

fn id_opt(o: &Native, key: &str) -> Option<u32> {
    opt_num(o, key).map(|n| n as i32 as u32)
}

fn command(cmd: &str, args: &Native, o: &Native) -> Command {
    let mut c = Command::new(cmd);
    if let Native::Array(a) = args {
        for a in a {
            if let Native::Str(s) = a {
                c.arg(s);
            }
        }
    }
    if let Some(dir) = opt_str(o, "cwd") {
        c.current_dir(dir);
    }
    if let Some(Native::Object(env)) = opt(o, "env") {
        c.env_clear();
        for (k, v) in env {
            if let Native::Str(v) = v {
                c.env(k, v);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let detached = matches!(opt(o, "detached"), Some(Native::Bool(true)));
        // SAFETY: only async-signal-safe libc calls, no allocation. Like libuv: default signal
        // dispositions and an empty mask, so a host that ignores SIGINT/SIGPIPE cannot make children unkillable.
        unsafe {
            c.pre_exec(move || {
                for s in 1..32 {
                    if s != libc::SIGKILL && s != libc::SIGSTOP {
                        libc::signal(s, libc::SIG_DFL);
                    }
                }
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
                if detached && libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        if let Some(g) = id_opt(o, "gid") {
            c.gid(g);
        }
        if let Some(u) = id_opt(o, "uid") {
            c.uid(u);
        }
    }
    c
}

fn spawn_retrying(c: &mut Command) -> std::io::Result<std::process::Child> {
    for _ in 0..50 {
        match c.spawn() {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            r => return r,
        }
    }
    c.spawn()
}

fn stdio_for(mode: &str) -> Stdio {
    match mode {
        "ignore" => Stdio::null(),
        "inherit" => Stdio::inherit(),
        _ => Stdio::piped(),
    }
}

fn pump(mut pipe: impl Read + Send + 'static, tx: Sender<Ev>, stderr: bool) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let v = buf[..n].to_vec();
                    if tx
                        .send(if stderr { Ev::Err(v) } else { Ev::Out(v) })
                        .is_err()
                    {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        let _ = tx.send(Ev::Eof);
    });
}

fn error_events(handle: u32, msg: &str, code: &str) -> Vec<String> {
    vec![
        format!(
            "{{\"handle\":{handle},\"type\":\"error\",\"message\":{},\"code\":{}}}",
            q(msg),
            q(code)
        ),
        format!("{{\"handle\":{handle},\"type\":\"exit\",\"code\":null,\"signal\":null}}"),
    ]
}

fn spawn(cmd: &str, args: &str, options: &str) -> u32 {
    let (args, o) = (
        parse_native(args).unwrap_or(Native::Undefined),
        parse_native(options).unwrap_or(Native::Undefined),
    );
    let handle = TABLE.with(|t| {
        let mut t = t.borrow_mut();
        t.next += 1;
        t.next
    });
    let modes: Vec<String> = match opt(&o, "stdio") {
        Some(Native::Array(a)) => a
            .iter()
            .map(|m| match m {
                Native::Str(s) => s.clone(),
                _ => "pipe".into(),
            })
            .collect(),
        _ => vec![],
    };
    let mode = |i: usize| modes.get(i).map_or("pipe", String::as_str);
    let mut c = command(cmd, &args, &o);
    c.stdin(stdio_for(mode(0)))
        .stdout(stdio_for(mode(1)))
        .stderr(stdio_for(mode(2)));
    match spawn_retrying(&mut c) {
        Err(e) => TABLE.with(|t| {
            t.borrow_mut()
                .failed
                .insert(handle, error_events(handle, &e.to_string(), error_code(&e)));
        }),
        Ok(mut child) => {
            let (tx, rx) = mpsc::channel();
            let mut open_pipes = 0;
            if let Some(p) = child.stdout.take() {
                pump(p, tx.clone(), false);
                open_pipes += 1;
            }
            if let Some(p) = child.stderr.take() {
                pump(p, tx.clone(), true);
                open_pipes += 1;
            }
            // Writes go through their own thread: a child that stops reading must not block the loop.
            let stdin = child.stdin.take().map(|mut pipe| {
                let (s, r) = mpsc::channel::<Vec<u8>>();
                std::thread::spawn(move || {
                    while let Ok(chunk) = r.recv() {
                        if pipe.write_all(&chunk).and_then(|()| pipe.flush()).is_err() {
                            break;
                        }
                    }
                });
                s
            });
            let pid = child.id();
            TABLE.with(|t| {
                t.borrow_mut().kids.insert(
                    handle,
                    Kid {
                        child,
                        rx,
                        stdin,
                        refd: true,
                        open_pipes,
                        exited: false,
                        last: None,
                        pending: vec![format!(
                            "{{\"handle\":{handle},\"type\":\"spawn\",\"pid\":{pid}}}"
                        )],
                    },
                );
            });
        }
    }
    handle
}

fn status_parts(s: &std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    #[cfg(unix)]
    let sig = std::os::unix::process::ExitStatusExt::signal(s);
    #[cfg(not(unix))]
    let sig: Option<i32> = None;
    (s.code(), sig)
}

fn num(v: Option<i32>) -> String {
    v.map_or("null".into(), |n| n.to_string())
}

/// Drain every child's queued output and lifecycle changes as a JSON array of events.
pub(crate) fn poll() -> String {
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        let mut out: Vec<String> = std::mem::take(&mut t.failed)
            .into_values()
            .flatten()
            .collect();
        let mut done = vec![];
        for (&h, k) in t.kids.iter_mut() {
            out.append(&mut k.pending);
            while let Ok(ev) = k.rx.try_recv() {
                match ev {
                    Ev::Out(v) => out.push(format!(
                        "{{\"handle\":{h},\"type\":\"stdout\",\"value\":\"{}\"}}",
                        hex(&v)
                    )),
                    Ev::Err(v) => out.push(format!(
                        "{{\"handle\":{h},\"type\":\"stderr\",\"value\":\"{}\"}}",
                        hex(&v)
                    )),
                    Ev::Eof => k.open_pipes -= 1,
                }
            }
            if !k.exited {
                if let Ok(Some(st)) = k.child.try_wait() {
                    k.exited = true;
                    k.stdin = None;
                    let (code, sig) = status_parts(&st);
                    // `exited` is the process ending; `exit` (close) additionally waits for pipe EOF.
                    out.push(format!(
                        "{{\"handle\":{h},\"type\":\"exited\",\"code\":{},\"signal\":{}}}",
                        num(code),
                        num(sig)
                    ));
                    // Stash the final record on the entry; emitted once the pipes close.
                    k.last = Some(format!(
                        "{{\"handle\":{h},\"type\":\"exit\",\"code\":{},\"signal\":{}}}",
                        num(code),
                        num(sig)
                    ));
                }
            }
            if k.exited && k.open_pipes == 0 {
                out.extend(k.last.take());
                done.push(h);
            }
        }
        for h in done {
            t.kids.remove(&h);
        }
        format!("[{}]", out.join(","))
    })
}

/// Referenced children (or undelivered spawn failures) keep the event loop alive.
pub(crate) fn active() -> bool {
    TABLE.with(|t| {
        let t = t.borrow();
        !t.failed.is_empty() || t.kids.values().any(|k| k.refd)
    })
}

fn kill(handle: u32, sig: i32) -> bool {
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(k) = t.kids.get_mut(&handle) else {
            return false;
        };
        if k.exited {
            return false;
        }
        let sent = unsafe { libc::kill(k.child.id() as libc::pid_t, sig) } == 0;
        if sent && sig != 0 {
            k.stdin = None;
        }
        sent
    })
}

#[derive(Default)]
struct Capture {
    data: Mutex<Vec<u8>>,
    finished: AtomicBool,
}

fn capture(
    mut pipe: impl Read + Send + 'static,
    total: Arc<AtomicUsize>,
    max: Option<usize>,
    over: Arc<AtomicBool>,
) -> Arc<Capture> {
    let cap = Arc::new(Capture::default());
    let c = cap.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n @ 1..) = pipe.read(&mut buf) {
            if over.load(Ordering::SeqCst) {
                continue; // keep draining so the child dies of our signal, not a closed pipe
            }
            c.data
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .extend_from_slice(&buf[..n]);
            if max.is_some_and(|m| total.fetch_add(n, Ordering::SeqCst) + n > m) {
                over.store(true, Ordering::SeqCst);
            }
        }
        c.finished.store(true, Ordering::SeqCst);
    });
    cap
}

fn sync(cmd: &str, args: &str, options: &str) -> String {
    let (args, o) = (
        parse_native(args).unwrap_or(Native::Undefined),
        parse_native(options).unwrap_or(Native::Undefined),
    );
    let fail = |e: &std::io::Error, code: &str| {
        format!(
            "{{\"error\":{},\"code\":{},\"errno\":null,\"path\":{}}}",
            q(&e.to_string()),
            q(code),
            q(cmd)
        )
    };
    let mut c = command(cmd, &args, &o);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let sig = opt_num(&o, "killSignal").map_or(15, |n| n as i32);
    let timeout = opt_num(&o, "timeout")
        .filter(|n| *n > 0.0)
        .map(|n| Duration::from_millis(n as u64));
    let max = opt_num(&o, "maxBuffer").map(|n| n as usize);
    let mut child = match spawn_retrying(&mut c) {
        Ok(c) => c,
        Err(e) => return fail(&e, error_code(&e)),
    };
    let pid = child.id();
    let (total, over) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
    );
    let out = capture(
        child.stdout.take().expect("piped"),
        total.clone(),
        max,
        over.clone(),
    );
    let err = capture(
        child.stderr.take().expect("piped"),
        total,
        max,
        over.clone(),
    );
    let input = opt_str(&o, "input").map(unhex).unwrap_or_default();
    let mut stdin = child.stdin.take().expect("piped");
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let started = Instant::now();
    let (mut killed_at, mut timed_out) = (None, false);
    let status = loop {
        if let Ok(Some(st)) = child.try_wait() {
            let both =
                || out.finished.load(Ordering::SeqCst) && err.finished.load(Ordering::SeqCst);
            // A grandchild may hold the pipes open forever after we killed the child: stop waiting shortly.
            let abandon =
                killed_at.is_some_and(|t: Instant| t.elapsed() > Duration::from_millis(100));
            if both() || abandon {
                break st;
            }
        } else if killed_at.is_none() {
            if timeout.is_some_and(|l| started.elapsed() >= l) {
                timed_out = true;
            }
            if timed_out || over.load(Ordering::SeqCst) {
                unsafe { libc::kill(pid as libc::pid_t, sig) };
                killed_at = Some(Instant::now());
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let (code, signal) = status_parts(&status);
    let take = |c: &Capture| {
        hex(&std::mem::take(
            &mut *c.data.lock().unwrap_or_else(|p| p.into_inner()),
        ))
    };
    format!(
        "{{\"pid\":{pid},\"status\":{},\"signal\":{},\"stdout\":\"{}\",\"stderr\":\"{}\",\"timedOut\":{timed_out},\"maxBufferExceeded\":{}}}",
        num(code),
        num(signal),
        take(&out),
        take(&err),
        over.load(Ordering::SeqCst)
    )
}

impl Interp {
    pub(crate) fn install_child_process(&mut self) {
        let g = self.global.clone();
        let s = |a: &[Value], i: usize, it: &mut Interp| it.to_str(&arg(a, i)).to_string();
        let f = self.native(move |it, _, a| {
            // Flush parent output first so an inheriting child's lines land after it.
            it.flush_partial();
            Ok(Value::Number(f64::from(spawn(
                &s(a, 0, it),
                &s(a, 1, it),
                &s(a, 2, it),
            ))))
        });
        g.declare("__thaw_child_process_spawn", f, true);
        let f = self.native(move |it, _, a| {
            Ok(Value::str(&sync(&s(a, 0, it), &s(a, 1, it), &s(a, 2, it))))
        });
        g.declare("__thaw_child_process_sync", f, true);
        let f = self.native(|it, _, a| {
            let (h, sig) = (it.num(&arg(a, 0)) as u32, it.num(&arg(a, 1)) as i32);
            Ok(Value::Bool(kill(h, sig)))
        });
        g.declare("__thaw_child_process_kill", f, true);
        let f = self.native(|it, _, a| {
            let (h, r) = (it.num(&arg(a, 0)) as u32, arg(a, 1).truthy());
            TABLE.with(|t| t.borrow_mut().kids.get_mut(&h).map(|k| k.refd = r));
            Ok(Value::Undefined)
        });
        g.declare("__thaw_child_process_ref", f, true);
        let f = self.native(move |it, _, a| {
            let (h, data, end) = (it.num(&arg(a, 0)) as u32, s(a, 1, it), arg(a, 2).truthy());
            TABLE.with(|t| {
                if let Some(k) = t.borrow_mut().kids.get_mut(&h) {
                    if end {
                        k.stdin = None;
                    } else if let Some(tx) = &k.stdin {
                        let _ = tx.send(unhex(&data));
                    }
                }
            });
            Ok(Value::Bool(true))
        });
        g.declare("__thaw_child_process_stdin", f, true);
        let f = self.native(|_, _, _| Ok(Value::str(&poll())));
        g.declare("__thaw_child_process_poll", f, true);
    }
}
