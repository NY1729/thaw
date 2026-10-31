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
  put('__thaw_update_dynamic_value', function (value, increment) { var updated = value; var old = increment ? updated++ : updated--; return [old, updated]; });
  put('__thaw_dynamic_add', function (l, r) { return l + r; });
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
        // ponytail: per-interface addresses need getifaddrs; add when a package reads os.networkInterfaces()
        let f = self.native(|it, _, _| {
            it.throw_error(
                "Error",
                "os.networkInterfaces() is not supported by thaw-js yet",
            )
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
// (string-flag `fd_open` and read/write/stat), chown/utimes/statfs/link/cp and friends answer ENOSYS.
// ponytail: add the remaining operations when a package needs them (fd_*, utimes, link, cp).

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
        for entry in std::fs::read_dir(dir)? {
            let entry = match entry {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
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

thread_local! {
    static FDS: std::cell::RefCell<(u32, std::collections::HashMap<u32, std::fs::File>)> =
        std::cell::RefCell::new((10, std::collections::HashMap::new()));
}

/// `fd_open` (string flags: `r`, `w`, `a+`, `wx`, ...) and the `fd_*` operations on the files it opened.
// ponytail: numeric (`#bits`) open flags, fd_stat-family and fd_chmod/utimes/truncate/sync are not ported
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
        return FDS.with(|t| {
            let mut t = t.borrow_mut();
            let fd = t.0;
            t.0 += 1;
            t.1.insert(fd, file);
            Ok(format!("{{\"ok\":true,\"fd\":{fd}}}"))
        });
    }
    let fd: u32 = path
        .parse()
        .map_err(|_| std::io::Error::from_raw_os_error(libc::EBADF))?;
    FDS.with(|t| {
        let mut t = t.borrow_mut();
        if op == "fd_close" {
            return t
                .1
                .remove(&fd)
                .map(|_| "{\"ok\":true}".to_string())
                .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EBADF));
        }
        let file =
            t.1.get_mut(&fd)
                .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EBADF))?;
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
            _ => Err(std::io::Error::from_raw_os_error(libc::ENOSYS)),
        }
    })
}

#[cfg(unix)]
fn host_fs(op: &str, path: &str, value: &str, recursive: bool) -> String {
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
