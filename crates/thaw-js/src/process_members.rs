//! The value members of `process` that Node 22 has and thaw-js lacked: libc or procfs facts where Node
//! reads the host (`cpuUsage`, `resourceUsage`, `ppid`, `getgroups`, the `set*id` family, `availableMemory`),
//! and fixed shapes for the rest. `name`, `length` and the attribute flags come from `builtin_table.rs`
//! (the table rows of `process`), so this installer only has to create each member.

use crate::eventloop::node_error;
use crate::process_env::apply_dotenv;
use crate::{arg, define_accessor, put, put_with, Ctl, Interp, Kind, Obj, Value, R};
use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::io::Write;
use std::rc::Rc;

const JS_MEMBERS: &str = include_str!("web/process_members.js");

/// A `process.finalization` callback, run once at `event` ('exit' or 'beforeExit') as `callback(target, event)`.
pub(crate) struct Finalizer {
    target: Value,
    callback: Value,
    event: &'static str,
}

type MemberFn = fn(&mut Interp, &Value, &[Value]) -> R<Value>;

/// Function members: (name, length, native). The length is set at install; the table row fixes it again.
const FUNCTIONS: &[(&str, f64, MemberFn)] = &[
    ("abort", 0.0, abort),
    ("reallyExit", 0.0, really_exit),
    ("_kill", 0.0, kill_pid),
    ("_rawDebug", 0.0, raw_debug),
    ("_linkedBinding", 1.0, linked_binding),
    ("_getActiveHandles", 0.0, empty_array),
    ("_getActiveRequests", 0.0, empty_array),
    ("getActiveResourcesInfo", 0.0, empty_array),
    ("cpuUsage", 1.0, cpu_usage),
    ("threadCpuUsage", 1.0, thread_cpu_usage),
    ("resourceUsage", 0.0, resource_usage),
    ("availableMemory", 0.0, available_memory),
    ("constrainedMemory", 0.0, constrained_memory),
    ("getgroups", 0.0, getgroups),
    ("setuid", 1.0, setuid),
    ("seteuid", 1.0, seteuid),
    ("setgid", 1.0, setgid),
    ("setegid", 1.0, setegid),
    ("setgroups", 1.0, setgroups),
    ("initgroups", 2.0, initgroups),
    ("setSourceMapsEnabled", 1.0, set_source_maps_enabled),
    (
        "hasUncaughtExceptionCaptureCallback",
        0.0,
        has_capture_callback,
    ),
    (
        "setUncaughtExceptionCaptureCallback",
        1.0,
        set_capture_callback,
    ),
    ("loadEnvFile", 0.0, load_env_file),
    ("dlopen", 0.0, dlopen),
    ("execve", 1.0, execve),
    // internal members: shape only (user-approved), they do nothing
    ("_fatalException", 2.0, noop),
    ("_debugProcess", 0.0, noop),
    ("_debugEnd", 0.0, noop),
    ("_startProfilerIdleNotifier", 0.0, noop),
    ("_stopProfilerIdleNotifier", 0.0, noop),
    ("_tickCallback", 0.0, tick_callback),
];

/// Node's `process.resourceUsage()` keys, in Node's order.
const RESOURCE_KEYS: [&str; 16] = [
    "userCPUTime",
    "systemCPUTime",
    "maxRSS",
    "sharedMemorySize",
    "unsharedDataSize",
    "unsharedStackSize",
    "minorPageFault",
    "majorPageFault",
    "swappedOut",
    "fsRead",
    "fsWrite",
    "ipcSent",
    "ipcReceived",
    "signalsCount",
    "voluntaryContextSwitches",
    "involuntaryContextSwitches",
];

thread_local! {
    /// `process.sourceMapsEnabled`. Node's default is off; no source-map support exists here to switch on.
    static SOURCE_MAPS: Cell<bool> = const { Cell::new(false) };
}

impl Interp {
    /// Installs the table members on `p` (eventloop.rs `install_process`).
    pub(crate) fn install_process_members(&mut self, p: &Obj) {
        // Node's EventEmitter internal flag, an own plain symbol of `process` (`process[Symbol(kCapture)]` is false).
        // Installed before the table pass, so it precedes `Symbol.toStringTag` in `getOwnPropertySymbols`, as in Node.
        let capture = self
            .new_symbol(Some("kCapture".into()))
            .unwrap_or_else(|_| panic!("thaw-js kCapture symbol must be created"));
        if let Value::Object(s) = &capture {
            if let Kind::Symbol { id, .. } = &*s.kind.borrow() {
                put_with(
                    p,
                    &format!("@@sym:{id}"),
                    Value::Bool(false),
                    true,
                    true,
                    true,
                );
            }
        }
        for &(name, _, f) in FUNCTIONS {
            self.set_native(p, name, f);
        }
        put(
            p,
            "argv0",
            Value::str(&std::env::args().next().unwrap_or_default()),
        );
        put(p, "ppid", Value::Number(unsafe { libc::getppid() } as f64));
        put(p, "debugPort", Value::Number(9229.0));
        let features = self.plain();
        for (key, value) in [
            ("inspector", false),
            ("debug", false),
            ("uv", true),
            ("ipv6", true),
            ("tls_alpn", true),
            ("tls_sni", true),
            ("tls_ocsp", true),
            ("tls", true),
            ("openssl_is_boringssl", false),
            ("cached_builtins", true),
            ("require_module", true),
        ] {
            put(&features, key, Value::Bool(value));
        }
        put(&features, "typescript", Value::Bool(false));
        put(p, "features", Value::Object(features));
        let report = self.native(|it, _, _| Ok(Value::Object(report_object(it))));
        define_accessor(p, "report", Some(report), None, true);
        let source_maps = self.native(|_, _, _| Ok(Value::Bool(SOURCE_MAPS.with(Cell::get))));
        define_accessor(p, "sourceMapsEnabled", Some(source_maps), None, true);
        // ponytail: always empty; thaw-js does not record which builtins it has loaded
        let load_list = self.array(Vec::new());
        put(p, "moduleLoadList", load_list);
        // ponytail: `_preload_modules` is always empty (no preload list is read); its -ec flags come from the table
        let preload = self.array(Vec::new());
        put(p, "_preload_modules", preload);
        put(p, "domain", Value::Null);
        // the main-file loader sets it (web/main_file.js); -e and the plain realm have none, as in Node
        put(p, "mainModule", Value::Undefined);
        let exiting_get = self.native(|it, _, _| Ok(Value::Bool(it.rt.exiting)));
        let exiting_set = self.native(|it, _, a| {
            it.rt.exiting = arg(a, 0).truthy();
            Ok(Value::Undefined)
        });
        define_accessor(p, "_exiting", Some(exiting_get), Some(exiting_set), true);
        let fin = self.plain();
        for (name, length, f) in [
            (
                "register",
                2.0,
                self.native(|it, _, a| add_finalizer(it, a, "exit")),
            ),
            (
                "registerBeforeExit",
                2.0,
                self.native(|it, _, a| add_finalizer(it, a, "beforeExit")),
            ),
            ("unregister", 1.0, self.native(unregister_finalizer)),
        ] {
            if let Value::Object(o) = &f {
                crate::builtin_table::set_fn_head(o, name, length);
            }
            put(&fin, name, f);
        }
        // `process.finalization` is an accessor whose setter replaces the object, as in Node
        let fin_cell = Rc::new(RefCell::new(Value::Object(fin)));
        let set_cell = fin_cell.clone();
        let fin_get = self.native(move |_, _, _| Ok(fin_cell.borrow().clone()));
        let fin_set = self.native(move |_, _, a| {
            *set_cell.borrow_mut() = arg(a, 0);
            Ok(Value::Undefined)
        });
        define_accessor(p, "finalization", Some(fin_get), Some(fin_set), true);
    }

    /// Runs the JS members (web/process_members.js) in this realm; after `install_process_stdin`.
    pub(crate) fn install_process_members_js(&mut self) {
        self.run_builtin(JS_MEMBERS)
            .unwrap_or_else(|_| panic!("thaw-js process members must evaluate"));
    }
}

/// Runs the `process.finalization` callbacks for `event`, once each: the `exit` ones before the `exit` listeners.
pub(crate) fn run_finalizers(it: &mut Interp, event: &'static str) -> R<()> {
    if it.rt.finalizers.is_empty() {
        return Ok(());
    }
    let (due, keep): (Vec<Finalizer>, Vec<Finalizer>) = std::mem::take(&mut it.rt.finalizers)
        .into_iter()
        .partition(|f| f.event == event);
    it.rt.finalizers = keep;
    for f in due {
        it.call(
            &f.callback,
            &Value::Undefined,
            &[f.target, Value::str(event)],
        )?;
    }
    Ok(())
}

fn add_finalizer(it: &mut Interp, a: &[Value], event: &'static str) -> R<Value> {
    let target = arg(a, 0);
    if !matches!(target, Value::Object(_)) {
        return Err(invalid_arg(it, "obj", "object", &target));
    }
    let callback = arg(a, 1);
    if !callback.is_callable() {
        return Err(invalid_arg(it, "onFinalization", "function", &callback));
    }
    it.rt.finalizers.push(Finalizer {
        target,
        callback,
        event,
    });
    Ok(Value::Undefined)
}

fn unregister_finalizer(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let target = arg(a, 0);
    it.rt.finalizers.retain(|f| {
        !matches!((&f.target, &target), (Value::Object(x), Value::Object(y)) if Rc::ptr_eq(x, y))
    });
    Ok(Value::Undefined)
}

/// Node's `ERR_INVALID_ARG_TYPE` for argument `name` of `expected` type.
fn invalid_arg(it: &mut Interp, name: &str, expected: &str, v: &Value) -> Ctl {
    let received = match v {
        Value::Undefined => "undefined".to_string(),
        Value::Null => "null".to_string(),
        // ponytail: always "Object"; Node names the constructor
        Value::Object(_) => "an instance of Object".to_string(),
        Value::Str(_) => format!("type string ('{}')", it.to_str(v)),
        Value::Number(_) => format!("type number ({})", it.to_str(v)),
        Value::Bool(_) => format!("type boolean ({})", it.to_str(v)),
        _ => format!("type bigint ({})", it.to_str(v)),
    };
    let msg = format!("The \"{name}\" argument must be of type {expected}. Received {received}");
    node_error(it, "TypeError", "ERR_INVALID_ARG_TYPE", &msg)
}

fn has_capture_callback(it: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    Ok(Value::Bool(it.rt.capture_fn.is_some()))
}

/// `setUncaughtExceptionCaptureCallback(fn | null)`: while set, an uncaught exception goes to `fn`, not to
/// `uncaughtException` listeners (eventloop.rs `uncaught`).
fn set_capture_callback(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    match arg(a, 0) {
        Value::Null => it.rt.capture_fn = None,
        f if f.is_callable() => {
            if it.rt.capture_fn.is_some() {
                return Err(node_error(
                    it,
                    "Error",
                    "ERR_UNCAUGHT_EXCEPTION_CAPTURE_ALREADY_SET",
                    "`process.setupUncaughtExceptionCapture()` was called while a capture callback was already active",
                ));
            }
            it.rt.capture_fn = Some(f);
        }
        other => return Err(invalid_arg(it, "fn", "function or null", &other)),
    }
    Ok(Value::Undefined)
}

/// A fresh `process.report` object: Node's default settings, the two methods as functions.
fn report_object(it: &mut Interp) -> Obj {
    let o = it.plain();
    let write = it.native(|_, _, _| Ok(Value::str("")));
    put(&o, "writeReport", write);
    let get = it.native(|it, _, _| Ok(Value::Object(it.plain())));
    put(&o, "getReport", get);
    put(&o, "directory", Value::str(""));
    put(&o, "filename", Value::str(""));
    put(&o, "compact", Value::Bool(false));
    put(&o, "excludeNetwork", Value::Bool(false));
    put(&o, "signal", Value::str("SIGUSR2"));
    put(&o, "reportOnFatalError", Value::Bool(false));
    put(&o, "reportOnSignal", Value::Bool(false));
    put(&o, "reportOnUncaughtException", Value::Bool(false));
    put(&o, "excludeEnv", Value::Bool(false));
    o
}

fn abort(_: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    unsafe { libc::abort() }
}

fn really_exit(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let code = it.num(&arg(a, 0));
    std::process::exit(if code.is_finite() { code as i32 } else { 0 })
}

/// `process._kill(pid, sig)`: 0, or the negated errno (Node's internal contract).
fn kill_pid(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let pid = it.num(&arg(a, 0)) as libc::pid_t;
    let sig = it.num(&arg(a, 1)) as libc::c_int;
    let r = unsafe { libc::kill(pid, sig) };
    let code = if r == 0 {
        0
    } else {
        -std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(libc::EINVAL)
    };
    Ok(Value::Number(code as f64))
}

fn raw_debug(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let text: Vec<String> = a.iter().map(|v| it.to_str(v).to_string()).collect();
    eprintln!("{}", text.join(" "));
    Ok(Value::Undefined)
}

fn linked_binding(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let name = it.to_str(&arg(a, 0)).to_string();
    let msg = format!("No such binding was linked: {name}");
    Err(node_error(it, "Error", "ERR_INVALID_MODULE", &msg))
}

fn noop(_: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    Ok(Value::Undefined)
}

/// `process._tickCallback()`: runs the queued `nextTick` callbacks, then the promise jobs, as Node's runNextTicks.
fn tick_callback(it: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    it.run_jobs()?;
    Ok(Value::Undefined)
}

fn empty_array(it: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    Ok(it.array(Vec::new()))
}

/// `{user, system}` in microseconds of `who` (`RUSAGE_SELF` or `RUSAGE_THREAD`), minus `previous` when given.
fn cpu_usage_of(it: &mut Interp, who: libc::c_int, previous: &Value) -> R<Value> {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(who, &mut ru) };
    let micros = |t: libc::timeval| (t.tv_sec * 1_000_000 + t.tv_usec) as f64;
    let (mut user, mut system) = (micros(ru.ru_utime), micros(ru.ru_stime));
    if let Value::Object(_) = previous {
        let before_user = it.get(previous, "user")?;
        let before_system = it.get(previous, "system")?;
        user -= it.num(&before_user);
        system -= it.num(&before_system);
    }
    let o = it.plain();
    put(&o, "user", Value::Number(user.max(0.0)));
    put(&o, "system", Value::Number(system.max(0.0)));
    Ok(Value::Object(o))
}

fn cpu_usage(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    cpu_usage_of(it, libc::RUSAGE_SELF, &arg(a, 0))
}

fn thread_cpu_usage(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    cpu_usage_of(it, libc::RUSAGE_THREAD, &arg(a, 0))
}

fn resource_usage(it: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let micros = |t: libc::timeval| (t.tv_sec * 1_000_000 + t.tv_usec) as f64;
    let values = [
        micros(ru.ru_utime),
        micros(ru.ru_stime),
        ru.ru_maxrss as f64,
        ru.ru_ixrss as f64,
        ru.ru_idrss as f64,
        ru.ru_isrss as f64,
        ru.ru_minflt as f64,
        ru.ru_majflt as f64,
        ru.ru_nswap as f64,
        ru.ru_inblock as f64,
        ru.ru_oublock as f64,
        ru.ru_msgsnd as f64,
        ru.ru_msgrcv as f64,
        ru.ru_nsignals as f64,
        ru.ru_nvcsw as f64,
        ru.ru_nivcsw as f64,
    ];
    let o = it.plain();
    for (key, v) in RESOURCE_KEYS.iter().zip(values) {
        put(&o, key, Value::Number(v));
    }
    Ok(Value::Object(o))
}

/// `MemAvailable` from /proc/meminfo, in bytes (0 when unreadable).
fn available_memory(_: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    let kb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|l| l.strip_prefix("MemAvailable:"))
                .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
        })
        .unwrap_or(0);
    Ok(Value::Number((kb * 1024) as f64))
}

/// The cgroup v2 memory limit in bytes; u64::MAX (Node's "no limit") when there is none.
fn constrained_memory(_: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    Ok(Value::Number(limit as f64))
}

fn getgroups(it: &mut Interp, _: &Value, _: &[Value]) -> R<Value> {
    let n = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if n < 0 {
        return Ok(it.array(Vec::new()));
    }
    let mut groups = vec![0 as libc::gid_t; n as usize];
    let got = unsafe { libc::getgroups(n, groups.as_mut_ptr()) };
    groups.truncate(got.max(0) as usize);
    Ok(it.array(
        groups
            .into_iter()
            .map(|g| Value::Number(g as f64))
            .collect(),
    ))
}

/// A failed `set*id` / `setgroups` / `initgroups` throws Node's errno error (`EPERM: operation not permitted, setuid`).
fn errno_throw(it: &mut Interp, syscall: &str) -> R<Value> {
    let err = std::io::Error::last_os_error();
    let code = match err.raw_os_error() {
        Some(libc::EINVAL) => "EINVAL",
        Some(libc::ENOENT) => "ENOENT",
        Some(libc::EACCES) => "EACCES",
        _ => "EPERM",
    };
    let msg = format!("{code}: {}, {syscall}", err.to_string().to_lowercase());
    Err(node_error(it, "Error", code, &msg))
}

fn setuid(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let id = it.num(&arg(a, 0)) as libc::uid_t;
    if unsafe { libc::setuid(id) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "setuid")
    }
}

fn seteuid(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let id = it.num(&arg(a, 0)) as libc::uid_t;
    if unsafe { libc::seteuid(id) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "seteuid")
    }
}

fn setgid(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let id = it.num(&arg(a, 0)) as libc::gid_t;
    if unsafe { libc::setgid(id) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "setgid")
    }
}

fn setegid(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let id = it.num(&arg(a, 0)) as libc::gid_t;
    if unsafe { libc::setegid(id) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "setegid")
    }
}

fn setgroups(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    it.sparse_unsupported(&arg(a, 0))?;
    let groups: Vec<libc::gid_t> = it
        .array_items(&arg(a, 0))
        .iter()
        .map(|v| it.num(v) as libc::gid_t)
        .collect();
    if unsafe { libc::setgroups(groups.len() as _, groups.as_ptr()) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "setgroups")
    }
}

fn initgroups(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let user = std::ffi::CString::new(it.to_str(&arg(a, 0)).to_string()).unwrap_or_default();
    let gid = it.num(&arg(a, 1)) as libc::gid_t;
    if unsafe { libc::initgroups(user.as_ptr(), gid as _) } == 0 {
        Ok(Value::Undefined)
    } else {
        errno_throw(it, "initgroups")
    }
}

fn set_source_maps_enabled(_: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let on = arg(a, 0).truthy();
    SOURCE_MAPS.with(|c| c.set(on));
    Ok(Value::Undefined)
}

/// The `Received ...` tail of an ERR_INVALID_ARG_VALUE message: objects render as `{}`.
fn received_arg(it: &mut Interp, v: &Value) -> String {
    match v {
        Value::Object(_) => "{}".into(),
        _ => received(it, v),
    }
}

/// The `Received ...` tail of Node's ERR_INVALID_ARG_TYPE messages, for the values the members validate.
fn received(it: &mut Interp, v: &Value) -> String {
    match v {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Str(_) => format!("type string ('{}')", it.to_str(v)),
        Value::Number(_) => format!("type number ({})", it.to_str(v)),
        Value::Bool(_) => format!("type boolean ({})", it.to_str(v)),
        _ => "an instance of Object".into(),
    }
}

fn invalid_type(it: &mut Interp, name: &str, expected: &str, v: &Value) -> Ctl {
    let msg = format!(
        "The \"{name}\" argument {expected}. Received {}",
        received(it, v)
    );
    node_error(it, "TypeError", "ERR_INVALID_ARG_TYPE", &msg)
}

/// `process.loadEnvFile(path = '.env')`: dotenv lines into the environment, unset names only. A missing file throws
/// the fs error shape (`ENOENT: no such file or directory, open '<path>'`, errno, syscall, path).
fn load_env_file(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let v = arg(a, 0);
    let path = match v {
        Value::Undefined | Value::Null => ".env".to_string(),
        Value::Str(_) => it.to_str(&v).to_string(),
        _ => {
            return Err(invalid_type(
                it,
                "path",
                "must be of type string or an instance of Buffer or URL",
                &v,
            ))
        }
    };
    let text = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => {
            let code = match e.kind() {
                std::io::ErrorKind::NotFound => "ENOENT",
                std::io::ErrorKind::PermissionDenied => "EACCES",
                std::io::ErrorKind::IsADirectory => "EISDIR",
                _ => "EIO",
            };
            let errno = -e.raw_os_error().unwrap_or(libc::EIO);
            let reason = e
                .to_string()
                .split(" (os error")
                .next()
                .unwrap_or("")
                .to_lowercase();
            let err = it.make_error("Error", &format!("{code}: {reason}, open '{path}'"))?;
            if let Value::Object(o) = &err {
                put(o, "errno", Value::Number(errno as f64));
                put(o, "code", Value::str(code));
                put(o, "syscall", Value::str("open"));
                put(o, "path", Value::str(&path));
            }
            return Err(Ctl::Throw(err));
        }
    };
    apply_dotenv(it, &text);
    Ok(Value::Undefined)
}

/// Global object `package -> () => addon` that `host.rs` fills for every package loader while the napi bridge is
/// registered (`register_dlopen_addon`).
pub(crate) const DLOPEN_ADDONS: &str = "__thaw_dlopen_addons";

/// `process.dlopen(module, filename)`: argument validation as Node. A path that names the napi addon of a registered
/// package (`/thaw_modules/<pkg>/`, `node_modules/<pkg>/`, or any `*.node` when exactly one addon is registered, as for
/// Prisma's generated engine) sets `module.exports` to that package's addon, which thaw-napi hosts; any other path
/// throws ERR_DLOPEN_FAILED, as the runtime loads no other native library.
fn dlopen(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    if a.len() < 2 {
        return Err(node_error(
            it,
            "TypeError",
            "ERR_MISSING_ARGS",
            "process.dlopen needs at least 2 arguments",
        ));
    }
    if matches!(arg(a, 0), Value::Undefined | Value::Null) {
        let err = it.make_error("TypeError", "Cannot convert undefined or null to object")?;
        return Err(Ctl::Throw(err));
    }
    let target = arg(a, 0);
    let global = it.global.clone();
    if let (Some(addons), Value::Str(_)) = (it.lookup(&global, DLOPEN_ADDONS), arg(a, 1)) {
        let path = it.to_str(&arg(a, 1)).to_string();
        let module_file = match it.get(&target, "filename")? {
            Value::Str(s) => s.to_string(),
            _ => String::new(),
        };
        let names = it.own_keys(&addons);
        // the executable embeds every addon: its own path (`process.execPath`, `/proc/<pid>/exe`; Prisma's
        // `PRISMA_QUERY_ENGINE_LIBRARY=$self`) names all of them at once, earlier-registered winning a name clash
        let exec_path = match it.lookup(&global, "process") {
            Some(process) => match it.get(&process, "execPath")? {
                Value::Str(p) => p.to_string(),
                _ => String::new(),
            },
            None => String::new(),
        };
        let self_path = path == "/proc/self/exe"
            || path == format!("/proc/{}/exe", std::process::id())
            || (!exec_path.is_empty() && path == exec_path);
        if self_path {
            let merged = Value::Object(it.plain());
            for name in &names {
                let thunk = it.get(&addons, name)?;
                let exports = it.call(&thunk, &Value::Undefined, &[])?;
                for key in it.own_keys(&exports) {
                    if !it.own_keys(&merged).contains(&key) {
                        let v = it.get(&exports, &key)?;
                        it.set(&merged, &key, v)?;
                    }
                }
            }
            it.set(&target, "exports", merged)?;
            return Ok(Value::Undefined);
        }
        let owns = |name: &str| {
            [
                format!("/thaw_modules/{name}/"),
                format!("/node_modules/{name}/"),
            ]
            .iter()
            .any(|marker| path.contains(marker) || module_file.contains(marker))
        };
        let chosen = names
            .iter()
            .filter(|n| owns(n))
            .max_by_key(|n| n.len())
            .cloned()
            .or_else(|| (names.len() == 1 && path.ends_with(".node")).then(|| names[0].clone()));
        if let Some(name) = chosen {
            let thunk = it.get(&addons, &name)?;
            let exports = it.call(&thunk, &Value::Undefined, &[])?;
            it.set(&target, "exports", exports)?;
            return Ok(Value::Undefined);
        }
    }
    let file = it.to_str(&arg(a, 1)).to_string();
    let reason = match std::fs::metadata(&file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "No such file or directory".to_string()
        }
        _ => "native addons cannot be loaded at runtime on thaw-js".to_string(),
    };
    let msg = format!("{file}: cannot open shared object file: {reason}");
    Err(node_error(it, "Error", "ERR_DLOPEN_FAILED", &msg))
}

/// `process.execve(execPath, args = [execPath], env = process.env)`: validates like Node, flushes stdio, then replaces
/// the process. Returns only on failure, which throws the errno error.
fn execve(it: &mut Interp, _: &Value, a: &[Value]) -> R<Value> {
    let path_v = arg(a, 0);
    if !matches!(path_v, Value::Str(_)) {
        return Err(invalid_type(
            it,
            "execPath",
            "must be of type string",
            &path_v,
        ));
    }
    let path = it.to_str(&path_v).to_string();
    let args_v = arg(a, 1);
    let items: Vec<Value> = match &args_v {
        Value::Undefined => vec![path_v.clone()],
        Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
            it.sparse_unsupported(&args_v)?;
            it.array_items(&args_v)
        }
        _ => {
            return Err(invalid_type(
                it,
                "args",
                "must be an instance of Array",
                &args_v,
            ))
        }
    };
    let mut argv = Vec::with_capacity(items.len());
    for (i, v) in items.iter().enumerate() {
        // ponytail: non-string elements are shown as `{}` in the message, not inspected
        let c = match v {
            Value::Str(_) => CString::new(it.to_str(v).to_string()).ok(),
            _ => None,
        };
        match c {
            Some(c) => argv.push(c),
            None => {
                let msg = format!(
                    "The argument 'args[{i}]' must be a string without null bytes. Received {}",
                    received_arg(it, v)
                );
                return Err(node_error(it, "TypeError", "ERR_INVALID_ARG_VALUE", &msg));
            }
        }
    }
    let env_pairs: Vec<String> = match arg(a, 2) {
        Value::Undefined => std::env::vars_os()
            .map(|(k, v)| format!("{}={}", k.to_string_lossy(), v.to_string_lossy()))
            .collect(),
        Value::Object(_) => {
            let env = arg(a, 2);
            it.own_keys(&env)
                .into_iter()
                .map(|k| {
                    let v = it
                        .get(&env, &k)
                        .map(|v| it.to_str(&v).to_string())
                        .unwrap_or_default();
                    format!("{k}={v}")
                })
                .collect()
        }
        other => return Err(invalid_type(it, "env", "must be of type object", &other)),
    };
    let envp = env_pairs
        .into_iter()
        .map(CString::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            node_error(
                it,
                "TypeError",
                "ERR_INVALID_ARG_VALUE",
                "The environment must not contain null bytes",
            )
        })?;
    let cpath = CString::new(path).map_err(|_| {
        node_error(
            it,
            "TypeError",
            "ERR_INVALID_ARG_VALUE",
            "The execPath must not contain null bytes",
        )
    })?;
    let mut argv_p: Vec<*const libc::c_char> = argv.iter().map(|c| c.as_ptr()).collect();
    argv_p.push(std::ptr::null());
    let mut envp_p: Vec<*const libc::c_char> = envp.iter().map(|c| c.as_ptr()).collect();
    envp_p.push(std::ptr::null());
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe { libc::execve(cpath.as_ptr(), argv_p.as_ptr(), envp_p.as_ptr()) };
    errno_throw(it, "execve")
}

#[cfg(test)]
mod tests {
    use crate::Interp;

    /// Like `output`, but also runs the event loop (for `exit`/`beforeExit` listeners).
    fn run_loop_plain(src: &str) -> String {
        let mut it = Interp::new();
        it.run(src)
            .unwrap_or_else(|_| panic!("script must run: {src}"));
        it.run_event_loop().expect("loop runs");
        it.out.join("|")
    }

    /// Runs `src` in a fresh realm and returns what it logged, joined by `|`.
    fn output(src: &str) -> String {
        let mut it = Interp::new();
        it.run(src)
            .unwrap_or_else(|_| panic!("script must run: {src}"));
        it.out.join("|")
    }

    #[test]
    fn members_have_node_typeof() {
        assert_eq!(
            output(
                "const want = {argv0:'string', ppid:'number', debugPort:'number', features:'object', report:'object', \
                 sourceMapsEnabled:'boolean', abort:'function', reallyExit:'function', _kill:'function', \
                 _rawDebug:'function', _linkedBinding:'function', _getActiveHandles:'function', \
                 _getActiveRequests:'function', getActiveResourcesInfo:'function', cpuUsage:'function', \
                 threadCpuUsage:'function', resourceUsage:'function', availableMemory:'function', \
                 constrainedMemory:'function', getgroups:'function', setuid:'function', seteuid:'function', \
                 setgid:'function', setegid:'function', setgroups:'function', initgroups:'function', \
                 setSourceMapsEnabled:'function'}; \
                 const bad = Object.keys(want).filter(k => typeof process[k] !== want[k]); console.log(bad.join(','));"
            ),
            ""
        );
    }

    #[test]
    fn function_members_have_node_length_and_name() {
        assert_eq!(
            output(
                "const want = {cpuUsage:1, threadCpuUsage:1, resourceUsage:0, availableMemory:0, constrainedMemory:0, \
                 getgroups:0, setuid:1, seteuid:1, setgid:1, setegid:1, setgroups:1, initgroups:2, abort:0, \
                 reallyExit:0, _kill:0, _rawDebug:0, _linkedBinding:1, getActiveResourcesInfo:0, \
                 setSourceMapsEnabled:1}; \
                 const bad = Object.keys(want).filter(k => process[k].length !== want[k]); console.log(bad.join(','));"
            ),
            ""
        );
        // Node names the set*id family '' (the table row carries that)
        assert_eq!(
            output("console.log(JSON.stringify(process.setuid.name), process.cpuUsage.name)"),
            "\"\" cpuUsage"
        );
    }

    #[test]
    fn data_members_have_node_attribute_flags() {
        assert_eq!(
            output(
                "const d = (k) => { const x = Object.getOwnPropertyDescriptor(process, k); return [x.writable, x.enumerable, x.configurable].join(); }; \
                 console.log(d('ppid'), d('debugPort'), d('argv0'), d('features'));"
            ),
            "true,true,true true,true,true false,true,false false,true,false"
        );
        assert_eq!(
            output("const x = Object.getOwnPropertyDescriptor(process, 'report'); console.log(typeof x.get, x.set, x.enumerable, x.configurable);"),
            "function undefined true true"
        );
    }

    #[test]
    fn cpu_and_resource_usage_have_node_shapes() {
        assert_eq!(
            output("console.log(Object.keys(process.cpuUsage()).join(), Object.keys(process.threadCpuUsage()).join(), Object.keys(process.resourceUsage()).join());"),
            "user,system user,system userCPUTime,systemCPUTime,maxRSS,sharedMemorySize,unsharedDataSize,unsharedStackSize,minorPageFault,majorPageFault,swappedOut,fsRead,fsWrite,ipcSent,ipcReceived,signalsCount,voluntaryContextSwitches,involuntaryContextSwitches"
        );
        assert_eq!(
            output("const a = process.cpuUsage(); const b = process.cpuUsage(a); console.log(b.user >= 0, b.system >= 0, typeof process.resourceUsage().maxRSS);"),
            "true true number"
        );
    }

    #[test]
    fn report_and_features_have_node_keys() {
        assert_eq!(
            output("console.log(Object.keys(process.report).join());"),
            "writeReport,getReport,directory,filename,compact,excludeNetwork,signal,reportOnFatalError,reportOnSignal,reportOnUncaughtException,excludeEnv"
        );
        assert_eq!(
            output("console.log(process.report.compact, process.report.signal, Object.keys(process.features).join());"),
            "false SIGUSR2 inspector,debug,uv,ipv6,tls_alpn,tls_sni,tls_ocsp,tls,openssl_is_boringssl,cached_builtins,require_module,typescript"
        );
    }

    #[test]
    fn host_facts_are_real_numbers() {
        assert_eq!(
            output("console.log(process.ppid > 0, process.debugPort, typeof process.argv0, process.availableMemory() > 0, typeof process.constrainedMemory(), Array.isArray(process.getgroups()), process.getgroups().every(g => typeof g === 'number'));"),
            "true 9229 string true number true true"
        );
        assert_eq!(
            output("console.log(process._kill(process.pid, 0), process._kill(2147483646, 0) < 0);"),
            "0 true"
        );
        assert_eq!(output("console.log(Array.isArray(process.getActiveResourcesInfo()), Array.isArray(process._getActiveHandles()));"), "true true");
    }

    #[test]
    fn source_maps_flag_round_trips() {
        assert_eq!(
            output("const a = process.sourceMapsEnabled; process.setSourceMapsEnabled(true); const b = process.sourceMapsEnabled; process.setSourceMapsEnabled(false); console.log(a, b, process.sourceMapsEnabled);"),
            "false true false"
        );
    }

    /// Writes `content` to a fresh file under the temp dir (TMPDIR-aware) and returns its path.
    fn temp_env_file(tag: &str, content: &str) -> String {
        let path = std::env::temp_dir().join(format!("thaw-lef-{}-{tag}.env", std::process::id()));
        std::fs::write(&path, content).expect("write temp env file");
        path.display().to_string()
    }

    #[test]
    fn load_env_file_parses_dotenv_and_keeps_preset_values() {
        let path = temp_env_file(
            "parse",
            "TJS_LEF_A=1\nexport TJS_LEF_B=two\nTJS_LEF_C = spaced  \nTJS_LEF_D=\"dq\\nx\"\nTJS_LEF_E='sq\\nraw'\n\
             TJS_LEF_F=val # comment\n# full comment\nTJS_LEF_G=\nTJS_LEF_H=\"multi\nline\"\nTJS_LEF_PRE=from-file\nno-equals-line\n",
        );
        std::env::set_var("TJS_LEF_PRE", "preset");
        let got = output(&format!(
            "const r = process.loadEnvFile('{path}'); \
             console.log(r, ['A','B','C','D','E','F','G','H','PRE'].map(k => JSON.stringify(process.env['TJS_LEF_' + k])).join(' '));"
        ));
        assert_eq!(got, "undefined \"1\" \"two\" \"spaced\" \"dq\\nx\" \"sq\\\\nraw\" \"val\" \"\" \"multi\\nline\" \"preset\"");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_env_file_errors_have_node_shape() {
        assert_eq!(
            output("try { process.loadEnvFile('/nonexistent-tjs/.env'); } catch (e) { console.log(e.message, e.code, e.errno, e.syscall, e.path); }"),
            "ENOENT: no such file or directory, open '/nonexistent-tjs/.env' ENOENT -2 open /nonexistent-tjs/.env"
        );
        assert_eq!(
            output("try { process.loadEnvFile(5); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"path\" argument must be of type string or an instance of Buffer or URL. Received type number (5)"
        );
    }

    #[test]
    fn dlopen_validates_and_fails_with_node_shapes() {
        assert_eq!(
            output(
                "try { process.dlopen(); } catch (e) { console.log(e.name, e.code, e.message); }"
            ),
            "TypeError ERR_MISSING_ARGS process.dlopen needs at least 2 arguments"
        );
        assert_eq!(
            output("try { process.dlopen(undefined, 'x'); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError undefined Cannot convert undefined or null to object"
        );
        assert_eq!(
            output("try { process.dlopen({exports: {}}, '/nonexistent-tjs/x.node'); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "Error ERR_DLOPEN_FAILED /nonexistent-tjs/x.node: cannot open shared object file: No such file or directory"
        );
        assert_eq!(
            output("try { process.dlopen({exports: {}}, 5); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "Error ERR_DLOPEN_FAILED 5: cannot open shared object file: No such file or directory"
        );
    }

    #[test]
    fn execve_validates_arguments_before_exec() {
        assert_eq!(
            output("try { process.execve(); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"execPath\" argument must be of type string. Received undefined"
        );
        assert_eq!(
            output("try { process.execve(5, []); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"execPath\" argument must be of type string. Received type number (5)"
        );
        assert_eq!(
            output("try { process.execve('/bin/true', 'x'); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"args\" argument must be an instance of Array. Received type string ('x')"
        );
        assert_eq!(
            output("try { process.execve('/bin/true', ['a'], 'x'); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"env\" argument must be of type object. Received type string ('x')"
        );
        assert_eq!(
            output("try { process.execve('/bin/true', ['a'], null); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_TYPE The \"env\" argument must be of type object. Received null"
        );
        assert_eq!(
            output("try { process.execve('/bin/true', [{}], {}); } catch (e) { console.log(e.name, e.code, e.message); }"),
            "TypeError ERR_INVALID_ARG_VALUE The argument 'args[0]' must be a string without null bytes. Received {}"
        );
    }

    #[test]
    fn env_and_addon_members_have_node_lengths() {
        assert_eq!(
            output("console.log(process.loadEnvFile.length, process.dlopen.length, process.execve.length, typeof process.loadEnvFile, typeof process.dlopen, typeof process.execve);"),
            "0 0 1 function function function"
        );
    }

    #[test]
    fn linked_binding_throws_node_error() {
        assert_eq!(
            output("try { process._linkedBinding('nope'); } catch (e) { console.log(e.message, e.code); }"),
            "No such binding was linked: nope ERR_INVALID_MODULE"
        );
    }

    #[test]
    fn c1_members_have_node_shape() {
        assert_eq!(
            output(
                "console.log([Array.isArray(process.moduleLoadList), process.domain === null, process._exiting, \
                 typeof process.ref, process.ref.length, process.unref.length, typeof process.ref.prototype, \
                 typeof process.finalization.register, process.finalization.register.length, \
                 process.finalization.unregister.length, process.finalization.registerBeforeExit.length, \
                 process.allowedNodeEnvironmentFlags.size > 0, process.allowedNodeEnvironmentFlags.has('--max-old-space-size'), \
                 process.allowedNodeEnvironmentFlags.constructor.name, process.hasUncaughtExceptionCaptureCallback(), \
                 process.setUncaughtExceptionCaptureCallback.length, process.getBuiltinModule('nope'), \
                 process.mainModule === undefined, typeof process.assert, process.assert.length, process.assert.name, \
                 process.openStdin.length, typeof process.openStdin.prototype, typeof process.getBuiltinModule.prototype, \
                 process.getBuiltinModule.length].join(';'));"
            ),
            "true;true;false;function;1;1;object;function;2;1;2;true;true;NodeEnvironmentFlagsSet;false;1;;true;function;2;deprecated;0;object;object;1"
        );
    }

    #[test]
    fn ref_and_unref_call_the_resource_method_like_node() {
        assert_eq!(
            output(
                "const o = { n: 0, ref() { this.n += 1; }, unref() { this.n += 10; } }; \
                 process.ref(o); process.unref(o); process.ref(null); process.unref(undefined); process.ref({}); \
                 console.log(o.n, process.ref.name, process.unref.name);"
            ),
            "11 ref unref"
        );
    }

    // ponytail: Node's set reports size 271 with 3 duplicate names; a real Set holds the 268 distinct ones.
    #[test]
    fn allowed_node_environment_flags_is_a_read_only_set_like_node() {
        assert_eq!(
            output(
                "const s = process.allowedNodeEnvironmentFlags; \
                 console.log(s.constructor.name, s.size > 0, s.add('--zzz') === s, s.has('--zzz'), s.delete('--max-old-space-size'), \
                 s.has('--max-old-space-size'), s.clear(), s.size > 0, s instanceof Set, process.allowedNodeEnvironmentFlags === s);"
            ),
            "NodeEnvironmentFlagsSet true true false false true undefined true true true"
        );
    }

    #[test]
    fn finalization_callbacks_run_at_before_exit_and_exit_like_node() {
        assert_eq!(
            run_loop_plain(
                "const o = {}; process.finalization.register(o, (ref, ev) => console.log('fin', ev, ref === o));\n\
                 const u = {}; process.finalization.register(u, () => console.log('never'));\n\
                 console.log('unreg', process.finalization.unregister(u));\n\
                 const b = {}; process.finalization.registerBeforeExit(b, (ref, ev) => console.log('bx', ev, ref === b));\n\
                 process.on('beforeExit', () => console.log('beforeExit'));\n\
                 process.on('exit', (c) => console.log('exit', c));"
            ),
            "unreg undefined|bx beforeExit true|beforeExit|fin exit true|exit 0"
        );
    }

    #[test]
    // ponytail: builtin identity (`getBuiltinModule('fs') === require('fs')`) needs the bundle's builtin loader,
    // which the plain realm lacks; the binary path covers it.
    fn main_module_and_get_builtin_module_follow_the_main_file() {
        let dir = std::env::temp_dir().join(format!("thaw-c1-main-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.js"),
            "console.log(process.mainModule === module, process.getBuiltinModule('lodash'));\n",
        )
        .unwrap();
        let mut it = Interp::new();
        let main = dir.join("main.js").display().to_string();
        assert_eq!(it.run_main_file(&main).ok(), Some(0));
        assert_eq!(it.out.join("|"), "true undefined");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn internal_members_have_node_shape() {
        assert_eq!(
            output(
                "const want = {_fatalException:'function', _tickCallback:'function', _debugProcess:'function', \
                 _debugEnd:'function', _startProfilerIdleNotifier:'function', _stopProfilerIdleNotifier:'function'}; \
                 const bad = Object.keys(want).filter(k => typeof process[k] !== want[k]); \
                 const d = (k) => { const x = Object.getOwnPropertyDescriptor(process, k); return [x.writable, x.enumerable, x.configurable].join(); }; \
                 console.log(bad.join(','), process._fatalException.length, process._tickCallback.name, process._tickCallback.length, \
                 process._debugProcess.name, process._fatalException.name === '', Array.isArray(process._preload_modules), \
                 process._preload_modules.length, d('_preload_modules'));"
            ),
            " 2 runNextTicks 0 _debugProcess true true 0 false,true,true"
        );
    }

    #[test]
    fn stdio_fds_and_accessor_descriptors_have_node_shape() {
        assert_eq!(
            output(
                "const acc = (k) => { const x = Object.getOwnPropertyDescriptor(process, k); return [typeof x.get, x.set, x.enumerable, x.configurable].join(); }; \
                 const fd = (s) => { const x = Object.getOwnPropertyDescriptor(s, 'fd'); return [x.value, x.writable, x.enumerable, x.configurable].join(); }; \
                 console.log(process.stdout.fd, process.stderr.fd, fd(process.stdout), fd(process.stderr), \
                 acc('stdout'), acc('stderr'), acc('stdin'), process.stdout === process.stdout);"
            ),
            "1 2 1,true,true,true 2,true,true,true function,,true,true function,,true,true function,,true,true true"
        );
    }

    /// Node 22 (`node -e "for (const s of Object.getOwnPropertySymbols(process)) console.log(String(s), Object.getOwnPropertyDescriptor(process, s))"`):
    /// `Symbol(shapeMode)` (waived W1), then `Symbol(kCapture) { value: false, writable: true, enumerable: true,
    /// configurable: true }`, then `Symbol(Symbol.toStringTag)`. The kCapture symbol is a plain non-registry symbol.
    #[test]
    fn kcapture_own_symbol_matches_node_22() {
        assert_eq!(
            output(
                "const syms = Object.getOwnPropertySymbols(process); \
                 const s = syms.find((x) => String(x) === 'Symbol(kCapture)'); \
                 const d = Object.getOwnPropertyDescriptor(process, s); \
                 console.log(s.description, Symbol.keyFor(s), d.value, d.writable, d.enumerable, d.configurable, \
                 syms.indexOf(s) < syms.indexOf(Symbol.toStringTag));"
            ),
            "kCapture undefined false true true true true"
        );
    }

    #[test]
    fn tick_callback_drains_ticks_then_microtasks_like_node() {
        assert_eq!(
            output(
                "const log = []; process.nextTick(() => log.push('t')); Promise.resolve().then(() => log.push('p')); \
                 process._tickCallback(); console.log(log.join(','));"
            ),
            "t,p"
        );
    }
}
