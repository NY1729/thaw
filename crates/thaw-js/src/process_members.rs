//! The value members of `process` that Node 22 has and thaw-js lacked: libc or procfs facts where Node
//! reads the host (`cpuUsage`, `resourceUsage`, `ppid`, `getgroups`, the `set*id` family, `availableMemory`),
//! and fixed shapes for the rest. `name`, `length` and the attribute flags come from `builtin_table.rs`
//! (the table rows of `process`), so this installer only has to create each member.

use crate::eventloop::node_error;
use crate::{arg, define_accessor, put, Interp, Obj, Value, R};
use std::cell::Cell;

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
    }
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
        Some(libc::EPERM) => "EPERM",
        Some(libc::EINVAL) => "EINVAL",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interp;

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

    #[test]
    fn linked_binding_throws_node_error() {
        assert_eq!(
            output("try { process._linkedBinding('nope'); } catch (e) { console.log(e.message, e.code); }"),
            "No such binding was linked: nope ERR_INVALID_MODULE"
        );
    }
}
