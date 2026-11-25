#!/usr/bin/env python3
"""Boundary-value matrix probes for tools/parity-aot: node:* functions x argument shape x value origin.

Motivation: a value made in AOT-compiled code (an `URL`, a Buffer, a Date, ...) crosses into a node:* builtin that is
implemented in the engine prelude or natively, with an argument shape the other probes never use (they pass strings
and numbers). The engine then sees a value that fails an `instanceof` check or has another representation.

For every function template (SPECS) and every parameter slot that takes a value, one probe is generated per
(shape, origin). One slot varies at a time, the others keep a valid default.
  shapes   SHAPES below (string, Buffer, Uint8Array, DataView, ArrayBuffer, URL object, file: URL string, objects with
           toString / valueOf, Date, Map, Set, Symbol, BigInt, null, undefined, array, plain object, class instance with
           a getter, function, number). Path slots build every shape from a real path in the probe's temp dir.
  origins  direct (typed, inferred; only shapes the TS signature accepts), any (declared `any`), box (array element via
           a map callback), json (JSON.parse of the JSON text), clone (structuredClone), eval (`new Function` body).
Each probe runs in a fresh temp dir (PA_TMP/t<N>, cwd inside it) prepared with f.txt, sub/g.txt and a symlink, and
prints the result (or `THROW name/code`) plus a TREE snapshot of the dir, so a stray `file:` directory is visible.

Output (OUT_DIR): bnd.<module>.<origin>.cNN.ts / .mts, manifest.json (gen_probes schema, `imports` per program,
`renderer: boundary`), skips.json. Probe lines are one line `p("<id>", async (d: string) => { ... });`.

Usage: gen_boundary.py OUT_DIR [--modules a,b] [--profile full|ci] [--per-program N]
"""
import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen_probes  # noqa: E402  (HEADER, show() prelude)

PER_PROGRAM = 60

# ---- skip list (documented in README "Boundary-value matrix"): never probed, with a reason.
SKIPS = {
    "interactive": "readline, repl, tty, stdin readers: block on input",
    "network": "net, http, https, http2, dns, tls, dgram, fetch-like: sockets and name resolution",
    "process-state": "process.exit/kill/chdir targets, os.setPriority, fs.chown, setuid: change state later probes use",
    "nondeterministic": "fs.watch/watchFile, os.cpus/uptime/loadavg, crypto random output (only the shape of the result "
                        "is printed), mkdtemp names, timestamps",
    "stream-heavy": "fs streams are probed only for open errors (createReadStream/createWriteStream of a slot value)",
    "child-process": "spawn/exec/fork",
}

# ---- argument shapes. Each: name -> expression builder f(kind, s) with s the JS expression of the payload.
FILEURL = {"P": lambda s: f'new URL("file://" + {s})', "U": lambda s: f"new URL({s})"}
FILESTR = {"P": lambda s: f'"file://" + {s}', "U": lambda s: s}


def _url(k, s):
    return FILEURL.get(k, lambda s: f'new URL("http://x/" + {s})')(s)


def _urlstr(k, s):
    return FILESTR.get(k, lambda s: f'"http://x/" + {s}')(s)


SHAPES = {
    "string": lambda k, s: s,
    "buffer": lambda k, s: f"Buffer.from({s})",
    "u8": lambda k, s: f"new Uint8Array(Buffer.from({s}))",
    "dataview": lambda k, s: f"new DataView(new Uint8Array(Buffer.from({s})).buffer)",
    "arraybuffer": lambda k, s: f"new Uint8Array(Buffer.from({s})).buffer",
    "url": _url,
    "urlstr": _urlstr,
    "tostring": lambda k, s: f"({{ toString() {{ return {s}; }} }})",
    "valueof": lambda k, s: f"({{ valueOf() {{ return {s}; }} }})",
    "date": lambda k, s: "new Date(0)",
    "map": lambda k, s: f'new Map([["a", {s}]])',
    "set": lambda k, s: f"new Set([{s}])",
    "symbol": lambda k, s: 'Symbol("x")',
    "bigint": lambda k, s: "BigInt(5)",
    "null": lambda k, s: "null",
    "undefined": lambda k, s: "undefined",
    "array": lambda k, s: f"[{s}]",
    "plainobj": lambda k, s: f"({{ a: 1, path: {s} }})",
    "classgetter": lambda k, s: f"new (class G {{ get path() {{ return {s}; }} }})()",
    "fn": lambda k, s: f"(() => {s})",
    "number": lambda k, s: "987654" if k == "P" else "5",  # a path slot reads a number as a (bad) fd
}
CORE = ("string", "buffer", "u8", "url", "urlstr", "tostring", "date", "array", "plainobj", "null", "number")
JSONABLE = ("string", "array", "plainobj", "null", "number", "urlstr")
CLONABLE = ("string", "u8", "arraybuffer", "dataview", "date", "map", "set", "array", "plainobj", "bigint", "null",
            "undefined", "number")

# Kinds: P path (payload from PATHS), T text/bytes data, U file: URL text, N number, S plain string, A any-typed param.
# Shapes the TS signature accepts (origin direct only; other shapes need `any`).
DIRECT_OK = {
    "P": ("string", "buffer", "url", "urlstr"),
    "T": ("string", "buffer", "u8", "dataview"),
    "U": ("string", "url", "urlstr"),
    "N": ("number",),
    "S": ("string",),
    "A": ("string", "buffer", "u8", "url", "urlstr", "tostring", "date", "array", "plainobj", "null", "number"),
}
PATHS = {"src": 'd + "/f.txt"', "dir": 'd + "/sub"', "new": 'd + "/new.txt"', "new2": 'd + "/new2.txt"',
         "newdir": 'd + "/newdir"', "link": 'd + "/ln"', "pfx": 'd + "/pfx-"'}
PAYLOAD = {"T": '"hello"', "U": '"file:///a/b.txt"', "N": '"5"', "S": '"a/b.txt"', "A": '"hello"'}
DEFAULT = {"N": "5"}


def payload(kind):
    if kind.startswith("P:"):
        return PATHS[kind[2:]]
    return PAYLOAD[kind[0]]


def default_expr(kind):
    return DEFAULT.get(kind, payload(kind))


# ---- function templates. (module alias, id, template with $0.., slot kinds). Express results so they are deterministic.
def fs_specs():
    s = []
    A = s.append
    def sync(i, t, *k): A(("fs", i, t, k))
    sync("existsSync", "fs.existsSync($0)", "P:src")
    sync("readFileSync", 'fs.readFileSync($0, "utf8")', "P:src")
    sync("readFileSync_buf", "fs.readFileSync($0).length", "P:src")
    sync("writeFileSync", 'fs.writeFileSync($0, "x")', "P:new")
    sync("writeFileSync_data", "fs.writeFileSync(" + PATHS["new"] + ", $0)", "T")
    sync("appendFileSync", 'fs.appendFileSync($0, "x")', "P:new")
    sync("appendFileSync_data", "fs.appendFileSync(" + PATHS["src"] + ", $0)", "T")
    sync("statSync", "fs.statSync($0).isFile()", "P:src")
    sync("lstatSync", "fs.lstatSync($0).isSymbolicLink()", "P:link")
    sync("accessSync", "fs.accessSync($0)", "P:src")
    sync("readdirSync", "fs.readdirSync($0).sort()", "P:dir")
    sync("mkdirSync", "fs.mkdirSync($0, { recursive: true })", "P:newdir")
    sync("mkdirSync_flat", "fs.mkdirSync($0)", "P:newdir")
    sync("rmSync_file", "fs.rmSync($0, { force: true })", "P:src")
    sync("rmSync_dir", "fs.rmSync($0, { recursive: true, force: true })", "P:dir")
    sync("rmdirSync", "fs.rmdirSync($0)", "P:newdir")
    sync("unlinkSync", "fs.unlinkSync($0)", "P:src")
    sync("copyFileSync", "fs.copyFileSync($0, " + PATHS["new"] + ")", "P:src")
    sync("copyFileSync_dest", "fs.copyFileSync(" + PATHS["src"] + ", $0)", "P:new")
    sync("cpSync", "fs.cpSync($0, " + PATHS["newdir"] + ", { recursive: true })", "P:dir")
    sync("cpSync_dest", "fs.cpSync(" + PATHS["dir"] + ", $0, { recursive: true })", "P:newdir")
    sync("renameSync", "fs.renameSync($0, " + PATHS["new"] + ")", "P:src")
    sync("renameSync_dest", "fs.renameSync(" + PATHS["src"] + ", $0)", "P:new")
    sync("symlinkSync", "fs.symlinkSync($0, " + PATHS["new"] + ")", "P:src")
    sync("symlinkSync_dest", "fs.symlinkSync(" + PATHS["src"] + ", $0)", "P:new")
    sync("linkSync", "fs.linkSync($0, " + PATHS["new"] + ")", "P:src")
    sync("linkSync_dest", "fs.linkSync(" + PATHS["src"] + ", $0)", "P:new")
    sync("readlinkSync", "fs.readlinkSync($0)", "P:link")
    sync("realpathSync", "fs.realpathSync($0)", "P:src")
    sync("truncateSync", "fs.truncateSync($0, 2)", "P:src")
    sync("utimesSync", "fs.utimesSync($0, 1, 1)", "P:src")
    sync("chmodSync", "fs.chmodSync($0, 0o644)", "P:src")
    sync("mkdtempSync", "fs.mkdtempSync($0).length > 0", "P:pfx")
    sync("openSync", 'fs.closeSync(fs.openSync($0, "r"))', "P:src")
    sync("opendirSync", "fs.opendirSync($0).closeSync()", "P:dir")
    sync("writeSync_data", '(() => { const fd = fs.openSync(' + PATHS["new"] + ', "w"); try { return fs.writeSync(fd, $0); } finally { fs.closeSync(fd); } })()', "T")
    sync("statfsSync", "typeof fs.statfsSync($0).bsize", "P:dir")
    sync("rmSync_missing", "fs.rmSync($0, { force: true })", "P:new")
    # callback and stream forms of the slot values
    def cb(i, call, k):
        A(("fs", i, "await new Promise((r: any) => { " + call + " })", (k,)))
    R = '(e: any, v: any) => r(e ? "E" + e.name + "/" + e.code : v)'
    cb("readFile_cb", f'fs.readFile($0, "utf8", {R});', "P:src")
    cb("writeFile_cb", f'fs.writeFile($0, "x", {R});', "P:new")
    cb("mkdir_cb", f"fs.mkdir($0, {{ recursive: true }}, {R});", "P:newdir")
    cb("rm_cb", f"fs.rm($0, {{ force: true }}, {R});", "P:src")
    cb("copyFile_cb", f"fs.copyFile($0, {PATHS['new']}, {R});", "P:src")
    cb("stat_cb", f"fs.stat($0, (e: any, v: any) => r(e ? \"E\" + e.name + \"/\" + e.code : v.isFile()));", "P:src")
    cb("access_cb", f"fs.access($0, {R});", "P:src")
    cb("readdir_cb", f"fs.readdir($0, {R});", "P:dir")
    cb("rename_cb", f"fs.rename($0, {PATHS['new']}, {R});", "P:src")
    cb("unlink_cb", f"fs.unlink($0, {R});", "P:src")
    cb("cp_cb", f"fs.cp($0, {PATHS['newdir']}, {{ recursive: true }}, {R});", "P:dir")
    cb("createWriteStream", 'const w = fs.createWriteStream($0); w.on("error", (e: any) => r("E" + e.name + "/" + e.code)); w.end("x", () => r("ok"));', "P:new")
    cb("createReadStream", 'const w = fs.createReadStream($0, "utf8"); let s = ""; w.on("data", (c: any) => { s += c; }); w.on("error", (e: any) => r("E" + e.name + "/" + e.code)); w.on("end", () => r(s));', "P:src")
    return s


def fsp_specs():
    s = []
    def a(i, t, *k): s.append(("fsp", i, "await " + t, k))
    a("readFile", 'fsp.readFile($0, "utf8")', "P:src")
    a("writeFile", 'fsp.writeFile($0, "x")', "P:new")
    a("writeFile_data", "fsp.writeFile(" + PATHS["new"] + ", $0)", "T")
    a("appendFile", 'fsp.appendFile($0, "x")', "P:new")
    a("stat", "(await fsp.stat($0)).isFile()", "P:src")
    a("lstat", "(await fsp.lstat($0)).isSymbolicLink()", "P:link")
    a("access", "fsp.access($0)", "P:src")
    a("readdir", "(await fsp.readdir($0)).sort()", "P:dir")
    a("mkdir", "fsp.mkdir($0, { recursive: true })", "P:newdir")
    a("mkdir_flat", "fsp.mkdir($0)", "P:newdir")
    a("rm_file", "fsp.rm($0, { force: true })", "P:src")
    a("rm_dir", "fsp.rm($0, { recursive: true, force: true })", "P:dir")
    a("rmdir", "fsp.rmdir($0)", "P:newdir")
    a("unlink", "fsp.unlink($0)", "P:src")
    a("copyFile", "fsp.copyFile($0, " + PATHS["new"] + ")", "P:src")
    a("copyFile_dest", "fsp.copyFile(" + PATHS["src"] + ", $0)", "P:new")
    a("cp", "fsp.cp($0, " + PATHS["newdir"] + ", { recursive: true })", "P:dir")
    a("cp_dest", "fsp.cp(" + PATHS["dir"] + ", $0, { recursive: true })", "P:newdir")
    a("rename", "fsp.rename($0, " + PATHS["new"] + ")", "P:src")
    a("rename_dest", "fsp.rename(" + PATHS["src"] + ", $0)", "P:new")
    a("symlink", "fsp.symlink($0, " + PATHS["new"] + ")", "P:src")
    a("symlink_dest", "fsp.symlink(" + PATHS["src"] + ", $0)", "P:new")
    a("link", "fsp.link($0, " + PATHS["new"] + ")", "P:src")
    a("readlink", "fsp.readlink($0)", "P:link")
    a("realpath", "fsp.realpath($0)", "P:src")
    a("truncate", "fsp.truncate($0, 2)", "P:src")
    a("utimes", "fsp.utimes($0, 1, 1)", "P:src")
    a("chmod", "fsp.chmod($0, 0o644)", "P:src")
    a("mkdtemp", "(await fsp.mkdtemp($0)).length > 0", "P:pfx")
    a("open", '(await fsp.open($0, "r")).close()', "P:src")
    a("opendir", "(await fsp.opendir($0)).close()", "P:dir")
    a("statfs", "typeof (await fsp.statfs($0)).bsize", "P:dir")
    a("filehandle_write", '(async () => { const h = await fsp.open(' + PATHS["new"] + ', "w"); try { return await h.writeFile($0); } finally { await h.close(); } })()', "T")
    return s


def misc_specs():
    s = []
    def a(m, i, t, *k): s.append((m, i, t, k))
    # path (S: any string-like; resolve/relative use the probe cwd, which is the temp dir on both sides)
    for fn in ("normalize", "basename", "dirname", "extname", "isAbsolute", "parse", "toNamespacedPath", "resolve", "relative"):
        a("path", fn, f"path.{fn}($0)" if fn != "relative" else 'path.relative($0, "/x")', "S")
    a("path", "join", 'path.join($0, "b")', "S")
    a("path", "join_second", 'path.join("a", $0)', "S")
    a("path", "basename_ext", 'path.basename("a/b.txt", $0)', "S")
    a("path", "format", "path.format($0)", "A")
    a("path", "posix_join", 'path.posix.join($0, "b")', "S")
    a("path", "win32_join", 'path.win32.join($0, "b")', "S")
    # url
    a("nurl", "fileURLToPath", "nurl.fileURLToPath($0)", "U")
    a("nurl", "pathToFileURL", "nurl.pathToFileURL($0).href", "S")
    a("nurl", "URL_ctor", "new URL($0).href", "U")
    a("nurl", "URL_base", 'new URL("x", $0).href', "U")
    a("nurl", "URL_canParse", "URL.canParse($0)", "U")
    a("nurl", "URLSearchParams_ctor", "new URLSearchParams($0).toString()", "A")
    a("nurl", "parse", "nurl.parse($0).href", "U")
    a("nurl", "format", "nurl.format($0)", "A")
    a("nurl", "resolve", 'nurl.resolve("http://a/b/", $0)', "S")
    a("nurl", "domainToASCII", "nurl.domainToASCII($0)", "S")
    a("nurl", "urlToHttpOptions", "nurl.urlToHttpOptions($0).pathname", "A")
    # buffer
    a("buffer", "Buffer_from", "Buffer.from($0)", "A")
    a("buffer", "Buffer_from_enc", 'Buffer.from("aGk=", $0)', "A")
    a("buffer", "Buffer_byteLength", "Buffer.byteLength($0)", "A")
    a("buffer", "Buffer_concat", "Buffer.concat($0)", "A")
    a("buffer", "Buffer_concat_elem", "Buffer.concat([$0])", "A")
    a("buffer", "Buffer_compare", 'Buffer.compare($0, Buffer.from("a"))', "A")
    a("buffer", "Buffer_isBuffer", "Buffer.isBuffer($0)", "A")
    a("buffer", "Buffer_isEncoding", "Buffer.isEncoding($0)", "A")
    a("buffer", "Buffer_alloc", "Buffer.alloc($0)", "A")
    a("buffer", "btoa", "buffer.btoa($0)", "A")
    a("buffer", "atob", "buffer.atob($0)", "A")
    a("buffer", "isUtf8", "buffer.isUtf8($0)", "A")
    a("buffer", "isAscii", "buffer.isAscii($0)", "A")
    a("buffer", "equals", 'Buffer.from("abc").equals($0)', "A")
    a("buffer", "includes", 'Buffer.from("abc").includes($0)', "A")
    a("buffer", "indexOf", 'Buffer.from("abc").indexOf($0)', "A")
    a("buffer", "write", 'Buffer.alloc(8).write($0)', "A")
    a("buffer", "fill", 'Buffer.alloc(4).fill($0)', "A")
    a("buffer", "toString_enc", 'Buffer.from("abc").toString($0)', "A")
    # string_decoder
    a("sd", "ctor_enc", "new sd.StringDecoder($0).encoding", "A")
    a("sd", "write", 'new sd.StringDecoder("utf8").write($0)', "A")
    a("sd", "end", 'new sd.StringDecoder("utf8").end($0)', "A")
    # util
    for fn in ("inspect", "format", "stripVTControlCharacters", "toUSVString", "isDeepStrictEqual"):
        a("util", fn, f"util.{fn}($0)" if fn != "isDeepStrictEqual" else "util.isDeepStrictEqual($0, $0)", "A")
    a("util", "format_s", 'util.format("%s|%d|%j", $0, $0, $0)', "A")
    a("util", "inspect_opts", "util.inspect({ k: $0 }, { depth: 0 })", "A")
    for t in ("isDate", "isUint8Array", "isArrayBuffer", "isMap", "isSet", "isPromise", "isTypedArray", "isAnyArrayBuffer",
              "isBoxedPrimitive", "isRegExp", "isNativeError", "isDataView", "isSymbolObject"):
        a("util", "types_" + t, f"util.types.{t}($0)", "A")
    a("util", "TextEncoder_encode", "new util.TextEncoder().encode($0)", "A")
    a("util", "TextDecoder_decode", "new util.TextDecoder().decode($0)", "A")
    a("util", "TextDecoder_enc", "new util.TextDecoder($0).encoding", "A")
    a("util", "styleText", 'util.styleText("red", $0)', "A")
    a("util", "promisify", "typeof util.promisify($0)", "A")
    a("util", "isArray", "util.isArray($0)", "A")
    # os (slot-less probes carry kind ())
    a("os", "getPriority", "os.getPriority($0)", "N")
    a("os", "userInfo_enc", "typeof os.userInfo({ encoding: $0 }).username", "A")
    for fn in ("platform", "arch", "type", "endianness", "tmpdir_t", "devNull"):
        a("os", fn, f"os.{fn}()" if fn not in ("tmpdir_t", "devNull") else ("typeof os.tmpdir()" if fn == "tmpdir_t" else "os.devNull"))
    # crypto
    a("crypto", "createHash_update", 'crypto.createHash("sha256").update($0).digest("hex")', "T")
    a("crypto", "createHash_alg", 'crypto.createHash($0).update("x").digest("hex")', "A")
    a("crypto", "createHmac_key", 'crypto.createHmac("sha256", $0).update("x").digest("hex")', "T")
    a("crypto", "createHmac_data", 'crypto.createHmac("sha256", "k").update($0).digest("hex")', "T")
    a("crypto", "randomBytes", "crypto.randomBytes($0).length", "N")
    a("crypto", "timingSafeEqual", 'crypto.timingSafeEqual($0, Buffer.from("hello"))', "T")
    a("crypto", "hash", 'crypto.hash("sha1", $0)', "T")
    a("crypto", "pbkdf2Sync_salt", 'crypto.pbkdf2Sync("pw", $0, 1, 8, "sha1").toString("hex")', "T")
    a("crypto", "createHash_enc", 'crypto.createHash("md5").update("x", $0).digest("hex")', "A")
    a("crypto", "randomUUID_opts", "crypto.randomUUID($0).length", "A")
    # zlib (sync; round trip so the output is deterministic)
    for c, dflt in (("gzipSync", "gunzipSync"), ("deflateSync", "inflateSync"), ("deflateRawSync", "inflateRawSync"),
                    ("brotliCompressSync", "brotliDecompressSync")):
        a("zlib", c, f"zlib.{dflt}(zlib.{c}($0)).toString()", "T")
    a("zlib", "deflateSync_opts", 'zlib.inflateSync(zlib.deflateSync("hello", $0)).toString()', "A")
    a("zlib", "deflateSync_level", 'zlib.inflateSync(zlib.deflateSync("hello", { level: $0 })).toString()', "N")
    a("zlib", "inflateSync_arg", "zlib.inflateSync($0)", "T")
    a("zlib", "crc32", "zlib.crc32($0)", "T")
    # events
    a("events", "on_name", "(() => { const e = new events.EventEmitter(); e.on($0, () => 1); return e.emit($0); })()", "A")
    a("events", "emit_arg", '(() => { const e = new events.EventEmitter(); let g: any = "none"; e.on("x", (v: any) => { g = v; }); e.emit("x", $0); return g; })()', "A")
    a("events", "listenerCount", "new events.EventEmitter().listenerCount($0)", "A")
    a("events", "setMaxListeners", "new events.EventEmitter().setMaxListeners($0).getMaxListeners()", "A")
    a("events", "on_listener", 'new events.EventEmitter().on("x", $0).listenerCount("x")', "A")
    a("events", "once_name", "(() => { const e = new events.EventEmitter(); e.once($0, () => 1); return e.emit($0); })()", "A")
    # querystring
    a("qs", "parse", "qs.parse($0)", "A")
    a("qs", "stringify", "qs.stringify($0)", "A")
    a("qs", "escape", "qs.escape($0)", "A")
    a("qs", "unescape", "qs.unescape($0)", "A")
    a("qs", "parse_sep", 'qs.parse("a=1;b=2", $0)', "A")
    # timers
    a("timers", "delay", 'typeof timers.setTimeout(() => 1, $0).ref', "A")
    a("timers", "callback", "(() => { const t = timers.setTimeout($0, 1000); timers.clearTimeout(t); return typeof t; })()", "A")
    a("timers", "clear", "timers.clearTimeout($0)", "A")
    a("timers", "setImmediate_cb", "(() => { const t = timers.setImmediate($0); timers.clearImmediate(t); return typeof t; })()", "A")
    return s


MODULES = {  # program module key -> (imports, spec source filter)
    "fs": ['import * as fs from "node:fs";'],
    "fsp": ['import * as fsp from "node:fs/promises";'],
    "path": ['import * as path from "node:path";'],
    "nurl": ['import * as nurl from "node:url";'],
    "buffer": ['import * as buffer from "node:buffer";'],
    "sd": ['import * as sd from "node:string_decoder";'],
    "util": ['import * as util from "node:util";'],
    "os": ['import * as os from "node:os";'],
    "crypto": ['import * as crypto from "node:crypto";'],
    "zlib": ['import * as zlib from "node:zlib";'],
    "events": ['import * as events from "node:events";'],
    "qs": ['import * as qs from "node:querystring";'],
    "timers": ['import * as timers from "node:timers";'],
}
PRELUDE_IMPORTS = ['import * as __fs from "node:fs";', 'import * as __path from "node:path";']
DEFAULT_MODULES = ["fs", "fsp", "path", "nurl", "buffer", "sd", "util", "os", "crypto", "zlib", "events", "qs", "timers"]
ORIGINS = ("direct", "any", "box", "json", "clone", "eval")

BOUNDARY_PRELUDE = r'''
const __Q: any[] = [];
let __N: number = 0;
function p(name: string, f: (d: string) => any): void { __Q.push([name, f]); }
function __tree(d: string, rel: string, out: string[]): void {
  const names: string[] = __fs.readdirSync(d + rel).sort();
  for (let i = 0; i < names.length; i++) {
    const r: string = rel + "/" + names[i];
    const label: string = names[i].startsWith("pfx-") ? rel + "/pfx-*" : r;  // mkdtemp names are random
    const st: any = __fs.lstatSync(d + r);
    if (st.isSymbolicLink()) { out.push("  l " + label + " -> " + __fs.readlinkSync(d + r).split(d).join("<d>")); }
    else if (st.isDirectory()) { out.push("  d " + label); __tree(d, r, out); }
    else { out.push("  f " + label + " " + st.size + " " + JSON.stringify(__fs.readFileSync(d + r, "utf8").slice(0, 16))); }
  }
}
async function __runAll(): Promise<void> {
  const base: string = process.cwd();
  const root: string = String(process.env.PA_TMP);
  for (let i = 0; i < __Q.length; i++) {
    const name: string = __Q[i][0];
    const d: string = root + "/t" + (++__N);
    console.log("@@ " + name);
    let line: string = "";
    try {
      __fs.rmSync(d, { recursive: true, force: true });
      __fs.mkdirSync(d + "/sub", { recursive: true });
      __fs.writeFileSync(d + "/f.txt", "hello");
      __fs.writeFileSync(d + "/sub/g.txt", "x");
      __fs.symlinkSync("f.txt", d + "/ln");
      process.chdir(d);
      try {
        line = show(await __Q[i][1](d), 0);
      } catch (e: any) {
        line = "THROW " + String(e.name) + "/" + String(e.code) + ": ";
      }
      process.chdir(base);
      const tree: string[] = [];
      __tree(d, "", tree);
      console.log((line + "\nTREE\n" + tree.join("\n")).split(d).join("<d>"));
    } catch (e: any) {
      process.chdir(base);
      console.log("THROW HARNESS: " + String(e.message).split(d).join("<d>"));
    }
  }
}
'''


def render(lines, imports, node):
    text = gen_probes.HEADER + "\n".join(PRELUDE_IMPORTS + imports) + "\n" + gen_probes.PRELUDE.split("function p(")[0] + BOUNDARY_PRELUDE
    text += "\nfunction main(): void {\n" + "".join("  " + l + "\n" for l in lines) + "  __runAll();\n}\n"
    if node:
        text += "main();\n"
    return text


def origin_decl(origin, shape, kind, s, var):
    """Declaration text binding `var`, or None when the (origin, shape, kind) cell does not apply."""
    v = SHAPES[shape](kind[0], s)
    if origin == "direct":
        return f"const {var} = {v};" if shape in DIRECT_OK[kind[0]] else None
    if origin == "any":
        return f"const {var}: any = {v};"
    if shape not in CORE and origin != "any":
        return None
    if origin == "box":
        return f"const {var}: any = [{v}].map((y: any) => y)[0];"
    if origin == "json":
        return f"const {var}: any = JSON.parse(JSON.stringify({v}));" if shape in JSONABLE else None
    if origin == "clone":
        return f"const {var}: any = structuredClone({v});" if shape in CLONABLE else None
    if origin == "eval":
        return f"const {var}: any = new Function(\"d\", {json.dumps('return ' + v)})(d);"
    raise AssertionError(origin)


def probes_for(modkey, specs, profile):
    """Yield (id, origin, line) for every cell of the matrix of one module key."""
    for mod, fid, tmpl, kinds in specs:
        if mod != modkey:
            continue
        slots = list(range(len(kinds))) or [None]
        for slot in slots:
            shapes = [None] if slot is None else list(SHAPES)
            for shape in shapes:
                for origin in ORIGINS:
                    if profile == "ci" and origin in ("box", "json", "clone", "eval") and shape not in ("url", "u8", "null"):
                        continue
                    decls, body = [], tmpl
                    if slot is None:
                        pid = f"{modkey}.{fid}.none.none.{origin}"
                        if origin != "direct":
                            continue
                    else:
                        for j, k in enumerate(kinds):
                            if j == slot:
                                continue
                            decls.append(f"const a{j}: any = {default_expr(k)};")
                        d0 = origin_decl(origin, shape, kinds[slot], payload(kinds[slot]), f"a{slot}")
                        if d0 is None:
                            continue
                        decls.insert(0, d0)
                        pid = f"{modkey}.{fid}.{slot}.{shape}.{origin}"
                    for j in range(len(kinds)):
                        body = body.replace(f"${j}", f"a{j}")
                    line = f'p("{pid}", async (d: string) => {{ {" ".join(decls)} return {body}; }});'
                    yield pid, origin, line


def generate(out_dir, modules, profile, per_program):
    specs = fs_specs() + fsp_specs() + misc_specs()
    os.makedirs(out_dir, exist_ok=True)
    programs, counts = [], {}
    for modkey in modules:
        by_origin = {}
        for pid, origin, line in probes_for(modkey, specs, profile):
            by_origin.setdefault(origin, []).append((pid, line))
        for origin in ORIGINS:
            rows = by_origin.get(origin, [])
            for n, start in enumerate(range(0, len(rows), per_program)):
                chunk = rows[start:start + per_program]
                name = f"bnd.{modkey}.{origin}.c{n:02d}"
                corpus = f"bnd.{modkey}.{origin}"
                programs.append({
                    "name": name, "corpus": corpus, "family": "bnd." + modkey, "mode": origin, "module": False,
                    "renderer": "boundary", "imports": MODULES[modkey],
                    "probes": [{"name": pid, "api": [f"boundary:{pid}"], "mode": origin, "line": line}
                               for pid, line in chunk],
                })
                counts[corpus] = counts.get(corpus, 0) + len(chunk)
    names = [p["name"] for pr in programs for p in pr["probes"]]
    assert len(names) == len(set(names)), "duplicate probe ids"
    for prog in programs:
        lines = [p["line"] for p in prog["probes"]]
        for suffix, node in ((".ts", False), (".mts", True)):
            with open(os.path.join(out_dir, prog["name"] + suffix), "w", encoding="utf-8") as fh:
                fh.write(render(lines, prog["imports"], node))
    manifest = {"per_program": per_program, "probes": len(names), "skipped": 0, "counts": counts, "findings": [],
                "programs": programs}
    with open(os.path.join(out_dir, "manifest.json"), "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=1, sort_keys=True)
        fh.write("\n")
    with open(os.path.join(out_dir, "skips.json"), "w", encoding="utf-8") as fh:
        json.dump({"skips": SKIPS}, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return manifest


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("out_dir")
    ap.add_argument("--modules", default=",".join(DEFAULT_MODULES), help="comma list of module keys: " + ",".join(MODULES))
    ap.add_argument("--profile", choices=("full", "ci"), default="full",
                    help="ci: the dynamic origins (box, json, clone, eval) only for url, u8 and null shapes")
    ap.add_argument("--per-program", type=int, default=PER_PROGRAM)
    args = ap.parse_args(argv)
    mods = [m for m in args.modules.split(",") if m]
    bad = [m for m in mods if m not in MODULES]
    if bad:
        print(f"gen_boundary: unknown modules {bad}", file=sys.stderr)
        return 2
    m = generate(args.out_dir, mods, args.profile, args.per_program)
    print(f"probes: {m['probes']} in {len(m['programs'])} programs")
    for k, n in sorted(m["counts"].items()):
        print(f"  {k:<24}{n:>6}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
