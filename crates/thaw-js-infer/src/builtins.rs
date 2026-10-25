//! Typed tables for Node built-in modules and globals (`require('path')`, `console`, `process`,
//! `Buffer`, `Math` constants, `Object` statics). A built-in namespace is an `Atom::Ns(id)`; calls
//! on it are resolved here, and pure ones do not make their arguments escape.

use crate::analyze::An;
use crate::ty::*;

pub const PATH: u8 = 0;
pub const OS: u8 = 1;
pub const FS: u8 = 2;
pub const UTIL: u8 = 3;
pub const ASSERT: u8 = 4;
pub const CONSOLE: u8 = 5;
pub const PROCESS: u8 = 6;
pub const CRYPTO: u8 = 7;
pub const BUFFER: u8 = 8;
/// a Node built-in without a table (events, stream, http, ...)
pub const OPAQUE: u8 = 9;

/// `require(spec)` of a Node built-in.
pub fn ns_of_require(spec: &str) -> Option<u8> {
    let s = spec.strip_prefix("node:").unwrap_or(spec);
    Some(match s {
        "path" | "path/posix" => PATH,
        "os" => OS,
        "fs" => FS,
        "util" => UTIL,
        "assert" | "assert/strict" => ASSERT,
        "crypto" => CRYPTO,
        "buffer" => BUFFER,
        "events" | "stream" | "url" | "http" | "https" | "http2" | "zlib" | "net" | "tls"
        | "dns" | "child_process" | "readline" | "querystring" | "string_decoder" | "timers"
        | "vm" | "worker_threads" | "module" | "perf_hooks" | "async_hooks" | "cluster"
        | "dgram" | "tty" | "v8" | "punycode" | "constants" | "fs/promises" | "stream/promises"
        | "timers/promises" => OPAQUE,
        _ => return None,
    })
}

/// Unresolved global identifiers that denote a built-in namespace.
pub fn ns_of_global(name: &str) -> Option<u8> {
    Some(match name {
        "console" => CONSOLE,
        "process" => PROCESS,
        "Buffer" => BUFFER,
        _ => return None,
    })
}

impl An {
    fn arr_of(&mut self, key: u32, tag: u8, elem: Ty) -> Ty {
        let s = self.site_pub((key, tag), Site::Arr(elem));
        Ty::one(Atom::Arr(s))
    }

    /// `ns.name` read as a value.
    pub(crate) fn ns_prop(&mut self, ns: u8, name: &str, key: u32) -> Ty {
        match (ns, name) {
            (PATH, "sep" | "delimiter") | (OS, "EOL" | "devNull") => Ty::str(),
            (PATH, "posix" | "win32") => Ty::one(Atom::Ns(PATH)),
            (PROCESS, "platform" | "arch" | "version" | "execPath" | "title") => Ty::str(),
            (PROCESS, "pid" | "ppid") => Ty::num(),
            (PROCESS, "argv" | "execArgv") => self.arr_of(key, 11, Ty::str()),
            (PROCESS, "stdout" | "stderr" | "stdin") => Ty::one(Atom::Ns(OPAQUE)),
            (FS, "promises") => Ty::one(Atom::Ns(OPAQUE)),
            (UTIL, "types") => Ty::one(Atom::Ns(OPAQUE)),
            _ => Ty::Any(Reason::Builtin),
        }
    }

    /// `ns.name(args)`; `None` = not in the table (unknown call).
    pub(crate) fn ns_call(
        &mut self,
        ns: u8,
        name: &str,
        args: &[(Ty, bool)],
        key: u32,
    ) -> Option<Ty> {
        let str_arg = |i: usize| args.get(i).is_some_and(|a| a.0.is_str());
        Some(match (ns, name) {
            (
                PATH,
                "join" | "resolve" | "normalize" | "dirname" | "basename" | "extname" | "relative"
                | "format" | "toNamespacedPath",
            ) => Ty::str(),
            (PATH, "isAbsolute") => Ty::bool(),
            (PATH, "parse") => {
                let s = self.site_pub((key, 12), Site::Obj(Default::default()));
                for f in ["root", "dir", "base", "ext", "name"] {
                    self.site_field_pub(s, f, &Ty::str());
                }
                Ty::one(Atom::Obj(s))
            }
            (
                OS,
                "platform" | "arch" | "type" | "release" | "hostname" | "homedir" | "tmpdir"
                | "endianness" | "version" | "machine",
            ) => Ty::str(),
            (OS, "totalmem" | "freemem" | "uptime" | "availableParallelism") => Ty::num(),
            (FS, "existsSync") => Ty::bool(),
            (FS, "readFileSync") if str_arg(1) => Ty::str(),
            (FS, "realpathSync") => Ty::str(),
            (
                FS,
                "writeFileSync" | "appendFileSync" | "unlinkSync" | "rmSync" | "renameSync"
                | "copyFileSync" | "rmdirSync" | "chmodSync" | "accessSync" | "utimesSync",
            ) => Ty::undef(),
            (FS, "readdirSync") if args.len() == 1 => self.arr_of(key, 13, Ty::str()),
            (UTIL, "format" | "inspect" | "stripVTControlCharacters") => Ty::str(),
            (UTIL, "isDeepStrictEqual") => Ty::bool(),
            (
                ASSERT,
                "ok" | "equal" | "notEqual" | "strictEqual" | "notStrictEqual" | "deepEqual"
                | "notDeepEqual" | "deepStrictEqual" | "notDeepStrictEqual" | "fail" | "ifError"
                | "match" | "doesNotMatch",
            ) => Ty::undef(),
            (CONSOLE, _) => Ty::undef(),
            (PROCESS, "cwd") => Ty::str(),
            (PROCESS, "uptime") => Ty::num(),
            (PROCESS, "exit" | "emitWarning" | "chdir") => Ty::undef(),
            (CRYPTO, "randomUUID") => Ty::str(),
            (CRYPTO, "randomInt") => Ty::num(),
            (BUFFER, "isBuffer") => Ty::bool(),
            (BUFFER, "byteLength") => Ty::num(),
            _ => return None,
        })
    }

    /// Constants of unresolved globals: `Math.PI`, `Number.MAX_SAFE_INTEGER`, ...
    pub(crate) fn global_prop(&self, obj: &str, name: &str) -> Option<Ty> {
        Some(match (obj, name) {
            ("Math", "PI" | "E" | "LN2" | "LN10" | "LOG2E" | "LOG10E" | "SQRT2" | "SQRT1_2") => {
                Ty::num()
            }
            (
                "Number",
                "MAX_SAFE_INTEGER" | "MIN_SAFE_INTEGER" | "EPSILON" | "MAX_VALUE" | "MIN_VALUE"
                | "POSITIVE_INFINITY" | "NEGATIVE_INFINITY" | "NaN",
            ) => Ty::num(),
            _ => return None,
        })
    }
}
