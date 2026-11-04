//! Host functions behind the registry's `node:os`, `node:tty` and raw `process` stream shims:
//! `__thaw_os_*`, `__thaw_tty_write`, `__thaw_process_write_bytes`, `__thaw_process_is_tty`.
//! Plain std/libc (no dependency on the Thaw runtime crates); the JSON shapes match the QuickJS host.

use std::io::Write;

use crate::builtins::json_quote;
use crate::*;

fn q(s: &str) -> String {
    let mut out = String::new();
    json_quote(s, &mut out);
    out
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(unix)]
fn user_info(home: &str) -> String {
    use std::ffi::CStr;
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut found) };
    let text = |p: *const libc::c_char| {
        if p.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    let (name, dir, shell) = if rc == 0 && !found.is_null() {
        (text(pw.pw_name), text(pw.pw_dir), text(pw.pw_shell))
    } else {
        (
            std::env::var("USER").unwrap_or_default(),
            home.to_string(),
            std::env::var("SHELL").unwrap_or_default(),
        )
    };
    format!(
        "{{\"username\":{},\"homedir\":{},\"shell\":{},\"uid\":{uid},\"gid\":{gid}}}",
        q(&name),
        q(&dir),
        q(&shell)
    )
}

#[cfg(not(unix))]
fn user_info(home: &str) -> String {
    format!(
        "{{\"username\":{},\"homedir\":{},\"shell\":{},\"uid\":0,\"gid\":0}}",
        q(&std::env::var("USER").unwrap_or_default()),
        q(home),
        q(&std::env::var("SHELL").unwrap_or_default())
    )
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        v => v,
    }
}

fn identity() -> String {
    let platform = platform();
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        v => v,
    };
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let mut tmp = ["TMPDIR", "TMP", "TEMP"]
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "/tmp".into());
    if tmp.len() > 1 && tmp.ends_with('/') {
        tmp.pop();
    }
    let mut host = read("/etc/hostname");
    if host.is_empty() {
        host = "localhost".into();
    }
    format!(
        "{{\"platform\":{},\"arch\":{},\"type\":{},\"hostname\":{},\"homedir\":{},\"tmpdir\":{},\
         \"release\":{},\"version\":{},\"machine\":{},\"endianness\":{},\"userInfo\":{}}}",
        q(platform),
        q(arch),
        q(if platform == "linux" {
            "Linux"
        } else {
            platform
        }),
        q(&host),
        q(&home),
        q(&tmp),
        q(&read("/proc/sys/kernel/osrelease")),
        q(&read("/proc/sys/kernel/version")),
        q(std::env::consts::ARCH),
        q(if cfg!(target_endian = "little") {
            "LE"
        } else {
            "BE"
        }),
        user_info(&home)
    )
}

/// `/proc`-backed statistics (zeros where the host has none).
fn info() -> String {
    let cpuinfo = read("/proc/cpuinfo");
    let field = |key: &str| {
        cpuinfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    };
    let model = field("model name").unwrap_or_default();
    let speed = field("cpu MHz")
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
        .round();
    #[cfg(unix)]
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    #[cfg(not(unix))]
    let hz = 100.0;
    let cpus: Vec<String> = read("/proc/stat")
        .lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .map(|l| {
            let t: Vec<f64> = l
                .split_whitespace()
                .skip(1)
                .map(|v| v.parse::<f64>().unwrap_or(0.0) * 1000.0 / hz)
                .collect();
            let g = |i: usize| t.get(i).copied().unwrap_or(0.0).round();
            format!(
                "{{\"model\":{},\"speed\":{speed},\"times\":{{\"user\":{},\"nice\":{},\"sys\":{},\"idle\":{},\"irq\":{}}}}}",
                q(&model),
                g(0),
                g(1),
                g(2),
                g(3),
                g(5)
            )
        })
        .collect();
    let meminfo = read("/proc/meminfo");
    let kb = |key: &str| {
        meminfo
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0)
            * 1024.0
    };
    let uptime = read("/proc/uptime")
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);
    let load: Vec<f64> = read("/proc/loadavg")
        .split_whitespace()
        .take(3)
        .map(|v| v.parse().unwrap_or(0.0))
        .collect();
    let load = [0, 1, 2].map(|i| load.get(i).copied().unwrap_or(0.0).to_string());
    let par = std::thread::available_parallelism().map_or(1, |n| n.get());
    format!(
        "{{\"cpus\":[{}],\"totalmem\":{},\"freemem\":{},\"uptime\":{uptime},\"loadavg\":[{}],\"availableParallelism\":{par}}}",
        cpus.join(","),
        kb("MemTotal"),
        kb("MemAvailable"),
        load.join(",")
    )
}

fn priority_result(r: std::io::Result<i32>) -> String {
    match r {
        Ok(v) => format!("{{\"ok\":true,\"value\":{v}}}"),
        Err(e) => {
            let errno = e.raw_os_error();
            let code = match errno {
                Some(3) => "ESRCH",
                Some(13) => "EACCES",
                Some(1) => "EPERM",
                _ if e.kind() == std::io::ErrorKind::InvalidInput => "EINVAL",
                _ => "UNKNOWN",
            };
            format!(
                "{{\"ok\":false,\"code\":\"{code}\",\"errno\":{},\"message\":{}}}",
                errno.map_or(-4094, |n| -n),
                q(&e.to_string())
            )
        }
    }
}

#[cfg(unix)]
fn get_priority(pid: i32) -> std::io::Result<i32> {
    // getpriority can legitimately return -1: errno tells the failure apart
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = 0
    };
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = 0
    };
    let v = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
    match std::io::Error::last_os_error() {
        e if v == -1 && e.raw_os_error() != Some(0) => Err(e),
        _ => Ok(v),
    }
}

#[cfg(unix)]
fn set_priority(pid: i32, prio: i32) -> std::io::Result<i32> {
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, prio) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(0)
}

#[cfg(not(unix))]
fn get_priority(_: i32) -> std::io::Result<i32> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(not(unix))]
fn set_priority(_: i32, _: i32) -> std::io::Result<i32> {
    Err(std::io::ErrorKind::Unsupported.into())
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}

/// The compiler's dynamic-value helpers (`getDynamicValue("__thaw_...")`) the QuickJS realm defines in its
/// runtime prelude; only the ones that are plain JavaScript.
// ponytail: graph-origin identity (`__thaw_strict_equal_dynamic` on shadow containers) and native-callback
// comparison are QuickJS-only; add when a thaw-js program compares such values.
const DYNAMIC_HELPERS: &str = r#"
(function () {
  function put(name, fn) { Object.defineProperty(globalThis, name, { value: fn, writable: false, configurable: false, enumerable: false }); }
  put('__thaw_unary_plus', function (v) { return +v; });
  put('__thaw_defer_thenable', function (value) {
    if (value === null || (typeof value !== 'object' && typeof value !== 'function')) return value;
    var then;
    try { then = value.then; } catch (error) { return Promise.reject(error); }
    if (typeof then !== 'function') return value;
    return new Promise(function (resolve, reject) {
      Promise.resolve().then(function () {
        try { then.call(value, resolve, reject); } catch (error) { reject(error); }
      });
    });
  });
  put('__thaw_error_properties_json', function (error) {
    if (!error || (typeof error !== 'object' && typeof error !== 'function')) return '';
    var out = {};
    for (var key in error) {
      if (key === 'message' || key === 'name' || key === 'stack') continue;
      var value = error[key], type = typeof value;
      if (type === 'string' || type === 'number' || type === 'boolean' || value === null || type === 'object') out[key] = value;
    }
    // cycle- and depth-safe: a nested response/request graph (axios) is copied as far as it is a tree
    var stack = [];
    return JSON.stringify(out, function (k, v) {
      if (typeof v !== 'object' || v === null) return v;
      while (stack.length && stack[stack.length - 1] !== this) stack.pop();
      if (stack.length > 5 || stack.indexOf(v) >= 0) return undefined;
      stack.push(v);
      return v;
    });
  });
  put('__thaw_json_stringify_direct', function (v) { return JSON.stringify(v); });
  put('__thaw_update_dynamic_value', function (value, increment) { var updated = value; var old = increment ? updated++ : updated--; return [old, updated]; });
  put('__thaw_dynamic_add', function (l, r) { return l + r; });
  put('__thaw_dynamic_binary', function (op, l, r) { switch (op) { case '-': return l - r; case '*': return l * r; case '/': return l / r; case '%': return l % r; case '&': return l & r; case '|': return l | r; case '^': return l ^ r; case '<<': return l << r; default: return l >> r; } });
  put('__thaw_typeof_dynamic_value', function (v) { return typeof v; });
  put('__thaw_is_truthy_dynamic_value', function (v) { return !!v; });
  put('__thaw_is_undefined_dynamic_value', function (v) { return v === undefined; });
  put('__thaw_is_null_dynamic_value', function (v) { return v === null; });
  put('__thaw_is_nullish_dynamic_value', function (v) { return v == null; });
  put('__thaw_is_array_dynamic_value', function (v) { return Array.isArray(v); });
  put('__thaw_is_buffer_dynamic_value', function (v) { return typeof Buffer === 'function' && Buffer.isBuffer(v); });
  put('__thaw_strict_equal_dynamic', function (l, r) { return l === r; });
  put('__thaw_strict_equal_null_dynamic', function (v) { return v === null; });
  put('__thaw_strict_equal_undefined_dynamic', function (v) { return v === undefined; });
  put('__thaw_strict_equal_number_dynamic', function (text, v) { return typeof v === 'number' && v === Number(text); });
  put('__thaw_strict_equal_bigint_dynamic', function (digits, v) { return typeof v === 'bigint' && v === BigInt(digits); });
  put('__thaw_native_loose_equal', function (l, r, lk, rk) { return (lk === 1 ? BigInt(l) : l) == (rk === 1 ? BigInt(r) : r); });
  put('__thaw_instanceof_dynamic_value', function (v, target) { return typeof target === 'function' ? v instanceof target : false; });
  put('__thaw_instanceof_dynamic_value_by_name', function (name, v) { var c = globalThis[name]; return typeof c === 'function' && v instanceof c; });
})();
"#;

/// `os.networkInterfaces()` as JSON: IPv4/IPv6 entries grouped by interface name (POSIX `getifaddrs`; the
/// MAC comes from `/sys/class/net/<name>/address`, all zero when absent), the shape thaw-quickjs returns.
#[cfg(unix)]
fn network_interfaces_json() -> Result<String, String> {
    use std::ffi::CStr;
    use std::net::{Ipv4Addr, Ipv6Addr};

    let mut groups = std::collections::BTreeMap::<String, Vec<serde_json::Value>>::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid out-pointer; the list is walked read-only and freed below.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(format!(
            "getifaddrs failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut entry = head;
    while !entry.is_null() {
        // SAFETY: `entry` is a live node of the getifaddrs list.
        let ifa = unsafe { &*entry };
        entry = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: a non-null `ifa_addr` points to a sockaddr whose family tells its concrete type.
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        if family != libc::AF_INET && family != libc::AF_INET6 {
            continue;
        }
        // SAFETY: `ifa_name` is a NUL-terminated interface name.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let internal = ifa.ifa_flags & (libc::IFF_LOOPBACK as libc::c_uint) != 0;
        let mac = std::fs::read_to_string(format!("/sys/class/net/{name}/address"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "00:00:00:00:00:00".to_string());
        let entries = groups.entry(name).or_default();
        if family == libc::AF_INET {
            // SAFETY: AF_INET addresses are `sockaddr_in`.
            let address = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(address.sin_addr.s_addr));
            let mask_bits = if ifa.ifa_netmask.is_null() {
                0
            } else {
                // SAFETY: the netmask of an AF_INET address is a `sockaddr_in`.
                u32::from_be(
                    unsafe { *(ifa.ifa_netmask as *const libc::sockaddr_in) }
                        .sin_addr
                        .s_addr,
                )
            };
            entries.push(serde_json::json!({
                "address": ip.to_string(), "netmask": Ipv4Addr::from(mask_bits).to_string(), "family": "IPv4",
                "mac": mac, "internal": internal, "cidr": format!("{ip}/{}", mask_bits.count_ones()),
            }));
        } else {
            // SAFETY: AF_INET6 addresses are `sockaddr_in6`.
            let address = unsafe { *(ifa.ifa_addr as *const libc::sockaddr_in6) };
            let ip = Ipv6Addr::from(address.sin6_addr.s6_addr);
            let mask_bytes = if ifa.ifa_netmask.is_null() {
                [0u8; 16]
            } else {
                // SAFETY: the netmask of an AF_INET6 address is a `sockaddr_in6`.
                unsafe { *(ifa.ifa_netmask as *const libc::sockaddr_in6) }
                    .sin6_addr
                    .s6_addr
            };
            let prefix: u32 = mask_bytes.iter().map(|byte| byte.count_ones()).sum();
            entries.push(serde_json::json!({
                "address": ip.to_string(), "netmask": Ipv6Addr::from(mask_bytes).to_string(), "family": "IPv6",
                "mac": mac, "internal": internal, "cidr": format!("{ip}/{prefix}"), "scopeid": address.sin6_scope_id,
            }));
        }
    }
    // SAFETY: `head` came from a successful getifaddrs.
    unsafe { libc::freeifaddrs(head) };
    Ok(serde_json::to_string(&groups).unwrap_or_else(|_| "{}".to_string()))
}

#[cfg(not(unix))]
fn network_interfaces_json() -> Result<String, String> {
    Err("os.networkInterfaces() is not supported by thaw-js on this platform".into())
}

impl Interp {
    pub(crate) fn install_sysio(&mut self) {
        self.run(DYNAMIC_HELPERS)
            .unwrap_or_else(|_| panic!("thaw-js dynamic helper prelude must evaluate"));
        let g = self.global.clone();
        self.install_child_process();
        let f = self.native(|it, _, a| it.report_uncaught(arg(a, 0)));
        g.declare("__thaw_report_uncaught", f, true);
        let f = self.native(|_, _, _| Ok(Value::str(&identity())));
        g.declare("__thaw_os_identity", f, true);
        let f = self.native(|_, _, _| Ok(Value::str(&info())));
        g.declare("__thaw_os_info", f, true);
        let f = self.native(|it, _, a| {
            Ok(Value::str(&priority_result(get_priority(
                it.num(&arg(a, 0)) as i32,
            ))))
        });
        g.declare("__thaw_os_get_priority", f, true);
        let f = self.native(|it, _, a| {
            let (pid, prio) = (it.num(&arg(a, 0)) as i32, it.num(&arg(a, 1)) as i32);
            Ok(Value::str(&priority_result(set_priority(pid, prio))))
        });
        g.declare("__thaw_os_set_priority", f, true);
        let f = self.native(|it, _, _| match network_interfaces_json() {
            Ok(json) => Ok(Value::str(&json)),
            Err(message) => it.throw_error("Error", &message),
        });
        g.declare("__thaw_os_network_interfaces", f, true);
        let f = self.native(|it, _, a| {
            #[cfg(unix)]
            let tty = unsafe { libc::isatty(it.num(&arg(a, 0)) as i32) } == 1;
            #[cfg(not(unix))]
            let tty = false;
            let _ = (&it, &a);
            Ok(Value::Bool(tty))
        });
        g.declare("__thaw_process_is_tty", f, true);
        // (fd, hex) -> raw bytes to stdout/stderr, after any pending partial line
        let f = self.native(|it, _, a| {
            let fd = it.num(&arg(a, 0)) as i32;
            let Some(bytes) = unhex(&it.to_str(&arg(a, 1))).filter(|_| matches!(fd, 1 | 2)) else {
                return it.throw_error("TypeError", "invalid raw process output");
            };
            if fd == 1 {
                it.flush_partial();
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(&bytes).and_then(|()| out.flush());
            } else {
                let mut out = std::io::stderr().lock();
                let _ = out.write_all(&bytes).and_then(|()| out.flush());
            }
            Ok(Value::Undefined)
        });
        g.declare("__thaw_process_write_bytes", f, true);
        // (fd, hex) -> bytes to an arbitrary descriptor
        let f = self.native(|it, _, a| {
            let fd = it.num(&arg(a, 0)) as i32;
            let Some(bytes) = unhex(&it.to_str(&arg(a, 1))).filter(|_| fd >= 0) else {
                return it.throw_error("TypeError", "invalid tty output");
            };
            it.flush_partial();
            let _ = std::io::stdout().flush();
            #[cfg(unix)]
            {
                let mut left = &bytes[..];
                while !left.is_empty() {
                    let n = unsafe { libc::write(fd, left.as_ptr().cast(), left.len()) };
                    if n < 0 {
                        let e = std::io::Error::last_os_error();
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        return it.throw_error("Error", &e.to_string());
                    }
                    left = &left[n as usize..];
                }
            }
            Ok(Value::Undefined)
        });
        g.declare("__thaw_tty_write", f, true);
    }
}

// ----- `globalThis.__thaw_fs(operation, path, value, recursive)` -----
// The synchronous subset of the QuickJS host's filesystem bridge that the registry's `node:fs`
// shim needs for whole-file reads/writes, directory listing and stat. Descriptor operations
// (string-flag `fd_open`, read/write/stat/sync/truncate/tell/chmod/utimes/statfs) and chown/lchown
// answer ENOSYS where not ported.
// ponytail: add the rest when a package needs them; chown/lchown stay ENOSYS until the pinned test
// `os_tty_process_and_fs_host_functions` is updated (it asserts ENOSYS for chown).

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(unix)]
fn name_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(unix)]
fn path_from(bytes: Vec<u8>) -> std::path::PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes).into()
}

fn fs_code(e: &std::io::Error) -> String {
    #[cfg(unix)]
    if let Some(raw) = e.raw_os_error() {
        return match raw {
            libc::EPERM => "EPERM",
            libc::ENOENT => "ENOENT",
            libc::EIO => "EIO",
            libc::EBADF => "EBADF",
            libc::EACCES => "EACCES",
            libc::EBUSY => "EBUSY",
            libc::EEXIST => "EEXIST",
            libc::EXDEV => "EXDEV",
            libc::ENOTDIR => "ENOTDIR",
            libc::EISDIR => "EISDIR",
            libc::EINVAL => "EINVAL",
            libc::EMFILE => "EMFILE",
            libc::ENOSPC => "ENOSPC",
            libc::EROFS => "EROFS",
            libc::ENAMETOOLONG => "ENAMETOOLONG",
            libc::ENOSYS => "ENOSYS",
            libc::ENOTEMPTY => "ENOTEMPTY",
            libc::ELOOP => "ELOOP",
            n => return format!("ERRNO_{n}"),
        }
        .into();
    }
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "ENOENT",
        PermissionDenied => "EACCES",
        AlreadyExists => "EEXIST",
        InvalidInput => "EINVAL",
        IsADirectory => "EISDIR",
        NotADirectory => "ENOTDIR",
        _ => "EIO",
    }
    .into()
}

fn fs_err(op: &str, path: &str, e: &std::io::Error) -> String {
    format!(
        "{{\"ok\":false,\"code\":{},\"operation\":{},\"path\":{},\"message\":{}}}",
        q(&fs_code(e)),
        q(op),
        q(path),
        q(&e.to_string())
    )
}

fn ms(t: std::io::Result<std::time::SystemTime>) -> f64 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

fn ns(t: std::io::Result<std::time::SystemTime>) -> i128 {
    t.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128)
}

#[cfg(unix)]
fn stat_json(m: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    let ctime_ms = m.ctime() as f64 * 1000.0 + m.ctime_nsec() as f64 / 1e6;
    let t = |s: i64, n: i64| i128::from(s) * 1_000_000_000 + i128::from(n);
    format!(
        "{{\"ok\":true,\"length\":{},\"file\":{},\"directory\":{},\"symlink\":{},\"readonly\":{},\
         \"dev\":{},\"ino\":{},\"mode\":{},\"nlink\":{},\"uid\":{},\"gid\":{},\"rdev\":{},\"blksize\":{},\"blocks\":{},\
         \"atimeMs\":{},\"mtimeMs\":{},\"ctimeMs\":{ctime_ms},\"birthtimeMs\":{},\
         \"exact\":{{\"length\":\"{}\",\"dev\":\"{}\",\"ino\":\"{}\",\"mode\":\"{}\",\"nlink\":\"{}\",\"uid\":\"{}\",\"gid\":\"{}\",\
         \"rdev\":\"{}\",\"blksize\":\"{}\",\"blocks\":\"{}\",\"atimeNs\":\"{}\",\"mtimeNs\":\"{}\",\"ctimeNs\":\"{}\",\"birthtimeNs\":\"{}\"}}}}",
        m.len(),
        m.is_file(),
        m.is_dir(),
        m.file_type().is_symlink(),
        m.permissions().readonly(),
        m.dev(),
        m.ino(),
        m.mode(),
        m.nlink(),
        m.uid(),
        m.gid(),
        m.rdev(),
        m.blksize(),
        m.blocks(),
        ms(m.accessed()),
        ms(m.modified()),
        ms(m.created()),
        m.len(),
        m.dev(),
        m.ino(),
        m.mode(),
        m.nlink(),
        m.uid(),
        m.gid(),
        m.rdev(),
        m.blksize(),
        m.blocks(),
        t(m.atime(), m.atime_nsec()),
        t(m.mtime(), m.mtime_nsec()),
        t(m.ctime(), m.ctime_nsec()),
        ns(m.created())
    )
}

#[cfg(unix)]
fn readdir_json(dir: &std::path::Path, typed: bool, recursive: bool) -> std::io::Result<String> {
    fn visit(
        dir: &std::path::Path,
        prefix: &[u8],
        typed: bool,
        recursive: bool,
        out: &mut Vec<String>,
    ) -> std::io::Result<()> {
        // Sorted per directory by name bytes, like the QuickJS host (libuv scandir order).
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            match entry {
                Ok(e) => entries.push(e),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
        }
        entries.sort_by_cached_key(|e| name_bytes(&e.file_name()));
        for entry in entries {
            let name = name_bytes(&entry.file_name());
            let mut rel = prefix.to_vec();
            if !rel.is_empty() {
                rel.push(b'/');
            }
            rel.extend_from_slice(&name);
            let path = entry.path();
            let meta = if typed || recursive {
                match std::fs::symlink_metadata(&path) {
                    Ok(m) => Some(m),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(e) => return Err(e),
                }
            } else {
                None
            };
            out.push(format!(
                "{{\"nameHex\":\"{}\",\"relativeHex\":\"{}\",\"pathHex\":\"{}\",\"parentPath\":{},\"stat\":{}}}",
                hex(&name),
                hex(&rel),
                hex(&name_bytes(path.as_os_str())),
                q(&dir.to_string_lossy()),
                meta.as_ref().map_or("null".into(), stat_json)
            ));
            if recursive && meta.is_some_and(|m| m.is_dir()) {
                visit(&path, &rel, typed, recursive, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(dir, &[], typed, recursive, &mut out)?;
    Ok(format!("{{\"ok\":true,\"entries\":[{}]}}", out.join(",")))
}

// Process-wide: async fn bodies run on coroutine threads, so fds opened on one thread must be
// visible from another. The lock covers only table access, never file IO; ops work on a dup
// (`try_clone`, same open file description, so the offset is shared as with a real fd).
// ponytail: one global table lock, per-fd locks if fd-heavy threads contend
static FDS: std::sync::Mutex<(u32, std::collections::BTreeMap<u32, std::fs::File>)> =
    std::sync::Mutex::new((10, std::collections::BTreeMap::new()));

fn fds() -> std::sync::MutexGuard<'static, (u32, std::collections::BTreeMap<u32, std::fs::File>)> {
    FDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// `fd_open` (string flags: `r`, `w`, `a+`, `wx`, ...) and the `fd_*` operations on the files it opened.
// ponytail: numeric (`#bits`) open flags and fd_chown are not ported (chown stays ENOSYS, see below)
fn fd_op(op: &str, p: &std::path::Path, path: &str, value: &str) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidInput, m.to_string());
    if op == "fd_open" {
        let (flag, mode) = value
            .split_once(',')
            .ok_or_else(|| bad("missing open mode"))?;
        let mut o = std::fs::OpenOptions::new();
        match flag {
            "r" | "rs" => o.read(true),
            "r+" | "rs+" => o.read(true).write(true),
            "w" => o.write(true).create(true).truncate(true),
            "wx" | "xw" => o.write(true).create_new(true),
            "w+" => o.read(true).write(true).create(true).truncate(true),
            "wx+" | "xw+" => o.read(true).write(true).create_new(true),
            "a" | "as" => o.append(true).create(true),
            "ax" | "xa" => o.append(true).create_new(true),
            "a+" | "as+" => o.read(true).append(true).create(true),
            "ax+" | "xa+" => o.read(true).append(true).create_new(true),
            _ => return Err(std::io::Error::from_raw_os_error(libc::ENOSYS)),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(mode.parse::<u32>().unwrap_or(0o666));
        }
        let file = o.open(p)?;
        let mut t = fds();
        let fd = t.0;
        t.0 += 1;
        t.1.insert(fd, file);
        return Ok(format!("{{\"ok\":true,\"fd\":{fd}}}"));
    }
    let fd: u32 = path
        .parse()
        .map_err(|_| std::io::Error::from_raw_os_error(libc::EBADF))?;
    if op == "fd_close" {
        return fds()
            .1
            .remove(&fd)
            .map(|_| "{\"ok\":true}".to_string())
            .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EBADF));
    }
    let mut file = fds()
        .1
        .get(&fd)
        .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EBADF))?
        .try_clone()?;
    match op {
        "fd_read_all" => {
            let mut b = Vec::new();
            file.read_to_end(&mut b)?;
            Ok(format!("{{\"ok\":true,\"data\":\"{}\"}}", hex(&b)))
        }
        "fd_write_all" => {
            file.write_all(&unhex(value).ok_or_else(|| bad("invalid hex"))?)?;
            Ok("{\"ok\":true}".into())
        }
        "fd_read" => {
            let (pos, len) = value
                .split_once(',')
                .ok_or_else(|| bad("invalid read options"))?;
            let mut b = vec![
                0;
                len.parse::<usize>()
                    .map_err(|_| bad("invalid read length"))?
            ];
            let n = if pos == "-1" {
                file.read(&mut b)?
            } else {
                let cursor = file.stream_position()?;
                file.seek(SeekFrom::Start(
                    pos.parse().map_err(|_| bad("invalid read position"))?,
                ))?;
                let r = file.read(&mut b);
                file.seek(SeekFrom::Start(cursor))?;
                r?
            };
            b.truncate(n);
            Ok(format!(
                "{{\"ok\":true,\"data\":\"{}\",\"length\":{n}}}",
                hex(&b)
            ))
        }
        "fd_write" => {
            let (pos, data) = value
                .split_once(':')
                .ok_or_else(|| bad("invalid write options"))?;
            let b = unhex(data).ok_or_else(|| bad("invalid hex"))?;
            let n = if pos == "-1" {
                file.write(&b)?
            } else {
                let cursor = file.stream_position()?;
                file.seek(SeekFrom::Start(
                    pos.parse().map_err(|_| bad("invalid write position"))?,
                ))?;
                let r = file.write(&b);
                file.seek(SeekFrom::Start(cursor))?;
                r?
            };
            Ok(format!("{{\"ok\":true,\"length\":{n}}}"))
        }
        "fd_stat" => file.metadata().map(|m| stat_json(&m)),
        "fd_sync" => file.sync_all().map(|()| "{\"ok\":true}".into()),
        "fd_datasync" => file.sync_data().map(|()| "{\"ok\":true}".into()),
        "fd_truncate" => value
            .parse::<u64>()
            .map_err(|_| bad("invalid truncate length"))
            .and_then(|len| file.set_len(len))
            .map(|()| "{\"ok\":true}".into()),
        "fd_tell" => file
            .stream_position()
            .map(|position| format!("{{\"ok\":true,\"position\":{position}}}")),
        #[cfg(unix)]
        "fd_chmod" => fs_parse_mode(value)
            .and_then(|mode| {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(mode))
            })
            .map(|()| "{\"ok\":true}".into()),
        #[cfg(unix)]
        "fd_utimes" => {
            use std::os::fd::AsRawFd;
            let mut values = value.split(',');
            let times = [fs_timespec(values.next())?, fs_timespec(values.next())?];
            if unsafe { libc::futimens(file.as_raw_fd(), times.as_ptr()) } == 0 {
                Ok("{\"ok\":true}".into())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
        #[cfg(unix)]
        "fd_statfs" => {
            use std::os::fd::AsRawFd;
            let fd = file.as_raw_fd();
            let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if unsafe { libc::fstatvfs(fd, st.as_mut_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let st = unsafe { st.assume_init() };
            fs_fstatfs_type(fd).map(|fs_type| fs_statfs_record(&st, fs_type))
        }
        _ => Err(std::io::Error::from_raw_os_error(libc::ENOSYS)),
    }
}

#[cfg(unix)]
fn fs_invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.to_string())
}

#[cfg(unix)]
fn fs_c_path(p: &std::path::Path) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(name_bytes(p.as_os_str()))
        .map_err(|_| fs_invalid("path contains a NUL byte"))
}

#[cfg(unix)]
fn fs_parse_mode(value: &str) -> std::io::Result<u32> {
    match value.parse::<u32>() {
        Ok(mode) if mode <= 0o7777 => Ok(mode),
        _ => Err(fs_invalid("invalid file mode")),
    }
}

#[cfg(unix)]
fn fs_timespec(value: Option<&str>) -> std::io::Result<libc::timespec> {
    let seconds = value
        .ok_or_else(|| fs_invalid("missing timestamp"))?
        .parse::<f64>()
        .map_err(|_| fs_invalid("invalid timestamp"))?;
    if !seconds.is_finite() {
        return Err(fs_invalid("invalid timestamp"));
    }
    let mut sec = seconds.trunc() as i128;
    let mut nsec = ((seconds - sec as f64) * 1_000_000_000.0).trunc() as i128;
    if nsec < 0 {
        sec -= 1;
        nsec += 1_000_000_000;
    } else if nsec >= 1_000_000_000 {
        sec += 1;
        nsec -= 1_000_000_000;
    }
    if sec < libc::time_t::MIN as i128
        || sec > libc::time_t::MAX as i128
        || !(0..1_000_000_000).contains(&nsec)
    {
        return Err(fs_invalid("timestamp out of range"));
    }
    Ok(libc::timespec {
        tv_sec: sec as libc::time_t,
        tv_nsec: nsec as libc::c_long,
    })
}

#[cfg(unix)]
fn fs_path_utimes(p: &std::path::Path, value: &str, flags: libc::c_int) -> std::io::Result<()> {
    let mut values = value.split(',');
    let times = [fs_timespec(values.next())?, fs_timespec(values.next())?];
    let c = fs_c_path(p)?;
    if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "aix"
))]
fn fs_statfs_type(c: &std::ffi::CString) -> std::io::Result<u64> {
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { st.assume_init() }.f_type as u64)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "aix"
)))]
fn fs_statfs_type(_: &std::ffi::CString) -> std::io::Result<u64> {
    Ok(0)
}

#[cfg(unix)]
fn fs_statfs_record(st: &libc::statvfs, fs_type: u64) -> String {
    serde_json::json!({
        "ok": true, "type": fs_type, "bsize": st.f_bsize, "blocks": st.f_blocks,
        "bfree": st.f_bfree, "bavail": st.f_bavail, "files": st.f_files, "ffree": st.f_ffree,
        "exact": {
            "type": fs_type.to_string(), "bsize": st.f_bsize.to_string(), "blocks": st.f_blocks.to_string(),
            "bfree": st.f_bfree.to_string(), "bavail": st.f_bavail.to_string(),
            "files": st.f_files.to_string(), "ffree": st.f_ffree.to_string(),
        },
    })
    .to_string()
}

#[cfg(unix)]
fn fs_statfs(p: &std::path::Path) -> std::io::Result<String> {
    let c = fs_c_path(p)?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let st = unsafe { st.assume_init() };
    Ok(fs_statfs_record(&st, fs_statfs_type(&c)?))
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "aix"
))]
fn fs_fstatfs_type(fd: std::os::fd::RawFd) -> std::io::Result<u64> {
    let mut st = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(fd, st.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { st.assume_init() }.f_type as u64)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "aix"
)))]
fn fs_fstatfs_type(_: std::os::fd::RawFd) -> std::io::Result<u64> {
    Ok(0)
}

// ---- mkdtemp ----
#[cfg(unix)]
fn fs_mkdtemp(prefix: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    use std::os::unix::fs::DirBuilderExt;
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for _ in 0..8 {
        let mut random = [0u8; 6];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let mut name = prefix.as_os_str().to_os_string();
        name.push(
            random
                .iter()
                .map(|b| CHARS[usize::from(*b) % CHARS.len()] as char)
                .collect::<String>(),
        );
        let created = std::path::PathBuf::from(name);
        match std::fs::DirBuilder::new().mode(0o700).create(&created) {
            Ok(()) => {
                return Ok(format!(
                    "{{\"ok\":true,\"path\":{},\"pathHex\":\"{}\"}}",
                    q(&created.to_string_lossy()),
                    hex(&name_bytes(created.as_os_str()))
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::from_raw_os_error(libc::EEXIST))
}

// ---- cp / copy_guard / copy_excl (ported from the QuickJS host) ----
#[cfg(unix)]
fn fs_same_metadata(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(unix)]
fn fs_same_file(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<bool> {
    let source_meta = std::fs::metadata(source)?;
    match std::fs::metadata(destination) {
        Ok(dest_meta) => Ok(fs_same_metadata(&source_meta, &dest_meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(unix)]
fn fs_copy_destination_resolved(
    destination: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    use std::path::Component;
    let absolute = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        std::env::current_dir()?.join(destination)
    };
    let mut resolved = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                resolved.push(name);
                match std::fs::canonicalize(&resolved) {
                    Ok(path) => resolved = path,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if std::fs::symlink_metadata(&resolved)
                            .is_ok_and(|m| m.file_type().is_symlink())
                        {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "unresolved destination symlink",
                            ));
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(resolved)
}

#[cfg(unix)]
fn fs_copy_unsafe_relation(
    source: &std::path::Path,
    destination: &std::path::Path,
    dereference: bool,
) -> std::io::Result<bool> {
    let source_link = std::fs::symlink_metadata(source)?;
    if source_link.file_type().is_symlink() && !dereference {
        return match std::fs::symlink_metadata(destination) {
            Ok(dest_link) => Ok(fs_same_metadata(&source_link, &dest_link)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        };
    }
    if fs_same_file(source, destination)? {
        return Ok(true);
    }
    let canonical_source = std::fs::canonicalize(source)?;
    let canonical_destination = fs_copy_destination_resolved(destination)?;
    Ok(canonical_destination == canonical_source
        || (source_link.is_dir() || (dereference && std::fs::metadata(source)?.is_dir()))
            && canonical_destination.starts_with(&canonical_source))
}

#[cfg(unix)]
fn fs_copy_file(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<u64> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let same = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "source and destination are the same file",
        )
    };
    let mut input = std::fs::File::open(source)?;
    let source_meta = input.metadata()?;
    if fs_same_file(source, destination)? {
        return Err(same());
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(source_meta.permissions().mode() & 0o7777)
        .open(destination)?;
    if fs_same_metadata(&source_meta, &output.metadata()?) {
        return Err(same());
    }
    output.set_len(0)?;
    let bytes = std::io::copy(&mut input, &mut output)?;
    output.set_permissions(source_meta.permissions())?;
    Ok(bytes)
}

#[cfg(unix)]
fn fs_copy_exclusive(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<u64> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut input = std::fs::File::open(source)?;
    let source_meta = input.metadata()?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(source_meta.permissions().mode() & 0o7777)
        .open(destination)?;
    let bytes = std::io::copy(&mut input, &mut output)?;
    output.set_permissions(source_meta.permissions())?;
    Ok(bytes)
}

#[cfg(unix)]
fn fs_copy_recursive(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> std::io::Result<u64> {
    if std::fs::symlink_metadata(source)?.is_dir() {
        if fs_copy_unsafe_relation(source, destination, false)? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "destination is source or its descendant",
            ));
        }
        std::fs::create_dir_all(destination)?;
        let mut copied = 0;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copied += fs_copy_recursive(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(copied)
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fs_copy_file(source, destination)
    }
}

#[cfg(unix)]
pub(crate) fn host_fs(op: &str, path: &str, value: &str, recursive: bool) -> String {
    let (op, raw_value) = op
        .strip_suffix("_raw_value")
        .map_or((op, false), |b| (b, true));
    let (op, raw_path) = op.strip_suffix("_raw").map_or((op, false), |b| (b, true));
    let bad = |m: &str| {
        fs_err(
            op,
            path,
            &std::io::Error::new(std::io::ErrorKind::InvalidInput, m.to_string()),
        )
    };
    let to_path = |s: &str, raw: bool| -> Option<std::path::PathBuf> {
        if raw {
            unhex(s).map(path_from)
        } else {
            Some(s.into())
        }
    };
    let Some(p) = to_path(path, raw_path) else {
        return bad("invalid raw path");
    };
    if path.contains('\0') || value.contains('\0') {
        return bad("path contains a NUL byte");
    }
    let v = to_path(value, raw_value).unwrap_or_default();
    let ok = |r: std::io::Result<()>| r.map(|()| "{\"ok\":true}".to_string());
    let result: std::io::Result<String> = match op {
        _ if op.starts_with("fd_") => fd_op(op, &p, path, value),
        "exists" => Ok(format!("{{\"ok\":true,\"exists\":{}}}", p.exists())),
        "access" => {
            let mode = value.parse::<i32>().unwrap_or(0);
            let c = std::ffi::CString::new(name_bytes(p.as_os_str())).unwrap_or_default();
            if unsafe { libc::access(c.as_ptr(), mode) } == 0 {
                Ok("{\"ok\":true}".into())
            } else {
                Err(std::io::Error::last_os_error())
            }
        }
        "read" => std::fs::read(&p).map(|b| format!("{{\"ok\":true,\"data\":\"{}\"}}", hex(&b))),
        "write" => ok(std::fs::write(&p, unhex(value).unwrap_or_default())),
        "append" => ok(std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&p)
            .and_then(|mut f| f.write_all(&unhex(value).unwrap_or_default()))),
        "mkdir" | "mkdir_mode" => {
            let existed = p.exists();
            let r = if recursive {
                std::fs::create_dir_all(&p)
            } else {
                std::fs::create_dir(&p)
            };
            r.map(|()| {
                if existed {
                    "{\"ok\":true,\"created\":null,\"createdHex\":null}".to_string()
                } else {
                    format!(
                        "{{\"ok\":true,\"created\":{},\"createdHex\":\"{}\"}}",
                        q(&p.to_string_lossy()),
                        hex(&name_bytes(p.as_os_str()))
                    )
                }
            })
        }
        "readdir" => readdir_json(&p, value == "typed", recursive),
        "stat" => std::fs::metadata(&p).map(|m| stat_json(&m)),
        "lstat" => std::fs::symlink_metadata(&p).map(|m| stat_json(&m)),
        "unlink" => ok(std::fs::remove_file(&p)),
        "rmdir" => ok(if recursive {
            std::fs::remove_dir_all(&p)
        } else {
            std::fs::remove_dir(&p)
        }),
        "rename" => ok(std::fs::rename(&p, &v)),
        "copy" => std::fs::copy(&p, &v).map(|n| format!("{{\"ok\":true,\"length\":{n}}}")),
        "realpath" | "readlink" => (if op == "realpath" {
            std::fs::canonicalize(&p)
        } else {
            std::fs::read_link(&p)
        })
        .map(|r| {
            format!(
                "{{\"ok\":true,\"path\":{},\"pathHex\":\"{}\"}}",
                q(&r.to_string_lossy()),
                hex(&name_bytes(r.as_os_str()))
            )
        }),
        "chmod" => ok(fs_parse_mode(value).and_then(|mode| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode))
        })),
        "symlink" => ok(std::os::unix::fs::symlink(&p, &v)),
        "link" => ok(std::fs::hard_link(&p, &v)),
        "truncate" => ok(value
            .parse::<u64>()
            .map_err(|_| fs_invalid("invalid truncate length"))
            .and_then(|len| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&p)
                    .and_then(|f| f.set_len(len))
            })),
        "utimes" => ok(fs_path_utimes(&p, value, 0)),
        "lutimes" => ok(fs_path_utimes(&p, value, libc::AT_SYMLINK_NOFOLLOW)),
        "statfs" => fs_statfs(&p),
        "mkdtemp" => fs_mkdtemp(&p),
        "copy_excl" => fs_copy_exclusive(&p, &v).map(|n| format!("{{\"ok\":true,\"length\":{n}}}")),
        "copy_guard" => fs_copy_unsafe_relation(&p, &v, recursive)
            .map(|u| format!("{{\"ok\":true,\"unsafe\":{u}}}")),
        "cp" => if recursive {
            fs_copy_recursive(&p, &v)
        } else {
            fs_copy_file(&p, &v)
        }
        .map(|n| format!("{{\"ok\":true,\"length\":{n}}}")),
        _ => Err(std::io::Error::from_raw_os_error(libc::ENOSYS)),
    };
    result.unwrap_or_else(|e| fs_err(op, path, &e))
}

#[cfg(not(unix))]
fn host_fs(op: &str, path: &str, _value: &str, _recursive: bool) -> String {
    fs_err(op, path, &std::io::ErrorKind::Unsupported.into())
}

impl Interp {
    pub(crate) fn install_fs_host(&mut self) {
        let f = self.native(|it, _, a| {
            let (op, path, value) = (
                it.to_str(&arg(a, 0)),
                it.to_str(&arg(a, 1)),
                it.to_str(&arg(a, 2)),
            );
            Ok(Value::str(&host_fs(&op, &path, &value, arg(a, 3).truthy())))
        });
        self.global.declare("__thaw_fs", f, true);
    }
}
