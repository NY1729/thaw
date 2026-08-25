//! V1 package registry: a plain local directory, one subdirectory per
//! package (see docs/design/registry.md):
//!
//! ```text
//! <registry-dir>/<package>/
//!   package.d.ts   (required)  -- fed to thaw-bridge's parse_dts/classify
//!   native.a       (optional)  -- prebuilt static lib providing the
//!                                  package's Fast path native symbols;
//!                                  auto-linked by thaw-cli (replaces a
//!                                  manual `--link`)
//!   native.node    (optional)  -- matching bundled N-API prebuild selected
//!                                  by `add`; loaded by thaw-napi
//!   native-addon.json (optional) -- source path, target tuple, and SHA-256
//!                                  for `native.node`
//!   bundle.js      (optional)  -- real JS implementation backing the
//!                                  package's Fallback functions; its
//!                                  source is fed to thaw-bridge's
//!                                  `generate_module_init` so it's loaded
//!                                  automatically at program startup
//!                                  (replaces a manual `loadScript` call)
//!   subpaths/<path>/package.d.ts -- declarations for an exact `exports`
//!                                  subpath such as `./feature`
//!   subpaths/<path>/bundle.js    -- independently bundled runtime entry
//!                                  for that subpath
//!   version.txt    (optional)  -- the exact version `add` resolved and
//!                                  fetched for the package itself (see
//!                                  `add`'s doc comment); purely
//!                                  informational
//!   lock.json      (optional)  -- every *other* real npm package folded
//!                                  into `bundle.js` (transitive same-
//!                                  registry-install dependencies), each
//!                                  mapped to the version `npm` actually
//!                                  resolved it to; written only when
//!                                  there's at least one (a single-file
//!                                  package with no dependencies has
//!                                  nothing to record here beyond
//!                                  `version.txt`'s own package). Purely
//!                                  informational, same as `version.txt`.
//! ```
//!
//! Still no source build step for native code, and no real dependency-graph
//! *resolution* (no semver range solving of our own -- `npm install`
//! already did that once, for one `add` call, and `lock.json` just
//! records what it picked) -- `add` resolves and records versions for
//! exactly the packages one `npm install <package>@<spec>` call actually
//! pulled in, independently each time `add` runs. This crate only
//! resolves a package name to the files already sitting on disk (plus,
//! now, the versions `add` recorded there). The native-lib build pipeline
//! is still exactly what the project's design doc calls "the actual
//! differentiator" left undone; this is a placeholder for the local half
//! of it, real enough to remove the remaining manual
//! `--bridge`/`--link`/`loadScript` steps for a package that's already
//! been fetched/built by some other means. `add` selects already-bundled
//! `.node` prebuilds and `prebuild-install`-style GitHub Release assets
//! described by the package manifest; it never runs package install scripts
//! or `node-gyp`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// A package resolved from a local registry directory. `native_lib`,
/// `native_addon`, and `bundle_js` are independent runtime backends.
#[derive(Debug)]
pub struct ResolvedPackage {
    pub name: String,
    pub dts_source: String,
    pub native_lib: Option<PathBuf>,
    /// A synchronous Node-API addon loaded through thaw-napi. Kept separate
    /// from `native_lib` because `.node` uses N-API handles, not the Fast-path
    /// C layout.
    pub native_addon: Option<PathBuf>,
    pub bundle_js: Option<String>,
    /// The version `add` recorded in `version.txt`, if this package went
    /// through `add` (rather than hand-curation, or an `add` run before
    /// this field existed) -- see `AddedPackage::resolved_version`.
    pub version: Option<String>,
    /// Every *other* real npm package `add` folded into `bundle.js`,
    /// mapped to its resolved version, read back from `lock.json` -- see
    /// `AddedPackage::dependency_versions`. `None` if there's no
    /// `lock.json` (a single-file package with no dependencies never gets
    /// one written; nor does a hand-curated or pre-`lock.json` package).
    pub dependency_versions: Option<BTreeMap<String, String>>,
}

/// Resolves `name` against `registry_dir/<name>/`. Fails only if
/// `package.d.ts` is missing or unreadable -- that's the one required
/// file, since a package with neither a native lib nor a bundle would
/// have nothing for thaw-bridge's shim to call.
pub fn resolve(registry_dir: &Path, name: &str) -> Result<ResolvedPackage, String> {
    let (package, subpath) = split_bare_spec(name);
    let mut dir = registry_dir.join(package);
    if let Some(subpath) = subpath {
        validate_export_subpath(subpath)?;
        dir = dir.join("subpaths").join(subpath);
    }

    let dts_path = dir.join("package.d.ts");
    let dts_source = fs::read_to_string(&dts_path).map_err(|e| {
        format!(
            "registry package `{name}`: failed to read `{}`: {e}",
            dts_path.display()
        )
    })?;

    let native_lib_path = dir.join("native.a");
    let native_lib = native_lib_path.is_file().then_some(native_lib_path);
    let native_addon_path = dir.join("native.node");
    let native_addon = native_addon_path.is_file().then_some(native_addon_path);

    let bundle_js_path = dir.join("bundle.js");
    let bundle_js = if bundle_js_path.is_file() {
        Some(fs::read_to_string(&bundle_js_path).map_err(|e| {
            format!(
                "registry package `{name}`: failed to read `{}`: {e}",
                bundle_js_path.display()
            )
        })?)
    } else {
        None
    };

    let version = fs::read_to_string(dir.join("version.txt"))
        .ok()
        .map(|s| s.trim().to_string());

    let dependency_versions = fs::read_to_string(dir.join("lock.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(&s).ok());

    Ok(ResolvedPackage {
        name: name.to_string(),
        dts_source,
        native_lib,
        native_addon,
        bundle_js,
        version,
        dependency_versions,
    })
}

/// Resolves the deliberately small built-in surface exposed to user imports.
/// The implementation is the same CommonJS polyfill already used while
/// bundling npm dependencies, paired with a Thaw-friendly declaration whose
/// fallback functions accept the existing JSON positional-argument array.
pub fn resolve_builtin(specifier: &str) -> Result<ResolvedPackage, String> {
    let name = specifier.strip_prefix("node:").unwrap_or(specifier);
    let dts_source = match name {
        "util" => {
            "export declare function inspect(argsArray: any): any;\nexport declare function format(argsArray: any): any;\nexport declare function formatWithOptions(argsArray: any): any;\nexport declare function inherits(argsArray: any): void;\nexport declare function promisify(argsArray: any): any;\nexport declare function callbackify(argsArray: any): any;\nexport declare function deprecate(argsArray: any): any;\nexport declare function stripVTControlCharacters(argsArray: any): any;\nexport declare function toUSVString(argsArray: any): any;\nexport declare function parseArgs(argsArray: any): any;\nexport declare const TextEncoder: any;\nexport declare const TextDecoder: any;\n"
        }
        "util/types" => {
            "export declare function isDate(argsArray: any): boolean;\nexport declare function isRegExp(argsArray: any): boolean;\nexport declare function isMap(argsArray: any): boolean;\nexport declare function isSet(argsArray: any): boolean;\nexport declare function isPromise(argsArray: any): boolean;\nexport declare function isArrayBuffer(argsArray: any): boolean;\nexport declare function isTypedArray(argsArray: any): boolean;\nexport declare function isNativeError(argsArray: any): boolean;\n"
        }
        "path" => {
            "export declare function resolve(argsArray: any): any;\nexport declare function join(argsArray: any): any;\nexport declare function dirname(argsArray: any): any;\nexport declare function basename(argsArray: any): any;\nexport declare function extname(argsArray: any): any;\nexport declare function normalize(argsArray: any): any;\nexport declare function relative(argsArray: any): any;\nexport declare function isAbsolute(argsArray: any): any;\nexport declare function parse(argsArray: any): any;\nexport declare function format(argsArray: any): any;\nexport declare function toNamespacedPath(argsArray: any): any;\n"
        }
        "process" => {
            "export declare function cwd(argsArray: any): any;\nexport declare function chdir(argsArray: any): void;\nexport declare function uptime(argsArray: any): any;\nexport declare function hrtime(argsArray: any): any;\nexport declare function memoryUsage(argsArray: any): any;\nexport declare function cpuUsage(argsArray: any): any;\nexport declare function emitWarning(argsArray: any): void;\n"
        }
        "child_process" => {
            "export declare const ChildProcess: any;\nexport declare function spawn(argsArray: any): any;\nexport declare function exec(argsArray: any): any;\nexport declare function execFile(argsArray: any): any;\nexport declare function spawnSync(argsArray: any): any;\nexport declare function execFileSync(argsArray: any): any;\nexport declare function execSync(argsArray: any): any;\n"
        }
        "punycode" => {
            "export declare function encode(argsArray: any): string;\nexport declare function decode(argsArray: any): string;\nexport declare function toASCII(argsArray: any): string;\nexport declare function toUnicode(argsArray: any): string;\n"
        }
        "buffer" => {
            "export declare const Buffer: any;\nexport declare const SlowBuffer: any;\nexport declare function byteLength(argsArray: any): any;\nexport declare function isUtf8(argsArray: any): any;\nexport declare function isAscii(argsArray: any): any;\nexport declare function transcode(argsArray: any): any;\n"
        }
        "string_decoder" => {
            "export declare function StringDecoder(argsArray: any): any;\n"
        }
        "timers" => {
            "export declare function setTimeout(argsArray: any): any;\nexport declare function clearTimeout(argsArray: any): void;\nexport declare function setInterval(argsArray: any): any;\nexport declare function clearInterval(argsArray: any): void;\nexport declare function setImmediate(argsArray: any): any;\nexport declare function clearImmediate(argsArray: any): void;\n"
        }
        "timers/promises" => {
            "export declare function setTimeout(argsArray: any): any;\nexport declare function setImmediate(argsArray: any): any;\nexport declare function setInterval(argsArray: any): any;\n"
        }
        "stream" => {
            "export declare function Stream(argsArray: any): any;\nexport declare function Readable(argsArray: any): any;\nexport declare function Writable(argsArray: any): any;\nexport declare function Duplex(argsArray: any): any;\nexport declare function Transform(argsArray: any): any;\nexport declare function PassThrough(argsArray: any): any;\nexport declare function pipeline(argsArray: any): any;\nexport declare function finished(argsArray: any): any;\nexport declare function addAbortSignal(argsArray: any): any;\n"
        }
        "stream/promises" => {
            "export declare function pipeline(argsArray: any): any;\nexport declare function finished(argsArray: any): any;\n"
        }
        "stream/consumers" => {
            "export declare function arrayBuffer(argsArray: any): any;\nexport declare function blob(argsArray: any): any;\nexport declare function buffer(argsArray: any): any;\nexport declare function json(argsArray: any): any;\nexport declare function text(argsArray: any): any;\n"
        }
        "stream/web" => {
            "export declare const ReadableStream: any;\nexport declare const WritableStream: any;\nexport declare const TransformStream: any;\nexport declare const TextEncoderStream: any;\nexport declare const TextDecoderStream: any;\nexport declare const CompressionStream: any;\nexport declare const DecompressionStream: any;\n"
        }
        "readline" => {
            "export declare function createInterface(argsArray: any): any;\nexport declare function Interface(argsArray: any): any;\nexport declare function clearLine(argsArray: any): boolean;\nexport declare function clearScreenDown(argsArray: any): boolean;\nexport declare function cursorTo(argsArray: any): boolean;\nexport declare function moveCursor(argsArray: any): boolean;\n"
        }
        "readline/promises" => {
            "export declare function createInterface(argsArray: any): any;\nexport declare function Interface(argsArray: any): any;\n"
        }
        "diagnostics_channel" => {
            "export declare function channel(argsArray: any): any;\nexport declare function hasSubscribers(argsArray: any): any;\nexport declare function subscribe(argsArray: any): void;\nexport declare function unsubscribe(argsArray: any): any;\nexport declare function tracingChannel(argsArray: any): any;\n"
        }
        "dns" => {
            "export declare function lookup(argsArray: any): void;\nexport declare function resolve(argsArray: any): void;\nexport declare function reverse(argsArray: any): void;\nexport declare function getDefaultResultOrder(argsArray: any): string;\nexport declare function setDefaultResultOrder(argsArray: any): void;\n"
        }
        "dns/promises" => {
            "export declare function lookup(argsArray: any): any;\nexport declare function resolve(argsArray: any): any;\nexport declare function reverse(argsArray: any): any;\n"
        }
        "dgram" => {
            "export declare function createSocket(argsArray: any): any;\nexport declare function Socket(argsArray: any): any;\n"
        }
        "async_hooks" => {
            "export declare function AsyncLocalStorage(argsArray: any): any;\nexport declare function AsyncResource(argsArray: any): any;\nexport declare function createHook(argsArray: any): any;\nexport declare function executionAsyncId(argsArray: any): any;\nexport declare function triggerAsyncId(argsArray: any): any;\nexport declare function executionAsyncResource(argsArray: any): any;\n"
        }
        "tty" => {
            "export declare function isatty(argsArray: any): any;\nexport declare function ReadStream(argsArray: any): any;\nexport declare function WriteStream(argsArray: any): any;\n"
        }
        "tls" => {
            "export declare function connect(argsArray: any): any;\nexport declare function TLSSocket(argsArray: any): any;\nexport declare function createSecureContext(argsArray: any): any;\nexport declare function checkServerIdentity(argsArray: any): any;\nexport declare function getCiphers(argsArray: any): any;\nexport declare function Server(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\n"
        }
        "module" => {
            "export declare function createRequire(argsArray: any): any;\nexport declare function isBuiltin(argsArray: any): any;\nexport declare function syncBuiltinESMExports(argsArray: any): void;\nexport declare function findSourceMap(argsArray: any): any;\nexport declare function SourceMap(argsArray: any): any;\nexport declare function register(argsArray: any): any;\nexport declare function registerHooks(argsArray: any): any;\n"
        }
        "net" => {
            "export declare function isIP(argsArray: any): number;\nexport declare function isIPv4(argsArray: any): boolean;\nexport declare function isIPv6(argsArray: any): boolean;\nexport declare function BlockList(argsArray: any): any;\nexport declare function SocketAddress(argsArray: any): any;\nexport declare function Socket(argsArray: any): any;\nexport declare function createConnection(argsArray: any): any;\nexport declare function connect(argsArray: any): any;\nexport declare function Server(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\n"
        }
        "console" => {
            "export declare function Console(argsArray: any): any;\nexport declare function log(argsArray: any): void;\nexport declare function info(argsArray: any): void;\nexport declare function warn(argsArray: any): void;\nexport declare function error(argsArray: any): void;\n"
        }
        "constants" => "export declare const F_OK: number;\nexport declare const R_OK: number;\nexport declare const W_OK: number;\nexport declare const X_OK: number;\nexport declare const O_RDONLY: number;\nexport declare const O_WRONLY: number;\nexport declare const O_RDWR: number;\n",
        "crypto" => {
            "export declare function createHash(argsArray: any): any;\nexport declare function createHmac(argsArray: any): any;\nexport declare function randomBytes(argsArray: any): any;\nexport declare function randomFill(argsArray: any): any;\nexport declare function randomFillSync(argsArray: any): any;\nexport declare function randomInt(argsArray: any): any;\nexport declare function randomUUID(argsArray: any): any;\nexport declare function timingSafeEqual(argsArray: any): any;\nexport declare function getHashes(argsArray: any): any;\n"
        }
        "perf_hooks" => {
            "export declare const performance: any;\nexport declare function monitorEventLoopDelay(argsArray: any): any;\nexport declare function createHistogram(argsArray: any): any;\n"
        }
        "v8" => {
            "export declare function serialize(argsArray: any): any;\nexport declare function deserialize(argsArray: any): any;\nexport declare function getHeapStatistics(argsArray: any): any;\nexport declare function getHeapSpaceStatistics(argsArray: any): any;\nexport declare function getHeapCodeStatistics(argsArray: any): any;\nexport declare function cachedDataVersionTag(argsArray: any): number;\nexport declare function setFlagsFromString(argsArray: any): void;\n"
        }
        "vm" => {
            "export declare function Script(argsArray: any): any;\nexport declare function createContext(argsArray: any): any;\nexport declare function isContext(argsArray: any): boolean;\nexport declare function runInContext(argsArray: any): any;\nexport declare function runInNewContext(argsArray: any): any;\nexport declare function runInThisContext(argsArray: any): any;\nexport declare function compileFunction(argsArray: any): any;\nexport declare function measureMemory(argsArray: any): any;\n"
        }
        "zlib" => {
            "export declare function gzipSync(argsArray: any): any;\nexport declare function gunzipSync(argsArray: any): any;\nexport declare function deflateSync(argsArray: any): any;\nexport declare function inflateSync(argsArray: any): any;\nexport declare function deflateRawSync(argsArray: any): any;\nexport declare function inflateRawSync(argsArray: any): any;\nexport declare function brotliCompressSync(argsArray: any): any;\nexport declare function brotliDecompressSync(argsArray: any): any;\nexport declare function gzip(argsArray: any): void;\nexport declare function gunzip(argsArray: any): void;\nexport declare function brotliCompress(argsArray: any): void;\nexport declare function brotliDecompress(argsArray: any): void;\nexport declare function createGzip(argsArray: any): any;\nexport declare function createGunzip(argsArray: any): any;\nexport declare function createDeflate(argsArray: any): any;\nexport declare function createInflate(argsArray: any): any;\nexport declare function createDeflateRaw(argsArray: any): any;\nexport declare function createInflateRaw(argsArray: any): any;\nexport declare function createBrotliCompress(argsArray: any): any;\nexport declare function createBrotliDecompress(argsArray: any): any;\n"
        }
        "worker_threads" => {
            "export declare const isMainThread: boolean;\nexport declare const threadId: number;\nexport declare const threadName: string;\nexport declare const workerData: any;\nexport declare const parentPort: any;\nexport declare const MessageChannel: any;\nexport declare const MessagePort: any;\nexport declare function Worker(argsArray: any): any;\nexport declare function receiveMessageOnPort(argsArray: any): any;\nexport declare function setEnvironmentData(argsArray: any): void;\nexport declare function getEnvironmentData(argsArray: any): any;\nexport declare function postMessageToThread(argsArray: any): Promise<void>;\n"
        }
        "os" => {
            "export declare function arch(argsArray: any): any;\nexport declare function platform(argsArray: any): any;\nexport declare function type(argsArray: any): any;\nexport declare function tmpdir(argsArray: any): any;\nexport declare function homedir(argsArray: any): any;\nexport declare function hostname(argsArray: any): any;\nexport declare function cpus(argsArray: any): any;\nexport declare function totalmem(argsArray: any): any;\nexport declare function freemem(argsArray: any): any;\nexport declare function uptime(argsArray: any): any;\nexport declare const EOL: string;\n"
        }
        "url" => {
            "export declare const URL: any;\nexport declare const URLSearchParams: any;\nexport declare function pathToFileURL(argsArray: any): any;\nexport declare function fileURLToPath(argsArray: any): any;\nexport declare function urlToHttpOptions(argsArray: any): any;\n"
        }
        "querystring" => {
            "export declare function stringify(argsArray: any): any;\nexport declare function encode(argsArray: any): any;\nexport declare function parse(argsArray: any): any;\nexport declare function decode(argsArray: any): any;\nexport declare function escape(argsArray: any): any;\nexport declare function unescape(argsArray: any): any;\n"
        }
        "events" => {
            "export declare function EventEmitter(argsArray: any): any;\nexport declare function once(argsArray: any): any;\nexport declare function on(argsArray: any): any;\nexport declare function getEventListeners(argsArray: any): any;\nexport declare function getMaxListeners(argsArray: any): number;\nexport declare function setMaxListeners(argsArray: any): void;\n"
        }
        "assert" | "assert/strict" => {
            "export declare function ok(argsArray: any): void;\nexport declare function equal(argsArray: any): void;\nexport declare function notEqual(argsArray: any): void;\nexport declare function strictEqual(argsArray: any): void;\nexport declare function notStrictEqual(argsArray: any): void;\nexport declare function deepEqual(argsArray: any): void;\nexport declare function notDeepEqual(argsArray: any): void;\nexport declare function deepStrictEqual(argsArray: any): void;\nexport declare function notDeepStrictEqual(argsArray: any): void;\nexport declare function fail(argsArray: any): void;\nexport declare function throws(argsArray: any): any;\nexport declare function doesNotThrow(argsArray: any): void;\n"
        }
        "fs" => {
            "export declare function existsSync(path: string): boolean;\nexport declare function readFileSync(path: string, encoding: string): string;\nexport declare function writeFileSync(path: string, data: string): boolean;\nexport declare function appendFileSync(path: string, data: string): any;\nexport declare function mkdirSync(path: string): boolean;\nexport declare function readdirSync(path: string): any;\nexport declare function statSync(path: string): any;\nexport declare function lstatSync(path: string): any;\nexport declare function unlinkSync(path: string): any;\nexport declare function rmSync(path: string): any;\nexport declare function rmdirSync(path: string): any;\nexport declare function renameSync(path: string, destination: string): any;\nexport declare function copyFileSync(path: string, destination: string): any;\nexport declare function cpSync(path: string, destination: string, options?: any): any;\nexport declare function realpathSync(path: string): any;\nexport declare function mkdtempSync(path: string): any;\nexport declare function mkdtempDisposableSync(path: string, options?: any): any;\nexport declare function openAsBlob(path: string, options?: any): Promise<any>;\nexport declare function linkSync(path: string, destination: string): any;\nexport declare function symlinkSync(path: string, destination: string): any;\nexport declare function readlinkSync(path: string): any;\nexport declare function chmodSync(path: string, mode: number): any;\nexport declare function createReadStream(argsArray: any): any;\nexport declare function createWriteStream(argsArray: any): any;\n"
        }
        "fs/promises" => {
            "export declare function access(argsArray: any): any;\nexport declare function open(argsArray: any): any;\nexport declare function readFile(argsArray: any): any;\nexport declare function readdir(argsArray: any): any;\nexport declare function stat(argsArray: any): any;\nexport declare function lstat(argsArray: any): any;\nexport declare function writeFile(argsArray: any): any;\nexport declare function appendFile(argsArray: any): any;\nexport declare function mkdir(argsArray: any): any;\nexport declare function unlink(argsArray: any): any;\nexport declare function rm(argsArray: any): any;\nexport declare function rmdir(argsArray: any): any;\nexport declare function rename(argsArray: any): any;\nexport declare function copyFile(argsArray: any): any;\nexport declare function realpath(argsArray: any): any;\nexport declare function mkdtemp(argsArray: any): any;\nexport declare function mkdtempDisposable(argsArray: any): any;\nexport declare function link(argsArray: any): any;\nexport declare function symlink(argsArray: any): any;\nexport declare function readlink(argsArray: any): any;\nexport declare function chmod(argsArray: any): any;\n"
        }
        "http" => {
            "export interface IncomingMessage { method: string; url: string; }\nexport interface ServerResponse { statusCode: number; setHeader: (name: string, value: string) => boolean; write: (chunk: string) => boolean; end: (chunk: string) => boolean; }\nexport interface Server { listen: (port: number) => string; __listenWithCallback: (port: number, callback: () => void) => string; listenMany: (port: number, count: number) => string; close: () => boolean; __closeWithCallback: (callback: () => void) => boolean; on: (event: string, callback: () => void) => boolean; __onError: (event: string, callback: (error: { message: string; code: string; syscall: string; address: string; port: number }) => void) => boolean; }\nexport declare function serveOnce(port: number, body: string): string;\nexport declare function serveOnceWith(port: number, callback: (target: string) => string): string;\nexport declare function createServerOnce(port: number, callback: (request: IncomingMessage, response: ServerResponse) => boolean): string;\nexport declare function createServer(callback: (request: IncomingMessage, response: ServerResponse) => boolean): Server;\n"
        }
        "https" => {
            "export declare function request(argsArray: any): any;\nexport declare function get(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\nexport declare const ClientRequest: any;\nexport declare const IncomingMessage: any;\nexport declare const ServerResponse: any;\nexport declare const Server: any;\nexport declare const globalAgent: any;\n"
        }
        _ => return Err(format!("unsupported Node built-in module `{specifier}`")),
    };
    let source = builtin_module_source(name)
        .ok_or_else(|| format!("unsupported Node built-in module `{specifier}`"))?;
    Ok(ResolvedPackage {
        name: format!("node:{name}"),
        dts_source: format!(
            "{dts_source}{}",
            match name {
                "fs" => "export declare function accessSync(path: string, mode?: number): void;\nexport declare function opendirSync(path: string, options?: any): any;\nexport declare function utimesSync(path: string, atime: any, mtime: any): any;\nexport declare function chownSync(path: string, uid: number, gid: number): any;\nexport declare function lchownSync(path: string, uid: number, gid: number): any;\nexport declare function watchFile(path: string, options: any, listener?: any): any;\nexport declare function unwatchFile(path: string, listener?: any): void;\nexport declare function watch(path: string, options?: any, listener?: any): any;\nexport declare function globSync(pattern: string | string[], options?: any): any[];\nexport declare function glob(pattern: string | string[], options: any, callback?: any): void;\nexport declare function openSync(path: string, flags: string, mode?: number): number;\nexport declare function closeSync(fd: number): void;\nexport declare function readSync(fd: number, buffer: any, offset: number, length: number, position: number | null): number;\nexport declare function writeSync(fd: number, data: any, offset?: any, length?: any, position?: any): number;\nexport declare function fstatSync(fd: number): any;\nexport declare function ftruncateSync(fd: number, length?: number): void;\nexport declare function readvSync(fd: number, buffers: any[], position?: number | null): number;\nexport declare function writevSync(fd: number, buffers: any[], position?: number | null): number;\nexport declare function statfsSync(path: string, options?: any): any;\nexport declare function lutimesSync(path: string, atime: any, mtime: any): void;\nexport declare function truncateSync(path: string, length?: number): void;\n",
                "fs/promises" => "export declare function opendir(argsArray: any): any;\nexport declare function cp(argsArray: any): any;\nexport declare function utimes(argsArray: any): any;\nexport declare function lutimes(argsArray: any): any;\nexport declare function chown(argsArray: any): any;\nexport declare function lchown(argsArray: any): any;\nexport declare function glob(pattern: string | string[], options?: any): AsyncIterable<any>;\nexport declare function watch(path: string, options?: any): AsyncIterable<any>;\nexport declare function statfs(path: string, options?: any): Promise<any>;\nexport declare function truncate(path: string, length?: number): Promise<void>;\n",
                _ => "",
            }
        ),
        native_lib: None,
        native_addon: None,
        bundle_js: Some(source.to_string()),
        version: None,
        dependency_versions: None,
    })
}

/// Where a freshly `add`ed package's declarations/JS entry came from,
/// inside the fetched package itself -- informational only, since the
/// scratch directory these were read from is deleted before `add`
/// returns. `bundled_file_count` is how many of the package's own files
/// (`main` plus everything it reaches via same-package relative
/// `require`s) got folded into `bundle.js` -- 1 for a package that's
/// just a single file.
#[derive(Debug, PartialEq)]
pub struct AddedPackage {
    pub dts_relative_path: String,
    pub js_relative_path: String,
    pub bundled_file_count: usize,
    /// The exact version `npm install` actually resolved `package`'s
    /// version-or-range specifier to (read back from the fetched
    /// package's own `package.json`, so a bare package name with no
    /// specifier at all still reports the real version `npm` picked as
    /// "latest", not just an echo of the empty request).
    pub resolved_version: String,
    /// Every real npm package folded into `bundle.js` -- `package` itself
    /// plus every transitive same-install dependency `bundle_commonjs_
    /// package`'s worklist actually walked into (Node builtin polyfills
    /// are not real npm packages and are never included here) -- each
    /// mapped to the version `npm install` resolved *it* to. Always
    /// contains at least `package`'s own entry. Written to `lock.json`
    /// only when it has more than that one entry (see this module's
    /// top-level doc comment).
    pub dependency_versions: BTreeMap<String, String>,
    /// Metadata for an automatically selected bundled `.node` prebuild.
    pub native_addon: Option<NativeAddonMetadata>,
    /// A non-fatal explanation when prebuilds existed but none matched the
    /// current target. The JavaScript fallback remains usable in that case.
    pub native_diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NativeAddonMetadata {
    pub source: String,
    pub sha256: String,
    pub platform: String,
    pub arch: String,
    pub libc: String,
}

#[derive(Debug)]
struct SelectedPrebuild {
    path: PathBuf,
    source: String,
    platform: String,
    arch: String,
    libc: String,
}

fn target_prebuild_components() -> (&'static str, &'static str, &'static str) {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if platform == "linux" {
        "glibc"
    } else {
        "system"
    };
    (platform, arch, libc)
}

fn select_prebuilt_addon(package_dir: &Path) -> Result<Option<SelectedPrebuild>, String> {
    let prebuilds = package_dir.join("prebuilds");
    if !prebuilds.is_dir() {
        return Ok(None);
    }
    let (platform, arch, libc) = target_prebuild_components();
    let target_dir = prebuilds.join(format!("{platform}-{arch}"));
    if !target_dir.is_dir() {
        let mut available = fs::read_dir(&prebuilds)
            .map_err(|error| format!("failed to inspect `{}`: {error}", prebuilds.display()))?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        available.sort();
        return Err(format!(
            "no bundled native addon matches {platform}-{arch}-{libc}; available targets: {}",
            if available.is_empty() {
                "none".into()
            } else {
                available.join(", ")
            }
        ));
    }
    let mut candidates = fs::read_dir(&target_dir)
        .map_err(|error| format!("failed to inspect `{}`: {error}", target_dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "node")
        })
        .filter(|path| {
            let musl = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".musl."));
            (libc == "musl") == musl || platform != "linux"
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let Some(path) = candidates.into_iter().next() else {
        return Err(format!(
            "bundled addons exist for {platform}-{arch}, but none match libc `{libc}`"
        ));
    };
    let relative_path = path
        .strip_prefix(package_dir)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();
    let source = fs::read_to_string(package_dir.join(".thaw-prebuild-source"))
        .ok()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .unwrap_or(relative_path);
    Ok(Some(SelectedPrebuild {
        path,
        source,
        platform: platform.into(),
        arch: arch.into(),
        libc: libc.into(),
    }))
}

fn select_optional_dependency_addon(
    node_modules_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<Option<SelectedPrebuild>, String> {
    let Some(optional) = manifest
        .get("optionalDependencies")
        .and_then(serde_json::Value::as_object)
    else {
        return Ok(None);
    };
    let (platform, arch, libc) = target_prebuild_components();
    let suffix = if platform == "linux" {
        format!("-{platform}-{arch}-{libc}")
    } else {
        format!("-{platform}-{arch}")
    };
    let mut names = optional
        .keys()
        .filter(|name| name.ends_with(&suffix))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        let dependency_dir = node_modules_dir.join(name);
        if !dependency_dir.is_dir() {
            continue;
        }
        let dependency_manifest = read_manifest(&dependency_dir)?;
        let main = dependency_manifest
            .get("main")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("index.js");
        let path = dependency_dir.join(main);
        if path
            .extension()
            .is_some_and(|extension| extension == "node")
            && path.is_file()
        {
            let relative_path = path
                .strip_prefix(node_modules_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            return Ok(Some(SelectedPrebuild {
                path,
                source: relative_path,
                platform: platform.into(),
                arch: arch.into(),
                libc: libc.into(),
            }));
        }
    }
    Ok(None)
}

fn github_repository(manifest: &serde_json::Value) -> Option<String> {
    let repository = manifest.get("repository")?;
    let raw = repository
        .as_str()
        .or_else(|| repository.get("url").and_then(serde_json::Value::as_str))?;
    let normalized = raw
        .strip_prefix("git+")
        .unwrap_or(raw)
        .strip_prefix("https://github.com/")?
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let mut parts = normalized.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if owner.is_empty() || repository.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn prebuild_install_asset(
    manifest: &serde_json::Value,
) -> Option<(String, String, String, String)> {
    let binary = manifest.get("binary")?;
    let napi = binary
        .get("napi_versions")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_u64)
        .filter(|version| *version <= 8)
        .max()?;
    let name = manifest.get("name")?.as_str()?;
    let version = manifest.get("version")?.as_str()?;
    let repository = github_repository(manifest)?;
    let (platform, arch, libc) = target_prebuild_components();
    let platform = if platform == "linux" && libc == "musl" {
        "linuxmusl"
    } else {
        platform
    };
    let asset = format!("{name}-v{version}-napi-v{napi}-{platform}-{arch}.tar.gz");
    let url = format!("https://github.com/{repository}/releases/download/v{version}/{asset}");
    Some((url, asset, platform.to_string(), arch.to_string()))
}

fn find_node_file(root: &Path) -> Result<Option<PathBuf>, String> {
    let mut directories = vec![root.to_path_buf()];
    let mut matches = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("failed to inspect `{}`: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to inspect an entry in `{}`: {error}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "node")
            {
                matches.push(path);
            }
        }
    }
    matches.sort();
    if matches.len() > 1 {
        return Err(format!(
            "downloaded prebuild contains multiple `.node` files: {}",
            matches
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(matches.pop())
}

fn download_prebuild_install_addon(
    package_dir: &Path,
    manifest: &serde_json::Value,
) -> Result<(), String> {
    if package_dir.join("prebuilds").is_dir() {
        return Ok(());
    }
    let Some((url, asset, _, _)) = prebuild_install_asset(manifest) else {
        return Ok(());
    };
    let mut response = ureq::get(&url)
        .call()
        .map_err(|error| format!("failed to download native prebuild `{url}`: {error}"))?;
    let archive_bytes = response
        .body_mut()
        .with_config()
        .limit(128 * 1024 * 1024)
        .read_to_vec()
        .map_err(|error| format!("failed to read native prebuild `{url}`: {error}"))?;
    let unpack_dir = package_dir.join(".thaw-prebuild");
    if unpack_dir.is_dir() {
        fs::remove_dir_all(&unpack_dir)
            .map_err(|error| format!("failed to clear `{}`: {error}", unpack_dir.display()))?;
    }
    fs::create_dir_all(&unpack_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", unpack_dir.display()))?;
    let decoder = flate2::read::GzDecoder::new(archive_bytes.as_slice());
    tar::Archive::new(decoder)
        .unpack(&unpack_dir)
        .map_err(|error| format!("failed to unpack native prebuild `{asset}`: {error}"))?;
    let addon = find_node_file(&unpack_dir)?
        .ok_or_else(|| format!("native prebuild `{asset}` contains no `.node` file"))?;
    let (platform, arch, _) = target_prebuild_components();
    let target = package_dir
        .join("prebuilds")
        .join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target)
        .map_err(|error| format!("failed to create `{}`: {error}", target.display()))?;
    let destination =
        target.join(addon.file_name().ok_or_else(|| {
            format!("native prebuild path `{}` has no filename", addon.display())
        })?);
    fs::copy(&addon, &destination).map_err(|error| {
        format!(
            "failed to copy downloaded native addon to `{}`: {error}",
            destination.display()
        )
    })?;
    fs::write(package_dir.join(".thaw-prebuild-source"), url)
        .map_err(|error| format!("failed to record downloaded native prebuild source: {error}"))?;
    Ok(())
}

/// Fetches `package` via `npm install` (into a throwaway scratch
/// directory -- `--ignore-scripts`, since this runs an arbitrary
/// third-party package's install unattended and its `postinstall` is not
/// something to execute automatically) and copies its declared type
/// definitions and CommonJS `main` entry point into `registry_dir/
/// <package>/` as `package.d.ts`/`bundle.js` -- the layout `resolve`
/// expects. This is the "automatic" half of the registry story (see
/// docs/design/registry.md): turns a bare package name into a usable
/// `--use <package>` entry, no hand-curation.
///
/// If `package` doesn't bundle its own type declarations (no `types`/
/// `typings` field in `package.json`, no same-named `index.d.ts`), this
/// falls back to fetching the corresponding DefinitelyTyped package
/// (`@types/<package>`, or `@types/<scope>__<name>` for a scoped
/// `@<scope>/<name>` package) and uses *its* declarations -- the JS
/// still always comes from `package` itself, since `@types/*` packages
/// carry no runtime code. Native addons never produce a `native.a`: bundled
/// `.node` files, platform optional dependencies, and GitHub-hosted
/// `prebuild-install` assets are selected independently of the JS fallback.
///
/// `package` may carry a version/tag/range specifier the same way `npm
/// install` accepts one (`left-pad@1.3.0`, `left-pad@^1.2.0`,
/// `left-pad@next`, or a bare `left-pad` for "whatever `npm` calls
/// latest") -- passed through to `npm install` completely unexamined
/// (`split_package_spec` only peels it off to know the bare package name
/// for the registry's own on-disk directory and for the `@types/*`
/// fallback name, which is versioned independently of whatever version
/// of `package` itself was requested). The version `npm` actually
/// resolved the specifier to is read back from the fetched package's own
/// `package.json` and recorded in `version.txt` next to `package.d.ts`/
/// `bundle.js` -- there's still no lockfile or cross-package version
/// *graph* (each `add` call is independent, exactly like one `npm
/// install <spec>` would be), but a specific version can now actually be
/// requested and later confirmed, rather than every `add` silently
/// meaning "whatever's newest today".
pub fn add(registry_dir: &Path, package: &str) -> Result<AddedPackage, String> {
    let scratch = std::env::temp_dir().join(format!(
        "thaw-registry-add-{}-{}",
        package.replace(['/', '@'], "_"),
        std::process::id()
    ));
    fs::create_dir_all(&scratch).map_err(|e| {
        format!(
            "failed to create scratch directory `{}`: {e}",
            scratch.display()
        )
    })?;

    let result = fetch_and_copy(&scratch, registry_dir, package);
    let _ = fs::remove_dir_all(&scratch);
    result
}

/// Splits an `add`/`npm install`-style package specifier into the bare
/// package name and an optional version/tag/range suffix. A scoped
/// package's leading `@scope/` is never mistaken for a version separator
/// -- only an `@` *after* that (or, for an unscoped name, anywhere at
/// all) starts a version: `"left-pad"` -> `("left-pad", None)`,
/// `"left-pad@1.3.0"` -> `("left-pad", Some("1.3.0"))`, `"@hapi/hoek"` ->
/// `("@hapi/hoek", None)`, `"@hapi/hoek@9.0.0"` -> `("@hapi/hoek",
/// Some("9.0.0"))`.
/// The bare-name half of [`split_package_spec`], exposed for callers
/// (thaw-cli's `registry add` reporting) that need to know which
/// registry directory a possibly-versioned `add` argument actually
/// landed in without duplicating the parsing themselves.
pub fn package_name(spec: &str) -> &str {
    split_package_spec(spec).0
}

fn split_package_spec(spec: &str) -> (&str, Option<&str>) {
    let search_from = if spec.starts_with('@') {
        spec.find('/').map(|i| i + 1).unwrap_or(spec.len())
    } else {
        0
    };
    match spec[search_from..].find('@') {
        Some(rel_i) => {
            let at = search_from + rel_i;
            (&spec[..at], Some(&spec[at + 1..]))
        }
        None => (spec, None),
    }
}

fn fetch_and_copy(
    scratch: &Path,
    registry_dir: &Path,
    package: &str,
) -> Result<AddedPackage, String> {
    let (name, _version_spec) = split_package_spec(package);

    npm_install(scratch, package)?;
    let node_modules_dir = scratch.join("node_modules");
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;
    download_prebuild_install_addon(&package_dir, &manifest)?;
    let fallback_dts = if find_own_dts(&manifest, &package_dir).is_none() {
        Some(fetch_types_package_dts(scratch, name)?)
    } else {
        None
    };
    add_installed_inner(registry_dir, &node_modules_dir, name, fallback_dts)
}

/// Registers a package that already exists under `node_modules_dir`.
/// This is the filesystem half of [`add`], exposed for offline/vendor
/// workflows that have already performed dependency installation.
pub fn add_installed(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
) -> Result<AddedPackage, String> {
    add_installed_inner(registry_dir, node_modules_dir, name, None)
}

fn add_installed_inner(
    registry_dir: &Path,
    node_modules_dir: &Path,
    name: &str,
    fallback_dts: Option<(String, String)>,
) -> Result<AddedPackage, String> {
    let package_dir = node_modules_dir.join(name);
    let manifest = read_manifest(&package_dir)?;

    let resolved_version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let main_field = package_export_target(&manifest, None, &["require", "import", "default"])
        .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
        .unwrap_or("index.js");
    let (js_source, js_relative_path, bundled_file_count, mut dependency_versions) =
        bundle_commonjs_package(node_modules_dir, name, &package_dir, main_field)?;

    let (dts_relative_path, dts_source) = match find_own_dts(&manifest, &package_dir) {
        Some((rel, abs)) => {
            let source =
                fs::read_to_string(&abs).map_err(|e| format!("failed to read `{rel}`: {e}"))?;
            (rel, source)
        }
        None => fallback_dts.ok_or_else(|| {
            format!(
                "`{name}` has no bundled type declarations; install its `@types` package or use `registry add`"
            )
        })?,
    };

    let dest_dir = registry_dir.join(name);
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("failed to create `{}`: {e}", dest_dir.display()))?;

    // Re-adding a package must never leave a stale binary selected for a
    // previous version/target.
    for stale in [
        dest_dir.join("native.node"),
        dest_dir.join("native-addon.json"),
    ] {
        if stale.is_file() {
            fs::remove_file(&stale).map_err(|error| {
                format!("failed to remove stale `{}`: {error}", stale.display())
            })?;
        }
    }
    let selected_addon = select_prebuilt_addon(&package_dir).and_then(|selected| match selected {
        Some(selected) => Ok(Some(selected)),
        None => select_optional_dependency_addon(node_modules_dir, &manifest),
    });
    let (native_addon, native_diagnostic) = match selected_addon {
        Ok(Some(selected)) => {
            let bytes = fs::read(&selected.path).map_err(|error| {
                format!(
                    "failed to read native addon `{}`: {error}",
                    selected.path.display()
                )
            })?;
            fs::write(dest_dir.join("native.node"), &bytes).map_err(|error| {
                format!(
                    "failed to write `{}`: {error}",
                    dest_dir.join("native.node").display()
                )
            })?;
            let metadata = NativeAddonMetadata {
                source: selected.source,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                platform: selected.platform,
                arch: selected.arch,
                libc: selected.libc,
            };
            let metadata_json = serde_json::to_string_pretty(&metadata)
                .map_err(|error| format!("failed to serialize native addon metadata: {error}"))?;
            fs::write(dest_dir.join("native-addon.json"), metadata_json).map_err(|error| {
                format!(
                    "failed to write `{}`: {error}",
                    dest_dir.join("native-addon.json").display()
                )
            })?;
            (Some(metadata), None)
        }
        Ok(None) => (None, None),
        Err(diagnostic) => (None, Some(diagnostic)),
    };
    fs::write(dest_dir.join("package.d.ts"), dts_source).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("package.d.ts").display()
        )
    })?;
    fs::write(dest_dir.join("bundle.js"), js_source).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("bundle.js").display()
        )
    })?;
    let subpaths_dir = dest_dir.join("subpaths");
    if subpaths_dir.is_dir() {
        fs::remove_dir_all(&subpaths_dir).map_err(|error| {
            format!(
                "failed to remove stale `{}`: {error}",
                subpaths_dir.display()
            )
        })?;
    }
    for export in package_subpath_exports(&manifest, &package_dir)? {
        let subpath = export.subpath;
        let (subpath_js, _, _, subpath_dependencies) =
            bundle_commonjs_package(node_modules_dir, name, &package_dir, &export.runtime_entry)?;
        dependency_versions.extend(subpath_dependencies);
        let types_path = package_dir.join(&export.types_entry);
        let subpath_dts = fs::read_to_string(&types_path).map_err(|error| {
            format!(
                "failed to read package export `./{subpath}` types `{}`: {error}",
                types_path.display()
            )
        })?;
        let subpath_dest = subpaths_dir.join(&subpath);
        fs::create_dir_all(&subpath_dest)
            .map_err(|error| format!("failed to create `{}`: {error}", subpath_dest.display()))?;
        fs::write(subpath_dest.join("package.d.ts"), subpath_dts).map_err(|error| {
            format!(
                "failed to write `{}`: {error}",
                subpath_dest.join("package.d.ts").display()
            )
        })?;
        fs::write(subpath_dest.join("bundle.js"), subpath_js).map_err(|error| {
            format!(
                "failed to write `{}`: {error}",
                subpath_dest.join("bundle.js").display()
            )
        })?;
    }
    fs::write(dest_dir.join("version.txt"), &resolved_version).map_err(|e| {
        format!(
            "failed to write `{}`: {e}",
            dest_dir.join("version.txt").display()
        )
    })?;
    // Only worth a `lock.json` at all once there's something beyond
    // `package`'s own entry (which `version.txt` already covers) --
    // a single-file package with no dependencies would otherwise get an
    // uninformative one-entry file next to `version.txt` saying the same
    // thing twice.
    if dependency_versions.len() > 1 {
        let lock_json = serde_json::to_string_pretty(&dependency_versions)
            .map_err(|e| format!("failed to serialize `lock.json` for `{name}`: {e}"))?;
        fs::write(dest_dir.join("lock.json"), lock_json).map_err(|e| {
            format!(
                "failed to write `{}`: {e}",
                dest_dir.join("lock.json").display()
            )
        })?;
    }

    Ok(AddedPackage {
        dts_relative_path,
        js_relative_path,
        bundled_file_count,
        resolved_version,
        dependency_versions,
        native_addon,
        native_diagnostic,
    })
}

fn npm_install(scratch: &Path, package: &str) -> Result<(), String> {
    let output = Command::new("npm")
        .arg("install")
        .arg("--prefix")
        .arg(scratch)
        .args(["--no-audit", "--no-fund", "--ignore-scripts", package])
        .output()
        .map_err(|e| format!("failed to invoke `npm` (is Node.js/npm installed?): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`npm install {package}` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn read_manifest(package_dir: &Path) -> Result<serde_json::Value, String> {
    let manifest_path = package_dir.join("package.json");
    let manifest_source = fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "failed to read `{}` after `npm install`: {e}",
            manifest_path.display()
        )
    })?;
    serde_json::from_str(&manifest_source)
        .map_err(|e| format!("`{}` is not valid JSON: {e}", manifest_path.display()))
}

fn package_export_target<'a>(
    manifest: &'a serde_json::Value,
    subpath: Option<&str>,
    conditions: &[&str],
) -> Option<&'a str> {
    let exports = manifest.get("exports")?;
    let target = match subpath {
        None => {
            if exports.is_string() {
                exports
            } else {
                exports
                    .as_object()
                    .and_then(|object| object.get("."))
                    .unwrap_or(exports)
            }
        }
        Some(subpath) => exports.as_object()?.get(&format!("./{subpath}"))?,
    };
    select_export_condition(target, conditions)
}

fn select_export_condition<'a>(
    value: &'a serde_json::Value,
    conditions: &[&str],
) -> Option<&'a str> {
    if let Some(path) = value.as_str() {
        return Some(path);
    }
    if let Some(candidates) = value.as_array() {
        return candidates
            .iter()
            .find_map(|candidate| select_export_condition(candidate, conditions));
    }
    let object = value.as_object()?;
    for condition in conditions {
        if let Some(path) = object
            .get(*condition)
            .and_then(|value| select_export_condition(value, conditions))
        {
            return Some(path);
        }
    }
    if conditions == ["types"] {
        for child in object.values() {
            if let Some(path) = select_export_condition(child, conditions) {
                return Some(path);
            }
        }
    }
    None
}

fn validate_export_subpath(subpath: &str) -> Result<(), String> {
    if subpath.is_empty()
        || Path::new(subpath).is_absolute()
        || subpath
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!("invalid package export subpath `{subpath}`"));
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
struct PackageSubpathExport {
    subpath: String,
    runtime_entry: String,
    types_entry: String,
}

fn collect_relative_files(root: &Path, dir: &Path, output: &mut Vec<String>) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|error| format!("failed to read `{}`: {error}", dir.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(root, &path, output)?;
        } else if path.is_file() {
            output.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn wildcard_capture<'a>(pattern: &str, path: &'a str) -> Option<&'a str> {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let (prefix, suffix) = pattern.split_once('*')?;
    if suffix.contains('*') || !path.starts_with(prefix) || !path.ends_with(suffix) {
        return None;
    }
    Some(&path[prefix.len()..path.len() - suffix.len()])
}

fn package_subpath_exports(
    manifest: &serde_json::Value,
    package_dir: &Path,
) -> Result<Vec<PackageSubpathExport>, String> {
    let Some(exports) = manifest.get("exports").and_then(|value| value.as_object()) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    collect_relative_files(package_dir, package_dir, &mut files)?;
    let mut subpaths = Vec::new();
    for (key, target) in exports {
        let Some(subpath) = key.strip_prefix("./") else {
            continue;
        };
        let Some(runtime) = select_export_condition(target, &["require", "import", "default"])
        else {
            continue;
        };
        let Some(types) = select_export_condition(target, &["types"]) else {
            continue;
        };
        if subpath.contains('*') {
            if subpath.matches('*').count() != 1
                || runtime.matches('*').count() < 1
                || types.matches('*').count() < 1
            {
                return Err(format!(
                    "package export pattern `{key}` must contain exactly one `*` in its key and at least one in its runtime and types targets"
                ));
            }
            for file in &files {
                let Some(capture) = wildcard_capture(types, file) else {
                    continue;
                };
                let expanded = subpath.replacen('*', capture, 1);
                validate_export_subpath(&expanded)?;
                subpaths.push(PackageSubpathExport {
                    subpath: expanded,
                    runtime_entry: runtime.replace('*', capture),
                    types_entry: types.replace('*', capture),
                });
            }
            continue;
        }
        validate_export_subpath(subpath)?;
        subpaths.push(PackageSubpathExport {
            subpath: subpath.to_string(),
            runtime_entry: runtime.to_string(),
            types_entry: types.to_string(),
        });
    }
    subpaths.sort_by(|left, right| left.subpath.cmp(&right.subpath));
    subpaths.dedup_by(|left, right| left.subpath == right.subpath);
    Ok(subpaths)
}

/// `package` has no bundled type declarations of its own -- fetch the
/// corresponding DefinitelyTyped package instead. `@types/*` packages
/// carry no runtime JS of their own (`main` is typically empty), just
/// declarations, so this only ever contributes the `.d.ts`; the actual
/// `bundle.js` still always comes from `package`.
fn fetch_types_package_dts(scratch: &Path, package: &str) -> Result<(String, String), String> {
    let types_package = types_package_name(package);
    npm_install(scratch, &types_package).map_err(|e| {
        format!("`{package}` has no bundled type declarations, and fetching `{types_package}` also failed: {e}")
    })?;

    let types_dir = scratch.join("node_modules").join(&types_package);
    let manifest = read_manifest(&types_dir).map_err(|e| {
        format!("`{package}` has no bundled type declarations, and reading `{types_package}`'s manifest failed: {e}")
    })?;
    let (rel, abs) = find_own_dts(&manifest, &types_dir).ok_or_else(|| {
        format!("`{package}` has no bundled type declarations, and `{types_package}` doesn't provide a usable one either")
    })?;
    let source = fs::read_to_string(&abs)
        .map_err(|e| format!("failed to read `{rel}` from `{types_package}`: {e}"))?;

    Ok((format!("{types_package}/{rel}"), source))
}

/// The DefinitelyTyped naming convention: `foo` -> `@types/foo`,
/// `@scope/name` -> `@types/scope__name` (the `/` becomes `__`, since a
/// types package itself is unscoped).
fn types_package_name(package: &str) -> String {
    match package
        .strip_prefix('@')
        .and_then(|rest| rest.split_once('/'))
    {
        Some((scope, name)) => format!("@types/{scope}__{name}"),
        None => format!("@types/{package}"),
    }
}

/// `types`/`typings` field first (in that order -- both spellings are
/// common in the wild), then a same-named `index.d.ts` next to `main` as
/// a last resort (common for older packages predating the `types` field
/// convention, e.g. left-pad/slugify). Returns both the path as recorded
/// (relative to `package_dir`) and the resolved absolute path to read.
fn find_own_dts(manifest: &serde_json::Value, package_dir: &Path) -> Option<(String, PathBuf)> {
    if let Some(path) = package_export_target(manifest, None, &["types"]) {
        let absolute = package_dir.join(path);
        if absolute.is_file() {
            return Some((path.to_string(), absolute));
        }
    }
    for field in ["types", "typings"] {
        if let Some(path) = manifest.get(field).and_then(|v| v.as_str()) {
            return Some((path.to_string(), package_dir.join(path)));
        }
    }
    let index = package_dir.join("index.d.ts");
    if index.is_file() {
        return Some(("index.d.ts".to_string(), index));
    }
    None
}

/// Approximates enough of Node's CommonJS resolution algorithm to read a
/// module path relative to `package_dir`: try the path exactly as
/// written, then with a `.js` extension appended, then as a directory
/// containing `index.js`. Real packages commonly write `"main": "./index"`
/// (no extension, resolved by Node at require-time) or `"main": "./lib"`
/// (a directory) -- reading the literal string as a path fails for both.
/// Also used, the same way, to resolve a same-package relative `require`
/// spec against the requiring file's directory (`bundle_commonjs_package`).
/// Doesn't attempt the rest of Node's real algorithm (`package.json`
/// `exports` maps, `.json`/`.node` candidates, etc.) -- just these two
/// common shapes.
fn resolve_module_path(package_dir: &Path, path: &str) -> Result<(String, PathBuf), String> {
    let trimmed = path.trim_end_matches('/');
    let directory = package_dir.join(trimmed);
    if directory.is_dir() {
        if let Ok(manifest) = read_manifest(&directory) {
            let entry =
                package_export_target(&manifest, None, &["require", "import", "node", "default"])
                    .or_else(|| manifest.get("main").and_then(|value| value.as_str()));
            if let Some(entry) = entry {
                if let Ok((relative, absolute)) = resolve_module_path(&directory, entry) {
                    return Ok((
                        normalize_path_string(&format!("{trimmed}/{relative}")),
                        absolute,
                    ));
                }
            }
        }
    }
    let candidates = [
        path.to_string(),
        format!("{trimmed}.js"),
        format!("{trimmed}.cjs"),
        // Real ESM packages commonly use an explicit `.mjs` extension
        // (sometimes alongside a separate `.cjs` build) rather than
        // relying on `package.json`'s `"type": "module"`.
        format!("{trimmed}.mjs"),
        format!("{trimmed}.json"),
        format!("{trimmed}/index.js"),
        format!("{trimmed}/index.cjs"),
        format!("{trimmed}/index.mjs"),
        format!("{trimmed}/index.json"),
    ];
    for candidate in &candidates {
        let resolved = package_dir.join(candidate);
        if resolved.is_file() {
            return Ok((candidate.clone(), resolved));
        }
    }
    Err(format!(
        "couldn't find a JS module for `\"{path}\"` (tried `{}`)",
        candidates.join("`, `")
    ))
}

/// One file pulled into a `bundle_commonjs_package` bundle. `key` is
/// `"<owning package name>/<path resolved by resolve_module_path,
/// relative to that package's own root>"` -- package-qualified so two
/// different packages' identically-named files (`index.js` is extremely
/// common) can't collide in the same bundle. Used both as the emitted
/// module map's key and, by stripping the package-name prefix and
/// resolving against that package's own directory, to read the file.
/// `requires` is every `require` spec this file's source contains that
/// was actually resolved (same-package relative, or a bare specifier
/// resolved to another bundled package), each mapped to its own target
/// `key`.
struct BundledModule {
    key: String,
    source: String,
    requires: Vec<(String, String)>,
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    async_module: bool,
}

fn declared_runtime_dependencies(package_dir: &Path) -> Vec<String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Vec::new();
    };
    let mut dependencies = Vec::new();
    for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
        let Some(entries) = manifest.get(field).and_then(serde_json::Value::as_object) else {
            continue;
        };
        for name in entries.keys() {
            if !dependencies.contains(name) {
                dependencies.push(name.clone());
            }
        }
    }
    dependencies
}

fn split_module_suffix(specifier: &str) -> (&str, &str) {
    specifier
        .char_indices()
        .find_map(|(index, character)| {
            (character == '?' || (character == '#' && index > 0)).then_some(index)
        })
        .map(|index| specifier.split_at(index))
        .unwrap_or((specifier, ""))
}

fn runtime_export_specifiers(
    package_name: &str,
    package_dir: &Path,
) -> Result<Vec<String>, String> {
    let Ok(source) = fs::read_to_string(package_dir.join("package.json")) else {
        return Ok(Vec::new());
    };
    let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&source) else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    let mut specifiers = Vec::new();
    let Some(exports_value) = manifest.get("exports") else {
        collect_relative_files(package_dir, package_dir, &mut files)?;
        for file in files.iter().filter(|file| {
            !file.split('/').any(|component| component == "node_modules")
                && matches!(
                    Path::new(file)
                        .extension()
                        .and_then(|extension| extension.to_str()),
                    Some("js" | "cjs" | "mjs" | "json")
                )
        }) {
            specifiers.push(format!("{package_name}/{file}"));
            if let Some(extensionless) = file
                .strip_suffix(".js")
                .or_else(|| file.strip_suffix(".cjs"))
                .or_else(|| file.strip_suffix(".mjs"))
            {
                specifiers.push(format!("{package_name}/{extensionless}"));
            }
            if let Some(directory) = file
                .strip_suffix("/index.js")
                .or_else(|| file.strip_suffix("/index.cjs"))
                .or_else(|| file.strip_suffix("/index.mjs"))
                .or_else(|| file.strip_suffix("/index.json"))
            {
                specifiers.push(format!("{package_name}/{directory}"));
            }
        }
        specifiers.sort();
        specifiers.dedup();
        return Ok(specifiers);
    };
    let Some(exports) = exports_value.as_object() else {
        return Ok(Vec::new());
    };
    for (key, target) in exports {
        let Some(subpath) = key.strip_prefix("./") else {
            continue;
        };
        let Some(runtime) = select_export_condition(target, &["require", "import", "default"])
        else {
            continue;
        };
        if subpath.contains('*') {
            if files.is_empty() {
                collect_relative_files(package_dir, package_dir, &mut files)?;
            }
            for file in &files {
                if let Some(capture) = wildcard_capture(runtime, file) {
                    let expanded = subpath.replacen('*', capture, 1);
                    if validate_export_subpath(&expanded).is_ok() {
                        specifiers.push(format!("{package_name}/{expanded}"));
                    }
                }
            }
        } else if validate_export_subpath(subpath).is_ok() {
            specifiers.push(format!("{package_name}/{subpath}"));
        }
    }
    specifiers.sort();
    specifiers.dedup();
    Ok(specifiers)
}

fn rewrite_static_worker_urls(
    source: &str,
    module_path: &Path,
    package_name: &str,
    package_dir: &Path,
) -> Result<String, String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        BindingIdent, Callee, Decl, Expr, ExprOrSpread, ImportDecl, ImportSpecifier, Lit,
        MemberProp, ModuleItem, NewExpr, Pat, Prop, PropName, PropOrSpread, Stmt, VarDeclKind,
        VarDeclarator,
    };
    use thaw_parser::common::Spanned;

    #[derive(Default)]
    struct WorkerBindings {
        constructors: BTreeSet<String>,
        namespaces: BTreeSet<String>,
        path_namespaces: BTreeSet<String>,
    }
    impl Visit for WorkerBindings {
        fn visit_import_decl(&mut self, declaration: &ImportDecl) {
            if !matches!(
                declaration.src.value.as_str(),
                Some("worker_threads" | "node:worker_threads")
            ) {
                return;
            }
            for specifier in &declaration.specifiers {
                match specifier {
                    ImportSpecifier::Named(named)
                        if named
                            .imported
                            .as_ref()
                            .map(|name| match name {
                                thaw_parser::ast::ModuleExportName::Ident(name) => {
                                    name.sym.as_ref()
                                }
                                thaw_parser::ast::ModuleExportName::Str(name) => {
                                    name.value.as_str().unwrap_or("")
                                }
                            })
                            .unwrap_or(named.local.sym.as_ref())
                            == "Worker" =>
                    {
                        self.constructors.insert(named.local.sym.to_string());
                    }
                    ImportSpecifier::Namespace(namespace) => {
                        self.namespaces.insert(namespace.local.sym.to_string());
                    }
                    _ => {}
                }
            }
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            let Some(initializer) = declaration.init.as_deref() else {
                return;
            };
            let (call, property) = match initializer {
                Expr::Call(call) => (call, None),
                Expr::Member(member) => {
                    let Expr::Call(call) = member.obj.as_ref() else {
                        return;
                    };
                    let property = match &member.prop {
                        MemberProp::Ident(property) => Some(property.sym.as_ref()),
                        _ => None,
                    };
                    (call, property)
                }
                _ => return,
            };
            let required_module = match call.args.first().map(|argument| argument.expr.as_ref()) {
                Some(Expr::Lit(Lit::Str(value))) => value.value.as_str(),
                _ => None,
            };
            let is_require = matches!(
                &call.callee,
                Callee::Expr(callee) if matches!(callee.as_ref(), Expr::Ident(name) if name.sym == "require")
            );
            if is_require
                && matches!(required_module, Some("path" | "node:path"))
                && property.is_none()
            {
                if let Pat::Ident(binding) = &declaration.name {
                    self.path_namespaces.insert(binding.id.sym.to_string());
                }
                return;
            }
            let is_worker_threads = is_require
                && matches!(
                    required_module,
                    Some("worker_threads" | "node:worker_threads")
                );
            if !is_worker_threads {
                return;
            }
            let Pat::Ident(binding) = &declaration.name else {
                return;
            };
            if property == Some("Worker") {
                self.constructors.insert(binding.id.sym.to_string());
            } else if property.is_none() {
                self.namespaces.insert(binding.id.sym.to_string());
            }
        }
    }

    struct WorkerUrlSpan {
        lo: u32,
        hi: u32,
        import_meta_base: Option<(u32, u32)>,
        relative: String,
    }

    fn static_file_options(arguments: &[ExprOrSpread]) -> bool {
        let Some(options) = arguments.get(1) else {
            return true;
        };
        if matches!(options.expr.as_ref(), Expr::Ident(identifier) if identifier.sym == "undefined")
        {
            return true;
        }
        let Expr::Object(options) = options.expr.as_ref() else {
            return false;
        };
        for property in &options.props {
            let PropOrSpread::Prop(property) = property else {
                return false;
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                continue;
            };
            let is_eval = matches!(&property.key, PropName::Ident(name) if name.sym == "eval")
                || matches!(&property.key, PropName::Str(name) if name.value.as_str() == Some("eval"));
            if is_eval {
                return matches!(property.value.as_ref(), Expr::Lit(Lit::Bool(value)) if !value.value);
            }
        }
        true
    }

    fn static_worker_path(
        expression: &Expr,
        constants: &BTreeMap<String, String>,
        path_namespaces: &BTreeSet<String>,
    ) -> Option<String> {
        match expression {
            Expr::Lit(Lit::Str(path)) => Some(path.value.to_string_lossy().into_owned()),
            Expr::Ident(identifier) => constants.get(identifier.sym.as_ref()).cloned(),
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut path = String::new();
                for (index, quasi) in template.quasis.iter().enumerate() {
                    path.push_str(
                        &quasi
                            .cooked
                            .as_ref()
                            .map(|value| value.to_string_lossy().into_owned())
                            .unwrap_or_else(|| quasi.raw.to_string()),
                    );
                    if let Some(expression) = template.exprs.get(index) {
                        path.push_str(&static_worker_path(expression, constants, path_namespaces)?);
                    }
                }
                Some(path)
            }
            Expr::Paren(parenthesized) => {
                static_worker_path(&parenthesized.expr, constants, path_namespaces)
            }
            Expr::Bin(binary) if binary.op == thaw_parser::ast::BinaryOp::Add => Some(format!(
                "{}{}",
                static_worker_path(&binary.left, constants, path_namespaces)?,
                static_worker_path(&binary.right, constants, path_namespaces)?
            )),
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return None;
                };
                let Expr::Member(member) = callee.as_ref() else {
                    return None;
                };
                let Expr::Ident(namespace) = member.obj.as_ref() else {
                    return None;
                };
                let MemberProp::Ident(operation) = &member.prop else {
                    return None;
                };
                if !path_namespaces.contains(namespace.sym.as_ref())
                    || !matches!(operation.sym.as_ref(), "join" | "resolve")
                {
                    return None;
                }
                let mut parts = Vec::new();
                for (index, argument) in call.args.iter().enumerate() {
                    if index == 0
                        && matches!(argument.expr.as_ref(), Expr::Ident(name) if name.sym == "__dirname")
                    {
                        continue;
                    }
                    parts.push(static_worker_path(
                        &argument.expr,
                        constants,
                        path_namespaces,
                    )?);
                }
                let normalized = normalize_path_string(&parts.join("/"));
                (!normalized.is_empty()).then(|| format!("./{normalized}"))
            }
            _ => None,
        }
    }

    fn top_level_worker_path_constants(
        module: &thaw_parser::ast::Module,
        binding_counts: &BTreeMap<String, usize>,
        path_namespaces: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut constants = BTreeMap::new();
        for item in &module.body {
            let ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) = item else {
                continue;
            };
            if declaration.kind != VarDeclKind::Const {
                continue;
            }
            for declarator in &declaration.decls {
                let (Pat::Ident(binding), Some(initializer)) =
                    (&declarator.name, declarator.init.as_deref())
                else {
                    continue;
                };
                if binding_counts.get(binding.id.sym.as_ref()) != Some(&1) {
                    continue;
                }
                if let Some(path) = static_worker_path(initializer, &constants, path_namespaces) {
                    constants.insert(binding.id.sym.to_string(), path);
                }
            }
        }
        constants
    }

    #[derive(Default)]
    struct BindingCounts(BTreeMap<String, usize>);
    impl Visit for BindingCounts {
        fn visit_binding_ident(&mut self, binding: &BindingIdent) {
            *self.0.entry(binding.id.sym.to_string()).or_default() += 1;
        }
    }

    struct WorkerUrls {
        constructors: BTreeSet<String>,
        namespaces: BTreeSet<String>,
        path_constants: BTreeMap<String, String>,
        path_namespaces: BTreeSet<String>,
        spans: Vec<WorkerUrlSpan>,
    }
    impl Visit for WorkerUrls {
        fn visit_new_expr(&mut self, expression: &NewExpr) {
            let is_worker = match expression.callee.as_ref() {
                Expr::Ident(callee) => self.constructors.contains(callee.sym.as_ref()),
                Expr::Member(member) => {
                    matches!(member.obj.as_ref(), Expr::Ident(namespace) if self.namespaces.contains(namespace.sym.as_ref()))
                        && matches!(&member.prop, MemberProp::Ident(property) if property.sym == "Worker")
                }
                _ => false,
            };
            if !is_worker {
                expression.visit_children_with(self);
                return;
            }
            let Some(arguments) = expression.args.as_ref() else {
                return;
            };
            let Some(first) = arguments.first() else {
                return;
            };
            if let Some(path) =
                static_worker_path(&first.expr, &self.path_constants, &self.path_namespaces)
            {
                if !static_file_options(arguments) {
                    expression.visit_children_with(self);
                    return;
                }
                let span = first.expr.span();
                self.spans.push(WorkerUrlSpan {
                    lo: span.lo.0,
                    hi: span.hi.0,
                    import_meta_base: None,
                    relative: path,
                });
                expression.visit_children_with(self);
                return;
            }
            let Expr::New(url) = first.expr.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let Expr::Ident(url_callee) = url.callee.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let Some(url_arguments) = url.args.as_ref() else {
                return;
            };
            if url_callee.sym != "URL" || url_arguments.len() != 2 {
                expression.visit_children_with(self);
                return;
            }
            let Expr::Lit(Lit::Str(path)) = url_arguments[0].expr.as_ref() else {
                expression.visit_children_with(self);
                return;
            };
            let span = first.expr.span();
            let base_span = url_arguments[1].expr.span();
            self.spans.push(WorkerUrlSpan {
                lo: span.lo.0,
                hi: span.hi.0,
                import_meta_base: Some((base_span.lo.0, base_span.hi.0)),
                relative: path.value.to_string_lossy().into_owned(),
            });
            expression.visit_children_with(self);
        }
    }

    let Ok((module, source_map)) = thaw_parser::parse_javascript_with_source_map(source) else {
        return Ok(source.to_string());
    };
    let mut bindings = WorkerBindings::default();
    module.visit_with(&mut bindings);
    let mut binding_counts = BindingCounts::default();
    module.visit_with(&mut binding_counts);
    let path_constants =
        top_level_worker_path_constants(&module, &binding_counts.0, &bindings.path_namespaces);
    let mut workers = WorkerUrls {
        constructors: bindings.constructors,
        namespaces: bindings.namespaces,
        path_constants,
        path_namespaces: bindings.path_namespaces,
        spans: Vec::new(),
    };
    module.visit_with(&mut workers);
    if workers.spans.is_empty() {
        return Ok(source.to_string());
    }
    workers.spans.sort_by_key(|span| span.lo);
    let directory = module_path.parent().unwrap_or(Path::new(""));
    let mut output = String::with_capacity(source.len());
    let mut worker_requires = Vec::new();
    let mut cursor = 0usize;
    for worker in workers.spans {
        let WorkerUrlSpan {
            lo,
            hi,
            import_meta_base,
            relative,
        } = worker;
        if !(relative.starts_with("./") || relative.starts_with("../")) {
            continue;
        }
        if let Some((base_lo, base_hi)) = import_meta_base {
            let base_lo = source_map
                .lookup_byte_offset(thaw_parser::common::BytePos(base_lo))
                .pos
                .0 as usize;
            let base_hi = source_map
                .lookup_byte_offset(thaw_parser::common::BytePos(base_hi))
                .pos
                .0 as usize;
            if source[base_lo..base_hi].trim() != "import.meta.url" {
                continue;
            }
        }
        let worker_path = directory.join(&relative);
        let worker_source = fs::read_to_string(&worker_path).map_err(|error| {
            format!(
                "failed to read Worker source `{}` referenced by `{}`: {error}",
                worker_path.display(),
                module_path.display()
            )
        })?;
        let worker_relative = worker_path.strip_prefix(package_dir).map_err(|_| {
            format!(
                "Worker source `{}` is outside package `{}`",
                worker_path.display(),
                package_dir.display()
            )
        })?;
        let worker_relative = normalize_path_string(&worker_relative.to_string_lossy());
        let worker_key = format!("{package_name}/{worker_relative}");
        let worker_bootstrap = format!(
            "var __thaw_worker_require = globalThis.__thaw_bundle_create_require({});\nvar require = function(name) {{ return name === 'worker_threads' || name === 'node:worker_threads' ? globalThis.__thaw_worker_module : __thaw_worker_require(name); }};\n",
            js_string_literal(&worker_key)
        );
        let worker_source =
            rewrite_esm_to_commonjs_mode(&worker_source, false).unwrap_or(worker_source);
        let encoded = worker_bootstrap
            .bytes()
            .chain(worker_source.bytes())
            .map(|byte| format!("%{byte:02X}"))
            .collect::<String>();
        let replacement = serde_json::to_string(&format!("data:text/javascript,{encoded}"))
            .expect("Worker data URL is serializable");
        let lo = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = source_map
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        if lo < cursor {
            continue;
        }
        output.push_str(&source[cursor..lo]);
        output.push_str(&replacement);
        cursor = hi;
        if !worker_requires.contains(&relative) {
            worker_requires.push(relative);
        }
    }
    output.push_str(&source[cursor..]);
    for relative in worker_requires {
        output.push_str(&format!(
            "\nif (false) require({});",
            js_string_literal(&relative)
        ));
    }
    Ok(output)
}

/// Bundles `root_package`'s own CommonJS module graph -- starting from
/// `main_relative` (its `main` field, or a default) -- into a single
/// self-contained JS string with a small embedded module-system
/// emulation, so `thaw-bridge`'s `wrap_as_commonjs_module` (which only
/// ever sees one JS string per package) can still run it correctly.
/// Recurses across *and within* package boundaries: both same-package
/// relative `require`s and `require`s of another real npm package
/// (found under `node_modules_dir`, e.g. a `dependencies` entry) get
/// bundled in, transitively -- `npm install` already fetched the whole
/// dependency tree into a flat `node_modules_dir` before this runs
/// (confirmed by inspecting a real install: `qs`'s own dependency,
/// `side-channel`, and *its* transitive dependencies all landed at the
/// top level), so no second network round-trip is needed here, only
/// local filesystem lookups.
///
/// Found necessary by running two real npm packages through `add`: `qs`
/// (whose `main`, `lib/index.js`, does `require('./stringify')` etc --
/// same-package relative requires, nothing to do with an external
/// dependency) and then, once that worked, `qs`'s own real dependency on
/// `side-channel` (a genuine external package) which the previous
/// implementation left for `wrap_as_commonjs_module`'s global `require`
/// stub to reject outright, exactly like a truly unresolvable dependency
/// would. Before either fix, *any* package split across more than one
/// file -- or depending on another package at all -- failed at load
/// time, which covers most real npm packages beyond a trivial
/// single-file utility.
///
/// A bare specifier's package (or, for a "deep import" spec like
/// `es-errors/type`, the package the subpath is resolved against --
/// `resolve_bare_require`/`split_bare_spec`) that isn't found under
/// `node_modules_dir` at all -- a Node core builtin like `fs`, or a
/// dependency that genuinely wasn't installed -- is left alone, same
/// as an unresolvable relative require -- it falls through to whatever
/// `require` is in scope at runtime, i.e. thaw-bridge's global stub that
/// throws a clear "not supported" error, rather than aborting the whole
/// bundle.
///
/// Returns the bundle text, the resolved key of `main_relative` (the
/// bundle's entry point, for `AddedPackage` reporting), the total number
/// of files folded in (across every package the bundle reaches), and
/// every real npm package the walk touched (`root_package` plus every
/// bare-specifier dependency actually resolved under `node_modules_dir`,
/// Node builtin polyfills excluded) mapped to its own resolved version --
/// see `AddedPackage::dependency_versions`.
fn add_builtin_module(
    name: &str,
    key: String,
    visited: &mut Vec<String>,
    modules: &mut Vec<BundledModule>,
) {
    if visited.contains(&key) {
        return;
    }
    let Some(source) = builtin_module_source(name) else {
        return;
    };
    visited.push(key.clone());
    let mut requires = Vec::new();
    for specifier in analyze_module(source).specs {
        let (resolution, suffix) = split_module_suffix(&specifier);
        let dependency_name = resolution
            .strip_prefix("node:")
            .unwrap_or(resolution)
            .to_string();
        if builtin_module_source(&dependency_name).is_none() {
            continue;
        }
        let dependency_key = format!("node:{dependency_name}{suffix}");
        requires.push((specifier, dependency_key.clone()));
        add_builtin_module(&dependency_name, dependency_key, visited, modules);
    }
    modules.push(BundledModule {
        key,
        source: source.to_string(),
        requires,
        static_esm_specs: Vec::new(),
        has_esm: false,
        has_top_level_await: false,
        async_module: false,
    });
}

fn bundle_commonjs_package(
    node_modules_dir: &Path,
    root_package: &str,
    root_package_dir: &Path,
    main_relative: &str,
) -> Result<(String, String, usize, BTreeMap<String, String>), String> {
    let (main_relative_key, main_abs) = resolve_module_path(root_package_dir, main_relative)?;
    let main_key = format!("{root_package}/{main_relative_key}");

    let mut modules: Vec<BundledModule> = Vec::new();
    let mut visited: Vec<String> = vec![main_key.clone()];
    let mut worklist: Vec<(String, PathBuf, String, PathBuf)> = vec![(
        main_key.clone(),
        main_abs,
        root_package.to_string(),
        root_package_dir.to_path_buf(),
    )];
    let mut dependency_versions: BTreeMap<String, String> = BTreeMap::new();
    record_package_version(&mut dependency_versions, root_package, root_package_dir);

    while let Some((key, abs_path, pkg_name, pkg_dir)) = worklist.pop() {
        let source = fs::read_to_string(&abs_path)
            .map_err(|e| format!("failed to read `{key}` while bundling: {e}"))?;
        let source = if abs_path.extension().is_some_and(|ext| ext == "json") {
            let value: serde_json::Value = serde_json::from_str(&source)
                .map_err(|error| format!("invalid JSON module `{key}`: {error}"))?;
            format!("module.exports = {};", value)
        } else {
            source
        };
        let source = rewrite_static_worker_urls(&source, &abs_path, &pkg_name, &pkg_dir)?;
        let analysis = analyze_module(&source);
        if let Some(error) = &analysis.attribute_error {
            return Err(format!("invalid import attributes in `{key}`: {error}"));
        }
        let module_specs = analysis.specs;

        let relative_in_pkg = key
            .strip_prefix(&format!("{pkg_name}/"))
            .unwrap_or(key.as_str());
        let requiring_dir = Path::new(relative_in_pkg).parent().unwrap_or(Path::new(""));

        let mut requires = Vec::new();

        if analysis.has_nonliteral_dynamic_import {
            let mut candidates = Vec::new();
            collect_relative_files(&pkg_dir, &pkg_dir, &mut candidates)?;
            for relative in candidates.into_iter().filter(|path| {
                !path.split('/').any(|component| component == "node_modules")
                    && matches!(
                        Path::new(path)
                            .extension()
                            .and_then(|extension| extension.to_str()),
                        Some("js" | "cjs" | "mjs" | "json")
                    )
            }) {
                let target_key = format!("{pkg_name}/{relative}");
                if target_key == key {
                    continue;
                }
                let specifier = relative_module_specifier(requiring_dir, Path::new(&relative));
                if !requires.iter().any(|(source, _)| source == &specifier) {
                    requires.push((specifier.clone(), target_key.clone()));
                }
                if let Some(extensionless) = specifier
                    .strip_suffix(".js")
                    .or_else(|| specifier.strip_suffix(".mjs"))
                    .or_else(|| specifier.strip_suffix(".cjs"))
                {
                    if !requires.iter().any(|(source, _)| source == extensionless) {
                        requires.push((extensionless.to_string(), target_key.clone()));
                    }
                }
                if !visited.contains(&target_key) {
                    visited.push(target_key.clone());
                    worklist.push((
                        target_key,
                        pkg_dir.join(&relative),
                        pkg_name.clone(),
                        pkg_dir.clone(),
                    ));
                }
            }
            for specifier in declared_runtime_dependencies(&pkg_dir) {
                if requires.iter().any(|(source, _)| source == &specifier) {
                    continue;
                }
                if let Some((dep_name, dep_relative, dep_abs, dep_dir)) =
                    resolve_bare_require(node_modules_dir, &specifier)
                {
                    let dep_key = format!("{dep_name}/{dep_relative}");
                    requires.push((specifier.clone(), dep_key.clone()));
                    if !visited.contains(&dep_key) {
                        visited.push(dep_key.clone());
                        record_package_version(&mut dependency_versions, &dep_name, &dep_dir);
                        worklist.push((dep_key, dep_abs, dep_name.clone(), dep_dir.clone()));
                    }
                    for subpath in runtime_export_specifiers(&specifier, &dep_dir)? {
                        if requires.iter().any(|(source, _)| source == &subpath) {
                            continue;
                        }
                        if let Some((sub_name, relative, absolute, directory)) =
                            resolve_bare_require(node_modules_dir, &subpath)
                        {
                            let target = format!("{sub_name}/{relative}");
                            requires.push((subpath, target.clone()));
                            if !visited.contains(&target) {
                                visited.push(target.clone());
                                record_package_version(
                                    &mut dependency_versions,
                                    &sub_name,
                                    &directory,
                                );
                                worklist.push((target, absolute, sub_name, directory));
                            }
                        }
                    }
                }
            }
        }

        for spec in module_specs
            .iter()
            .filter(|spec| spec.starts_with("./") || spec.starts_with("../"))
            .cloned()
        {
            let (resolution_spec, suffix) = split_module_suffix(&spec);
            let combined = if requiring_dir.as_os_str().is_empty() {
                resolution_spec.to_string()
            } else {
                format!("{}/{resolution_spec}", requiring_dir.display())
            };
            let normalized = normalize_path_string(&combined);
            // An unresolvable relative require (e.g. it targets a
            // `.json` file, which `resolve_module_path`'s candidates
            // don't cover) is left out of the map on purpose -- that one
            // call falls through to the external-require stub at
            // runtime instead of aborting the whole bundle.
            if let Ok((resolved_relative, resolved_abs)) =
                resolve_module_path(&pkg_dir, &normalized)
            {
                let resolved_key = format!("{pkg_name}/{resolved_relative}{suffix}");
                requires.push((spec, resolved_key.clone()));
                if !visited.contains(&resolved_key) {
                    visited.push(resolved_key.clone());
                    worklist.push((
                        resolved_key,
                        resolved_abs,
                        pkg_name.clone(),
                        pkg_dir.clone(),
                    ));
                }
            }
        }

        for spec in module_specs
            .iter()
            .filter(|spec| !(spec.starts_with("./") || spec.starts_with("../")))
            .cloned()
        {
            let (resolution_spec, suffix) = split_module_suffix(&spec);
            if resolution_spec.starts_with('#') {
                if let Some((resolved_relative, resolved_abs)) =
                    resolve_package_import(&pkg_dir, resolution_spec)
                {
                    let resolved_key = format!("{pkg_name}/{resolved_relative}{suffix}");
                    requires.push((spec, resolved_key.clone()));
                    if !visited.contains(&resolved_key) {
                        visited.push(resolved_key.clone());
                        worklist.push((
                            resolved_key,
                            resolved_abs,
                            pkg_name.clone(),
                            pkg_dir.clone(),
                        ));
                    }
                }
                continue;
            }
            if let Some((dep_name, dep_relative, dep_abs, dep_dir)) =
                resolve_bare_require(node_modules_dir, resolution_spec)
            {
                let dep_key = format!("{dep_name}/{dep_relative}{suffix}");
                requires.push((spec, dep_key.clone()));
                if !visited.contains(&dep_key) {
                    visited.push(dep_key.clone());
                    record_package_version(&mut dependency_versions, &dep_name, &dep_dir);
                    worklist.push((dep_key, dep_abs, dep_name, dep_dir));
                }
                continue;
            }
            // Not a real npm package under `node_modules_dir` -- maybe a
            // Node core builtin Thaw has a polyfill for.
            let builtin_name = resolution_spec
                .strip_prefix("node:")
                .unwrap_or(resolution_spec);
            if builtin_module_source(builtin_name).is_some() {
                let builtin_key = format!("node:{builtin_name}{suffix}");
                requires.push((spec.clone(), builtin_key.clone()));
                if !visited.contains(&builtin_key) {
                    add_builtin_module(builtin_name, builtin_key, &mut visited, &mut modules);
                }
            }
            // Otherwise left unresolved -- falls through to the runtime
            // external-require stub, same as always.
        }

        modules.push(BundledModule {
            key,
            source,
            requires,
            static_esm_specs: analysis.static_esm_specs,
            has_esm: analysis.has_esm,
            has_top_level_await: analysis.has_top_level_await,
            async_module: false,
        });
    }

    prepare_async_modules(&mut modules)?;

    let file_count = modules.len();
    Ok((
        render_bundle(&main_key, &modules),
        main_key,
        file_count,
        dependency_versions,
    ))
}

/// Records `name`'s resolved version (from its own real `package.json`)
/// into `versions`, if it has one -- best-effort: a package that somehow
/// lacks a readable `package.json`/`version` field (shouldn't happen for
/// anything `npm install` actually fetched, but this is metadata, not a
/// correctness dependency) is just silently left out rather than failing
/// the whole bundle over it.
fn record_package_version(versions: &mut BTreeMap<String, String>, name: &str, dir: &Path) {
    if let Ok(manifest) = read_manifest(dir) {
        if let Some(v) = manifest.get("version").and_then(|v| v.as_str()) {
            versions.insert(name.to_string(), v.to_string());
        }
    }
}

/// Parses a JavaScript module and collects its statically knowable dependency
/// edges. Direct `require`/`import` arguments are constant-folded when they
/// consist solely of string literals, expression-free templates, parentheses,
/// and string concatenation; runtime expressions remain dynamic. Shadowed or
/// member calls, comments, and strings are not mistaken for edges.
/// ESM declarations are collected directly from the module AST before they
/// are lowered to CommonJS.
#[derive(Default)]
struct ModuleAnalysis {
    specs: Vec<String>,
    static_esm_specs: Vec<String>,
    has_esm: bool,
    has_top_level_await: bool,
    attribute_error: Option<String>,
    has_nonliteral_dynamic_import: bool,
    _commonjs_exports: Vec<String>,
}

fn analyze_module(source: &str) -> ModuleAnalysis {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, AssignExpr, AssignTarget, AwaitExpr, CallExpr, Callee, Expr, Function,
        ImportSpecifier, Lit, MemberExpr, MemberProp, ModuleDecl, ModuleExportName, ModuleItem,
        ObjectLit, Pat, Prop, PropName, PropOrSpread, SimpleAssignTarget, VarDeclarator,
    };

    fn validate_attributes(source: &str, attributes: Option<&ObjectLit>) -> Result<(), String> {
        let Some(attributes) = attributes else {
            return Ok(());
        };
        let mut json = false;
        for property in &attributes.props {
            let PropOrSpread::Prop(property) = property else {
                return Err("spread import attributes are not supported".to_string());
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                return Err("only key/value import attributes are supported".to_string());
            };
            let key = match &property.key {
                PropName::Ident(identifier) => identifier.sym.to_string(),
                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                _ => String::new(),
            };
            let Expr::Lit(Lit::Str(value)) = property.value.as_ref() else {
                return Err("import attribute values must be strings".to_string());
            };
            if key == "type" && value.value.to_string_lossy() == "json" {
                json = true;
            } else {
                return Err(format!(
                    "unsupported import attribute `{key}` for `{source}`"
                ));
            }
        }
        let source_path = source.split(['?', '#']).next().unwrap_or(source);
        if !json || !source_path.ends_with(".json") {
            return Err(format!(
                "only JSON modules accept `type: json` import attributes (`{source}`)"
            ));
        }
        Ok(())
    }

    struct Calls {
        specs: Vec<String>,
        commonjs_exports: Vec<String>,
        has_nonliteral_dynamic_import: bool,
        attribute_error: Option<String>,
        require_functions: Vec<String>,
        create_require_functions: Vec<String>,
        module_namespaces: Vec<String>,
    }

    struct TopLevelAwait {
        found: bool,
    }

    const MAX_STATIC_SPECIFIER_CANDIDATES: usize = 64;

    fn combine_specifier_parts(left: Vec<String>, right: Vec<String>) -> Option<Vec<String>> {
        if left.len().saturating_mul(right.len()) > MAX_STATIC_SPECIFIER_CANDIDATES {
            return None;
        }
        let mut combined = Vec::new();
        for left in left {
            for right in &right {
                let value = format!("{left}{right}");
                if !combined.contains(&value) {
                    combined.push(value);
                }
            }
        }
        Some(combined)
    }

    fn static_module_specifiers(expr: &Expr) -> Option<Vec<String>> {
        match expr {
            Expr::Lit(Lit::Str(specifier)) => {
                Some(vec![specifier.value.to_string_lossy().into_owned()])
            }
            Expr::Tpl(template) if template.exprs.is_empty() && template.quasis.len() == 1 => {
                template.quasis[0]
                    .cooked
                    .as_ref()
                    .map(|value| value.to_string_lossy().into_owned())
                    .or_else(|| Some(template.quasis[0].raw.to_string()))
                    .map(|value| vec![value])
            }
            Expr::Tpl(template) if template.quasis.len() == template.exprs.len() + 1 => {
                let mut values = vec![String::new()];
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    values = combine_specifier_parts(values, vec![text])?;
                    if let Some(expr) = template.exprs.get(index) {
                        values = combine_specifier_parts(values, static_module_specifiers(expr)?)?;
                    }
                }
                Some(values)
            }
            Expr::Paren(parenthesized) => static_module_specifiers(&parenthesized.expr),
            Expr::Bin(binary) if binary.op == thaw_parser::ast::BinaryOp::Add => {
                combine_specifier_parts(
                    static_module_specifiers(&binary.left)?,
                    static_module_specifiers(&binary.right)?,
                )
            }
            Expr::Cond(conditional) => {
                let mut values = static_module_specifiers(&conditional.cons)?;
                for value in static_module_specifiers(&conditional.alt)? {
                    if !values.contains(&value) {
                        values.push(value);
                    }
                }
                (values.len() <= MAX_STATIC_SPECIFIER_CANDIDATES).then_some(values)
            }
            _ => None,
        }
    }

    fn dynamic_import_attributes(call: &CallExpr) -> Result<Option<&ObjectLit>, String> {
        if call.args.len() == 1 {
            return Ok(None);
        }
        if call.args.len() != 2 || call.args[1].spread.is_some() {
            return Err("dynamic import accepts one options object".to_string());
        }
        let Expr::Object(options) = call.args[1].expr.as_ref() else {
            return Err("dynamic import options must be an object literal".to_string());
        };
        let mut attributes = None;
        for property in &options.props {
            let PropOrSpread::Prop(property) = property else {
                return Err("spread dynamic import options are not supported".to_string());
            };
            let Prop::KeyValue(property) = property.as_ref() else {
                return Err("dynamic import options must be key/value properties".to_string());
            };
            let key = match &property.key {
                PropName::Ident(identifier) => identifier.sym.as_ref(),
                PropName::Str(value) => value.value.as_str().unwrap_or(""),
                _ => "",
            };
            if !matches!(key, "with" | "assert") {
                return Err(format!("unsupported dynamic import option `{key}`"));
            }
            if attributes.is_some() {
                return Err("dynamic import has duplicate attribute options".to_string());
            }
            let Expr::Object(object) = property.value.as_ref() else {
                return Err("dynamic import attributes must be an object literal".to_string());
            };
            attributes = Some(object);
        }
        Ok(attributes)
    }
    impl Visit for TopLevelAwait {
        fn visit_await_expr(&mut self, _: &AwaitExpr) {
            self.found = true;
        }
        fn visit_function(&mut self, _: &Function) {}
        fn visit_arrow_expr(&mut self, _: &ArrowExpr) {}
    }
    impl Visit for Calls {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            let is_require = matches!(
                &call.callee,
                Callee::Expr(callee)
                    if matches!(callee.as_ref(), Expr::Ident(ident) if self.require_functions.iter().any(|name| name == ident.sym.as_ref()))
            );
            let is_import = matches!(&call.callee, Callee::Import(_));
            if ((is_require && call.args.len() == 1) || (is_import && !call.args.is_empty()))
                && call.args[0].spread.is_none()
            {
                let specifiers = static_module_specifiers(&call.args[0].expr);
                if is_import && self.attribute_error.is_none() {
                    self.attribute_error = match dynamic_import_attributes(call) {
                        Ok(Some(attributes)) => match &specifiers {
                            Some(specifiers) => specifiers.iter().find_map(|specifier| {
                                validate_attributes(specifier, Some(attributes)).err()
                            }),
                            None => Some(
                                "attributed dynamic imports require a finite static specifier set"
                                    .to_string(),
                            ),
                        },
                        Ok(None) => None,
                        Err(error) => Some(error),
                    };
                }
                if let Some(specifiers) = specifiers {
                    self.specs.extend(specifiers);
                } else if is_import {
                    self.has_nonliteral_dynamic_import = true;
                }
            }
            call.visit_children_with(self);
        }

        fn visit_var_declarator(&mut self, declaration: &VarDeclarator) {
            let Pat::Ident(binding) = &declaration.name else {
                declaration.visit_children_with(self);
                return;
            };
            let Some(Expr::Call(call)) = declaration.init.as_deref() else {
                declaration.visit_children_with(self);
                return;
            };
            let creates_require = match &call.callee {
                Callee::Expr(callee) => match callee.as_ref() {
                    Expr::Ident(identifier) => self
                        .create_require_functions
                        .iter()
                        .any(|name| name == identifier.sym.as_ref()),
                    Expr::Member(member) => {
                        matches!(member.obj.as_ref(), Expr::Ident(identifier)
                            if self.module_namespaces.iter().any(|name| name == identifier.sym.as_ref()))
                            && property_name(&member.prop).as_deref() == Some("createRequire")
                    }
                    _ => false,
                },
                _ => false,
            };
            if creates_require {
                let name = binding.id.sym.to_string();
                if !self.require_functions.contains(&name) {
                    self.require_functions.push(name);
                }
            }
            declaration.visit_children_with(self);
        }

        fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
            let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left else {
                assignment.visit_children_with(self);
                return;
            };
            if let Some(name) = commonjs_export_name(member) {
                self.commonjs_exports.push(name);
            }
            assignment.visit_children_with(self);
        }
    }

    fn property_name(property: &MemberProp) -> Option<String> {
        match property {
            MemberProp::Ident(ident) => Some(ident.sym.to_string()),
            MemberProp::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(value)) => Some(value.value.to_string_lossy().into_owned()),
                _ => None,
            },
            MemberProp::PrivateName(_) => None,
        }
    }

    fn is_module_exports(member: &MemberExpr) -> bool {
        matches!(member.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
            && property_name(&member.prop).as_deref() == Some("exports")
    }

    fn commonjs_export_name(member: &MemberExpr) -> Option<String> {
        if matches!(member.obj.as_ref(), Expr::Ident(exports) if exports.sym == "exports") {
            return property_name(&member.prop);
        }
        if is_module_exports(member) {
            return Some("default".to_string());
        }
        if let Expr::Member(object) = member.obj.as_ref() {
            if is_module_exports(object) {
                return property_name(&member.prop);
            }
        }
        None
    }

    let Ok(module) = thaw_parser::parse_javascript(source) else {
        return ModuleAnalysis::default();
    };
    let mut create_require_functions = Vec::new();
    let mut module_namespaces = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if !matches!(import.src.value.as_str(), Some("module" | "node:module")) {
            continue;
        }
        for specifier in &import.specifiers {
            match specifier {
                ImportSpecifier::Named(named) => {
                    let imported = named.imported.as_ref().map_or_else(
                        || named.local.sym.to_string(),
                        |name| match name {
                            ModuleExportName::Ident(identifier) => identifier.sym.to_string(),
                            ModuleExportName::Str(value) => {
                                value.value.to_string_lossy().into_owned()
                            }
                        },
                    );
                    if imported == "createRequire" {
                        create_require_functions.push(named.local.sym.to_string());
                    }
                }
                ImportSpecifier::Namespace(namespace) => {
                    module_namespaces.push(namespace.local.sym.to_string());
                }
                ImportSpecifier::Default(default) => {
                    module_namespaces.push(default.local.sym.to_string());
                }
            }
        }
    }
    let mut calls = Calls {
        specs: Vec::new(),
        commonjs_exports: Vec::new(),
        has_nonliteral_dynamic_import: false,
        attribute_error: None,
        require_functions: vec!["require".to_string()],
        create_require_functions,
        module_namespaces,
    };
    module.visit_with(&mut calls);
    let mut top_level_await = TopLevelAwait { found: false };
    module.visit_with(&mut top_level_await);
    let mut static_esm_specs = Vec::new();
    let mut attribute_error = calls.attribute_error.take();
    for item in &module.body {
        let (source, attributes) = match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(decl)) => {
                (Some(&decl.src), decl.with.as_deref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(decl)) => {
                (decl.src.as_ref(), decl.with.as_deref())
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(decl)) => {
                (Some(&decl.src), decl.with.as_deref())
            }
            _ => (None, None),
        };
        if let Some(source) = source {
            let spec = source.value.to_string_lossy().into_owned();
            if attribute_error.is_none() {
                attribute_error = validate_attributes(&spec, attributes).err();
            }
            calls.specs.push(spec.clone());
            static_esm_specs.push(spec);
        }
    }
    let mut unique = Vec::new();
    for spec in calls.specs {
        if !unique.contains(&spec) {
            unique.push(spec);
        }
    }
    calls.commonjs_exports.sort();
    calls.commonjs_exports.dedup();
    ModuleAnalysis {
        specs: unique,
        static_esm_specs,
        has_esm: module
            .body
            .iter()
            .any(|item| matches!(item, ModuleItem::ModuleDecl(_))),
        has_top_level_await: top_level_await.found,
        attribute_error,
        has_nonliteral_dynamic_import: calls.has_nonliteral_dynamic_import,
        _commonjs_exports: calls.commonjs_exports,
    }
}

#[cfg(test)]
fn find_module_specs(source: &str) -> Vec<String> {
    analyze_module(source).specs
}

/// Converts literal dynamic imports to an asynchronous call through the
/// bundle's per-module `require` map. The `then` boundary ensures a missing or
/// throwing module rejects the returned Promise instead of throwing before a
/// Promise is returned.
fn rewrite_dynamic_imports(source: &str) -> Option<String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee};
    use thaw_parser::common::Spanned;

    struct Imports {
        spans: Vec<(u32, u32, u32, u32)>,
    }
    impl Visit for Imports {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if matches!(&call.callee, Callee::Import(_))
                && !call.args.is_empty()
                && call.args[0].spread.is_none()
            {
                let span = call.span();
                let argument = call.args[0].expr.span();
                self.spans
                    .push((span.lo.0, span.hi.0, argument.lo.0, argument.hi.0));
            }
            call.visit_children_with(self);
        }
    }

    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let mut imports = Imports { spans: Vec::new() };
    module.visit_with(&mut imports);
    if imports.spans.is_empty() {
        return None;
    }
    imports.spans.sort_by_key(|(lo, _, _, _)| *lo);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, argument_lo, argument_hi) in imports.spans {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        let argument_lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(argument_lo))
            .pos
            .0 as usize;
        let argument_hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(argument_hi))
            .pos
            .0 as usize;
        output.push_str(&source[cursor..lo]);
        output.push_str("requireAsync(String(");
        output.push_str(&source[argument_lo..argument_hi]);
        output.push_str("))");
        cursor = hi;
    }
    output.push_str(&source[cursor..]);
    Some(output)
}

fn rewrite_live_import_references(source: &str) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet};
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{
        ArrowExpr, BlockStmt, CatchClause, Decl, Expr, Function, ImportSpecifier, ModuleDecl,
        ModuleExportName, ModuleItem, Pat, Prop, Stmt,
    };
    use thaw_parser::common::Spanned;

    fn pattern_names(pattern: &Pat, names: &mut BTreeSet<String>) {
        match pattern {
            Pat::Ident(binding) => {
                names.insert(binding.id.sym.to_string());
            }
            Pat::Array(array) => {
                for element in array.elems.iter().flatten() {
                    pattern_names(element, names);
                }
            }
            Pat::Object(object) => {
                for property in &object.props {
                    match property {
                        thaw_parser::ast::ObjectPatProp::KeyValue(property) => {
                            pattern_names(&property.value, names);
                        }
                        thaw_parser::ast::ObjectPatProp::Assign(property) => {
                            names.insert(property.key.sym.to_string());
                        }
                        thaw_parser::ast::ObjectPatProp::Rest(property) => {
                            pattern_names(&property.arg, names);
                        }
                    }
                }
            }
            Pat::Assign(assign) => pattern_names(&assign.left, names),
            Pat::Rest(rest) => pattern_names(&rest.arg, names),
            Pat::Expr(_) | Pat::Invalid(_) => {}
        }
    }

    fn direct_block_bindings(block: &BlockStmt) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for statement in &block.stmts {
            if let Stmt::Decl(declaration) = statement {
                match declaration {
                    Decl::Var(variable) => {
                        for declarator in &variable.decls {
                            pattern_names(&declarator.name, &mut names);
                        }
                    }
                    Decl::Fn(function) => {
                        names.insert(function.ident.sym.to_string());
                    }
                    Decl::Class(class) => {
                        names.insert(class.ident.sym.to_string());
                    }
                    _ => {}
                }
            }
        }
        names
    }

    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let export_name = |name: &ModuleExportName| match name {
        ModuleExportName::Ident(identifier) => identifier.sym.to_string(),
        ModuleExportName::Str(value) => value.value.to_string_lossy().into_owned(),
    };
    let mut bindings = BTreeMap::<String, String>::new();
    let mut synthetic_count = 0usize;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let module_name = format!("__thaw_esm_import_{synthetic_count}");
                synthetic_count += 1;
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            let imported = named
                                .imported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| local.clone());
                            bindings.insert(
                                local,
                                format!("{module_name}[{}]", js_string_literal(&imported)),
                            );
                        }
                        ImportSpecifier::Default(default) => {
                            bindings.insert(
                                default.local.sym.to_string(),
                                format!(
                                    "(({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name})"
                                ),
                            );
                        }
                        ImportSpecifier::Namespace(_) => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if export.src.is_some() => {
                synthetic_count += 1;
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(_)) => synthetic_count += 1,
            _ => {}
        }
    }
    if bindings.is_empty() {
        return None;
    }

    struct References<'a> {
        bindings: &'a BTreeMap<String, String>,
        shadowed: Vec<BTreeSet<String>>,
        replacements: Vec<(u32, u32, String)>,
    }
    impl References<'_> {
        fn is_shadowed(&self, name: &str) -> bool {
            self.shadowed.iter().rev().any(|scope| scope.contains(name))
        }
        fn push_function_scope(&mut self, function: &Function) {
            let mut names = BTreeSet::new();
            for parameter in &function.params {
                pattern_names(&parameter.pat, &mut names);
            }
            self.shadowed.push(names);
            function.decorators.visit_with(self);
            function.body.visit_with(self);
            self.shadowed.pop();
        }
    }
    impl Visit for References<'_> {
        fn visit_function(&mut self, function: &Function) {
            self.push_function_scope(function);
        }

        fn visit_arrow_expr(&mut self, arrow: &ArrowExpr) {
            let mut names = BTreeSet::new();
            for parameter in &arrow.params {
                pattern_names(parameter, &mut names);
            }
            self.shadowed.push(names);
            arrow.body.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_block_stmt(&mut self, block: &BlockStmt) {
            self.shadowed.push(direct_block_bindings(block));
            block.stmts.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_catch_clause(&mut self, clause: &CatchClause) {
            let mut names = BTreeSet::new();
            if let Some(parameter) = &clause.param {
                pattern_names(parameter, &mut names);
            }
            self.shadowed.push(names);
            clause.body.visit_with(self);
            self.shadowed.pop();
        }

        fn visit_expr(&mut self, expression: &Expr) {
            if let Expr::Ident(identifier) = expression {
                let name = identifier.sym.as_str();
                if !self.is_shadowed(name) {
                    if let Some(replacement) = self.bindings.get(name) {
                        let span = identifier.span();
                        self.replacements
                            .push((span.lo.0, span.hi.0, replacement.clone()));
                        return;
                    }
                }
            }
            expression.visit_children_with(self);
        }

        fn visit_prop(&mut self, property: &Prop) {
            if let Prop::Shorthand(identifier) = property {
                let name = identifier.sym.as_str();
                if !self.is_shadowed(name) {
                    if let Some(replacement) = self.bindings.get(name) {
                        let span = identifier.span();
                        self.replacements.push((
                            span.lo.0,
                            span.hi.0,
                            format!("{name}: {replacement}"),
                        ));
                        return;
                    }
                }
            }
            property.visit_children_with(self);
        }
    }

    let mut references = References {
        bindings: &bindings,
        shadowed: vec![BTreeSet::new()],
        replacements: Vec::new(),
    };
    for item in &module.body {
        if !matches!(item, ModuleItem::ModuleDecl(ModuleDecl::Import(_))) {
            item.visit_with(&mut references);
        }
    }
    if references.replacements.is_empty() {
        return None;
    }
    references.replacements.sort_by_key(|(lo, _, _)| *lo);
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (lo, hi, replacement) in references.replacements {
        let lo = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(lo))
            .pos
            .0 as usize;
        let hi = cm
            .lookup_byte_offset(thaw_parser::common::BytePos(hi))
            .pos
            .0 as usize;
        if lo < cursor {
            continue;
        }
        output.push_str(&source[cursor..lo]);
        output.push_str(&replacement);
        cursor = hi;
    }
    output.push_str(&source[cursor..]);
    Some(output)
}

/// Rewrites ESM (`import`/`export`) syntax to the CommonJS shape the rest
/// of this bundler's require-graph resolution already understands:
/// the parser-backed dependency walk recognizes the synthesized
/// `require(...)` calls alongside original CommonJS calls, so deep imports,
/// builtins, and cross-package resolution share one graph.
///
/// Returns `None` (caller keeps the original source untouched) if the
/// file doesn't parse as JS at all, or parses but uses no `import`/
/// `export` syntax -- this only ever *adds* a transformation on top of
/// already-working CommonJS, never risks corrupting it.
///
/// A statement this doesn't need to touch is copied out **verbatim** via
/// its original source span (`SourceMap::span_to_snippet`), not
/// re-printed from the AST -- this project carries no general JS code
/// generator, and byte-for-byte preservation of untouched code avoids
/// ever needing one. Only the `import`/`export` declarations themselves
/// are replaced with synthesized `require`/`exports.x = ...` statements.
/// A destructuring `export const { a, b } = obj;` and a re-exported
/// string-literal name (`export { x as "weird name" }`, a rare ES2022
/// form) fall outside what's extracted -- silently contribute nothing to
/// `exports`, rather than aborting the whole rewrite.
#[cfg(test)]
fn rewrite_esm_to_commonjs(source: &str) -> Option<String> {
    rewrite_esm_to_commonjs_mode(source, false)
}

fn rewrite_esm_to_commonjs_mode(source: &str, await_imports: bool) -> Option<String> {
    use thaw_parser::ast::{
        Decl, DefaultDecl, ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName,
        ModuleItem, Pat,
    };
    use thaw_parser::common::{SourceMapper, Spanned};

    let dynamic_source = rewrite_dynamic_imports(source);
    let source = dynamic_source.as_deref().unwrap_or(source);
    let live_source = rewrite_live_import_references(source);
    let source = live_source.as_deref().unwrap_or(source);
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source).ok()?;
    let has_esm_syntax = module
        .body
        .iter()
        .any(|item| matches!(item, ModuleItem::ModuleDecl(_)));
    if !has_esm_syntax {
        return live_source.or(dynamic_source);
    }

    let snippet = |span: thaw_parser::common::Span| cm.span_to_snippet(span).ok();
    let export_name = |name: &ModuleExportName| match name {
        ModuleExportName::Ident(id) => id.sym.to_string(),
        // `Wtf8Atom` (arbitrary-string export names, a rare ES2022 form)
        // has no `Display`; lossily converting to UTF-8 is fine here --
        // this text only ever ends up embedded in generated JS source.
        ModuleExportName::Str(s) => s.value.to_string_lossy().into_owned(),
    };
    let names_declared_by = |decl: &Decl| -> Vec<String> {
        match decl {
            Decl::Fn(f) => vec![f.ident.sym.to_string()],
            Decl::Class(c) => vec![c.ident.sym.to_string()],
            Decl::Var(v) => v
                .decls
                .iter()
                .filter_map(|d| match &d.name {
                    Pat::Ident(id) => Some(id.id.sym.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };

    let mut prologue = String::new();
    let mut local_export_prologue = String::new();
    let mut rest = String::new();
    let mut synthetic_count = 0usize;
    let mut imported_bindings = BTreeMap::<String, String>::new();
    let mut binding_counter = 0usize;
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let module_name = format!("__thaw_esm_import_{binding_counter}");
                binding_counter += 1;
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            let imported = named
                                .imported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| local.clone());
                            imported_bindings.insert(
                                local,
                                format!("{module_name}[{}]", js_string_literal(&imported)),
                            );
                        }
                        ImportSpecifier::Default(default) => {
                            imported_bindings.insert(
                                default.local.sym.to_string(),
                                format!(
                                    "(({module_name} && {module_name}.__esModule) ? {module_name}.default : {module_name})"
                                ),
                            );
                        }
                        ImportSpecifier::Namespace(_) => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(export)) if export.src.is_some() => {
                binding_counter += 1;
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(_)) => binding_counter += 1,
            _ => {}
        }
    }

    for item in &module.body {
        match item {
            ModuleItem::Stmt(stmt) => {
                if let Some(text) = snippet(stmt.span()) {
                    rest.push_str(&text);
                    rest.push('\n');
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let var_name = format!("__thaw_esm_import_{synthetic_count}");
                synthetic_count += 1;
                let spec = import.src.value.to_string_lossy();
                let loader = if await_imports {
                    "await requireAsync"
                } else {
                    "require"
                };
                prologue.push_str(&format!(
                    "var {var_name} = {loader}({});\n",
                    js_string_literal(&spec)
                ));
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Default(d) => {
                            let _ = d;
                        }
                        ImportSpecifier::Namespace(n) => {
                            prologue.push_str(&format!("var {} = {var_name};\n", n.local.sym));
                        }
                        ImportSpecifier::Named(n) => {
                            let _ = n;
                        }
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export_decl)) => {
                if let Some(text) = snippet(export_decl.decl.span()) {
                    rest.push_str(&text);
                    rest.push('\n');
                }
                for name in names_declared_by(&export_decl.decl) {
                    local_export_prologue.push_str(&format!(
                        "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {name}; }} }});\n",
                        js_string_literal(&name)
                    ));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(default_decl)) => {
                let text = match &default_decl.decl {
                    DefaultDecl::Fn(f) => snippet(f.span()),
                    DefaultDecl::Class(c) => snippet(c.span()),
                    DefaultDecl::TsInterfaceDecl(_) => None,
                };
                if let Some(text) = text {
                    rest.push_str(&format!("module.exports.default = {text};\n"));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(default_expr)) => {
                if let Some(text) = snippet(default_expr.expr.span()) {
                    rest.push_str(&format!("module.exports.default = {text};\n"));
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportNamed(named)) => match &named.src {
                Some(src) => {
                    let var_name = format!("__thaw_esm_reexport_{synthetic_count}");
                    synthetic_count += 1;
                    let spec = src.value.to_string_lossy();
                    let loader = if await_imports {
                        "await requireAsync"
                    } else {
                        "require"
                    };
                    prologue.push_str(&format!(
                        "var {var_name} = {loader}({});\n",
                        js_string_literal(&spec)
                    ));
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n
                                .exported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| orig.clone());
                            rest.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {var_name}[{}]; }} }});\n",
                                js_string_literal(&exported),
                                js_string_literal(&orig)
                            ));
                        }
                        // `export * as ns from './y'`/`export v from './y'`:
                        // rare re-export forms, best-effort skipped.
                    }
                }
                None => {
                    for spec in &named.specifiers {
                        if let ExportSpecifier::Named(n) = spec {
                            let orig = export_name(&n.orig);
                            let exported = n
                                .exported
                                .as_ref()
                                .map(&export_name)
                                .unwrap_or_else(|| orig.clone());
                            let value = imported_bindings
                                .get(&orig)
                                .map(String::as_str)
                                .unwrap_or(orig.as_str());
                            local_export_prologue.push_str(&format!(
                                "Object.defineProperty(exports, {}, {{ enumerable: true, get: function() {{ return {value}; }} }});\n",
                                js_string_literal(&exported)
                            ));
                        }
                    }
                }
            },
            ModuleItem::ModuleDecl(ModuleDecl::ExportAll(export_all)) => {
                let var_name = format!("__thaw_esm_reexport_all_{synthetic_count}");
                synthetic_count += 1;
                let spec = export_all.src.value.to_string_lossy();
                let loader = if await_imports {
                    "await requireAsync"
                } else {
                    "require"
                };
                prologue.push_str(&format!(
                    "var {var_name} = {loader}({});\n",
                    js_string_literal(&spec)
                ));
                rest.push_str(&format!(
                    "for (let __thaw_esm_key in {var_name}) {{ if (__thaw_esm_key !== 'default' && __thaw_esm_key !== '__esModule') Object.defineProperty(exports, __thaw_esm_key, {{ enumerable: true, get: function() {{ return {var_name}[__thaw_esm_key]; }} }}); }}\n"
                ));
            }
            // `import foo = require(...)`/`export = foo`/`export as
            // namespace`: TS-only forms that shouldn't appear in real
            // runtime `.js` files; skip gracefully rather than crashing
            // if one somehow does.
            ModuleItem::ModuleDecl(_) => {}
        }
    }

    Some(format!(
        "module.exports.__esModule = true;\n{local_export_prologue}{prologue}{rest}"
    ))
}

/// Splits a bare require spec into its package name and, if present, a
/// "deep import" subpath: `"lodash"` -> `("lodash", None)`, `"lodash/fp"`
/// -> `("lodash", Some("fp"))`, `"@babel/core"` -> `("@babel/core", None)`,
/// `"@babel/core/lib/x"` -> `("@babel/core", Some("lib/x"))`. A scoped
/// name needs two `/`-segments (`@scope/name`) before any subpath starts.
fn split_bare_spec(spec: &str) -> (&str, Option<&str>) {
    if spec.starts_with('@') {
        match spec.match_indices('/').nth(1) {
            Some((idx, _)) => (&spec[..idx], Some(&spec[idx + 1..])),
            None => (spec, None),
        }
    } else {
        match spec.find('/') {
            Some(idx) => (&spec[..idx], Some(&spec[idx + 1..])),
            None => (spec, None),
        }
    }
}

/// Resolves a bare require spec (found under `node_modules_dir`) to the
/// file it actually points to. With no subpath, that's the target
/// package's own `main` field (or the `index.js` default); with a "deep
/// import" subpath (e.g. `require('es-errors/type')`, found necessary by
/// `qs`'s own transitive dependency chain), Node resolves the subpath
/// directly against the package root -- the target package's `main`
/// field is irrelevant in that case. Returns `(package name, path
/// resolved relative to the package root, its absolute path, the
/// package's own directory)`, or `None` if it can't be resolved (not
/// installed under `node_modules_dir`, or -- rare -- the subpath itself
/// doesn't exist) -- left for the runtime external-require stub to
/// report, same as any other unresolvable require.
/// A tiny, hand-maintained polyfill for a Node.js core builtin module --
/// *not* a real re-implementation of Node's standard library, just
/// enough surface for whatever a real npm package's dependency chain
/// has actually been found to touch unconditionally at load time, added
/// one module (and one function) at a time the same way every other gap
/// in this file was: hit a real error against a real package, fix
/// exactly that. A Node builtin has no `package.json`/`node_modules`
/// entry at all, so `resolve_bare_require` correctly never finds it;
/// this is the fallback checked only after that lookup fails.
///
/// `util`: found necessary by `qs`'s real dependency chain --
/// `object-inspect` (pulled in via `side-channel`) does `require('util')`
/// unconditionally at the top of `util.inspect.js`, only to read
/// `.inspect`/`.inspect.custom` off the result (as a fallback/symbol
/// source, not for `util.inspect`'s actual pretty-printing behavior,
/// which `object-inspect` itself reimplements) -- a real
/// `util.inspect`-quality implementation is unnecessary for that.
fn builtin_module_source(name: &str) -> Option<&'static str> {
    match name {
        "util" => Some(
            "var inspectCustom = Symbol.for('nodejs.util.inspect.custom');\n\
             function inspect(value, options) {\n\
             \x20\x20if (value && typeof value[inspectCustom] === 'function') return String(value[inspectCustom](2, options || {}, inspect));\n\
             \x20\x20if (typeof value === 'string') return \"'\" + value.replace(/\\\\/g, '\\\\\\\\').replace(/'/g, \"\\\\'\") + \"'\";\n\
             \x20\x20if (typeof value === 'function') return '[Function' + (value.name ? ': ' + value.name : '') + ']';\n\
             \x20\x20if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);\n\
             \x20\x20if (value instanceof Error) return value.stack || value.name + ': ' + value.message;\n\
             \x20\x20var seen = new Set();\n\
             \x20\x20function render(input) {\n\
             \x20\x20\x20\x20if (input === null || typeof input !== 'object') return typeof input === 'string' ? \"'\" + input + \"'\" : String(input);\n\
             \x20\x20\x20\x20if (seen.has(input)) return '[Circular]'; seen.add(input);\n\
             \x20\x20\x20\x20var result;\n\
             \x20\x20\x20\x20if (Array.isArray(input)) result = '[ ' + input.map(render).join(', ') + ' ]';\n\
             \x20\x20\x20\x20else if (input instanceof Date) result = isNaN(input.getTime()) ? 'Invalid Date' : input.toISOString();\n\
             \x20\x20\x20\x20else if (input instanceof RegExp) result = String(input);\n\
             \x20\x20\x20\x20else if (input instanceof Map) result = 'Map(' + input.size + ') { ' + Array.from(input).map(function(entry) { return render(entry[0]) + ' => ' + render(entry[1]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20else if (input instanceof Set) result = 'Set(' + input.size + ') { ' + Array.from(input).map(render).join(', ') + ' }';\n\
             \x20\x20\x20\x20else result = '{ ' + Object.keys(input).map(function(key) { return key + ': ' + render(input[key]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20seen.delete(input); return result;\n\
             \x20\x20}\n\
             \x20\x20return render(value);\n\
             }\n\
             inspect.custom = inspectCustom; inspect.defaultOptions = {};\n\
             function format() {\n\
             \x20\x20var args = Array.prototype.slice.call(arguments); if (args.length === 0) return '';\n\
             \x20\x20if (typeof args[0] !== 'string') return args.map(inspect).join(' ');\n\
             \x20\x20var index = 1; var output = args[0].replace(/%[sdifjoOc%]/g, function(token) {\n\
             \x20\x20\x20\x20if (token === '%%') return '%'; if (index >= args.length) return token; var value = args[index++];\n\
             \x20\x20\x20\x20if (token === '%s') return String(value); if (token === '%d') return String(Number(value));\n\
             \x20\x20\x20\x20if (token === '%i') return String(parseInt(value, 10)); if (token === '%f') return String(parseFloat(value));\n\
             \x20\x20\x20\x20if (token === '%j') { try { return JSON.stringify(value); } catch (_) { return '[Circular]'; } }\n\
             \x20\x20\x20\x20if (token === '%c') return ''; return inspect(value);\n\
             \x20\x20});\n\
             \x20\x20while (index < args.length) { var extra = args[index++]; output += ' ' + (typeof extra === 'string' ? extra : inspect(extra)); } return output;\n\
             }\n\
             function formatWithOptions(options) { return format.apply(null, Array.prototype.slice.call(arguments, 1)); }\n\
             function inherits(constructor, superConstructor) { if (constructor === undefined || superConstructor === undefined) throw new TypeError('constructors are required'); constructor.super_ = superConstructor; Object.setPrototypeOf(constructor.prototype, superConstructor.prototype); }\n\
             var promisifyCustom = Symbol.for('nodejs.util.promisify.custom');\n\
             function promisify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function'); if (original[promisifyCustom]) return original[promisifyCustom];\n\
             \x20\x20function wrapped() { var self = this; var args = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { args.push(function(error) { if (error) reject(error); else { var values = Array.prototype.slice.call(arguments, 1); resolve(values.length > 1 ? values : values[0]); } }); original.apply(self, args); }); }\n\
             \x20\x20Object.setPrototypeOf(wrapped, Object.getPrototypeOf(original)); return wrapped;\n\
             }\n\
             promisify.custom = promisifyCustom;\n\
             function callbackify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function');\n\
             \x20\x20return function() { var args = Array.prototype.slice.call(arguments); var callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); Promise.resolve(original.apply(this, args)).then(function(value) { queueMicrotask(function() { callback(null, value); }); }, function(error) { queueMicrotask(function() { callback(error || new Error('Promise was rejected with a falsy value')); }); }); };\n\
             }\n\
             function deprecate(fn) { return function() { return fn.apply(this, arguments); }; }\n\
             function stripVTControlCharacters(value) { return String(value).replace(/[\\u001B\\u009B][[\\]()#;?]*(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]+)*)?\\u0007|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-nq-uy=><~]))/g, ''); }\n\
             function toUSVString(value) { return String(value).replace(/[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(^|[^\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]/g, function(match, prefix) { return (prefix || '') + '\\uFFFD'; }); }\n\
             function parseArgs(config) { config = config || {}; var args = Array.from(config.args === undefined ? (process.argv || []).slice(2) : config.args, String), definitions = config.options || {}, values = Object.create(null), positionals = [], tokens = [], short = Object.create(null); Object.keys(definitions).forEach(function(name) { var definition = definitions[name] || {}; if (definition.short) short[String(definition.short)] = name; if (definition.default !== undefined) values[name] = definition.multiple ? Array.from(definition.default) : definition.default; else if (definition.multiple) values[name] = []; }); function assign(name, value, index, inline) { var definition = definitions[name]; if (!definition) { if (config.strict !== false) { var error = new TypeError('Unknown option --' + name); error.code = 'ERR_PARSE_ARGS_UNKNOWN_OPTION'; throw error; } definition = { type: 'boolean' }; } if (definition.type === 'string') { if (value === undefined) { var error = new TypeError('Option --' + name + ' argument is missing'); error.code = 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE'; throw error; } value = String(value); } else value = value === undefined ? true : Boolean(value); if (definition.multiple) (values[name] || (values[name] = [])).push(value); else values[name] = value; tokens.push({ kind: 'option', index: index, name: name, rawName: inline || '--' + name, value: definition.type === 'string' ? value : undefined, inlineValue: Boolean(inline && inline.indexOf('=') >= 0) }); } for (var index = 0; index < args.length; index++) { var argument = args[index]; if (argument === '--') { tokens.push({ kind: 'option-terminator', index: index }); for (index++; index < args.length; index++) { positionals.push(args[index]); tokens.push({ kind: 'positional', index: index, value: args[index] }); } break; } if (argument.slice(0, 2) === '--') { var equal = argument.indexOf('='), name = argument.slice(2, equal < 0 ? undefined : equal), value = equal < 0 ? undefined : argument.slice(equal + 1); if (name.slice(0, 3) === 'no-' && config.allowNegative && definitions[name.slice(3)] && definitions[name.slice(3)].type === 'boolean') { assign(name.slice(3), false, index, argument); continue; } var definition = definitions[name]; if (equal < 0 && definition && definition.type === 'string') value = args[++index]; assign(name, value, equal < 0 && definition && definition.type === 'string' ? index - 1 : index, argument); } else if (argument[0] === '-' && argument.length > 1) { var letters = argument.slice(1); for (var letterIndex = 0; letterIndex < letters.length; letterIndex++) { var name = short[letters[letterIndex]] || letters[letterIndex], definition = definitions[name], value; if (definition && definition.type === 'string') { value = letters.slice(letterIndex + 1) || args[++index]; letterIndex = letters.length; } assign(name, value, index, '-' + letters[letterIndex]); } } else { if (!config.allowPositionals && config.strict !== false) { var error = new TypeError('Unexpected argument ' + argument); error.code = 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL'; throw error; } positionals.push(argument); tokens.push({ kind: 'positional', index: index, value: argument }); } } var result = { values: values, positionals: positionals }; if (config.tokens) result.tokens = tokens; return result; }\n\
             var types = globalThis.__thaw_util_types || (globalThis.__thaw_util_types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return Object.prototype.toString.call(value) === '[object Arguments]'; } });\n\
             module.exports = { inspect: inspect, format: format, formatWithOptions: formatWithOptions, inherits: inherits, promisify: promisify, callbackify: callbackify, deprecate: deprecate, stripVTControlCharacters: stripVTControlCharacters, toUSVString: toUSVString, parseArgs: parseArgs, TextEncoder: globalThis.TextEncoder, TextDecoder: globalThis.TextDecoder, types: types };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "util/types" => Some(
            "var tag = function(value) { return Object.prototype.toString.call(value); }; var types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isWeakMap: function(value) { return value instanceof WeakMap; }, isWeakSet: function(value) { return value instanceof WeakSet; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer); }, isArrayBufferView: function(value) { return ArrayBuffer.isView(value); }, isDataView: function(value) { return value instanceof DataView; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isUint8Array: function(value) { return value instanceof Uint8Array; }, isUint8ClampedArray: function(value) { return value instanceof Uint8ClampedArray; }, isUint16Array: function(value) { return value instanceof Uint16Array; }, isUint32Array: function(value) { return value instanceof Uint32Array; }, isInt8Array: function(value) { return value instanceof Int8Array; }, isInt16Array: function(value) { return value instanceof Int16Array; }, isInt32Array: function(value) { return value instanceof Int32Array; }, isFloat32Array: function(value) { return value instanceof Float32Array; }, isFloat64Array: function(value) { return value instanceof Float64Array; }, isBigInt64Array: function(value) { return typeof BigInt64Array === 'function' && value instanceof BigInt64Array; }, isBigUint64Array: function(value) { return typeof BigUint64Array === 'function' && value instanceof BigUint64Array; }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return tag(value) === '[object Arguments]'; }, isNumberObject: function(value) { return tag(value) === '[object Number]'; }, isStringObject: function(value) { return tag(value) === '[object String]'; }, isBooleanObject: function(value) { return tag(value) === '[object Boolean]'; }, isBigIntObject: function(value) { return tag(value) === '[object BigInt]'; }, isSymbolObject: function(value) { return tag(value) === '[object Symbol]'; }, isBoxedPrimitive: function(value) { return /\\[object (Number|String|Boolean|BigInt|Symbol)\\]/.test(tag(value)); }, isAsyncFunction: function(value) { return tag(value) === '[object AsyncFunction]'; }, isGeneratorFunction: function(value) { return tag(value) === '[object GeneratorFunction]'; }, isGeneratorObject: function(value) { return tag(value) === '[object Generator]'; }, isExternal: function() { return false; }, isProxy: function() { return false; }, isModuleNamespaceObject: function() { return false; }, isKeyObject: function() { return false; }, isCryptoKey: function() { return false; } }; module.exports = types; module.exports.default = types; module.exports.__esModule = true;\n",
        ),
        // Found necessary by a real ESM package (`has-flag`): `import
        // process from 'process'` -- Node exposes `process` as both a
        // global and a core module; this is the module half. `.default`
        // is set too so the ESM-interop convention `rewrite_esm_to_commonjs`
        // generates for a default import (`.__esModule ? .default : ...`)
        // finds the same object either way. Only the couple of fields a
        // real package has actually been found to read.
        "process" => Some(
            "var __thaw_process = globalThis.process || { argv: [], env: {}, platform: 'linux', version: '', versions: {}, cwd: function() { return '/'; }, nextTick: function(fn) { var args = Array.prototype.slice.call(arguments, 1); Promise.resolve().then(function() { fn.apply(undefined, args); }); } };\n\
             if (!__thaw_process.cwd) __thaw_process.cwd = function() { return '/'; };\n\
             module.exports = __thaw_process;\n\
             module.exports.default = __thaw_process;\n\
             module.exports.__esModule = true;\n",
        ),
        "child_process" => Some(
            r#"var EventEmitter = require('node:events'), streams = require('node:stream');
             function normalize(command, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; command = String(command); args = args.map(String); if (options.shell) { var shell = typeof options.shell === 'string' ? options.shell : '/bin/sh', joined = [command].concat(args).map(function(value) { return "'" + value.replace(/'/g, "'\\''") + "'"; }).join(' '); command = shell; args = ['-c', joined]; } var environment; if (options.env) { environment = {}; Object.keys(options.env).forEach(function(name) { if (options.env[name] !== undefined) environment[name] = String(options.env[name]); }); } var input = options.input === undefined ? undefined : Buffer.from(options.input, options.encoding).toString('hex'); return { command: command, args: args, options: { cwd: options.cwd === undefined ? undefined : String(options.cwd), env: environment, input: input, detached: Boolean(options.detached) }, original: options }; }
             function processError(record, normalized) { var error = new Error(record.error || ('Command failed: ' + normalized.command)); error.code = record.code || null; error.errno = record.errno === undefined ? null : record.errno; error.path = record.path || normalized.command; error.spawnargs = normalized.args.slice(); error.status = record.status === undefined ? null : record.status; error.signal = record.signal === undefined ? null : record.signal; return error; }
             function decode(record, options) { var encoding = options.encoding === undefined ? null : options.encoding, stdout = Buffer.from(record.stdout || '', 'hex'), stderr = Buffer.from(record.stderr || '', 'hex'); if (encoding && encoding !== 'buffer') { stdout = stdout.toString(encoding); stderr = stderr.toString(encoding); } return { pid: record.pid || 0, output: [null, stdout, stderr], stdout: stdout, stderr: stderr, status: record.status === undefined ? null : record.status, signal: record.signal === undefined ? null : record.signal, error: record.error ? processError(record, { command: record.path || '', args: [] }) : undefined }; }
             function spawnSync(command, args, options) { var normalized = normalize(command, args, options), record = JSON.parse(__thaw_child_process_sync(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options))), result = decode(record, normalized.original), maxBuffer = normalized.original.maxBuffer === undefined ? 1048576 : Number(normalized.original.maxBuffer); if (!result.error && result.stdout.length + result.stderr.length > maxBuffer) { var error = new Error('spawnSync ' + normalized.command + ' ENOBUFS'); error.code = 'ENOBUFS'; result.error = error; result.status = null; } return result; }
             function execFileSync(file, args, options) { var normalized = normalize(file, args, options), result = spawnSync(file, args, options); if (result.error || result.status !== 0) { var error = result.error || processError({ status: result.status, signal: result.signal }, normalized); error.stdout = result.stdout; error.stderr = result.stderr; error.output = result.output; error.pid = result.pid; throw error; } return result.stdout; }
             function execSync(command, options) { options = options || {}; return execFileSync(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false })); }
             var children = new Map(), signals = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGUSR1: 10, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 }; function signalName(signal) { if (signal === null || signal === undefined) return null; var names = Object.keys(signals); for (var index = 0; index < names.length; index++) if (signals[names[index]] === signal) return names[index]; return 'SIG' + signal; } function signalNumber(signal) { if (signal === undefined || signal === null) return signals.SIGTERM; if (typeof signal === 'string') { signal = signal.toUpperCase(); if (signals[signal] !== undefined) return signals[signal]; } else if (Number.isInteger(signal) && signal > 0) return signal; var error = new TypeError('Unknown signal: ' + signal); error.code = 'ERR_UNKNOWN_SIGNAL'; throw error; }
             function ChildProcess(normalized) { EventEmitter.call(this); this.pid = 0; this.spawnfile = normalized.command; this.spawnargs = [normalized.command].concat(normalized.args); this.killed = false; this.connected = false; this.exitCode = null; this.signalCode = null; this.detached = Boolean(normalized.original.detached); this._closed = false; this._spawnError = false; this._timedOut = false; var child = this; this._stdin = new streams.Writable({ write: function(chunk, encoding, callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, Buffer.from(chunk).toString('hex'), false); callback(); }, final: function(callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, '', true); callback(); } }); this._stdout = new streams.PassThrough(); this._stderr = new streams.PassThrough(); var stdio = normalized.original.stdio === undefined ? 'pipe' : normalized.original.stdio, modes = Array.isArray(stdio) ? stdio.slice(0, 3) : [stdio, stdio, stdio]; while (modes.length < 3) modes.push('pipe'); modes = modes.map(function(mode) { mode = mode === null || mode === undefined ? 'pipe' : mode; if (mode !== 'pipe' && mode !== 'ignore' && mode !== 'inherit') { var error = new TypeError('The argument stdio is invalid. Received ' + mode); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } return mode; }); this._stdioModes = modes; this.stdin = modes[0] === 'pipe' ? this._stdin : null; this.stdout = modes[1] === 'pipe' ? this._stdout : null; this.stderr = modes[2] === 'pipe' ? this._stderr : null; this.stdio = [this.stdin, this.stdout, this.stderr]; this.channel = null; this._handle = __thaw_child_process_spawn(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options)); children.set(this._handle, this); if (modes[0] !== 'pipe') this._stdin.end(); var timeout = Number(normalized.original.timeout || 0); if (timeout > 0 && Number.isFinite(timeout)) this._timeout = setTimeout(function() { if (!child._closed) { child._timedOut = true; child.kill(normalized.original.killSignal); } }, timeout); var signal = normalized.original.signal; if (signal && typeof signal.addEventListener === 'function') { this._abortSignal = signal; this._abortListener = function() { if (child._closed || child._aborted) return; child._aborted = true; var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = signal.reason; child.emit('error', error); child.kill(normalized.original.killSignal); }; signal.addEventListener('abort', this._abortListener, { once: true }); if (signal.aborted) queueMicrotask(this._abortListener); } }
             ChildProcess.prototype = Object.create(EventEmitter.prototype); ChildProcess.prototype.constructor = ChildProcess; ChildProcess.prototype.kill = function(signal) { if (this._closed) return false; this.killed = __thaw_child_process_kill(this._handle, signalNumber(signal)); return this.killed; }; ChildProcess.prototype.ref = function() { return this; }; ChildProcess.prototype.unref = function() { return this; }; ChildProcess.prototype.disconnect = function() { if (!this.connected) { var error = new Error('IPC channel is already disconnected'); error.code = 'ERR_IPC_DISCONNECTED'; throw error; } this.connected = false; this.channel = null; if (this.stdin && !this.stdin.writableEnded) this.stdin.end(); this.emit('disconnect'); return this; }; ChildProcess.prototype.send = function(message, sendHandle, options, callback) { if (typeof sendHandle === 'function') callback = sendHandle; else if (typeof options === 'function') callback = options; if (!this.connected || !this._ipc) { var error = new Error('Channel closed'); error.code = 'ERR_IPC_CHANNEL_CLOSED'; if (callback) { queueMicrotask(function() { callback(error); }); return false; } throw error; } var serialized; try { serialized = JSON.stringify(message); } catch (error) { if (callback) queueMicrotask(function() { callback(error); }); else throw error; return false; } this.stdin.write('__THAW_IPC__' + serialized + '\n', function(error) { if (callback) callback(error || null); }); return true; };
             function spawn(command, args, options) { return new ChildProcess(normalize(command, args, options)); }
             function fork(modulePath, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; if (typeof modulePath !== 'string' && !(modulePath instanceof String)) { var pathError = new TypeError('The modulePath argument must be of type string'); pathError.code = 'ERR_INVALID_ARG_TYPE'; throw pathError; } var wrapper = "var prefix='__THAW_IPC__',buffer='';process.send=function(value,handle,options,callback){if(typeof handle==='function')callback=handle;else if(typeof options==='function')callback=options;try{process.stdout.write(prefix+JSON.stringify(value)+'\\n',function(){if(typeof callback==='function')callback(null);});return true;}catch(error){if(typeof callback==='function')callback(error);else throw error;return false;}};process.connected=true;process.disconnect=function(){if(!process.connected)return;process.connected=false;process.emit('disconnect');process.stdin.destroy();setTimeout(function(){process.exit(0);},0);};process.stdin.setEncoding('utf8');process.stdin.on('data',function(chunk){buffer+=chunk;for(;;){var end=buffer.indexOf('\\n');if(end<0)break;var line=buffer.slice(0,end);buffer=buffer.slice(end+1);if(line.indexOf(prefix)===0)process.emit('message',JSON.parse(line.slice(prefix.length)));}});process.stdin.on('end',process.disconnect);require(require('path').resolve(process.argv[1]));"; var childOptions = Object.assign({}, options, { stdio: 'pipe' }), execArgv = options.execArgv === undefined ? [] : Array.from(options.execArgv, String), child = spawn(options.execPath || process.execPath || '/usr/bin/node', execArgv.concat(['-e', wrapper, String(modulePath)]).concat(args.map(String)), childOptions); child.connected = true; child.channel = {}; child._ipc = true; child._ipcBuffer = ''; child._ipcOutputMode = options.silent ? 'pipe' : 'inherit'; if (!options.silent) { child.stdout = null; child.stderr = null; child.stdio = [child.stdin, null, null, child.channel]; } else child.stdio.push(child.channel); return child; }
             function execFile(file, args, options, callback) { if (typeof args === 'function') { callback = args; args = []; options = {}; } else if (!Array.isArray(args)) { callback = options; options = args || {}; args = []; } else if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; var child = spawn(file, args, options), stdout = [], stderr = [], encoding = options.encoding || 'utf8', maxBuffer = options.maxBuffer === undefined ? 1048576 : Number(options.maxBuffer); child.stdout.on('data', function(value) { stdout.push(Buffer.from(value)); }); child.stderr.on('data', function(value) { stderr.push(Buffer.from(value)); }); child.once('close', function(code, signal) { var out = Buffer.concat(stdout), err = Buffer.concat(stderr), decodedOut = encoding === 'buffer' ? out : out.toString(encoding), decodedErr = encoding === 'buffer' ? err : err.toString(encoding), error = null; if (out.length + err.length > maxBuffer) { error = new Error('stdout maxBuffer length exceeded'); error.code = 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER'; } else if (code !== 0) { error = new Error('Command failed: ' + file + '\n' + decodedErr); error.code = code; error.killed = child.killed; error.signal = signal; error.cmd = file; } if (error) { error.stdout = decodedOut; error.stderr = decodedErr; } if (typeof callback === 'function') callback(error, decodedOut, decodedErr); }); child.stdin.end(); return child; }
             function exec(command, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; return execFile(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false }), callback); }
             if (!globalThis.__thaw_child_process_poll_installed) { globalThis.__thaw_child_process_poll_installed = true; var previousPlatformPoll = globalThis.__thaw_poll_platform_events; globalThis.__thaw_poll_platform_events = function() { if (previousPlatformPoll) previousPlatformPoll(); JSON.parse(__thaw_child_process_poll()).forEach(function(event) { var child = children.get(event.handle); if (!child) return; if (event.type === 'spawn') { child.pid = Number(event.pid); child.emit('spawn'); } else if (event.type === 'stdout') { var stdout = Buffer.from(event.value, 'hex'); if (child._ipc) { child._ipcBuffer += stdout.toString(); for (;;) { var end = child._ipcBuffer.indexOf('\n'); if (end < 0) break; var line = child._ipcBuffer.slice(0, end); child._ipcBuffer = child._ipcBuffer.slice(end + 1); if (line.indexOf('__THAW_IPC__') === 0) { try { child.emit('message', JSON.parse(line.slice(12))); } catch (error) { child.emit('error', error); } } else if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(line + '\n')); else child._stdout.write(Buffer.from(line + '\n')); } } else if (child._stdioModes[1] === 'pipe') child._stdout.write(stdout); else if (child._stdioModes[1] === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(stdout); } else if (event.type === 'stderr') { var stderr = Buffer.from(event.value, 'hex'); if (child._ipc && child._ipcOutputMode === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); else if (child._stdioModes[2] === 'pipe') child._stderr.write(stderr); else if (child._stdioModes[2] === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); } else if (event.type === 'error') { child._spawnError = true; var error = new Error(event.message); error.code = event.code; error.path = child.spawnfile; error.spawnargs = child.spawnargs.slice(1); child.emit('error', error); } else if (event.type === 'exit') { child._closed = true; child.exitCode = event.code === null ? null : Number(event.code); child.signalCode = signalName(event.signal); if (child._ipcBuffer) { if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(child._ipcBuffer)); else child._stdout.write(Buffer.from(child._ipcBuffer)); } child._stdout.end(); child._stderr.end(); if (!child._stdin.writableEnded) child._stdin.end(); if (child.connected) { child.connected = false; child.channel = null; child.emit('disconnect'); } if (child._timeout) clearTimeout(child._timeout); if (child._abortSignal && child._abortListener) child._abortSignal.removeEventListener('abort', child._abortListener); if (!child._spawnError) child.emit('exit', child.exitCode, child.signalCode); child.emit('close', child.exitCode, child.signalCode); children.delete(event.handle); } }); }; }
             module.exports = { ChildProcess: ChildProcess, spawn: spawn, fork: fork, exec: exec, execFile: execFile, spawnSync: spawnSync, execFileSync: execFileSync, execSync: execSync }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "punycode" => Some(
            "var base = 36, tMin = 1, tMax = 26, skew = 38, damp = 700, initialBias = 72, initialN = 128, delimiter = '-'; function adapt(delta, points, first) { delta = first ? Math.floor(delta / damp) : delta >> 1; delta += Math.floor(delta / points); var k = 0; while (delta > Math.floor(((base - tMin) * tMax) / 2)) { delta = Math.floor(delta / (base - tMin)); k += base; } return k + Math.floor(((base - tMin + 1) * delta) / (delta + skew)); } function encodeDigit(value) { return String.fromCharCode(value + 22 + 75 * (value < 26)); } function decodeDigit(code) { if (code >= 48 && code <= 57) return code - 22; if (code >= 65 && code <= 90) return code - 65; if (code >= 97 && code <= 122) return code - 97; return base; } function codePoints(value) { return Array.from(String(value)).map(function(character) { return character.codePointAt(0); }); }\n\
             function encode(value) { var input = codePoints(value), output = [], n = initialN, delta = 0, bias = initialBias; input.forEach(function(point) { if (point < 128) output.push(String.fromCharCode(point)); }); var basic = output.length, handled = basic; if (basic) output.push(delimiter); while (handled < input.length) { var next = Infinity; input.forEach(function(point) { if (point >= n && point < next) next = point; }); delta += (next - n) * (handled + 1); n = next; input.forEach(function(point) { if (point < n) delta++; if (point === n) { var q = delta; for (var k = base;; k += base) { var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (q < threshold) break; output.push(encodeDigit(threshold + ((q - threshold) % (base - threshold)))); q = Math.floor((q - threshold) / (base - threshold)); } output.push(encodeDigit(q)); bias = adapt(delta, handled + 1, handled === basic); delta = 0; handled++; } }); delta++; n++; } return output.join(''); }\n\
             function decode(value) { var input = String(value), output = [], n = initialN, index = 0, bias = initialBias, i = 0, delimiterIndex = input.lastIndexOf(delimiter); if (delimiterIndex >= 0) { for (var basicIndex = 0; basicIndex < delimiterIndex; basicIndex++) { var basicCode = input.charCodeAt(basicIndex); if (basicCode >= 128) throw new RangeError('Illegal input'); output.push(basicCode); } index = delimiterIndex + 1; } while (index < input.length) { var oldI = i, weight = 1; for (var k = base;; k += base) { if (index >= input.length) throw new RangeError('Invalid input'); var digit = decodeDigit(input.charCodeAt(index++)); if (digit >= base) throw new RangeError('Invalid input'); i += digit * weight; var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (digit < threshold) break; weight *= base - threshold; } var length = output.length + 1; bias = adapt(i - oldI, length, oldI === 0); n += Math.floor(i / length); i %= length; output.splice(i, 0, n); i++; } return String.fromCodePoint.apply(String, output); }\n\
             function mapDomain(value, callback) { var input = String(value), parts = input.split('@'), local = ''; if (parts.length > 1) local = parts.shift() + '@'; return local + parts.join('@').replace(/[\\u3002\\uFF0E\\uFF61]/g, '.').split('.').map(callback).join('.'); } function toASCII(value) { return mapDomain(value, function(label) { return /[^\\x00-\\x7F]/.test(label) ? 'xn--' + encode(label) : label; }); } function toUnicode(value) { return mapDomain(value, function(label) { return /^xn--/i.test(label) ? decode(label.slice(4).toLowerCase()) : label; }); } var ucs2 = { decode: codePoints, encode: function(points) { return String.fromCodePoint.apply(String, points); } }; module.exports = { version: '2.1.0', ucs2: ucs2, decode: decode, encode: encode, toASCII: toASCII, toUnicode: toUnicode }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        // Found necessary chasing a real native addon's load path
        // (`bcrypt`, `utf-8-validate`): both depend on `node-gyp-build`,
        // which unconditionally does `require('path')`/`require('os')`/
        // `require('fs')` at the top of its own real, unmodified source
        // (`node-gyp-build.js`) before it ever gets to the actual
        // native-addon lookup. Pure string manipulation, no dependency on
        // any crate -- `path` never touches a real filesystem in Node
        // either (that's what `fs` is for).
        "path" => Some(
            "function __thaw_path_normalize(p) {\n\
             \x20\x20p = String(p); if (p === '') return '.'; var parts = p.split('/'); var out = []; var trailing = p.length > 1 && p.charAt(p.length - 1) === '/';\n\
             \x20\x20var abs = p.charAt(0) === '/';\n\
             \x20\x20for (var i = 0; i < parts.length; i++) {\n\
             \x20\x20\x20\x20var part = parts[i];\n\
             \x20\x20\x20\x20if (part === '' || part === '.') continue;\n\
             \x20\x20\x20\x20if (part === '..') { if (out.length && out[out.length - 1] !== '..') out.pop(); else if (!abs) out.push('..'); } else { out.push(part); }\n\
             \x20\x20}\n\
             \x20\x20var result = (abs ? '/' : '') + out.join('/'); if (result === '') result = abs ? '/' : '.'; if (trailing && result !== '/') result += '/'; return result;\n\
             }\n\
             function resolve() {\n\
             \x20\x20var p = '';\n\
             \x20\x20for (var i = 0; i < arguments.length; i++) {\n\
             \x20\x20\x20\x20var seg = String(arguments[i]);\n\
             \x20\x20\x20\x20if (seg.charAt(0) === '/') { p = seg; } else { p = p ? p + '/' + seg : seg; }\n\
             \x20\x20}\n\
             \x20\x20if (p.charAt(0) !== '/') p = (globalThis.process && process.cwd ? process.cwd() : '/') + '/' + p;\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20return n.charAt(0) === '/' ? n : '/' + n;\n\
             }\n\
             function join() {\n\
             \x20\x20return __thaw_path_normalize(Array.prototype.join.call(arguments, '/'));\n\
             }\n\
             function dirname(p) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20if (idx <= 0) return n.charAt(0) === '/' ? '/' : '.';\n\
             \x20\x20return n.substring(0, idx);\n\
             }\n\
             function basename(p, suffix) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20var base = idx === -1 ? n : n.substring(idx + 1); if (suffix && base.endsWith(String(suffix))) base = base.substring(0, base.length - String(suffix).length); return base;\n\
             }\n\
             function extname(p) { var base = basename(p); var index = base.lastIndexOf('.'); return index <= 0 ? '' : base.substring(index); }\n\
             function isAbsolute(p) { return String(p).charAt(0) === '/'; }\n\
             function relative(from, to) { var left = resolve(from).split('/').filter(Boolean); var right = resolve(to).split('/').filter(Boolean); var shared = 0; while (shared < left.length && shared < right.length && left[shared] === right[shared]) shared++; return left.slice(shared).map(function() { return '..'; }).concat(right.slice(shared)).join('/') || ''; }\n\
             function parse(p) { var root = isAbsolute(p) ? '/' : ''; var dir = dirname(p); var base = basename(p); var ext = extname(base); return { root: root, dir: dir, base: base, ext: ext, name: ext ? base.substring(0, base.length - ext.length) : base }; }\n\
             function format(value) { var dir = value.dir || value.root || ''; var base = value.base || String(value.name || '') + String(value.ext || ''); return dir ? (dir === '/' ? '/' : dir + '/') + base : base; }\n\
             function toNamespacedPath(p) { return p; }\n\
             var posix = { resolve: resolve, join: join, dirname: dirname, basename: basename, extname: extname, normalize: __thaw_path_normalize, relative: relative, isAbsolute: isAbsolute, parse: parse, format: format, toNamespacedPath: toNamespacedPath, sep: '/', delimiter: ':' };\n\
             function winInput(p) { return String(p).replace(/\\\\/g, '/').replace(/^([A-Za-z]):/, '/$1:'); }\n\
             function winOutput(p) { return String(p).replace(/^\\/([A-Za-z]:)/, '$1').replace(/\\//g, '\\\\'); }\n\
             var win32 = { resolve: function() { return winOutput(resolve.apply(null, Array.from(arguments, winInput))); }, join: function() { return winOutput(join.apply(null, Array.from(arguments, winInput))); }, dirname: function(p) { return winOutput(dirname(winInput(p))); }, basename: function(p, suffix) { return basename(winInput(p), suffix); }, extname: function(p) { return extname(winInput(p)); }, normalize: function(p) { return winOutput(__thaw_path_normalize(winInput(p))); }, relative: function(from, to) { return winOutput(relative(winInput(from), winInput(to))); }, isAbsolute: function(p) { return /^[A-Za-z]:[\\\\/]|^[\\\\/]{2}/.test(String(p)); }, parse: function(p) { var result = parse(winInput(p)); result.root = /^[A-Za-z]:/.test(String(p)) ? String(p).substring(0, 3) : result.root; result.dir = winOutput(result.dir); return result; }, format: function(value) { return winOutput(format(value)); }, toNamespacedPath: toNamespacedPath, sep: '\\\\', delimiter: ';' };\n\
             module.exports = posix; module.exports.posix = posix; module.exports.win32 = win32;\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "url" => Some(
            "function pathToFileURL(path) {\n\
             \x20\x20var value = String(path);\n\
             \x20\x20if (value.charAt(0) !== '/') value = '/' + value;\n\
             \x20\x20var encoded = value.split('/').map(function(part) { return encodeURIComponent(part); }).join('/');\n\
             \x20\x20return new globalThis.URL('file://' + encoded);\n\
             }\n\
             function fileURLToPath(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20if (url.protocol !== 'file:') throw new TypeError('URL must use the file: protocol');\n\
             \x20\x20if (url.hostname !== '' && url.hostname !== 'localhost') throw new TypeError('file URL host must be empty or localhost');\n\
             \x20\x20if (/%2f|%5c/i.test(url.pathname)) throw new TypeError('file URL path must not include encoded separators');\n\
             \x20\x20return decodeURIComponent(url.pathname);\n\
             }\n\
             function urlToHttpOptions(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20var options = { protocol: url.protocol, hostname: url.hostname, hash: url.hash, search: url.search, pathname: url.pathname, path: url.pathname + url.search, href: url.href };\n\
             \x20\x20if (url.port !== '') options.port = Number(url.port);\n\
             \x20\x20if (url.username !== '' || url.password !== '') options.auth = decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password);\n\
             \x20\x20return options;\n\
             }\n\
             module.exports = { URL: globalThis.URL, URLSearchParams: globalThis.URLSearchParams, pathToFileURL: pathToFileURL, fileURLToPath: fileURLToPath, urlToHttpOptions: urlToHttpOptions };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "querystring" => Some(
            "function escape(value) { return encodeURIComponent(String(value)); }\n\
             function unescape(value) {\n\
             \x20\x20try { return decodeURIComponent(String(value).replace(/\\+/g, ' ')); } catch (_) { return String(value); }\n\
             }\n\
             function primitive(value) {\n\
             \x20\x20return value === null || value === undefined ? '' : (typeof value === 'string' || typeof value === 'number' || typeof value === 'bigint' || typeof value === 'boolean' ? String(value) : '');\n\
             }\n\
             function stringify(object, separator, assignment, options) {\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var encoder = options && typeof options.encodeURIComponent === 'function' ? options.encodeURIComponent : escape;\n\
             \x20\x20if (object === null || typeof object !== 'object') return '';\n\
             \x20\x20var fields = [];\n\
             \x20\x20Object.keys(object).forEach(function(key) {\n\
             \x20\x20\x20\x20var values = Array.isArray(object[key]) ? object[key] : [object[key]];\n\
             \x20\x20\x20\x20if (values.length === 0) return;\n\
             \x20\x20\x20\x20values.forEach(function(value) { fields.push(encoder(key) + assignment + encoder(primitive(value))); });\n\
             \x20\x20});\n\
             \x20\x20return fields.join(separator);\n\
             }\n\
             function parse(text, separator, assignment, options) {\n\
             \x20\x20var result = Object.create(null);\n\
             \x20\x20var source = String(text);\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var decoder = options && typeof options.decodeURIComponent === 'function' ? options.decodeURIComponent : unescape;\n\
             \x20\x20var maxKeys = options && options.maxKeys !== undefined ? Number(options.maxKeys) : 1000;\n\
             \x20\x20var fields = source === '' ? [] : source.split(separator);\n\
             \x20\x20if (maxKeys > 0) fields = fields.slice(0, maxKeys);\n\
             \x20\x20fields.forEach(function(field) {\n\
             \x20\x20\x20\x20var index = field.indexOf(assignment);\n\
             \x20\x20\x20\x20var key = decoder(index < 0 ? field : field.substring(0, index));\n\
             \x20\x20\x20\x20var value = decoder(index < 0 ? '' : field.substring(index + assignment.length));\n\
             \x20\x20\x20\x20if (!Object.prototype.hasOwnProperty.call(result, key)) result[key] = value;\n\
             \x20\x20\x20\x20else if (Array.isArray(result[key])) result[key].push(value);\n\
             \x20\x20\x20\x20else result[key] = [result[key], value];\n\
             \x20\x20});\n\
             \x20\x20return result;\n\
             }\n\
             module.exports = { stringify: stringify, encode: stringify, parse: parse, decode: parse, escape: escape, unescape: unescape };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "events" => Some(
            "function EventEmitter() {\n\
             \x20\x20if (!(this instanceof EventEmitter)) return new EventEmitter();\n\
             \x20\x20this._events = Object.create(null); this._maxListeners = undefined;\n\
             }\n\
             EventEmitter.prototype._add = function(event, listener, prepend, once) {\n\
             \x20\x20if (typeof listener !== 'function') throw new TypeError('listener must be a function');\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name] || (this._events[name] = []);\n\
             \x20\x20var entry = { listener: listener, once: Boolean(once) };\n\
             \x20\x20if (prepend) list.unshift(entry); else list.push(entry);\n\
             \x20\x20return this;\n\
             };\n\
             EventEmitter.prototype.addListener = EventEmitter.prototype.on = function(event, listener) { return this._add(event, listener, false, false); };\n\
             EventEmitter.prototype.once = function(event, listener) { return this._add(event, listener, false, true); };\n\
             EventEmitter.prototype.prependListener = function(event, listener) { return this._add(event, listener, true, false); };\n\
             EventEmitter.prototype.prependOnceListener = function(event, listener) { return this._add(event, listener, true, true); };\n\
             EventEmitter.prototype.emit = function(event) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name];\n\
             \x20\x20if (!list || list.length === 0) {\n\
             \x20\x20\x20\x20if (name === 'error') { var error = arguments[1]; throw error instanceof Error ? error : new Error('Unhandled error event'); }\n\
             \x20\x20\x20\x20return false;\n\
             \x20\x20}\n\
             \x20\x20var args = Array.prototype.slice.call(arguments, 1);\n\
             \x20\x20list.slice().forEach(function(entry) {\n\
             \x20\x20\x20\x20if (entry.once) this.removeListener(name, entry.listener);\n\
             \x20\x20\x20\x20entry.listener.apply(this, args);\n\
             \x20\x20}, this);\n\
             \x20\x20return true;\n\
             };\n\
             EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(event, listener) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name]; if (!list) return this;\n\
             \x20\x20for (var index = list.length - 1; index >= 0; index--) if (list[index].listener === listener || list[index].listener.listener === listener) { list.splice(index, 1); break; }\n\
             \x20\x20if (list.length === 0) delete this._events[name]; return this;\n\
             };\n\
             EventEmitter.prototype.removeAllListeners = function(event) { if (event === undefined) this._events = Object.create(null); else delete this._events[typeof event === 'symbol' ? event : String(event)]; return this; };\n\
             EventEmitter.prototype.listeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener.listener || entry.listener; }); };\n\
             EventEmitter.prototype.rawListeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener; }); };\n\
             EventEmitter.prototype.listenerCount = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)]; return list ? list.length : 0; };\n\
             EventEmitter.prototype.eventNames = function() { return Reflect.ownKeys(this._events); };\n\
             EventEmitter.prototype.setMaxListeners = function(value) { value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); this._maxListeners = value; return this; }; EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? EventEmitter.defaultMaxListeners : this._maxListeners; }; EventEmitter.defaultMaxListeners = 10;\n\
             EventEmitter.listenerCount = function(emitter, event) { return emitter.listenerCount(event); };\n\
             function once(emitter, event, options) {\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } emitter.once(event, done); if (event !== 'error') emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(emitter, event, options) { options = options || {}; var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } emitter.on(event, received); if (event !== 'error') emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { return typeof emitter.listeners === 'function' ? emitter.listeners(event) : []; } function getMaxListeners(emitter) { return typeof emitter.getMaxListeners === 'function' ? emitter.getMaxListeners() : EventEmitter.defaultMaxListeners; } function setMaxListeners(value) { var emitters = Array.prototype.slice.call(arguments, 1); value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = Symbol.for('events.errorMonitor'); module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        ),
        "assert" | "assert/strict" => Some(
            "function AssertionError(options) {\n\
             \x20\x20options = options || {}; this.name = 'AssertionError'; this.code = 'ERR_ASSERTION';\n\
             \x20\x20this.actual = options.actual; this.expected = options.expected; this.operator = options.operator;\n\
             \x20\x20this.generatedMessage = options.message === undefined;\n\
             \x20\x20this.message = options.message === undefined ? 'Expected values to satisfy ' + (options.operator || 'assertion') : String(options.message);\n\
             \x20\x20if (Error.captureStackTrace) Error.captureStackTrace(this, options.stackStartFn || AssertionError);\n\
             }\n\
             AssertionError.prototype = Object.create(Error.prototype); AssertionError.prototype.constructor = AssertionError;\n\
             function failure(actual, expected, message, operator, start) { throw new AssertionError({ actual: actual, expected: expected, message: message, operator: operator, stackStartFn: start }); }\n\
             function deep(actual, expected, seen) {\n\
             \x20\x20if (Object.is(actual, expected)) return true;\n\
             \x20\x20if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;\n\
             \x20\x20if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;\n\
             \x20\x20seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);\n\
             \x20\x20if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();\n\
             \x20\x20if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;\n\
             \x20\x20if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }\n\
             \x20\x20var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;\n\
             \x20\x20for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !deep(actual[key], expected[key], seen)) return false; }\n\
             \x20\x20return true;\n\
             }\n\
             function ok(value, message) { if (!value) failure(value, true, message, '==', ok); }\n\
             function equal(actual, expected, message) { if (actual != expected) failure(actual, expected, message, '==', equal); }\n\
             function notEqual(actual, expected, message) { if (actual == expected) failure(actual, expected, message, '!=', notEqual); }\n\
             function strictEqual(actual, expected, message) { if (!Object.is(actual, expected)) failure(actual, expected, message, 'strictEqual', strictEqual); }\n\
             function notStrictEqual(actual, expected, message) { if (Object.is(actual, expected)) failure(actual, expected, message, 'notStrictEqual', notStrictEqual); }\n\
             function deepStrictEqual(actual, expected, message) { if (!deep(actual, expected)) failure(actual, expected, message, 'deepStrictEqual', deepStrictEqual); }\n\
             function notDeepStrictEqual(actual, expected, message) { if (deep(actual, expected)) failure(actual, expected, message, 'notDeepStrictEqual', notDeepStrictEqual); }\n\
             function fail(message) { failure(undefined, undefined, message || 'Failed', 'fail', fail); }\n\
             function matches(error, expected) { if (expected === undefined) return true; if (expected instanceof RegExp) return expected.test(String(error && error.message || error)); if (typeof expected === 'function') return error instanceof expected || expected(error) === true; return true; }\n\
             function throws(block, expected, message) { var caught; try { block(); } catch (error) { caught = error; } if (caught === undefined || !matches(caught, expected)) failure(caught, expected, message, 'throws', throws); return caught; }\n\
             function doesNotThrow(block, expected, message) { try { block(); } catch (error) { if (matches(error, expected)) failure(error, undefined, message, 'doesNotThrow', doesNotThrow); throw error; } }\n\
             ok.AssertionError = AssertionError; ok.ok = ok; ok.equal = equal; ok.notEqual = notEqual; ok.strictEqual = strictEqual; ok.notStrictEqual = notStrictEqual;\n\
             ok.deepEqual = deepStrictEqual; ok.notDeepEqual = notDeepStrictEqual; ok.deepStrictEqual = deepStrictEqual; ok.notDeepStrictEqual = notDeepStrictEqual;\n\
             ok.fail = fail; ok.throws = throws; ok.doesNotThrow = doesNotThrow; ok.strict = ok; ok.default = ok; ok.__esModule = true;\n\
             module.exports = ok;\n",
        ),
        // Same real dependency chain as `path` above (`node-gyp-build.js`
        // reads `os.arch()`/`os.platform()` to build its target string).
        "os" => Some(
            "var info = JSON.parse(__thaw_os_info()); var __thaw_os = { arch: function() { return info.arch; }, platform: function() { return info.platform; }, type: function() { return info.type; }, tmpdir: function() { return info.tmpdir; }, homedir: function() { return info.homedir; }, hostname: function() { return info.hostname; }, release: function() { return info.release; }, version: function() { return info.version; }, machine: function() { return info.machine; }, endianness: function() { return info.endianness; }, cpus: function() { return structuredClone(info.cpus); }, totalmem: function() { return info.totalmem; }, freemem: function() { return info.freemem; }, uptime: function() { return info.uptime; }, loadavg: function() { return info.loadavg.slice(); }, userInfo: function() { return structuredClone(info.userInfo); }, networkInterfaces: function() { return { lo: [{ address: '127.0.0.1', netmask: '255.0.0.0', family: 'IPv4', mac: '00:00:00:00:00:00', internal: true, cidr: '127.0.0.1/8' }, { address: '::1', netmask: 'ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff', family: 'IPv6', mac: '00:00:00:00:00:00', internal: true, cidr: '::1/128', scopeid: 0 }] }; }, getPriority: function() { return 0; }, setPriority: function() {}, EOL: info.platform === 'win32' ? '\\r\\n' : '\\n', devNull: info.platform === 'win32' ? '\\\\\\\\.\\\\nul' : '/dev/null', constants: { signals: { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGILL: 4, SIGTRAP: 5, SIGABRT: 6, SIGBUS: 7, SIGFPE: 8, SIGKILL: 9, SIGUSR1: 10, SIGSEGV: 11, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 }, errno: { EACCES: 13, EADDRINUSE: 98, ECONNREFUSED: 111, EEXIST: 17, EINVAL: 22, ENOENT: 2, ENOMEM: 12, ENOTDIR: 20, ETIMEDOUT: 110 } } };\n\
             module.exports = __thaw_os;\n\
             module.exports.default = __thaw_os;\n\
             module.exports.__esModule = true;\n",
        ),
        // Same chain again: `node-gyp-build.js` uses `fs.readdirSync` to
        // look for a prebuilt `.node` binary in `build/Release`,
        // `build/Debug`, and `prebuilds/`, always wrapped in its own
        // `try { fs.readdirSync(...) } catch (err) { return [] }` --
        // consistently reporting "nothing here" (the honest answer: the
        // registry never bundles real binaries, only `package.d.ts`/
        // `bundle.js`, see the "not yet supported" section) lets that real,
        // unmodified upstream logic run to completion and produce its own
        // specific `Error('No native build was found for ...')` -- a much
        // clearer failure than a generic "require('fs') is not supported"
        // would be, without Thaw needing to know anything about native
        // addons itself.
        "fs" => Some(
            "var stream = require('node:stream'); function pathValue(path) { return path instanceof URL ? decodeURIComponent(path.pathname) : String(path); } function invoke(operation, path, value, recursive) { path = pathValue(path); var record = JSON.parse(globalThis.__thaw_fs(operation, path, value || '', Boolean(recursive))); if (!record.ok) { var error = new Error(record.code + ': ' + record.message + ', ' + operation + ' ' + path); error.code = record.code; error.errno = -1; error.path = path; error.syscall = operation; throw error; } return record; } function encoding(options) { return typeof options === 'string' ? options : options && options.encoding; } function data(value, options) { return Buffer.isBuffer(value) || value instanceof Uint8Array ? Buffer.from(value) : Buffer.from(String(value), encoding(options)); } function exclusiveError(path, syscall) { var error = new Error('EEXIST: path already exists, ' + syscall + ' ' + pathValue(path)); error.code = 'EEXIST'; error.errno = -1; error.path = pathValue(path); error.syscall = syscall; return error; } function timeValue(value) { if (value instanceof Date) return value.getTime() / 1000; var number = Number(value); if (Number.isFinite(number)) return number; return new Date(value).getTime() / 1000; } function Stats(record, bigint) { var convert = bigint ? function(value) { return BigInt(Math.trunc(Number(value))); } : Number, numeric = ['dev','ino','mode','nlink','uid','gid','rdev','blksize','blocks']; this.size = convert(record.length); for (var index = 0; index < numeric.length; index++) this[numeric[index]] = convert(record[numeric[index]]); var atime = Number(record.atimeMs), mtime = Number(record.mtimeMs), ctime = Number(record.ctimeMs), birthtime = Number(record.birthtimeMs); this.atimeMs = convert(atime); this.mtimeMs = convert(mtime); this.ctimeMs = convert(ctime); this.birthtimeMs = convert(birthtime); if (bigint) { this.atimeNs = BigInt(Math.round(atime * 1000000)); this.mtimeNs = BigInt(Math.round(mtime * 1000000)); this.ctimeNs = BigInt(Math.round(ctime * 1000000)); this.birthtimeNs = BigInt(Math.round(birthtime * 1000000)); } this.atime = new Date(atime); this.mtime = new Date(mtime); this.ctime = new Date(ctime); this.birthtime = new Date(birthtime); this._file = record.file; this._directory = record.directory; this._symlink = Boolean(record.symlink); } Stats.prototype.isFile = function() { return this._file; }; Stats.prototype.isDirectory = function() { return this._directory; }; Stats.prototype.isSymbolicLink = function() { return this._symlink; }; Stats.prototype.isSocket = Stats.prototype.isFIFO = Stats.prototype.isCharacterDevice = Stats.prototype.isBlockDevice = function() { return false; };\n\
             var __thaw_fs = { existsSync: function(path) { return invoke('exists', path).exists; }, readFileSync: function(path, options) { var output = Buffer.from(invoke('read', path).data, 'hex'), target = encoding(options); return target ? output.toString(target) : output; }, writeFileSync: function(path, value, options) { var flag = options && typeof options === 'object' && options.flag || 'w'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke(String(flag)[0] === 'a' ? 'append' : 'write', path, data(value, options).toString('hex')); }, appendFileSync: function(path, value, options) { var flag = options && typeof options === 'object' && options.flag || 'a'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke('append', path, data(value, options).toString('hex')); }, mkdirSync: function(path, options) { invoke('mkdir', path, '', options && options.recursive); }, readdirSync: function(path, options) { options = options || {}; var root = pathValue(path), output = [], targetEncoding = encoding(options); function visit(directory, relative) { invoke('readdir', directory).entries.forEach(function(rawName) { var full = pathValue(directory).replace(/[/]$/, '') + '/' + rawName, name = relative ? relative + '/' + rawName : rawName, stat = invoke('lstat', full); if (options.withFileTypes) { var encodedName = targetEncoding === 'buffer' ? Buffer.from(rawName) : rawName; output.push({ name: encodedName, parentPath: pathValue(directory), path: pathValue(directory), isFile: function() { return stat.file; }, isDirectory: function() { return stat.directory; }, isSymbolicLink: function() { return Boolean(stat.symlink); } }); } else output.push(targetEncoding === 'buffer' ? Buffer.from(name) : name); if (options.recursive && stat.directory && !stat.symlink) visit(full, name); }); } visit(root, ''); return output; }, statSync: function(path, options) { return new Stats(invoke('stat', path), Boolean(options && options.bigint)); }, lstatSync: function(path, options) { return new Stats(invoke('lstat', path), Boolean(options && options.bigint)); }, unlinkSync: function(path) { invoke('unlink', path); }, rmSync: function(path, options) { var stat; try { stat = invoke('stat', path); } catch (error) { if (options && options.force && error.code === 'ENOENT') return; throw error; } invoke(stat.directory ? 'rmdir' : 'unlink', path, '', options && options.recursive); }, rmdirSync: function(path, options) { invoke('rmdir', path, '', options && options.recursive); }, renameSync: function(oldPath, newPath) { invoke('rename', oldPath, pathValue(newPath)); } };\n\
             __thaw_fs.copyFileSync = function(source, destination, mode) { destination = pathValue(destination); if ((Number(mode) & __thaw_fs.constants.COPYFILE_EXCL) && __thaw_fs.existsSync(destination)) { var error = new Error('EEXIST: destination already exists, copyfile ' + destination); error.code = 'EEXIST'; error.errno = -1; error.path = destination; error.syscall = 'copyfile'; throw error; } invoke('copy', source, destination); }; __thaw_fs.cpSync = function(source, destination, options) { options = options || {}; destination = pathValue(destination); if (__thaw_fs.existsSync(destination)) { if (options.force === false) { if (options.errorOnExist) { var error = new Error('EEXIST: destination already exists, cp ' + destination); error.code = 'EEXIST'; error.path = destination; error.syscall = 'cp'; throw error; } return; } __thaw_fs.rmSync(destination, { recursive: true, force: true }); } invoke('cp', source, destination, Boolean(options.recursive)); }; __thaw_fs.realpathSync = function(path, options) { var result = invoke('realpath', path).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.realpathSync.native = __thaw_fs.realpathSync; __thaw_fs.mkdtempSync = function(prefix, options) { var result = invoke('mkdtemp', prefix).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.mkdtempDisposableSync = function(prefix, options) { var path = __thaw_fs.mkdtempSync(prefix, options), removed = false, disposable = { path: path, remove: function() { if (!removed) { __thaw_fs.rmSync(path, { recursive: true, force: true }); removed = true; } } }; if (Symbol.dispose) disposable[Symbol.dispose] = disposable.remove; return disposable; }; function protectFileBlob(blob, verify) { var readArrayBuffer = blob.arrayBuffer.bind(blob), readBytes = blob.bytes.bind(blob), readText = blob.text.bind(blob), makeSlice = blob.slice.bind(blob), makeStream = blob.stream.bind(blob); blob.arrayBuffer = function() { return Promise.resolve().then(verify).then(readArrayBuffer); }; blob.bytes = function() { return Promise.resolve().then(verify).then(readBytes); }; blob.text = function() { return Promise.resolve().then(verify).then(readText); }; blob.slice = function(start, end, type) { return protectFileBlob(makeSlice(start, end, type), verify); }; blob.stream = function() { var stream = makeStream(); return { getReader: function() { var reader = stream.getReader(); return { read: function() { return Promise.resolve().then(verify).then(function() { return reader.read(); }); }, releaseLock: function() { return reader.releaseLock(); } }; }, [Symbol.asyncIterator]: function() { var iterator = stream[Symbol.asyncIterator](); return { next: function() { return Promise.resolve().then(verify).then(function() { return iterator.next(); }); }, [Symbol.asyncIterator]: function() { return this; } }; } }; }; return blob; } __thaw_fs.openAsBlob = function(path, options) { return Promise.resolve().then(function() { var initial = __thaw_fs.statSync(path), blob = new Blob([__thaw_fs.readFileSync(path)], { type: options && options.type }); function verify() { var current; try { current = __thaw_fs.statSync(path); } catch (error) { throw new DOMException('The blob could not be read', 'NotReadableError'); } if (current.size !== initial.size || current.mtimeMs !== initial.mtimeMs) throw new DOMException('The blob could not be read', 'NotReadableError'); } return protectFileBlob(blob, verify); }); };\n\
             __thaw_fs.linkSync = function(existingPath, newPath) { invoke('link', existingPath, pathValue(newPath)); }; __thaw_fs.symlinkSync = function(target, path) { invoke('symlink', target, pathValue(path)); }; __thaw_fs.readlinkSync = function(path, options) { var result = invoke('readlink', path).path; return encoding(options) === 'buffer' ? Buffer.from(result) : result; }; __thaw_fs.chmodSync = function(path, mode) { if (typeof mode === 'string') mode = parseInt(mode, 8); invoke('chmod', path, String(Number(mode))); }; __thaw_fs.utimesSync = function(path, atime, mtime) { invoke('utimes', path, String(timeValue(atime)) + ',' + String(timeValue(mtime))); }; __thaw_fs.lutimesSync = function(path, atime, mtime) { invoke('lutimes', path, String(timeValue(atime)) + ',' + String(timeValue(mtime))); }; __thaw_fs.chownSync = function(path, uid, gid) { invoke('chown', path, String(Number(uid)) + ',' + String(Number(gid))); }; __thaw_fs.lchownSync = function(path, uid, gid) { invoke('lchown', path, String(Number(uid)) + ',' + String(Number(gid))); }; __thaw_fs.truncateSync = function(path, length) { invoke('truncate', path, String(length === undefined ? 0 : Number(length))); }; __thaw_fs.accessSync = function(path, mode) { invoke('access', path, String(mode === undefined ? __thaw_fs.constants.F_OK : Number(mode))); }; function StatFs(record, bigint) { var convert = bigint ? function(value) { return BigInt(Math.trunc(Number(value))); } : Number; this.type = convert(record.type); this.bsize = convert(record.bsize); this.blocks = convert(record.blocks); this.bfree = convert(record.bfree); this.bavail = convert(record.bavail); this.files = convert(record.files); this.ffree = convert(record.ffree); } __thaw_fs.StatFs = StatFs; __thaw_fs.statfsSync = function(path, options) { return new StatFs(invoke('statfs', path), Boolean(options && options.bigint)); }; var statWatchers = new Map(); function StatWatcher(path, options) { this.path = pathValue(path); this._listeners = []; this._refed = !options || options.persistent !== false; this._previous = __thaw_fs.statSync(this.path); var self = this; this._timer = setInterval(function() { var current; try { current = __thaw_fs.statSync(self.path); } catch (error) { return; } var previous = self._previous; if (current.mtimeMs !== previous.mtimeMs || current.ctimeMs !== previous.ctimeMs || current.size !== previous.size) { self._previous = current; self._listeners.slice().forEach(function(listener) { listener(current, previous); }); } }, Math.max(1, Number(options && options.interval || 5007))); } StatWatcher.prototype.ref = function() { this._refed = true; return this; }; StatWatcher.prototype.unref = function() { this._refed = false; return this; }; StatWatcher.prototype.hasRef = function() { return this._refed; }; StatWatcher.prototype.stop = function() { if (this._timer !== null) { clearInterval(this._timer); this._timer = null; statWatchers.delete(this.path); } return this; }; __thaw_fs.StatWatcher = StatWatcher; __thaw_fs.watchFile = function(path, options, listener) { if (typeof options === 'function') { listener = options; options = {}; } if (typeof listener !== 'function') throw new TypeError('listener must be a function'); path = pathValue(path); var watcher = statWatchers.get(path); if (!watcher) { watcher = new StatWatcher(path, options || {}); statWatchers.set(path, watcher); } if (watcher._listeners.indexOf(listener) < 0) watcher._listeners.push(listener); return watcher; }; __thaw_fs.unwatchFile = function(path, listener) { var watcher = statWatchers.get(pathValue(path)); if (!watcher) return; if (typeof listener === 'function') watcher._listeners = watcher._listeners.filter(function(value) { return value !== listener; }); else watcher._listeners = []; if (!watcher._listeners.length) watcher.stop(); };\n\
             function Dir(path) { this.path = pathValue(path); this.closed = false; this._index = 0; this._entries = __thaw_fs.readdirSync(this.path, { withFileTypes: true }); } Dir.prototype._check = function() { if (this.closed) { var error = new Error('Directory handle was closed'); error.code = 'ERR_DIR_CLOSED'; throw error; } }; Dir.prototype.readSync = function() { this._check(); return this._index < this._entries.length ? this._entries[this._index++] : null; }; Dir.prototype.read = function(callback) { var self = this, operation = Promise.resolve().then(function() { return self.readSync(); }); if (typeof callback === 'function') { operation.then(function(value) { callback(null, value); }, callback); return; } return operation; }; Dir.prototype.closeSync = function() { this._check(); this.closed = true; }; Dir.prototype.close = function(callback) { var self = this, operation = Promise.resolve().then(function() { self.closeSync(); }); if (typeof callback === 'function') { operation.then(function() { callback(null); }, callback); return; } return operation; }; Dir.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { return self.read().then(function(value) { if (value === null) { if (!self.closed) self.closeSync(); return { value: undefined, done: true }; } return { value: value, done: false }; }); } }; }; __thaw_fs.Dir = Dir; __thaw_fs.opendirSync = function(path) { invoke('readdir', path); return new Dir(path); };\n\
             function ReadStream(path, options) { if (!(this instanceof ReadStream)) return new ReadStream(path, options); options = options || {}; stream.Readable.call(this, options); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesRead = 0; this.close = this.destroy.bind(this); var self = this; queueMicrotask(function() { try { var value = Buffer.from(invoke('read', self.path).data, 'hex'), start = options.start === undefined ? 0 : Number(options.start), end = options.end === undefined ? value.length - 1 : Number(options.end), selected = value.subarray(start, Math.min(value.length, end + 1)), size = Math.max(1, Number(options.highWaterMark || 65536)); self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); for (var offset = 0; offset < selected.length; offset += size) { var chunk = selected.subarray(offset, offset + size); self.bytesRead += chunk.length; self.push(chunk); } self.push(null); if (options.autoClose !== false) queueMicrotask(function() { self.fd = null; self.emit('close'); }); } catch (error) { self.pending = false; self.destroy(error); } }); } ReadStream.prototype = Object.create(stream.Readable.prototype); ReadStream.prototype.constructor = ReadStream;\n\
             function WriteStream(path, options) { if (!(this instanceof WriteStream)) return new WriteStream(path, options); options = options || {}; this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesWritten = 0; var chunks = [], self = this; stream.Writable.call(this, { write: function(chunk, encoding, callback) { chunks.push(Buffer.from(chunk)); self.bytesWritten += chunk.length; callback(); }, final: function(callback) { try { var value = Buffer.concat(chunks); invoke(options.flags === 'a' || options.flags === 'ax' ? 'append' : 'write', self.path, value.toString('hex')); callback(); if (options.autoClose !== false) queueMicrotask(function() { self.fd = null; self.emit('close'); }); } catch (error) { callback(error); } } }); queueMicrotask(function() { self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); }); } WriteStream.prototype = Object.create(stream.Writable.prototype); WriteStream.prototype.constructor = WriteStream; WriteStream.prototype.close = function(callback) { if (!this.writableEnded) this.end(callback); else if (callback) queueMicrotask(callback); }; __thaw_fs.ReadStream = ReadStream; __thaw_fs.WriteStream = WriteStream; __thaw_fs.createReadStream = function(path, options) { return new ReadStream(path, options); }; __thaw_fs.createWriteStream = function(path, options) { return new WriteStream(path, options); };\n\
             var fileDescriptors = new Map(), nextFileHandle = 10; function FileHandle(path, flags) { this.path = pathValue(path); this.flags = String(flags || 'r'); this.fd = nextFileHandle++; this.closed = false; this._position = 0; if (this.flags.indexOf('x') >= 0 && __thaw_fs.existsSync(this.path)) throw exclusiveError(this.path, 'open'); if (this.flags[0] === 'w') invoke('write', this.path, ''); else if (this.flags[0] === 'a') invoke('append', this.path, ''); else invoke('stat', this.path); fileDescriptors.set(this.fd, this); } FileHandle.prototype._check = function() { if (this.closed) { var error = new Error('file closed'); error.code = 'EBADF'; throw error; } }; FileHandle.prototype.close = function() { if (!this.closed) { fileDescriptors.delete(this.fd); this.closed = true; this.fd = -1; } return Promise.resolve(); }; FileHandle.prototype.readFile = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.readFileSync(self.path, options); }); }; FileHandle.prototype.writeFile = function(value, options) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.writeFileSync(self.path, value, options); }); }; FileHandle.prototype.appendFile = function(value, options) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.appendFileSync(self.path, value, options); }); }; FileHandle.prototype.read = function(buffer, offset, length, position) { var self = this; if (buffer && !Buffer.isBuffer(buffer) && !(buffer instanceof Uint8Array)) { var options = buffer; buffer = options.buffer || Buffer.alloc(options.length || 16384); offset = options.offset; length = options.length; position = options.position; } buffer = buffer || Buffer.alloc(16384); offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? buffer.length - offset : Number(length); return Promise.resolve().then(function() { self._check(); var source = __thaw_fs.readFileSync(self.path), start = position === null || position === undefined ? self._position : Number(position), count = Math.max(0, Math.min(length, source.length - start)); if (count) buffer.set(source.subarray(start, start + count), offset); if (position === null || position === undefined) self._position += count; return { bytesRead: count, buffer: buffer }; }); }; FileHandle.prototype.write = function(value, offset, length, position) { var self = this, stringValue = typeof value === 'string', resultValue = value; if (stringValue) { var encodingName = typeof length === 'string' ? length : 'utf8'; position = offset; value = Buffer.from(value, encodingName); offset = 0; length = value.length; } else { value = Buffer.from(value); resultValue = value; offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? value.length - offset : Number(length); } return Promise.resolve().then(function() { self._check(); var original = __thaw_fs.readFileSync(self.path), chunk = value.subarray(offset, offset + length), start = position === null || position === undefined ? (self.flags[0] === 'a' ? original.length : self._position) : Number(position), size = Math.max(original.length, start + chunk.length), output = Buffer.alloc(size); output.set(original, 0); output.set(chunk, start); __thaw_fs.writeFileSync(self.path, output); if (position === null || position === undefined) self._position = start + chunk.length; return { bytesWritten: chunk.length, buffer: resultValue }; }); }; FileHandle.prototype.stat = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.statSync(self.path, options); }); }; FileHandle.prototype.truncate = function(length) { var self = this; return Promise.resolve().then(function() { self._check(); invoke('truncate', self.path, String(length === undefined ? 0 : length)); }); }; FileHandle.prototype.createReadStream = function(options) { this._check(); return new ReadStream(this.path, options); }; FileHandle.prototype.createWriteStream = function(options) { this._check(); return new WriteStream(this.path, Object.assign({ flags: this.flags }, options || {})); }; FileHandle.prototype[Symbol.asyncDispose] = FileHandle.prototype.close; __thaw_fs.FileHandle = FileHandle;\n\
             __thaw_fs.constants = globalThis.__thaw_fs_constants || (globalThis.__thaw_fs_constants = { F_OK: 0, X_OK: 1, W_OK: 2, R_OK: 4, O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128, O_NOCTTY: 256, O_TRUNC: 512, O_APPEND: 1024, O_DIRECTORY: 65536, O_NOFOLLOW: 131072, O_SYNC: 1052672, S_IFMT: 61440, S_IFREG: 32768, S_IFDIR: 16384, S_IFCHR: 8192, S_IFBLK: 24576, S_IFIFO: 4096, S_IFLNK: 40960, S_IFSOCK: 49152, COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4 });\n\
             function fdHandle(fd) { var handle = fileDescriptors.get(Number(fd)); if (!handle || handle.closed) { var error = new Error('EBADF: bad file descriptor'); error.code = 'EBADF'; error.errno = -1; error.syscall = 'fd'; throw error; } return handle; } __thaw_fs.openSync = function(path, flags) { return new FileHandle(path, flags).fd; }; __thaw_fs.closeSync = function(fd) { var handle = fdHandle(fd); fileDescriptors.delete(handle.fd); handle.closed = true; handle.fd = -1; }; __thaw_fs.readSync = function(fd, buffer, offset, length, position) { var handle = fdHandle(fd), source = __thaw_fs.readFileSync(handle.path), start = position === null || position === undefined ? handle._position : Number(position), count = Math.max(0, Math.min(Number(length), source.length - start)); if (count) buffer.set(source.subarray(start, start + count), Number(offset)); if (position === null || position === undefined) handle._position += count; return count; }; __thaw_fs.writeSync = function(fd, value, offset, length, position) { var handle = fdHandle(fd), stringValue = typeof value === 'string'; if (stringValue) { var encodingName = typeof length === 'string' ? length : 'utf8'; position = offset; value = Buffer.from(value, encodingName); offset = 0; length = value.length; } else { value = Buffer.from(value); offset = offset === undefined ? 0 : Number(offset); length = length === undefined ? value.length - offset : Number(length); } var original = __thaw_fs.readFileSync(handle.path), chunk = value.subarray(offset, offset + length), start = position === null || position === undefined ? (handle.flags[0] === 'a' ? original.length : handle._position) : Number(position), output = Buffer.alloc(Math.max(original.length, start + chunk.length)); output.set(original); output.set(chunk, start); __thaw_fs.writeFileSync(handle.path, output); if (position === null || position === undefined) handle._position = start + chunk.length; return chunk.length; }; __thaw_fs.fstatSync = function(fd, options) { return __thaw_fs.statSync(fdHandle(fd).path, options); }; __thaw_fs.ftruncateSync = function(fd, length) { invoke('truncate', fdHandle(fd).path, String(length === undefined ? 0 : length)); }; __thaw_fs.fchmodSync = function(fd, mode) { __thaw_fs.chmodSync(fdHandle(fd).path, mode); }; __thaw_fs.fchownSync = function(fd, uid, gid) { __thaw_fs.chownSync(fdHandle(fd).path, uid, gid); }; __thaw_fs.futimesSync = function(fd, atime, mtime) { __thaw_fs.utimesSync(fdHandle(fd).path, atime, mtime); }; __thaw_fs.fsyncSync = __thaw_fs.fdatasyncSync = function(fd) { fdHandle(fd); }; __thaw_fs.readvSync = function(fd, buffers, position) { var total = 0, cursor = position; buffers.forEach(function(buffer) { var count = __thaw_fs.readSync(fd, buffer, 0, buffer.length, cursor); total += count; if (cursor !== null && cursor !== undefined) cursor = Number(cursor) + count; }); return total; }; __thaw_fs.writevSync = function(fd, buffers, position) { var total = 0, cursor = position; buffers.forEach(function(buffer) { var count = __thaw_fs.writeSync(fd, buffer, 0, buffer.length, cursor); total += count; if (cursor !== null && cursor !== undefined) cursor = Number(cursor) + count; }); return total; };;\n\
             function promised(method) { return function() { var args = arguments; return new Promise(function(resolve, reject) { queueMicrotask(function() { try { resolve(method.apply(__thaw_fs, args)); } catch (error) { reject(error); } }); }); }; } __thaw_fs.promises = { access: promised(__thaw_fs.accessSync), open: function(path, flags) { return promised(function() { return new FileHandle(path, flags); })(); }, readFile: promised(__thaw_fs.readFileSync), readdir: promised(__thaw_fs.readdirSync), opendir: promised(__thaw_fs.opendirSync), stat: promised(__thaw_fs.statSync), lstat: promised(__thaw_fs.lstatSync), writeFile: promised(__thaw_fs.writeFileSync), appendFile: promised(__thaw_fs.appendFileSync), mkdir: promised(__thaw_fs.mkdirSync), unlink: promised(__thaw_fs.unlinkSync), rm: promised(__thaw_fs.rmSync), rmdir: promised(__thaw_fs.rmdirSync), rename: promised(__thaw_fs.renameSync), copyFile: promised(__thaw_fs.copyFileSync), cp: promised(__thaw_fs.cpSync), realpath: promised(__thaw_fs.realpathSync), mkdtemp: promised(__thaw_fs.mkdtempSync), statfs: promised(__thaw_fs.statfsSync), lutimes: promised(__thaw_fs.lutimesSync), truncate: promised(__thaw_fs.truncateSync) }; __thaw_fs.promises.mkdtempDisposable = function(prefix, options) { return __thaw_fs.promises.mkdtemp(prefix, options).then(function(path) { var removal, disposable = { path: path, remove: function() { if (!removal) removal = __thaw_fs.promises.rm(path, { recursive: true, force: true }); return removal; } }; if (Symbol.asyncDispose) disposable[Symbol.asyncDispose] = disposable.remove; return disposable; }); }; function callbackMethod(method, valueResult) { return function() { var args = Array.prototype.slice.call(arguments), callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { var value = method.apply(__thaw_fs, args); if (valueResult) callback(null, value); else callback(null); } catch (error) { callback(error); } }); }; } __thaw_fs.readFile = callbackMethod(__thaw_fs.readFileSync, true); __thaw_fs.readdir = callbackMethod(__thaw_fs.readdirSync, true); __thaw_fs.opendir = callbackMethod(__thaw_fs.opendirSync, true); __thaw_fs.stat = callbackMethod(__thaw_fs.statSync, true); __thaw_fs.lstat = callbackMethod(__thaw_fs.lstatSync, true); __thaw_fs.writeFile = callbackMethod(__thaw_fs.writeFileSync, false); __thaw_fs.appendFile = callbackMethod(__thaw_fs.appendFileSync, false); __thaw_fs.mkdir = callbackMethod(__thaw_fs.mkdirSync, false); __thaw_fs.unlink = callbackMethod(__thaw_fs.unlinkSync, false); __thaw_fs.rm = callbackMethod(__thaw_fs.rmSync, false); __thaw_fs.rmdir = callbackMethod(__thaw_fs.rmdirSync, false); __thaw_fs.rename = callbackMethod(__thaw_fs.renameSync, false); __thaw_fs.copyFile = callbackMethod(__thaw_fs.copyFileSync, false); __thaw_fs.cp = callbackMethod(__thaw_fs.cpSync, false); __thaw_fs.realpath = callbackMethod(__thaw_fs.realpathSync, true); __thaw_fs.realpath.native = __thaw_fs.realpath; __thaw_fs.mkdtemp = callbackMethod(__thaw_fs.mkdtempSync, true);\n\
             FileHandle.prototype.utimes = function(atime, mtime) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.utimesSync(self.path, atime, mtime); }); }; FileHandle.prototype.chmod = function(mode) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.chmodSync(self.path, mode); }); }; FileHandle.prototype.sync = function() { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.fsyncSync(self.fd); }); }; FileHandle.prototype.datasync = function() { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.fdatasyncSync(self.fd); }); }; FileHandle.prototype.chown = function(uid, gid) { var self = this; return Promise.resolve().then(function() { self._check(); __thaw_fs.chownSync(self.path, uid, gid); }); }; Object.assign(__thaw_fs.promises, { link: promised(__thaw_fs.linkSync), symlink: promised(__thaw_fs.symlinkSync), readlink: promised(__thaw_fs.readlinkSync), chmod: promised(__thaw_fs.chmodSync), utimes: promised(__thaw_fs.utimesSync), chown: promised(__thaw_fs.chownSync), lchown: promised(__thaw_fs.lchownSync) }); __thaw_fs.link = callbackMethod(__thaw_fs.linkSync, false); __thaw_fs.symlink = callbackMethod(__thaw_fs.symlinkSync, false); __thaw_fs.readlink = callbackMethod(__thaw_fs.readlinkSync, true); __thaw_fs.chmod = callbackMethod(__thaw_fs.chmodSync, false); __thaw_fs.utimes = callbackMethod(__thaw_fs.utimesSync, false); __thaw_fs.lutimes = callbackMethod(__thaw_fs.lutimesSync, false); __thaw_fs.chown = callbackMethod(__thaw_fs.chownSync, false); __thaw_fs.lchown = callbackMethod(__thaw_fs.lchownSync, false); __thaw_fs.access = callbackMethod(__thaw_fs.accessSync, false); __thaw_fs.statfs = callbackMethod(__thaw_fs.statfsSync, true); __thaw_fs.truncate = callbackMethod(__thaw_fs.truncateSync, false); __thaw_fs.open = callbackMethod(__thaw_fs.openSync, true); __thaw_fs.close = callbackMethod(__thaw_fs.closeSync, false); __thaw_fs.fstat = callbackMethod(__thaw_fs.fstatSync, true); __thaw_fs.ftruncate = callbackMethod(__thaw_fs.ftruncateSync, false); __thaw_fs.fchmod = callbackMethod(__thaw_fs.fchmodSync, false); __thaw_fs.fchown = callbackMethod(__thaw_fs.fchownSync, false); __thaw_fs.futimes = callbackMethod(__thaw_fs.futimesSync, false); __thaw_fs.fsync = callbackMethod(__thaw_fs.fsyncSync, false); __thaw_fs.fdatasync = callbackMethod(__thaw_fs.fdatasyncSync, false); __thaw_fs.read = function(fd, buffer, offset, length, position, callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.readSync(fd, buffer, offset, length, position), buffer); } catch (error) { callback(error); } }); }; __thaw_fs.write = function(fd, value, offset, length, position, callback) { var args = Array.prototype.slice.call(arguments), done = args.pop(); if (typeof done !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { done(null, __thaw_fs.writeSync.apply(__thaw_fs, args), value); } catch (error) { done(error); } }); }; FileHandle.prototype.statfs = function(options) { var self = this; return Promise.resolve().then(function() { self._check(); return __thaw_fs.statfsSync(self.path, options); }); }; FileHandle.prototype.readv = function(buffers, position) { var self = this; return Promise.resolve().then(function() { self._check(); return { bytesRead: __thaw_fs.readvSync(self.fd, buffers, position), buffers: buffers }; }); }; FileHandle.prototype.writev = function(buffers, position) { var self = this; return Promise.resolve().then(function() { self._check(); return { bytesWritten: __thaw_fs.writevSync(self.fd, buffers, position), buffers: buffers }; }); }; __thaw_fs.readv = function(fd, buffers, position, callback) { if (typeof position === 'function') { callback = position; position = null; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.readvSync(fd, buffers, position), buffers); } catch (error) { callback(error); } }); }; __thaw_fs.writev = function(fd, buffers, position, callback) { if (typeof position === 'function') { callback = position; position = null; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.writevSync(fd, buffers, position), buffers); } catch (error) { callback(error); } }); };;\n\
             function watchSnapshot(path, recursive, prefix, output) { output = output || {}; prefix = prefix || ''; var stats = __thaw_fs.statSync(path); if (!stats.isDirectory()) { output[prefix || pathValue(path).split('/').pop()] = String(stats.size) + ':' + String(stats.mtimeMs); return output; } __thaw_fs.readdirSync(path, { withFileTypes: true }).forEach(function(entry) { var name = prefix ? prefix + '/' + entry.name : entry.name, full = pathValue(path).replace(/\\/$/, '') + '/' + entry.name, value = __thaw_fs.statSync(full); output[name] = String(value.size) + ':' + String(value.mtimeMs); if (recursive && value.isDirectory()) watchSnapshot(full, true, name, output); }); return output; } function FSWatcher(path, options, listener) { this.path = pathValue(path); this.closed = false; this._refed = options.persistent !== false; this._listeners = { change: [], error: [], close: [] }; if (typeof listener === 'function') this.on('change', listener); this._options = options; this._snapshot = watchSnapshot(this.path, Boolean(options.recursive)); var self = this; this._timer = setInterval(function() { if (self.closed) return; var current; try { current = watchSnapshot(self.path, Boolean(options.recursive)); } catch (error) { self.emit('error', error); self.close(); return; } var previous = self._snapshot, names = Object.keys(Object.assign({}, previous, current)); names.forEach(function(name) { if (!(name in previous) || !(name in current)) self.emit('change', 'rename', options.encoding === 'buffer' ? Buffer.from(name) : name); else if (previous[name] !== current[name]) self.emit('change', 'change', options.encoding === 'buffer' ? Buffer.from(name) : name); }); self._snapshot = current; }, Math.max(1, Number(options.interval || 50))); if (options.signal) { if (options.signal.aborted) this.close(); else options.signal.addEventListener('abort', function() { self.close(); }, { once: true }); } } FSWatcher.prototype.on = FSWatcher.prototype.addListener = function(event, listener) { (this._listeners[event] || (this._listeners[event] = [])).push(listener); return this; }; FSWatcher.prototype.once = function(event, listener) { var self = this, wrapped = function() { self.removeListener(event, wrapped); return listener.apply(self, arguments); }; return this.on(event, wrapped); }; FSWatcher.prototype.removeListener = function(event, listener) { if (this._listeners[event]) this._listeners[event] = this._listeners[event].filter(function(value) { return value !== listener; }); return this; }; FSWatcher.prototype.emit = function(event) { var args = Array.prototype.slice.call(arguments, 1), self = this; (this._listeners[event] || []).slice().forEach(function(listener) { listener.apply(self, args); }); return Boolean((this._listeners[event] || []).length); }; FSWatcher.prototype.close = function() { if (!this.closed) { this.closed = true; clearInterval(this._timer); this._timer = null; this.emit('close'); } return this; }; FSWatcher.prototype.ref = function() { this._refed = true; return this; }; FSWatcher.prototype.unref = function() { this._refed = false; return this; }; FSWatcher.prototype.hasRef = function() { return this._refed; }; __thaw_fs.FSWatcher = FSWatcher; __thaw_fs.watch = function(path, options, listener) { if (typeof options === 'function') { listener = options; options = {}; } else if (typeof options === 'string') options = { encoding: options }; return new FSWatcher(path, options || {}, listener); };\n\
             function globMatch(pattern, value) { var memo = {}; function match(pi, vi) { var key = pi + ':' + vi; if (key in memo) return memo[key]; if (pi === pattern.length) return memo[key] = vi === value.length; if (pattern[pi] === '*') { var double = pattern[pi + 1] === '*'; if (double) { var next = pi + 2; while (pattern[next] === '*') next++; if (pattern[next] === '/' && match(next + 1, vi)) return memo[key] = true; if (match(next, vi)) return memo[key] = true; return memo[key] = vi < value.length && match(pi, vi + 1); } if (match(pi + 1, vi)) return memo[key] = true; return memo[key] = vi < value.length && value[vi] !== '/' && match(pi, vi + 1); } if (pattern[pi] === '?') return memo[key] = vi < value.length && value[vi] !== '/' && match(pi + 1, vi + 1); return memo[key] = vi < value.length && pattern[pi] === value[vi] && match(pi + 1, vi + 1); } return match(0, 0); } function globEntries(root, relative, output) { __thaw_fs.readdirSync(root, { withFileTypes: true }).forEach(function(entry) { var name = relative ? relative + '/' + entry.name : entry.name, full = pathValue(root).replace(/[/]$/, '') + '/' + entry.name; output.push({ name: name, entry: entry }); if (entry.isDirectory()) globEntries(full, name, output); }); return output; } function globExcluded(name, exclude) { if (!exclude) return false; if (typeof exclude === 'function') return Boolean(exclude(name)); return (Array.isArray(exclude) ? exclude : [exclude]).some(function(pattern) { return globMatch(String(pattern), name); }); } __thaw_fs.globSync = function(patterns, options) { options = options || {}; patterns = Array.isArray(patterns) ? patterns : [patterns]; var root = pathValue(options.cwd || process.cwd()), entries = globEntries(root, '', []), seen = {}; return entries.filter(function(record) { if (!patterns.some(function(pattern) { return globMatch(String(pattern), record.name); }) || globExcluded(record.name, options.exclude)) return false; if (seen[record.name]) return false; seen[record.name] = true; return true; }).map(function(record) { return options.withFileTypes ? record.entry : record.name; }); }; __thaw_fs.glob = function(patterns, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); queueMicrotask(function() { try { callback(null, __thaw_fs.globSync(patterns, options)); } catch (error) { callback(error); } }); }; __thaw_fs.promises.glob = function(patterns, options) { var values, index = 0; return { [Symbol.asyncIterator]: function() { return this; }, next: function() { return Promise.resolve().then(function() { if (!values) values = __thaw_fs.globSync(patterns, options); return index < values.length ? { value: values[index++], done: false } : { value: undefined, done: true }; }); } }; };\n\
             __thaw_fs.promises.watch = function(path, options) { options = options || {}; var queue = [], waiters = [], closed = false, failure, watchOptions = Object.assign({}, options); delete watchOptions.signal; var watcher = __thaw_fs.watch(path, watchOptions, function(eventType, filename) { var value = { eventType: eventType, filename: filename }; if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else queue.push(value); }); function finish(error) { if (closed) return; closed = true; failure = error; watcher.close(); while (waiters.length) { var waiter = waiters.shift(); error ? waiter.reject(error) : waiter.resolve({ value: undefined, done: true }); } } watcher.on('error', function(error) { finish(error); }); var signal = options.signal; if (signal) { var abort = function() { var error = new DOMException('The operation was aborted', 'AbortError'); error.cause = signal.reason; finish(error); }; if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true }); } return { [Symbol.asyncIterator]: function() { return this; }, next: function() { if (queue.length) return Promise.resolve({ value: queue.shift(), done: false }); if (failure) return Promise.reject(failure); if (closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { finish(error); return Promise.reject(error); } }; };\n\
             function cpError(path) { var error = new Error('EEXIST: destination already exists, cp ' + path); error.code = 'EEXIST'; error.errno = -1; error.path = path; error.syscall = 'cp'; return error; } function cpDirectoryError(path) { var error = new Error('ERR_FS_EISDIR: recursive option is required, cp ' + path); error.code = 'ERR_FS_EISDIR'; error.path = path; error.syscall = 'cp'; return error; } function cpParent(path) { var slash = path.lastIndexOf('/'); return slash > 0 ? path.slice(0, slash) : '.'; } function cpNormalize(path) { var absolute = path.charAt(0) === '/', values = []; path.split('/').forEach(function(part) { if (!part || part === '.') return; if (part === '..') values.pop(); else values.push(part); }); return (absolute ? '/' : '') + values.join('/'); } function cpEnsureParent(path) { var parent = cpParent(path); if (parent && parent !== '.' && !__thaw_fs.existsSync(parent)) __thaw_fs.mkdirSync(parent, { recursive: true }); } function cpPrepare(destination, directory, options) { if (!__thaw_fs.existsSync(destination)) return true; var current = __thaw_fs.lstatSync(destination); if (directory && current.isDirectory()) return true; if (options.force === false) { if (options.errorOnExist) throw cpError(destination); return false; } __thaw_fs.rmSync(destination, { recursive: true, force: true }); return true; } function cpPreserve(sourceStats, destination, options) { if (options.preserveTimestamps) __thaw_fs.utimesSync(destination, sourceStats.atime, sourceStats.mtime); } function cpLinkTarget(source, options) { var target = __thaw_fs.readlinkSync(source); if (options.verbatimSymlinks || target.charAt(0) === '/') return target; return cpNormalize(cpParent(source) + '/' + target); } function cpNodeSync(source, destination, options) { source = pathValue(source); destination = pathValue(destination); if (typeof options.filter === 'function') { var allowed = options.filter(source, destination); if (allowed && typeof allowed.then === 'function') throw new TypeError('fs.cpSync filter must return a boolean'); if (!allowed) return; } var stats = options.dereference ? __thaw_fs.statSync(source) : __thaw_fs.lstatSync(source); if (stats.isSymbolicLink() && !options.dereference) { if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.symlinkSync(cpLinkTarget(source, options), destination); return; } if (stats.isDirectory()) { if (!options.recursive) throw cpDirectoryError(source); if (!cpPrepare(destination, true, options)) return; if (!__thaw_fs.existsSync(destination)) __thaw_fs.mkdirSync(destination, { recursive: true }); __thaw_fs.readdirSync(source).forEach(function(name) { cpNodeSync(source.replace(/[/]$/, '') + '/' + name, destination.replace(/[/]$/, '') + '/' + name, options); }); cpPreserve(stats, destination, options); return; } if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.copyFileSync(source, destination); cpPreserve(stats, destination, options); } __thaw_fs.cpSync = function(source, destination, options) { cpNodeSync(source, destination, options || {}); }; function cpNodeAsync(source, destination, options) { source = pathValue(source); destination = pathValue(destination); var filtered = typeof options.filter === 'function' ? Promise.resolve().then(function() { return options.filter(source, destination); }) : Promise.resolve(true); return filtered.then(function(allowed) { if (!allowed) return; var stats = options.dereference ? __thaw_fs.statSync(source) : __thaw_fs.lstatSync(source); if (stats.isSymbolicLink() && !options.dereference) { if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.symlinkSync(cpLinkTarget(source, options), destination); return; } if (stats.isDirectory()) { if (!options.recursive) throw cpDirectoryError(source); if (!cpPrepare(destination, true, options)) return; if (!__thaw_fs.existsSync(destination)) __thaw_fs.mkdirSync(destination, { recursive: true }); return __thaw_fs.readdirSync(source).reduce(function(chain, name) { return chain.then(function() { return cpNodeAsync(source.replace(/[/]$/, '') + '/' + name, destination.replace(/[/]$/, '') + '/' + name, options); }); }, Promise.resolve()).then(function() { cpPreserve(stats, destination, options); }); } if (!cpPrepare(destination, false, options)) return; cpEnsureParent(destination); __thaw_fs.copyFileSync(source, destination); cpPreserve(stats, destination, options); }); } __thaw_fs.promises.cp = function(source, destination, options) { return cpNodeAsync(source, destination, options || {}); }; __thaw_fs.cp = function(source, destination, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } if (typeof callback !== 'function') throw new TypeError('callback must be a function'); cpNodeAsync(source, destination, options || {}).then(function() { callback(null); }, callback); };\n\
             ReadStream = function ReadStream(path, options) { if (!(this instanceof ReadStream)) return new ReadStream(path, options); options = options || {}; stream.Readable.call(this, options); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesRead = 0; this._position = options.start === undefined ? 0 : Number(options.start); this._end = options.end === undefined ? Infinity : Number(options.end); this._chunkSize = Math.max(1, Number(options.highWaterMark || 65536)); this.close = this.destroy.bind(this); if (options.encoding) this.setEncoding(options.encoding); var self = this; queueMicrotask(function() { try { invoke('stat', self.path); self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); function finish() { if (self.readableEnded || self.destroyed) return; self.push(null); if (options.autoClose !== false) queueMicrotask(function() { if (self.fd !== null) { self.fd = null; if (options.emitClose !== false) self.emit('close'); } }); } function pump() { if (self.destroyed || self.readableEnded) return; if (self._position > self._end) { finish(); return; } var length = Math.min(self._chunkSize, self._end === Infinity ? self._chunkSize : self._end - self._position + 1), record = invoke('read_range', self.path, String(self._position) + ',' + String(length)), chunk = Buffer.from(record.data, 'hex'); if (!chunk.length) { finish(); return; } self._position += chunk.length; self.bytesRead += chunk.length; self.push(chunk); if (chunk.length < length || self._position > self._end) finish(); else queueMicrotask(pump); } pump(); } catch (error) { self.pending = false; self.destroy(error); } }); if (options.signal) stream.addAbortSignal(options.signal, this); }; ReadStream.prototype = Object.create(stream.Readable.prototype); ReadStream.prototype.constructor = ReadStream; WriteStream = function WriteStream(path, options) { if (!(this instanceof WriteStream)) return new WriteStream(path, options); options = options || {}; initializeWriteQueue(this, options.highWaterMark); this.path = pathValue(path); this.pending = true; this.fd = null; this.bytesWritten = 0; this._position = options.start === undefined ? 0 : Number(options.start); this._flags = String(options.flags || 'w'); var self = this; if (this._flags.indexOf('x') >= 0 && __thaw_fs.existsSync(this.path)) throw exclusiveError(this.path, 'open'); if (this._flags.charAt(0) === 'w') invoke('write_mode', this.path, String(fsMode(options, 438)) + ','); else if (this._flags.charAt(0) === 'a') invoke('append_mode', this.path, String(fsMode(options, 438)) + ','); else invoke('stat', this.path); stream.Writable.call(this, { highWaterMark: options.highWaterMark, write: function(chunk, encoding, callback) { try { chunk = Buffer.from(chunk); if (self._flags.charAt(0) === 'a') invoke('append', self.path, chunk.toString('hex')); else { invoke('write_range', self.path, String(self._position) + ':' + chunk.toString('hex')); self._position += chunk.length; } self.bytesWritten += chunk.length; queueMicrotask(callback); } catch (error) { queueMicrotask(function() { callback(error); }); } }, final: function(callback) { callback(); if (options.autoClose !== false) queueMicrotask(function() { if (self.fd !== null) { self.fd = null; if (options.emitClose !== false) self.emit('close'); } }); } }); queueMicrotask(function() { if (self.destroyed) return; self.fd = 1; self.pending = false; self.emit('open', self.fd); self.emit('ready'); }); if (options.signal) stream.addAbortSignal(options.signal, this); }; WriteStream.prototype = Object.create(stream.Writable.prototype); WriteStream.prototype.constructor = WriteStream; WriteStream.prototype.close = function(callback) { if (!this.writableEnded) this.end(callback); else if (callback) queueMicrotask(callback); }; __thaw_fs.ReadStream = ReadStream; __thaw_fs.WriteStream = WriteStream; __thaw_fs.createReadStream = function(path, options) { return new ReadStream(path, options); }; __thaw_fs.createWriteStream = function(path, options) { return new WriteStream(path, options); };\n\
             function fsMode(options, fallback) { var mode = typeof options === 'number' ? options : options && typeof options === 'object' ? options.mode : undefined; if (mode === undefined) return fallback; return typeof mode === 'string' ? parseInt(mode, 8) : Number(mode); } function fsWriteMode(path, value, options, defaultFlag) { var flag = options && typeof options === 'object' && options.flag || defaultFlag, operation = String(flag).charAt(0) === 'a' ? 'append_mode' : 'write_mode'; if (String(flag).indexOf('x') >= 0 && __thaw_fs.existsSync(path)) throw exclusiveError(path, 'open'); invoke(operation, path, String(fsMode(options, 438)) + ',' + data(value, options).toString('hex')); } __thaw_fs.writeFileSync = function(path, value, options) { fsWriteMode(path, value, options, 'w'); }; __thaw_fs.appendFileSync = function(path, value, options) { fsWriteMode(path, value, options, 'a'); }; __thaw_fs.mkdirSync = function(path, options) { var recursive = Boolean(options && typeof options === 'object' && options.recursive); invoke('mkdir_mode', path, String(fsMode(options, 511)), recursive); }; function fsOpenHandle(path, flags, mode) { flags = flags === undefined ? 'r' : String(flags); var exists = __thaw_fs.existsSync(path); if (flags.indexOf('x') >= 0 && exists) throw exclusiveError(path, 'open'); if ((flags.charAt(0) === 'w' || flags.charAt(0) === 'a') && !exists) invoke(flags.charAt(0) === 'a' ? 'append_mode' : 'write_mode', path, String(fsMode(mode, 438)) + ','); var handle = new FileHandle(path, flags.replace('x', '')); handle.flags = flags; return handle; } __thaw_fs.openSync = function(path, flags, mode) { return fsOpenHandle(path, flags, mode).fd; }; __thaw_fs.promises.open = function(path, flags, mode) { return Promise.resolve().then(function() { return fsOpenHandle(path, flags, mode); }); }; __thaw_fs.promises.writeFile = promised(__thaw_fs.writeFileSync); __thaw_fs.promises.appendFile = promised(__thaw_fs.appendFileSync); __thaw_fs.promises.mkdir = promised(__thaw_fs.mkdirSync); __thaw_fs.writeFile = callbackMethod(__thaw_fs.writeFileSync, false); __thaw_fs.appendFile = callbackMethod(__thaw_fs.appendFileSync, false); __thaw_fs.mkdir = callbackMethod(__thaw_fs.mkdirSync, false); __thaw_fs.open = callbackMethod(__thaw_fs.openSync, true);\n\
             function initializeWriteQueue(streamValue, highWaterMark) { if (streamValue._writeQueue) return; streamValue.writableHighWaterMark = Math.max(1, Number(highWaterMark || 16384)); streamValue.writableLength = 0; streamValue._writeQueue = []; streamValue._writing = false; streamValue._ending = false; streamValue._finishing = false; streamValue._needsDrain = false; streamValue._endCallbacks = []; } WriteStream.prototype._pumpWrites = function() { var self = this; if (this._writing || !this._writeQueue.length) { if (!this._writing && !this._writeQueue.length && this._ending) this._finishWrites(); return; } this._writing = true; var entry = this._writeQueue[0]; this._write(entry.chunk, entry.encoding, function(error) { self._writing = false; self._writeQueue.shift(); self.writableLength -= entry.chunk.length; if (entry.callback) entry.callback(error); if (error) { self.destroy(error); return; } if (self._needsDrain && self.writableLength < self.writableHighWaterMark) { self._needsDrain = false; self.emit('drain'); } self._pumpWrites(); }); }; WriteStream.prototype.write = function(chunk, encoding, callback) { initializeWriteQueue(this); if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) { var error = new Error('write after end'); error.code = 'ERR_STREAM_WRITE_AFTER_END'; throw error; } var value = typeof chunk === 'string' ? Buffer.from(chunk, encoding) : Buffer.from(chunk); this._writeQueue.push({ chunk: value, encoding: encoding || 'buffer', callback: callback }); this.writableLength += value.length; var available = this.writableLength < this.writableHighWaterMark; if (!available) this._needsDrain = true; this._pumpWrites(); return available; }; WriteStream.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, finish = function(error) { if (error) { self.destroy(error); return; } self.writable = false; self.writableFinished = true; self.emit('finish'); self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); }; if (this._final) this._final(finish); else finish(); }; WriteStream.prototype.end = function(chunk, encoding, callback) { initializeWriteQueue(this); if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (callback) this._endCallbacks.push(callback); this.writableEnded = true; this._ending = true; this._pumpWrites(); return this; }; var createWriteStreamWithoutQueue = __thaw_fs.createWriteStream; __thaw_fs.createWriteStream = function(path, options) { var output = createWriteStreamWithoutQueue(path, options); initializeWriteQueue(output, options && options.highWaterMark); return output; }; FileHandle.prototype.createWriteStream = function(options) { this._check(); return __thaw_fs.createWriteStream(this.path, Object.assign({ flags: this.flags }, options || {})); };\n\
             module.exports = __thaw_fs;\n\
             module.exports.default = __thaw_fs;\n\
             module.exports.__esModule = true;\n",
        ),
        "fs/promises" => Some(
            "var promises = require('node:fs').promises; module.exports = promises; module.exports.default = promises; module.exports.__esModule = true;\n",
        ),
        "http" => Some(
            r#"var net = require('node:net'), EventEmitter = require('node:events');
             function IncomingMessage(socket) { EventEmitter.call(this); this.socket = this.connection = socket; this.statusCode = null; this.statusMessage = null; this.headers = {}; this.headersDistinct = {}; this.rawHeaders = []; this.trailers = {}; this.rawTrailers = []; this.complete = false; this.aborted = false; this.readable = true; this.method = null; this.url = ''; this.httpVersion = '1.1'; }
             IncomingMessage.prototype = Object.create(EventEmitter.prototype); IncomingMessage.prototype.constructor = IncomingMessage; IncomingMessage.prototype.setEncoding = function(encoding) { this._encoding = encoding; return this; }; IncomingMessage.prototype.destroy = function(error) { this.aborted = true; this.readable = false; if (this.socket) this.socket.destroy(error); return this; }; IncomingMessage.prototype.resume = function() { return this; };
             function normalizeOptions(input, options) { var result = {}; if (typeof input === 'string' || input instanceof URL) { var url = input instanceof URL ? input : new URL(String(input)); result.protocol = url.protocol; result.hostname = url.hostname; if (url.port) result.port = Number(url.port); result.path = url.pathname + url.search; result.auth = url.username ? decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password) : undefined; } else if (input) Object.assign(result, input); if (options) Object.assign(result, options); var expectedProtocol = result._defaultProtocol || 'http:'; result.protocol = result.protocol || expectedProtocol; if (result.protocol !== expectedProtocol) throw new Error('Protocol "' + result.protocol + '" not supported. Expected "' + expectedProtocol + '"'); result.hostname = result.hostname || result.host || 'localhost'; result.port = result.port === undefined ? (expectedProtocol === 'https:' ? 443 : 80) : Number(result.port); result.path = result.path || '/'; result.method = String(result.method || 'GET').toUpperCase(); return result; }
             function ClientRequest(input, options, callback) { if (!(this instanceof ClientRequest)) return new ClientRequest(input, options, callback); EventEmitter.call(this); if (typeof options === 'function') { callback = options; options = undefined; } this._options = normalizeOptions(input, options); this.method = this._options.method; this.path = this._options.path; this.host = this._options.hostname; this.protocol = this._options.protocol; this.agent = this._options.agent === false ? undefined : (this._options.agent || this._options._globalAgent || globalAgent); this.socket = this.connection = null; this.aborted = false; this.destroyed = false; this.finished = false; this.writableEnded = false; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; if (this._options.headers) for (var name of Object.keys(this._options.headers)) this.setHeader(name, this._options.headers[name]); var defaultPort = this.protocol === 'https:' ? 443 : 80; if (!this.hasHeader('host')) this.setHeader('Host', this.host + (this._options.port === defaultPort ? '' : ':' + this._options.port)); if (this._options.auth && !this.hasHeader('authorization')) this.setHeader('Authorization', 'Basic ' + Buffer.from(this._options.auth).toString('base64')); if (typeof callback === 'function') this.once('response', callback); }
             function validateHeaderName(name, label) { var value = String(name); if (!value || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value)) { var error = new TypeError((label || 'Header name') + ' must be a valid HTTP token ["' + value + '"]'); error.code = 'ERR_INVALID_HTTP_TOKEN'; throw error; } return value; } function validateHeaderValue(name, value) { if (value === undefined) { var missing = new TypeError('Invalid value "undefined" for header "' + name + '"'); missing.code = 'ERR_HTTP_INVALID_HEADER_VALUE'; throw missing; } var values = Array.isArray(value) ? value : [value]; for (var item of values) if (/[^\t\x20-\x7e\x80-\xff]/.test(String(item))) { var error = new TypeError('Invalid character in header content ["' + name + '"]'); error.code = 'ERR_INVALID_CHAR'; throw error; } return value; }
             ClientRequest.prototype = Object.create(EventEmitter.prototype); ClientRequest.prototype.constructor = ClientRequest; ClientRequest.prototype.setHeader = function(name, value) { name = validateHeaderName(name); validateHeaderValue(name, value); var key = name.toLowerCase(); this._headers[key] = value; this._headerNames[key] = name; return this; }; ClientRequest.prototype.getHeader = function(name) { return this._headers[String(name).toLowerCase()]; }; ClientRequest.prototype.getHeaders = function() { return Object.assign({}, this._headers); }; ClientRequest.prototype.getHeaderNames = function() { return Object.keys(this._headers); }; ClientRequest.prototype.hasHeader = function(name) { return Object.prototype.hasOwnProperty.call(this._headers, String(name).toLowerCase()); }; ClientRequest.prototype.removeHeader = function(name) { var key = String(name).toLowerCase(); delete this._headers[key]; delete this._headerNames[key]; }; ClientRequest.prototype.flushHeaders = function() { return this; }; ClientRequest.prototype.write = function(chunk, encoding, callback) { if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk), encoding); this._chunks.push(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };
             function decodeChunked(body) { var offset = 0, output = []; while (offset < body.length) { var line = body.indexOf('\r\n', offset); if (line < 0) throw new Error('Parse Error: Invalid chunk size'); var size = parseInt(body.slice(offset, line).toString(), 16); if (!Number.isFinite(size)) throw new Error('Parse Error: Invalid chunk size'); offset = line + 2; if (size === 0) break; output.push(body.slice(offset, offset + size)); offset += size + 2; } return Buffer.concat(output); }
             function parseResponse(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? '' : buffer.slice(0, marker).toString(), body = marker < 0 ? buffer : buffer.slice(marker + 4), lines = head.split('\r\n'), status = (lines.shift() || '').match(/^HTTP\/(\d+\.\d+)\s+(\d+)(?:\s+(.*))?$/); if (!status) throw new Error('Parse Error: Invalid HTTP response'); var response = new IncomingMessage(socket); response.httpVersion = status[1]; response.statusCode = Number(status[2]); response.statusMessage = status[3] || ''; lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); response.rawHeaders.push(name, value); (response.headersDistinct[key] || (response.headersDistinct[key] = [])).push(value); if (key === 'set-cookie') response.headers[key] = response.headersDistinct[key].slice(); else response.headers[key] = response.headersDistinct[key].join(', '); }); if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) body = decodeChunked(body); response.complete = true; return { response: response, body: body }; }
             ClientRequest.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var request = this, body = Buffer.concat(this._chunks); if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n'; var received = [], transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port }); var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions); this.emit('socket', socket); socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); }); socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); }); socket.on('data', function(data) { received.push(Buffer.from(data)); }); socket.on('end', function() { try { var parsed = parseResponse(Buffer.concat(received), socket), response = parsed.response; request.emit('response', response); var value = response._encoding ? parsed.body.toString(response._encoding) : parsed.body; if (parsed.body.length) response.emit('data', value); response.readable = false; response.emit('end'); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } }); return this; }; ClientRequest.prototype.abort = function() { this.aborted = true; this.emit('abort'); return this.destroy(); }; ClientRequest.prototype.destroy = function(error) { this.destroyed = true; if (this.socket) this.socket.destroy(error); return this; }; ClientRequest.prototype.setTimeout = function(timeout, callback) { if (typeof callback === 'function') this.once('timeout', callback); return this; }; ClientRequest.prototype.setNoDelay = ClientRequest.prototype.setSocketKeepAlive = function() { return this; };
             var endWithoutAgent = ClientRequest.prototype.end; ClientRequest.prototype.end = function() { if (this.agent && typeof this.agent.createConnection === 'function') { var agent = this.agent; this._options._transport = { createConnection: function(options) { return agent.createConnection(options); } }; } return endWithoutAgent.apply(this, arguments); };
             function request(input, options, callback) { return new ClientRequest(input, options, callback); } function get(input, options, callback) { var result = request(input, options, callback); result.end(); return result; }
             function ServerResponse(request) { EventEmitter.call(this); this.req = request; this.socket = this.connection = request.socket; this.statusCode = 200; this.statusMessage = null; this.sendDate = true; this.headersSent = false; this.finished = false; this.writableEnded = false; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; }
             ServerResponse.prototype = Object.create(EventEmitter.prototype); ServerResponse.prototype.constructor = ServerResponse; ServerResponse.prototype.setHeader = ClientRequest.prototype.setHeader; ServerResponse.prototype.getHeader = ClientRequest.prototype.getHeader; ServerResponse.prototype.getHeaders = ClientRequest.prototype.getHeaders; ServerResponse.prototype.getHeaderNames = ClientRequest.prototype.getHeaderNames; ServerResponse.prototype.hasHeader = ClientRequest.prototype.hasHeader; ServerResponse.prototype.removeHeader = ClientRequest.prototype.removeHeader; ServerResponse.prototype.writeHead = function(statusCode, statusMessage, headers) { this.statusCode = Number(statusCode); if (typeof statusMessage === 'object') { headers = statusMessage; statusMessage = undefined; } if (statusMessage !== undefined) this.statusMessage = String(statusMessage); if (headers) for (var name of Object.keys(headers)) this.setHeader(name, headers[name]); this.headersSent = true; return this; }; ServerResponse.prototype.flushHeaders = function() { this.headersSent = true; return this; }; ServerResponse.prototype.write = function(chunk, encoding, callback) { if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : Buffer.from(String(chunk), encoding); this._chunks.push(value); if (typeof callback === 'function') queueMicrotask(callback); return true; }; ServerResponse.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var body = Buffer.concat(this._chunks); if (!this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var message = this.statusMessage === null ? (STATUS_CODES[this.statusCode] || '') : this.statusMessage, response = 'HTTP/1.1 ' + this.statusCode + ' ' + message + '\r\n' + Object.keys(this._headers).map(function(key) { var value = this._headers[key]; if (Array.isArray(value)) return value.map(function(item) { return this._headerNames[key] + ': ' + item; }, this).join('\r\n'); return this._headerNames[key] + ': ' + value; }, this).join('\r\n') + '\r\n\r\n'; this.headersSent = true; var target = this; this.socket.end(Buffer.concat([Buffer.from(response), body]), function() { if (typeof callback === 'function') callback(); target.emit('finish'); target.emit('close'); }); return this; }; ServerResponse.prototype.destroy = function(error) { this.socket.destroy(error); return this; };
             function parseRequest(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? buffer.toString() : buffer.slice(0, marker).toString(), body = marker < 0 ? Buffer.alloc(0) : buffer.slice(marker + 4), lines = head.split('\r\n'), start = (lines.shift() || '').match(/^(\S+)\s+(\S+)\s+HTTP\/(\d+\.\d+)$/); if (!start) throw new Error('Parse Error: Invalid HTTP request'); var message = new IncomingMessage(socket); message.method = start[1]; message.url = start[2]; message.httpVersion = start[3]; lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); message.rawHeaders.push(name, value); (message.headersDistinct[key] || (message.headersDistinct[key] = [])).push(value); message.headers[key] = message.headersDistinct[key].join(', '); }); message.complete = true; return { request: message, body: body }; }
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); EventEmitter.call(this); if (typeof options === 'function') { listener = options; options = {}; } options = options || {}; this.requestTimeout = 300000; this.headersTimeout = 60000; this.keepAliveTimeout = 5000; this.maxHeadersCount = null; this.listening = false; var transport = options._transport || net, server = this, accept = function(socket) { var chunks = [], handled = false; socket.on('data', function(chunk) { if (!handled) chunks.push(Buffer.from(chunk)); }); socket.on('end', function() { if (handled) return; handled = true; try { var parsed = parseRequest(Buffer.concat(chunks), socket), request = parsed.request, response = new ServerResponse(request); server.emit('request', request, response); if (parsed.body.length) request.emit('data', parsed.body); request.readable = false; request.emit('end'); } catch (error) { server.emit('clientError', error, socket); } }); }; this._net = transport.createServer(options, accept); this._net.on('listening', function() { server.listening = true; server.emit('listening'); }); this._net.on('close', function() { server.listening = false; server.emit('close'); }); this._net.on('error', function(error) { server.emit('error', error); }); this._net.on('connection', function(socket) { server.emit('connection', socket); }); this._net.on('secureConnection', function(socket) { server.emit('secureConnection', socket); }); if (typeof listener === 'function') this.on('request', listener); }
             Server.prototype = Object.create(EventEmitter.prototype); Server.prototype.constructor = Server; Server.prototype.listen = function() { this._net.listen.apply(this._net, arguments); return this; }; Server.prototype.close = function(callback) { this._net.close(callback); return this; }; Server.prototype.address = function() { return this._net.address(); }; Server.prototype.getConnections = function(callback) { return this._net.getConnections(callback); }; Server.prototype.closeAllConnections = function() { return this._net.closeAllConnections(); }; Server.prototype.closeIdleConnections = function() { return this._net.closeIdleConnections(); }; Server.prototype.ref = function() { this._net.ref(); return this; }; Server.prototype.unref = function() { this._net.unref(); return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var METHODS = ['ACL','BIND','CHECKOUT','CONNECT','COPY','DELETE','GET','HEAD','LINK','LOCK','M-SEARCH','MERGE','MKACTIVITY','MKCALENDAR','MKCOL','MOVE','NOTIFY','OPTIONS','PATCH','POST','PROPFIND','PROPPATCH','PURGE','PUT','REBIND','REPORT','SEARCH','SOURCE','SUBSCRIBE','TRACE','UNBIND','UNLINK','UNLOCK','UNSUBSCRIBE']; var STATUS_CODES = { 200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 301: 'Moved Permanently', 302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 500: 'Internal Server Error', 502: 'Bad Gateway', 503: 'Service Unavailable' };
             function Agent(options, transport, protocol) { if (!(this instanceof Agent)) return new Agent(options, transport, protocol); EventEmitter.call(this); options = options || {}; this.options = Object.assign({}, options); this.keepAlive = Boolean(options.keepAlive); this.keepAliveMsecs = options.keepAliveMsecs === undefined ? 1000 : Number(options.keepAliveMsecs); this.maxSockets = options.maxSockets === undefined ? Infinity : Number(options.maxSockets); this.maxFreeSockets = options.maxFreeSockets === undefined ? 256 : Number(options.maxFreeSockets); this.maxTotalSockets = options.maxTotalSockets === undefined ? Infinity : Number(options.maxTotalSockets); this.totalSocketCount = 0; this.requests = Object.create(null); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.protocol = protocol || 'http:'; this.defaultPort = this.protocol === 'https:' ? 443 : 80; this._transport = transport || net; }
             Agent.prototype = Object.create(EventEmitter.prototype); Agent.prototype.constructor = Agent; Agent.prototype.getName = function(options) { options = options || {}; var host = options.host || options.hostname || 'localhost', port = options.port || this.defaultPort, localAddress = options.localAddress || '', family = options.family || ''; return host + ':' + port + ':' + localAddress + ':' + family; }; Agent.prototype.createConnection = function(options, callback) { var agent = this, name = this.getName(options), socket = this._transport.createConnection ? this._transport.createConnection(options) : this._transport.connect(options); (this.sockets[name] || (this.sockets[name] = [])).push(socket); this.totalSocketCount++; socket.once('close', function() { agent.removeSocket(socket, options); }); if (typeof callback === 'function') socket.once(this.protocol === 'https:' ? 'secureConnect' : 'connect', function() { callback(null, socket); }); return socket; }; Agent.prototype.keepSocketAlive = function(socket) { socket.setKeepAlive(true, this.keepAliveMsecs); socket.unref(); return true; }; Agent.prototype.reuseSocket = function(socket) { socket.ref(); }; Agent.prototype.removeSocket = function(socket, options) { var name = this.getName(options), list = this.sockets[name] || [], index = list.indexOf(socket); if (index >= 0) list.splice(index, 1); if (!list.length) delete this.sockets[name]; this.totalSocketCount = Math.max(0, this.totalSocketCount - 1); }; Agent.prototype.destroy = function() { for (var group of [this.sockets, this.freeSockets]) for (var name of Object.keys(group)) for (var socket of group[name]) socket.destroy(); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.totalSocketCount = 0; };
             var globalAgent = new Agent({ keepAlive: true }, net, 'http:'); function setMaxIdleHTTPParsers() {}
             function createSecureModule(tls) { function SecureAgent(options) { Agent.call(this, options, tls, 'https:'); } SecureAgent.prototype = Object.create(Agent.prototype); SecureAgent.prototype.constructor = SecureAgent; var secureGlobalAgent = new SecureAgent({ keepAlive: true }); function secureOptions(options) { return Object.assign({}, options || {}, { _transport: tls, _defaultProtocol: 'https:', _globalAgent: secureGlobalAgent }); } function secureRequest(input, options, callback) { if (typeof options === 'function') { callback = options; options = undefined; } return new ClientRequest(input, secureOptions(options), callback); } function secureGet(input, options, callback) { var result = secureRequest(input, options, callback); result.end(); return result; } function secureCreateServer(options, listener) { if (typeof options === 'function') { listener = options; options = {}; } return new Server(Object.assign({}, options || {}, { _transport: tls }), listener); } return { request: secureRequest, get: secureGet, createServer: secureCreateServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: secureGlobalAgent, Agent: SecureAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers }; }
             module.exports = { request: request, get: get, createServer: createServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, Agent: Agent, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: globalAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers, __createSecureModule: createSecureModule }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "https" => Some(
            "var http = require('node:http'), tls = require('node:tls'); module.exports = http.__createSecureModule(tls); module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "buffer" => Some(
            "function byteLength(value, encoding) { return globalThis.Buffer.byteLength(value, encoding); }\n\
             function isUtf8(value) { try { new TextDecoder('utf-8', { fatal: true }).decode(value); return true; } catch (_) { return false; } }\n\
             function isAscii(value) { return Array.from(value).every(function(byte) { return byte <= 127; }); }\n\
             function transcode(value, fromEncoding, toEncoding) { return globalThis.Buffer.from(globalThis.Buffer.from(value).toString(fromEncoding), toEncoding); }\n\
             module.exports = { Buffer: globalThis.Buffer, SlowBuffer: globalThis.SlowBuffer, byteLength: byteLength, isUtf8: isUtf8, isAscii: isAscii, transcode: transcode, atob: globalThis.atob, btoa: globalThis.btoa, constants: { MAX_LENGTH: 4294967296, MAX_STRING_LENGTH: 536870888 } };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "string_decoder" => Some(
            "function normalizeEncoding(encoding) { var value = String(encoding || 'utf8').toLowerCase().replace(/[-_]/g, ''); if (!globalThis.Buffer.isEncoding(value)) throw new TypeError('Unknown encoding: ' + encoding); return value; }\n\
             function utf8CompleteLength(buffer) {\n\
             \x20\x20var index = buffer.length - 1; while (index >= 0 && (buffer[index] & 192) === 128) index--;\n\
             \x20\x20if (index < 0) return Math.max(0, buffer.length - Math.min(buffer.length, 3));\n\
             \x20\x20var lead = buffer[index]; var expected = lead >= 240 && lead <= 247 ? 4 : lead >= 224 && lead <= 239 ? 3 : lead >= 192 && lead <= 223 ? 2 : 1;\n\
             \x20\x20return buffer.length - index < expected ? index : buffer.length;\n\
             }\n\
             function StringDecoder(encoding) {\n\
             \x20\x20if (!(this instanceof StringDecoder)) return new StringDecoder(encoding);\n\
             \x20\x20this.encoding = normalizeEncoding(encoding); this._pending = globalThis.Buffer.alloc(0); this.lastNeed = 0; this.lastTotal = 0; this.lastChar = globalThis.Buffer.alloc(4);\n\
             }\n\
             StringDecoder.prototype.write = function(value) {\n\
             \x20\x20var input = globalThis.Buffer.from(value); var combined = this._pending.length ? globalThis.Buffer.concat([this._pending, input]) : input; var complete = combined.length;\n\
             \x20\x20if (this.encoding === 'utf8' || this.encoding === 'utf') complete = utf8CompleteLength(combined);\n\
             \x20\x20else if (this.encoding === 'utf16le' || this.encoding === 'ucs2') complete -= complete % 2;\n\
             \x20\x20else if (this.encoding === 'base64' || this.encoding === 'base64url') complete -= complete % 3;\n\
             \x20\x20this._pending = globalThis.Buffer.from(combined.subarray(complete)); this.lastNeed = this._pending.length; this.lastTotal = complete === combined.length ? 0 : this.lastNeed + 1;\n\
             \x20\x20return combined.subarray(0, complete).toString(this.encoding);\n\
             };\n\
             StringDecoder.prototype.text = function(value, offset) { return this.write(globalThis.Buffer.from(value).subarray(offset || 0)); };\n\
             StringDecoder.prototype.end = function(value) { var output = value === undefined ? '' : this.write(value); if (this._pending.length) output += this._pending.toString(this.encoding); this._pending = globalThis.Buffer.alloc(0); this.lastNeed = 0; this.lastTotal = 0; return output; };\n\
             module.exports = { StringDecoder: StringDecoder };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "timers" => Some(
            "module.exports = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, setInterval: globalThis.setInterval, clearInterval: globalThis.clearInterval, setImmediate: globalThis.setImmediate, clearImmediate: globalThis.clearImmediate };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "timers/promises" => Some(
            "function abortReason(signal) { return signal && signal.reason !== undefined ? signal.reason : new DOMException('This operation was aborted', 'AbortError'); }\n\
             function promiseTimer(schedule, delay, value, options) {\n\
             \x20\x20options = options || {}; var signal = options.signal;\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20if (signal && signal.aborted) { reject(abortReason(signal)); return; }\n\
             \x20\x20\x20\x20var id; function aborted() { if (schedule === globalThis.setImmediate) globalThis.clearImmediate(id); else globalThis.clearTimeout(id); reject(abortReason(signal)); }\n\
             \x20\x20\x20\x20id = schedule(function() { if (signal) signal.removeEventListener('abort', aborted); resolve(value); }, delay);\n\
             \x20\x20\x20\x20if (signal) signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function setTimeoutPromise(delay, value, options) { return promiseTimer(globalThis.setTimeout, delay, value, options); }\n\
             function setImmediatePromise(value, options) { return promiseTimer(globalThis.setImmediate, 0, value, options); }\n\
             function setIntervalPromise(delay, value, options) {\n\
             \x20\x20options = options || {}; var signal = options.signal; var values = []; var waiters = []; var done = false; var failure;\n\
             \x20\x20var id = globalThis.setInterval(function() { var result = { value: value, done: false }; if (waiters.length) waiters.shift().resolve(result); else values.push(result); }, delay);\n\
             \x20\x20function stop(error) { if (done) return; done = true; failure = error; globalThis.clearInterval(id); while (waiters.length) { var waiter = waiters.shift(); if (error) waiter.reject(error); else waiter.resolve({ value: undefined, done: true }); } }\n\
             \x20\x20if (signal) { if (signal.aborted) stop(abortReason(signal)); else signal.addEventListener('abort', function() { stop(abortReason(signal)); }, { once: true }); }\n\
             \x20\x20return { next: function() { if (values.length) return Promise.resolve(values.shift()); if (done) return failure ? Promise.reject(failure) : Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { stop(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } };\n\
             }\n\
             var scheduler = { wait: function(delay, options) { return setTimeoutPromise(delay, undefined, options); }, yield: function() { return setImmediatePromise(); } };\n\
             module.exports = { setTimeout: setTimeoutPromise, setImmediate: setImmediatePromise, setInterval: setIntervalPromise, scheduler: scheduler };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "readline" => Some(
            "function Interface(input, output, completer, terminal) { if (!(this instanceof Interface)) return new Interface(input, output, completer, terminal); var options = input && input.input ? input : { input: input, output: output, completer: completer, terminal: terminal }; this.input = options.input || null; this.output = options.output || null; this.completer = options.completer || null; this.terminal = Boolean(options.terminal); this.history = Array.isArray(options.history) ? options.history.slice() : []; this.historySize = options.historySize === undefined ? 30 : Number(options.historySize); this.removeHistoryDuplicates = Boolean(options.removeHistoryDuplicates); this.line = ''; this.cursor = 0; this.closed = false; this.paused = false; this._prompt = options.prompt === undefined ? '> ' : String(options.prompt); this._events = Object.create(null); this._buffer = ''; this._iteratorValues = []; this._iteratorWaiters = []; var self = this; this._onData = function(chunk) { self.write(chunk); }; this._onEnd = function() { if (self._buffer) { self._emitLine(self._buffer); self._buffer = ''; } self.close(); }; if (this.input && this.input.on) { this.input.on('data', this._onData); this.input.on('end', this._onEnd); } }\n\
             Interface.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Interface.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Interface.prototype.off = Interface.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Interface.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; };\n\
             Interface.prototype._emitLine = function(line) { this.line = String(line); this.cursor = this.line.length; if (this.line && this.historySize > 0) { if (!this.removeHistoryDuplicates || this.history[0] !== this.line) this.history.unshift(this.line); this.history.length = Math.min(this.history.length, this.historySize); } this.emit('line', this.line); var result = { value: this.line, done: false }; if (this._iteratorWaiters.length) this._iteratorWaiters.shift().resolve(result); else this._iteratorValues.push(result); }; Interface.prototype.write = function(data) { if (this.closed) return; this._buffer += Buffer.isBuffer(data) ? data.toString() : String(data); var lines = this._buffer.split(/\\r?\\n|\\r/); this._buffer = lines.pop(); for (var index = 0; index < lines.length; index++) this._emitLine(lines[index]); };\n\
             Interface.prototype.setPrompt = function(prompt) { this._prompt = String(prompt); }; Interface.prototype.getPrompt = function() { return this._prompt; }; Interface.prototype.prompt = function() { if (this.output && this.output.write) this.output.write(this._prompt); return this; }; Interface.prototype.question = function(query, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; if (this.output && this.output.write) this.output.write(String(query)); var self = this; function answered(value) { if (options.signal) options.signal.removeEventListener('abort', aborted); callback(value); } function aborted() { self.off('line', answered); } if (options.signal) { if (options.signal.aborted) return; options.signal.addEventListener('abort', aborted, { once: true }); } this.once('line', answered); }; Interface.prototype.pause = function() { this.paused = true; if (this.input && this.input.pause) this.input.pause(); this.emit('pause'); return this; }; Interface.prototype.resume = function() { this.paused = false; if (this.input && this.input.resume) this.input.resume(); this.emit('resume'); return this; }; Interface.prototype.close = function() { if (this.closed) return; this.closed = true; if (this.input && this.input.off) { this.input.off('data', this._onData); this.input.off('end', this._onEnd); } this.emit('close'); while (this._iteratorWaiters.length) this._iteratorWaiters.shift().resolve({ value: undefined, done: true }); }; Interface.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { if (self._iteratorValues.length) return Promise.resolve(self._iteratorValues.shift()); if (self.closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { self._iteratorWaiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { self.close(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } }; };\n\
             function createInterface() { return new (Function.prototype.bind.apply(Interface, [null].concat(Array.prototype.slice.call(arguments))))(); } function write(output, value, callback) { if (!output || typeof output.write !== 'function') return false; var result = output.write(value); if (typeof callback === 'function') queueMicrotask(callback); return result !== false; } function clearLine(output, direction, callback) { return write(output, '\\u001b[' + (direction < 0 ? '1' : direction > 0 ? '0' : '2') + 'K', callback); } function clearScreenDown(output, callback) { return write(output, '\\u001b[0J', callback); } function cursorTo(output, x, y, callback) { if (typeof y === 'function') { callback = y; y = undefined; } return write(output, y === undefined ? '\\u001b[' + (Number(x) + 1) + 'G' : '\\u001b[' + (Number(y) + 1) + ';' + (Number(x) + 1) + 'H', callback); } function moveCursor(output, dx, dy, callback) { var value = ''; dx = Number(dx); dy = Number(dy); if (dx < 0) value += '\\u001b[' + (-dx) + 'D'; else if (dx > 0) value += '\\u001b[' + dx + 'C'; if (dy < 0) value += '\\u001b[' + (-dy) + 'A'; else if (dy > 0) value += '\\u001b[' + dy + 'B'; return write(output, value, callback); }\n\
             module.exports = { Interface: Interface, ReadLine: Interface, createInterface: createInterface, clearLine: clearLine, clearScreenDown: clearScreenDown, cursorTo: cursorTo, moveCursor: moveCursor, emitKeypressEvents: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "readline/promises" => Some(
            "function Interface(input, output) { if (!(this instanceof Interface)) return new Interface(input, output); var options = input && input.input ? input : { input: input, output: output }; this.input = options.input || null; this.output = options.output || null; this.closed = false; this.line = ''; this._events = Object.create(null); this._buffer = ''; this._iteratorValues = []; this._iteratorWaiters = []; var self = this; this._onData = function(chunk) { self.write(chunk); }; this._onEnd = function() { self.close(); }; if (this.input && this.input.on) { this.input.on('data', this._onData); this.input.on('end', this._onEnd); } } Interface.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push(listener); return this; }; Interface.prototype.off = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry !== listener; }); return this; }; Interface.prototype._emitLine = function(line) { this.line = line; var listeners = (this._events.line || []).slice(); this._events.line = []; listeners.forEach(function(listener) { listener(line); }); var result = { value: line, done: false }; if (this._iteratorWaiters.length) this._iteratorWaiters.shift()(result); else this._iteratorValues.push(result); }; Interface.prototype.write = function(data) { this._buffer += Buffer.isBuffer(data) ? data.toString() : String(data); var lines = this._buffer.split(/\\r?\\n|\\r/); this._buffer = lines.pop(); for (var index = 0; index < lines.length; index++) this._emitLine(lines[index]); }; Interface.prototype.question = function(query, options) { var self = this; options = options || {}; if (this.output && this.output.write) this.output.write(String(query)); return new Promise(function(resolve, reject) { if (options.signal && options.signal.aborted) { reject(options.signal.reason); return; } function answered(value) { if (options.signal) options.signal.removeEventListener('abort', aborted); resolve(value); } function aborted() { self.off('line', answered); reject(options.signal.reason); } if (options.signal) options.signal.addEventListener('abort', aborted, { once: true }); self.once('line', answered); }); }; Interface.prototype.close = function() { if (this.closed) return; this.closed = true; while (this._iteratorWaiters.length) this._iteratorWaiters.shift()({ value: undefined, done: true }); }; Interface.prototype[Symbol.asyncIterator] = function() { var self = this; return { next: function() { if (self._iteratorValues.length) return Promise.resolve(self._iteratorValues.shift()); if (self.closed) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve) { self._iteratorWaiters.push(resolve); }); }, return: function() { self.close(); return Promise.resolve({ value: undefined, done: true }); }, [Symbol.asyncIterator]: function() { return this; } }; }; function createInterface() { return new (Function.prototype.bind.apply(Interface, [null].concat(Array.prototype.slice.call(arguments))))(); } module.exports = { Interface: Interface, ReadLine: Interface, createInterface: createInterface }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream" => Some(
            "function Stream() { this._events = Object.create(null); this.destroyed = false; }\n\
             Stream.prototype.on = function(event, listener) { var name = String(event); (this._events[name] || (this._events[name] = [])).push({ listener: listener, once: false }); if (name === 'data' && this._drainReadable) this._drainReadable(); if (name === 'end' && this.readableEnded) queueMicrotask(function() { listener(); }); return this; };\n\
             Stream.prototype.once = function(event, listener) { var name = String(event); (this._events[name] || (this._events[name] = [])).push({ listener: listener, once: true }); return this; };\n\
             Stream.prototype.off = Stream.prototype.removeListener = function(event, listener) { var name = String(event); this._events[name] = (this._events[name] || []).filter(function(entry) { return entry.listener !== listener; }); return this; };\n\
             Stream.prototype.emit = function(event) { var name = String(event); var entries = (this._events[name] || []).slice(); var args = Array.prototype.slice.call(arguments, 1); entries.forEach(function(entry) { if (entry.once) this.off(name, entry.listener); entry.listener.apply(this, args); }, this); return entries.length > 0; };\n\
             Stream.prototype.destroy = function(error) { if (this.destroyed) return this; this.destroyed = true; if (error) this.emit('error', error); this.emit('close'); return this; };\n\
             function inherit(child, parent) { child.prototype = Object.create(parent.prototype); child.prototype.constructor = child; }\n\
             function Readable(options) { Stream.call(this); options = options || {}; this.readable = true; this.readableEnded = false; this.readableEncoding = null; this._chunks = []; this._read = typeof options.read === 'function' ? options.read : function() {}; }\n\
             inherit(Readable, Stream);\n\
             Readable.prototype.setEncoding = function(encoding) { this.readableEncoding = encoding; this._chunks = this._chunks.map(function(chunk) { return typeof chunk === 'string' ? chunk : chunk.toString(encoding); }); return this; };\n\
             Readable.prototype.push = function(chunk, encoding) { if (chunk === null) { this.readableEnded = true; this.readable = false; if (this._chunks.length === 0) this.emit('end'); return false; } var value = typeof chunk === 'string' ? globalThis.Buffer.from(chunk, encoding) : globalThis.Buffer.from(chunk); if (this.readableEncoding) value = value.toString(this.readableEncoding); if ((this._events.data || []).length) this.emit('data', value); else this._chunks.push(value); return true; };\n\
             Readable.prototype._drainReadable = function() { while (this._chunks.length && (this._events.data || []).length) this.emit('data', this._chunks.shift()); if (this.readableEnded && this._chunks.length === 0) this.emit('end'); };\n\
             Readable.prototype.read = function(size) { if (this._chunks.length === 0) { if (!this.readableEnded) this._read(size); return this._chunks.shift() || null; } if (size === undefined) { if (this._chunks.length === 1) return this._chunks.shift(); var joined = globalThis.Buffer.concat(this._chunks.map(function(chunk) { return typeof chunk === 'string' ? globalThis.Buffer.from(chunk) : chunk; })); this._chunks = []; return this.readableEncoding ? joined.toString(this.readableEncoding) : joined; } var first = this._chunks[0]; if (typeof first === 'string') { var text = first.substring(0, size); this._chunks[0] = first.substring(size); if (!this._chunks[0]) this._chunks.shift(); return text; } var result = first.subarray(0, size); this._chunks[0] = first.subarray(size); if (!this._chunks[0].length) this._chunks.shift(); return result; };\n\
             Readable.prototype.pipe = function(destination, options) { this.on('data', function(chunk) { destination.write(chunk); }); if (!options || options.end !== false) this.once('end', function() { destination.end(); }); this.once('error', function(error) { destination.destroy(error); }); destination.emit('pipe', this); return destination; };\n\
             Readable.prototype.unpipe = function(destination) { if (destination) destination.emit('unpipe', this); return this; };\n\
             Readable.from = function(iterable) { var stream = new Readable(); queueMicrotask(async function() { try { for await (var value of iterable) stream.push(value); stream.push(null); } catch (error) { stream.destroy(error); } }); return stream; };\n\
             function initWritableState(target, options) { options = options || {}; target.writable = true; target.writableEnded = false; target.writableFinished = false; target.writableHighWaterMark = Math.max(1, Number(options.highWaterMark || 16384)); target.writableLength = 0; target._writeQueue = []; target._writing = false; target._ending = false; target._finishing = false; target._needsDrain = false; target._endCallbacks = []; target._write = typeof options.write === 'function' ? options.write : function(chunk, encoding, callback) { callback(); }; target._final = typeof options.final === 'function' ? options.final : null; } function Writable(options) { Stream.call(this); initWritableState(this, options); }\n\
             inherit(Writable, Stream);\n\
             Writable.prototype._pumpWrites = function() { var self = this; if (this._writing || !this._writeQueue.length) { if (!this._writing && !this._writeQueue.length && this._ending) this._finishWrites(); return; } this._writing = true; var entry = this._writeQueue[0]; this._write(entry.chunk, entry.encoding, function(error) { self._writing = false; self._writeQueue.shift(); self.writableLength -= entry.chunk.length; if (entry.callback) entry.callback(error); if (error) { self.destroy(error); return; } if (self._needsDrain && self.writableLength < self.writableHighWaterMark) { self._needsDrain = false; self.emit('drain'); } self._pumpWrites(); }); }; Writable.prototype._finishWrites = function() { if (this._finishing || this.writableFinished) return; this._finishing = true; var self = this, finish = function(error) { if (error) { self.destroy(error); return; } self.writable = false; self.writableFinished = true; self.emit('finish'); self._endCallbacks.splice(0).forEach(function(callback) { callback(); }); }; if (this._final) this._final(finish); else finish(); }; Writable.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) { var error = new Error('write after end'); error.code = 'ERR_STREAM_WRITE_AFTER_END'; throw error; } var value = typeof chunk === 'string' ? globalThis.Buffer.from(chunk, encoding) : globalThis.Buffer.from(chunk); this._writeQueue.push({ chunk: value, encoding: encoding || 'buffer', callback: callback }); this.writableLength += value.length; var available = this.writableLength < this.writableHighWaterMark; if (!available) this._needsDrain = true; this._pumpWrites(); return available; };\n\
             Writable.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (callback) this._endCallbacks.push(callback); this.writableEnded = true; this._ending = true; this._pumpWrites(); return this; };\
             function Duplex(options) { Readable.call(this, options); initWritableState(this, options); }\n\
             inherit(Duplex, Readable); Duplex.prototype.write = Writable.prototype.write; Duplex.prototype.end = Writable.prototype.end; Duplex.prototype._pumpWrites = Writable.prototype._pumpWrites; Duplex.prototype._finishWrites = Writable.prototype._finishWrites;\n\
             function Transform(options) { options = options || {}; Duplex.call(this, options); this._transform = typeof options.transform === 'function' ? options.transform : function(chunk, encoding, callback) { callback(null, chunk); }; this._flush = typeof options.flush === 'function' ? options.flush : null; var self = this; this._write = function(chunk, encoding, callback) { self._transform(chunk, encoding, function(error, output) { if (!error && output !== undefined && output !== null) self.push(output); callback(error); }); }; this._final = function(callback) { function finish(error, output) { if (!error && output !== undefined && output !== null) self.push(output); if (!error) self.push(null); callback(error); } if (self._flush) self._flush(finish); else finish(); }; }\n\
             inherit(Transform, Duplex); Transform.prototype.write = Writable.prototype.write; Transform.prototype.end = Writable.prototype.end; Transform.prototype._pumpWrites = Writable.prototype._pumpWrites; Transform.prototype._finishWrites = Writable.prototype._finishWrites;\n\
             function PassThrough(options) { Transform.call(this, options); } inherit(PassThrough, Transform); PassThrough.prototype._transform = function(chunk, encoding, callback) { callback(null, chunk); };\n\
             function finished(stream, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } var done = false; function finish(error) { if (done) return; done = true; callback(error); } stream.once('error', finish); if (stream.writable !== undefined) stream.once('finish', function() { finish(); }); else stream.once('end', function() { finish(); }); return function() { done = true; }; }\n\
             function pipeline() { var streams = Array.prototype.slice.call(arguments); var callback = typeof streams[streams.length - 1] === 'function' ? streams.pop() : function(error) { if (error) throw error; }; for (var index = 0; index + 1 < streams.length; index++) streams[index].pipe(streams[index + 1]); var settled = false; streams.forEach(function(stream) { stream.once('error', function(error) { if (!settled) { settled = true; streams.forEach(function(item) { item.destroy(); }); callback(error); } }); }); streams[streams.length - 1].once('finish', function() { if (!settled) { settled = true; callback(); } }); return streams[streams.length - 1]; }\n\
             function addAbortSignal(signal, stream) { if (signal.aborted) stream.destroy(signal.reason); else signal.addEventListener('abort', function() { stream.destroy(signal.reason); }, { once: true }); return stream; }\n\
             module.exports = Stream; Object.assign(module.exports, { Stream: Stream, Readable: Readable, Writable: Writable, Duplex: Duplex, Transform: Transform, PassThrough: PassThrough, pipeline: pipeline, finished: finished, addAbortSignal: addAbortSignal });\n\
             module.exports.default = Stream; module.exports.__esModule = true;\n",
        ),
        "stream/promises" => Some(
            "function finished(stream, options) { return new Promise(function(resolve, reject) { var settled = false; function done(error) { if (settled) return; settled = true; if (error) reject(error); else resolve(); } stream.once('error', done); if (stream.writable !== undefined) stream.once('finish', function() { done(); }); else stream.once('end', function() { done(); }); if (options && options.signal) { if (options.signal.aborted) done(options.signal.reason); else options.signal.addEventListener('abort', function() { done(options.signal.reason); }, { once: true }); } }); }\n\
             function pipeline() { var streams = Array.prototype.slice.call(arguments); var options = streams.length && streams[streams.length - 1] && streams[streams.length - 1].signal && typeof streams[streams.length - 1].pipe !== 'function' ? streams.pop() : {}; return new Promise(function(resolve, reject) { var settled = false; function fail(error) { if (settled) return; settled = true; streams.forEach(function(stream) { if (stream.destroy) stream.destroy(); }); reject(error); } streams.forEach(function(stream) { stream.once('error', fail); }); for (var index = 0; index + 1 < streams.length; index++) streams[index].pipe(streams[index + 1]); var last = streams[streams.length - 1]; last.once('finish', function() { if (!settled) { settled = true; resolve(last); } }); if (options.signal) { if (options.signal.aborted) fail(options.signal.reason); else options.signal.addEventListener('abort', function() { fail(options.signal.reason); }, { once: true }); } }); }\n\
             module.exports = { pipeline: pipeline, finished: finished }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/consumers" => Some(
            "function collect(stream) { if (stream && typeof stream[Symbol.asyncIterator] === 'function') return (async function() { var chunks = []; for await (var chunk of stream) chunks.push(Buffer.from(chunk)); return Buffer.concat(chunks); })(); return new Promise(function(resolve, reject) { var chunks = []; function cleanup() { if (stream.off) { stream.off('data', data); stream.off('end', end); stream.off('error', reject); } } function data(chunk) { chunks.push(Buffer.from(chunk)); } function end() { cleanup(); resolve(Buffer.concat(chunks)); } if (!stream || typeof stream.on !== 'function') { reject(new TypeError('stream must be readable')); return; } stream.on('data', data); stream.once('end', end); stream.once('error', reject); }); } function buffer(stream) { return collect(stream); } function text(stream) { return collect(stream).then(function(value) { return value.toString('utf8'); }); } function json(stream) { return text(stream).then(JSON.parse); } function arrayBuffer(stream) { return collect(stream).then(function(value) { var copy = Uint8Array.from(value); return copy.buffer; }); } function blob(stream) { return collect(stream).then(function(value) { return new Blob([value]); }); } module.exports = { arrayBuffer: arrayBuffer, blob: blob, buffer: buffer, json: json, text: text }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "stream/web" => Some(
            "module.exports = { ReadableStream: globalThis.ReadableStream, ReadableStreamDefaultReader: globalThis.ReadableStreamDefaultReader, ReadableStreamDefaultController: globalThis.ReadableStreamDefaultController, WritableStream: globalThis.WritableStream, WritableStreamDefaultWriter: globalThis.WritableStreamDefaultWriter, TransformStream: globalThis.TransformStream, ByteLengthQueuingStrategy: globalThis.ByteLengthQueuingStrategy, CountQueuingStrategy: globalThis.CountQueuingStrategy, TextEncoderStream: globalThis.TextEncoderStream, TextDecoderStream: globalThis.TextDecoderStream, CompressionStream: globalThis.CompressionStream, DecompressionStream: globalThis.DecompressionStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "diagnostics_channel" => Some(
            "var registry = globalThis.__thawDiagnosticChannels || (globalThis.__thawDiagnosticChannels = new Map());\n\
             function Channel(name) { this.name = String(name); this._subscribers = []; this._stores = []; }\n\
             Object.defineProperty(Channel.prototype, 'hasSubscribers', { get: function() { return this._subscribers.length !== 0; } });\n\
             Channel.prototype.subscribe = function(callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); if (this._subscribers.indexOf(callback) < 0) this._subscribers.push(callback); };\n\
             Channel.prototype.unsubscribe = function(callback) { var index = this._subscribers.indexOf(callback); if (index < 0) return false; this._subscribers.splice(index, 1); return true; };\n\
             Channel.prototype.bindStore = function(store, transform) { if (!store || typeof store.run !== 'function') throw new TypeError('store must provide run'); this._stores.push({ store: store, transform: typeof transform === 'function' ? transform : function(value) { return value; } }); };\n\
             Channel.prototype.unbindStore = function(store) { var before = this._stores.length; this._stores = this._stores.filter(function(binding) { return binding.store !== store; }); return before !== this._stores.length; };\n\
             Channel.prototype.runStores = function(data, callback, thisArg) { var args = Array.prototype.slice.call(arguments, 3); var bindings = this._stores.slice(); function invoke(index) { if (index === bindings.length) return callback.apply(thisArg, args); var binding = bindings[index]; return binding.store.run(binding.transform(data), function() { return invoke(index + 1); }); } return invoke(0); };\n\
             Channel.prototype.publish = function(data) { var self = this; return this.runStores(data, function() { self._subscribers.slice().forEach(function(callback) { callback(data, self.name); }); }); };\n\
             function channel(name) { var key = String(name); if (!registry.has(key)) registry.set(key, new Channel(key)); return registry.get(key); }\n\
             function hasSubscribers(name) { return channel(name).hasSubscribers; }\n\
             function subscribe(name, callback) { channel(name).subscribe(callback); }\n\
             function unsubscribe(name, callback) { return channel(name).unsubscribe(callback); }\n\
             function tracingChannel(nameOrChannels) {\n\
             \x20\x20var channels = typeof nameOrChannels === 'string' ? { start: channel('tracing:' + nameOrChannels + ':start'), end: channel('tracing:' + nameOrChannels + ':end'), asyncStart: channel('tracing:' + nameOrChannels + ':asyncStart'), asyncEnd: channel('tracing:' + nameOrChannels + ':asyncEnd'), error: channel('tracing:' + nameOrChannels + ':error') } : nameOrChannels;\n\
             \x20\x20return { start: channels.start, end: channels.end, asyncStart: channels.asyncStart, asyncEnd: channels.asyncEnd, error: channels.error, traceSync: function(callback, context, thisArg) { var args = Array.prototype.slice.call(arguments, 3); context = context || {}; channels.start.publish(context); try { var result = channels.start.runStores(context, callback, thisArg, ...args); context.result = result; channels.end.publish(context); return result; } catch (error) { context.error = error; channels.error.publish(context); channels.end.publish(context); throw error; } }, tracePromise: function(callback, context, thisArg) { var args = Array.prototype.slice.call(arguments, 3); context = context || {}; channels.start.publish(context); return Promise.resolve().then(function() { return channels.start.runStores(context, callback, thisArg, ...args); }).then(function(result) { context.result = result; channels.asyncStart.publish(context); channels.asyncEnd.publish(context); channels.end.publish(context); return result; }, function(error) { context.error = error; channels.error.publish(context); channels.asyncStart.publish(context); channels.asyncEnd.publish(context); channels.end.publish(context); throw error; }); }, traceCallback: function(callback, position, context, thisArg) { var args = Array.prototype.slice.call(arguments, 4); context = context || {}; channels.start.publish(context); var original = args[position]; args[position] = function(error, result) { if (error) { context.error = error; channels.error.publish(context); } else context.result = result; channels.asyncStart.publish(context); try { return original.apply(this, arguments); } finally { channels.asyncEnd.publish(context); channels.end.publish(context); } }; return channels.start.runStores(context, callback, thisArg, ...args); } };\n\
             }\n\
             module.exports = { channel: channel, hasSubscribers: hasSubscribers, subscribe: subscribe, unsubscribe: unsubscribe, tracingChannel: tracingChannel, Channel: Channel }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "dgram" => Some(
            "function Socket(type, listener) { if (!(this instanceof Socket)) return new Socket(type, listener); var options = typeof type === 'object' ? type : { type: type }; this.type = options.type || 'udp4'; if (this.type !== 'udp4' && this.type !== 'udp6') throw new TypeError('Bad socket type'); this._events = Object.create(null); this._handle = 0; this._address = null; this._remote = null; this._refed = true; if (typeof listener === 'function') this.on('message', listener); } Socket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Socket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Socket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; };\n\
             Socket.prototype.bind = function(port, address, callback) { var options = typeof port === 'object' ? port : { port: port, address: address }; if (typeof address === 'function') callback = address; if (typeof callback === 'function') this.once('listening', callback); var host = String(options.address || (this.type === 'udp6' ? '::' : '0.0.0.0')); var outcome = __thaw_udp_bind(host, Number(options.port || 0)); if (outcome.indexOf('ok|') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split('|'); this._handle = Number(fields[1]); this._address = { address: fields[2], family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(fields[3]) }; queueMicrotask(() => { this.emit('listening'); var incoming = __thaw_udp_receive(this._handle); if (incoming.indexOf('ok|') !== 0) { if (this._handle) { var error = new Error(incoming.substring(4)); error.code = 'EIO'; this.emit('error', error); } return; } var parts = incoming.split('|'); var message = Buffer.from(parts[1], 'hex'); this.emit('message', message, { address: parts[2], family: parts[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(parts[3]), size: Number(parts[4]) }); }); return this; }; Socket.prototype.send = function(message) { var args = Array.prototype.slice.call(arguments, 1); var callback = typeof args[args.length - 1] === 'function' ? args.pop() : null; var port, address; if (args.length >= 4 && typeof args[0] === 'number' && typeof args[1] === 'number') { var offset = args.shift(), length = args.shift(); message = Buffer.from(message).subarray(offset, offset + length); } port = args.length ? Number(args.shift()) : this._remote && this._remote.port; address = args.length ? String(args.shift()) : this._remote && this._remote.address; if (!this._handle) { var bound = __thaw_udp_bind(this.type === 'udp6' ? '::' : '0.0.0.0', 0); if (bound.indexOf('ok|') !== 0) throw new Error(bound.substring(4)); var fields = bound.split('|'); this._handle = Number(fields[1]); this._address = { address: fields[2], family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(fields[3]) }; } if (!port || !address) throw new TypeError('Port and address are required'); var buffer = Array.isArray(message) ? Buffer.concat(message.map(function(value) { return Buffer.from(value); })) : Buffer.from(message); var outcome = __thaw_udp_send(this._handle, buffer.toString('hex'), address, port); if (outcome.indexOf('ok|') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EIO'; if (callback) queueMicrotask(function() { callback(error); }); else queueMicrotask(() => this.emit('error', error)); } else if (callback) queueMicrotask(function() { callback(null, Number(outcome.substring(3))); }); return this; };\n\
             Socket.prototype.connect = function(port, address, callback) { this._remote = { address: String(address || (this.type === 'udp6' ? '::1' : '127.0.0.1')), family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(port) }; if (callback) queueMicrotask(callback); return this; }; Socket.prototype.disconnect = function() { this._remote = null; }; Socket.prototype.address = function() { if (!this._address) throw new Error('Socket is not running'); return this._address; }; Socket.prototype.remoteAddress = function() { if (!this._remote) throw new Error('Socket is not connected'); return this._remote; }; Socket.prototype.close = function(callback) { if (callback) this.once('close', callback); if (this._handle) __thaw_udp_close(this._handle); this._handle = 0; queueMicrotask(() => this.emit('close')); return this; }; Socket.prototype.ref = function() { this._refed = true; return this; }; Socket.prototype.unref = function() { this._refed = false; return this; }; Socket.prototype.hasRef = function() { return this._refed; }; Socket.prototype.setBroadcast = Socket.prototype.setMulticastLoopback = Socket.prototype.setMulticastTTL = Socket.prototype.setTTL = function() { return this; }; Socket.prototype.addMembership = Socket.prototype.dropMembership = Socket.prototype.addSourceSpecificMembership = Socket.prototype.dropSourceSpecificMembership = function() { return this; }; Socket.prototype.getRecvBufferSize = Socket.prototype.getSendBufferSize = function() { return 212992; }; Socket.prototype.setRecvBufferSize = Socket.prototype.setSendBufferSize = function() { return this; }; function createSocket(type, listener) { return new Socket(type, listener); } module.exports = { Socket: Socket, createSocket: createSocket }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "dns" => Some(
            "var defaultOrder = globalThis.__thaw_dns_order || 'verbatim'; function familyOf(value) { value = String(value); if (/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value) && value.split('.').every(function(part) { return Number(part) <= 255; })) return 4; if (value.indexOf(':') >= 0) return 6; return 0; } function addresses(hostname, family) { var direct = familyOf(hostname); if (direct) return !family || family === direct ? [{ address: String(hostname), family: direct }] : []; if (String(hostname).toLowerCase() === 'localhost') { if (family === 4) return [{ address: '127.0.0.1', family: 4 }]; if (family === 6) return [{ address: '::1', family: 6 }]; return [{ address: '127.0.0.1', family: 4 }, { address: '::1', family: 6 }]; } return []; } function notFound(hostname, syscall) { var error = new Error('getaddrinfo ENOTFOUND ' + hostname); error.code = 'ENOTFOUND'; error.errno = -3008; error.syscall = syscall || 'getaddrinfo'; error.hostname = String(hostname); return error; } function lookup(hostname, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } else if (typeof options === 'number') options = { family: options }; options = options || {}; queueMicrotask(function() { var found = addresses(hostname, Number(options.family || 0)); if (!found.length) { callback(notFound(hostname)); return; } if (options.all) callback(null, found); else callback(null, found[0].address, found[0].family); }); } function resolve(hostname, rrtype, callback) { if (typeof rrtype === 'function') { callback = rrtype; rrtype = 'A'; } queueMicrotask(function() { var found = addresses(hostname, String(rrtype || 'A').toUpperCase() === 'AAAA' ? 6 : 4); if (!found.length) callback(notFound(hostname, 'query' + String(rrtype || 'A'))); else callback(null, found.map(function(entry) { return entry.address; })); }); } function reverse(ip, callback) { queueMicrotask(function() { if (ip === '127.0.0.1' || ip === '::1') callback(null, ['localhost']); else callback(notFound(ip, 'getHostByAddr')); }); } function Resolver() { this._servers = []; } Resolver.prototype.setServers = function(servers) { this._servers = Array.from(servers, String); }; Resolver.prototype.getServers = function() { return this._servers.slice(); }; Resolver.prototype.resolve = resolve; Resolver.prototype.reverse = reverse; var promises = { lookup: function(hostname, options) { return new Promise(function(resolvePromise, reject) { lookup(hostname, options, function(error, address, family) { if (error) reject(error); else if (options && options.all) resolvePromise(address); else resolvePromise({ address: address, family: family }); }); }); }, resolve: function(hostname, rrtype) { return new Promise(function(resolvePromise, reject) { resolve(hostname, rrtype, function(error, value) { error ? reject(error) : resolvePromise(value); }); }); }, reverse: function(ip) { return new Promise(function(resolvePromise, reject) { reverse(ip, function(error, value) { error ? reject(error) : resolvePromise(value); }); }); } }; module.exports = { lookup: lookup, resolve: resolve, resolve4: function(hostname, callback) { resolve(hostname, 'A', callback); }, resolve6: function(hostname, callback) { resolve(hostname, 'AAAA', callback); }, reverse: reverse, Resolver: Resolver, promises: promises, getDefaultResultOrder: function() { return defaultOrder; }, setDefaultResultOrder: function(order) { defaultOrder = String(order); globalThis.__thaw_dns_order = defaultOrder; }, getServers: function() { return []; }, setServers: function() {}, ADDRCONFIG: 32, V4MAPPED: 8, NODATA: 'ENODATA', FORMERR: 'EFORMERR', SERVFAIL: 'ESERVFAIL', NOTFOUND: 'ENOTFOUND', NOTIMP: 'ENOTIMP', REFUSED: 'EREFUSED' }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "dns/promises" => Some(
            "function familyOf(value) { value = String(value); if (/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value) && value.split('.').every(function(part) { return Number(part) <= 255; })) return 4; if (value.indexOf(':') >= 0) return 6; return 0; } function addresses(hostname, family) { var direct = familyOf(hostname); if (direct) return !family || family === direct ? [{ address: String(hostname), family: direct }] : []; if (String(hostname).toLowerCase() === 'localhost') { if (family === 4) return [{ address: '127.0.0.1', family: 4 }]; if (family === 6) return [{ address: '::1', family: 6 }]; return [{ address: '127.0.0.1', family: 4 }, { address: '::1', family: 6 }]; } return []; } function failure(hostname) { var error = new Error('getaddrinfo ENOTFOUND ' + hostname); error.code = 'ENOTFOUND'; error.errno = -3008; error.syscall = 'getaddrinfo'; error.hostname = String(hostname); return error; } function lookup(hostname, options) { options = typeof options === 'number' ? { family: options } : (options || {}); var found = addresses(hostname, Number(options.family || 0)); if (!found.length) return Promise.reject(failure(hostname)); return Promise.resolve(options.all ? found : found[0]); } function resolve(hostname, rrtype) { var found = addresses(hostname, String(rrtype || 'A').toUpperCase() === 'AAAA' ? 6 : 4); return found.length ? Promise.resolve(found.map(function(entry) { return entry.address; })) : Promise.reject(failure(hostname)); } function reverse(ip) { return ip === '127.0.0.1' || ip === '::1' ? Promise.resolve(['localhost']) : Promise.reject(failure(ip)); } function Resolver() { this._servers = []; } Resolver.prototype.setServers = function(servers) { this._servers = Array.from(servers, String); }; Resolver.prototype.getServers = function() { return this._servers.slice(); }; Resolver.prototype.resolve = resolve; Resolver.prototype.reverse = reverse; module.exports = { lookup: lookup, resolve: resolve, resolve4: function(hostname) { return resolve(hostname, 'A'); }, resolve6: function(hostname) { return resolve(hostname, 'AAAA'); }, reverse: reverse, Resolver: Resolver, getDefaultResultOrder: function() { return globalThis.__thaw_dns_order || 'verbatim'; }, setDefaultResultOrder: function(order) { globalThis.__thaw_dns_order = String(order); }, getServers: function() { return []; }, setServers: function() {} }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "async_hooks" => Some(
            "var storages = globalThis.__thawAsyncLocalStorages || (globalThis.__thawAsyncLocalStorages = new Set()); var nextAsyncId = globalThis.__thawNextAsyncId || 2; var currentAsyncId = 1; var currentTriggerId = 0; var currentResource = {};\n\
             function restoreAfter(storage, previous, result) { if (result && typeof result.then === 'function') return Promise.resolve(result).finally(function() { storage._store = previous; }); storage._store = previous; return result; }\n\
             function AsyncLocalStorage(options) { if (!(this instanceof AsyncLocalStorage)) return new AsyncLocalStorage(options); this._store = options && options.defaultValue; this.name = options && options.name || ''; this.enabled = true; storages.add(this); }\n\
             AsyncLocalStorage.prototype.disable = function() { this.enabled = false; this._store = undefined; };\n\
             AsyncLocalStorage.prototype.getStore = function() { return this.enabled ? this._store : undefined; };\n\
             AsyncLocalStorage.prototype.enterWith = function(store) { this.enabled = true; this._store = store; storages.add(this); };\n\
             AsyncLocalStorage.prototype.run = function(store, callback) { var args = Array.prototype.slice.call(arguments, 2); var previous = this._store; this.enabled = true; this._store = store; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             AsyncLocalStorage.prototype.exit = function(callback) { var args = Array.prototype.slice.call(arguments, 1); var previous = this._store; this._store = undefined; try { return restoreAfter(this, previous, callback.apply(null, args)); } catch (error) { this._store = previous; throw error; } };\n\
             function captureStores() { return Array.from(storages).map(function(storage) { return [storage, storage.getStore()]; }); }\n\
             function invokeCaptured(captured, callback, thisArg, args, index) { if (index === captured.length) return callback.apply(thisArg, args); var entry = captured[index]; return entry[0].run(entry[1], function() { return invokeCaptured(captured, callback, thisArg, args, index + 1); }); }\n\
             AsyncLocalStorage.bind = function(callback) { var captured = captureStores(); return function() { return invokeCaptured(captured, callback, this, Array.prototype.slice.call(arguments), 0); }; };\n\
             AsyncLocalStorage.snapshot = function() { var captured = captureStores(); return function(callback) { return invokeCaptured(captured, callback, null, Array.prototype.slice.call(arguments, 1), 0); }; };\n\
             function AsyncResource(type, options) { if (!(this instanceof AsyncResource)) return new AsyncResource(type, options); this.type = String(type); this._asyncId = nextAsyncId++; globalThis.__thawNextAsyncId = nextAsyncId; this._triggerAsyncId = options && options.triggerAsyncId !== undefined ? Number(options.triggerAsyncId) : currentAsyncId; this._destroyed = false; }\n\
             AsyncResource.prototype.asyncId = function() { return this._asyncId; }; AsyncResource.prototype.triggerAsyncId = function() { return this._triggerAsyncId; };\n\
             AsyncResource.prototype.runInAsyncScope = function(callback, thisArg) { var args = Array.prototype.slice.call(arguments, 2); var previousId = currentAsyncId, previousTrigger = currentTriggerId, previousResource = currentResource; currentAsyncId = this._asyncId; currentTriggerId = this._triggerAsyncId; currentResource = this; try { return callback.apply(thisArg, args); } finally { currentAsyncId = previousId; currentTriggerId = previousTrigger; currentResource = previousResource; } };\n\
             AsyncResource.prototype.emitDestroy = function() { this._destroyed = true; return this; }; AsyncResource.prototype.bind = function(callback, thisArg) { var self = this; return function() { return self.runInAsyncScope(callback, thisArg === undefined ? this : thisArg, ...arguments); }; };\n\
             AsyncResource.bind = function(callback, type, thisArg) { return new AsyncResource(type || callback.name || 'bound-anonymous-fn').bind(callback, thisArg); };\n\
             function createHook(callbacks) { return { enable: function() { return this; }, disable: function() { return this; }, callbacks: callbacks || {} }; }\n\
             function executionAsyncId() { return currentAsyncId; } function triggerAsyncId() { return currentTriggerId; } function executionAsyncResource() { return currentResource; }\n\
             module.exports = { AsyncLocalStorage: AsyncLocalStorage, AsyncResource: AsyncResource, createHook: createHook, executionAsyncId: executionAsyncId, triggerAsyncId: triggerAsyncId, executionAsyncResource: executionAsyncResource }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "tty" => Some(
            "function isatty(fd) { return false; }\n\
             function ReadStream(fd, options) { if (!(this instanceof ReadStream)) return new ReadStream(fd, options); this.fd = Number(fd); this.isRaw = false; this.isTTY = true; this.readable = true; this.destroyed = false; }\n\
             ReadStream.prototype.setRawMode = function(mode) { this.isRaw = Boolean(mode); return this; }; ReadStream.prototype.ref = function() { return this; }; ReadStream.prototype.unref = function() { return this; };\n\
             function WriteStream(fd) { if (!(this instanceof WriteStream)) return new WriteStream(fd); this.fd = Number(fd); this.isTTY = true; this.columns = 80; this.rows = 24; this.writable = true; this.destroyed = false; this._output = ''; }\n\
             WriteStream.prototype.write = function(value, callback) { this._output += String(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.getColorDepth = function(environment) { var env = environment || globalThis.process && process.env || {}; if (env.FORCE_COLOR === '0' || env.NO_COLOR !== undefined) return 1; if (env.FORCE_COLOR === '3' || env.COLORTERM === 'truecolor') return 24; if (env.FORCE_COLOR === '2' || /256color/i.test(env.TERM || '')) return 8; if (env.FORCE_COLOR === '1' || /color|ansi|xterm|screen/i.test(env.TERM || '')) return 4; return 1; };\n\
             WriteStream.prototype.hasColors = function(count, environment) { if (typeof count === 'object') { environment = count; count = 16; } count = count === undefined ? 16 : Number(count); return Math.pow(2, this.getColorDepth(environment)) >= count; };\n\
             WriteStream.prototype._ansi = function(sequence, callback) { this._output += sequence; if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.clearLine = function(direction, callback) { var sequence = Number(direction) < 0 ? '\\u001b[1K' : Number(direction) > 0 ? '\\u001b[0K' : '\\u001b[2K'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.clearScreenDown = function(callback) { return this._ansi('\\u001b[0J', callback); };\n\
             WriteStream.prototype.cursorTo = function(x, y, callback) { if (typeof y === 'function') { callback = y; y = undefined; } var sequence = y === undefined ? '\\u001b[' + (Number(x) + 1) + 'G' : '\\u001b[' + (Number(y) + 1) + ';' + (Number(x) + 1) + 'H'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.moveCursor = function(dx, dy, callback) { var sequence = ''; dx = Number(dx); dy = Number(dy); if (dx < 0) sequence += '\\u001b[' + -dx + 'D'; else if (dx > 0) sequence += '\\u001b[' + dx + 'C'; if (dy < 0) sequence += '\\u001b[' + -dy + 'A'; else if (dy > 0) sequence += '\\u001b[' + dy + 'B'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.getWindowSize = function() { return [this.columns, this.rows]; }; WriteStream.prototype.ref = function() { return this; }; WriteStream.prototype.unref = function() { return this; };\n\
             module.exports = { isatty: isatty, ReadStream: ReadStream, WriteStream: WriteStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "module" => Some(
            "var builtinModules = ['assert','assert/strict','async_hooks','buffer','console','constants','crypto','dgram','diagnostics_channel','dns','dns/promises','events','fs','fs/promises','http','module','net','os','path','perf_hooks','process','punycode','querystring','readline','readline/promises','stream','stream/consumers','stream/promises','stream/web','string_decoder','timers','timers/promises','tls','tty','url','util','util/types','v8','vm','worker_threads','zlib']; var builtinSet = new Set(builtinModules);\n\
             function isBuiltin(name) { var value = String(name); return builtinSet.has(value.replace(/^node:/, '')); }\n\
             function createRequire(filename) { if (typeof globalThis.__thaw_bundle_create_require !== 'function') throw new Error('createRequire is only available inside a Thaw bundle'); return globalThis.__thaw_bundle_create_require(filename); }\n\
             function Module(id, parent) { if (!(this instanceof Module)) return new Module(id, parent); this.id = id === undefined ? '' : String(id); this.path = this.id; this.exports = {}; this.filename = null; this.loaded = false; this.parent = parent || null; this.children = []; this.paths = []; if (parent && parent.children) parent.children.push(this); }\n\
             Module.builtinModules = builtinModules; Module.isBuiltin = isBuiltin; Module.createRequire = createRequire; Module._cache = {}; Module._extensions = { '.js': function() {}, '.json': function() {}, '.node': function() {} };\n\
             function syncBuiltinESMExports() {} function findSourceMap() { return undefined; }\n\
             function SourceMap(payload) { if (!(this instanceof SourceMap)) return new SourceMap(payload); this.payload = payload || {}; } SourceMap.prototype.findEntry = function(line, column) { return { generatedLine: Number(line), generatedColumn: Number(column || 0), originalSource: undefined, originalLine: undefined, originalColumn: undefined, name: undefined }; }; SourceMap.prototype.findOrigin = function(line, column) { return this.findEntry(line, column); };\n\
             function register() { return undefined; } function registerHooks(hooks) { var active = true; return { deregister: function() { active = false; }, get active() { return active; }, hooks: hooks }; }\n\
             Object.assign(Module, { Module: Module, createRequire: createRequire, builtinModules: builtinModules, isBuiltin: isBuiltin, syncBuiltinESMExports: syncBuiltinESMExports, findSourceMap: findSourceMap, SourceMap: SourceMap, register: register, registerHooks: registerHooks });\n\
             module.exports = Module; module.exports.default = Module; module.exports.__esModule = true;\n",
        ),
        "net" => Some(concat!(
            "function isIPv4(value) { if (typeof value !== 'string' || !/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value)) return false; return value.split('.').every(function(part) { return String(Number(part)) === part && Number(part) <= 255; }); } function isIPv6(value) { if (typeof value !== 'string') return false; var address = value.split('%')[0]; if (address.indexOf(':') < 0 || address.indexOf(':::') >= 0) return false; var halves = address.split('::'); if (halves.length > 2) return false; function count(part) { if (!part) return 0; var groups = part.split(':'); for (var index = 0; index < groups.length; index++) { if (isIPv4(groups[index])) { if (index !== groups.length - 1) return -1; } else if (!/^[0-9a-fA-F]{1,4}$/.test(groups[index])) return -1; } return groups.reduce(function(total, group) { return total + (isIPv4(group) ? 2 : 1); }, 0); } var total = count(halves[0]) + count(halves[1] || ''); return total >= 0 && (halves.length === 2 ? total < 8 : total === 8); } function isIP(value) { return isIPv4(value) ? 4 : isIPv6(value) ? 6 : 0; } function ipv4Number(value) { return value.split('.').reduce(function(result, part) { return (result * 256 + Number(part)) >>> 0; }, 0); }\n\
             function SocketAddress(options) { if (!(this instanceof SocketAddress)) return new SocketAddress(options); options = options || {}; this.address = options.address === undefined ? '127.0.0.1' : String(options.address); var detected = isIP(this.address); var requested = options.family === undefined ? detected : (String(options.family).toLowerCase() === 'ipv6' || Number(options.family) === 6 ? 6 : 4); if (!detected || detected !== requested) throw new TypeError('Invalid socket address'); this.family = requested === 6 ? 'ipv6' : 'ipv4'; this.port = options.port === undefined ? 0 : Number(options.port); if (!Number.isInteger(this.port) || this.port < 0 || this.port > 65535) throw new RangeError('port must be between 0 and 65535'); this.flowlabel = options.flowlabel === undefined ? 0 : Number(options.flowlabel); } SocketAddress.prototype.toJSON = function() { return { address: this.address, port: this.port, family: this.family, flowlabel: this.flowlabel }; };\n\
             function BlockList() { if (!(this instanceof BlockList)) return new BlockList(); this.rules = []; } BlockList.prototype.addAddress = function(address, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(address); if (!family) throw new TypeError('Invalid IP address'); this.rules.push({ kind: 'address', address: String(address), family: family }); }; BlockList.prototype.addRange = function(start, end, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(start); if (!family || family !== isIP(end)) throw new TypeError('Invalid IP range'); this.rules.push({ kind: 'range', start: String(start), end: String(end), family: family }); }; BlockList.prototype.addSubnet = function(network, prefix, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(network); prefix = Number(prefix); if (!family || !Number.isInteger(prefix) || prefix < 0 || prefix > (family === 4 ? 32 : 128)) throw new TypeError('Invalid subnet'); this.rules.push({ kind: 'subnet', network: String(network), prefix: prefix, family: family }); }; BlockList.prototype.check = function(address, type) { var family = type === 'ipv6' ? 6 : type === 'ipv4' ? 4 : isIP(address); if (!family) return false; return this.rules.some(function(rule) { if (rule.family !== family) return false; if (rule.kind === 'address') return rule.address === address; if (family === 4) { var value = ipv4Number(address); if (rule.kind === 'range') return value >= ipv4Number(rule.start) && value <= ipv4Number(rule.end); var mask = rule.prefix === 0 ? 0 : (0xffffffff << (32 - rule.prefix)) >>> 0; return (value & mask) === (ipv4Number(rule.network) & mask); } if (rule.kind === 'range') return address >= rule.start && address <= rule.end; return rule.prefix === 128 ? address === rule.network : address.toLowerCase().startsWith(rule.network.toLowerCase().split('::')[0]); }); };\n\
             function Socket(options) { if (!(this instanceof Socket)) return new Socket(options); this._events = Object.create(null); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.pending = true; this.bytesRead = 0; this.bytesWritten = 0; this.remoteAddress = undefined; this.remotePort = undefined; this.remoteFamily = undefined; this._refed = true; } Socket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Socket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Socket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; Socket.prototype.connect = function(port, host, listener) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') listener = host; if (typeof listener === 'function') this.once('connect', listener); var hostname = String(options.host || 'localhost'); var numericPort = Number(options.port); this.connecting = true; var outcome = __thaw_net_connect(hostname, numericPort); this.connecting = false; this.pending = false; if (outcome.indexOf('ok:') === 0) { this._handle = Number(outcome.substring(3)); this.readable = true; this.writable = true; this.remoteAddress = hostname; this.remotePort = numericPort; this.remoteFamily = isIPv6(hostname) ? 'IPv6' : 'IPv4'; queueMicrotask(() => this.emit('connect')); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNREFUSED'; error.syscall = 'connect'; error.address = hostname; error.port = numericPort; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; }; Socket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (!this._handle || !this.writable) throw new Error('Socket is not writable'); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_net_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') throw new Error(outcome.substring(4)); this.bytesWritten += buffer.length; if (typeof callback === 'function') queueMicrotask(callback); return true; }; Socket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (!this._handle) return this; var outcome = __thaw_net_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } this.emit('end'); if (typeof callback === 'function') callback(); this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); this.emit('close', true); } }); return this; }; Socket.prototype.destroy = function(error) { if (this._handle) __thaw_net_destroy(this._handle); this._handle = 0; this.destroyed = true; this.readable = false; this.writable = false; queueMicrotask(() => { if (error) this.emit('error', error); this.emit('close', Boolean(error)); }); return this; }; Socket.prototype.setEncoding = function(encoding) { var original = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return original.call(this, name, value); }; return this; }; Socket.prototype.setTimeout = function(timeout, callback) { if (typeof callback === 'function') this.once('timeout', callback); return this; }; Socket.prototype.setNoDelay = Socket.prototype.setKeepAlive = function() { return this; }; Socket.prototype.ref = function() { this._refed = true; return this; }; Socket.prototype.unref = function() { this._refed = false; return this; }; Socket.prototype.hasRef = function() { return this._refed; }; function createConnection() { var socket = new Socket(); return socket.connect.apply(socket, arguments); }\n\
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); if (typeof options === 'function') { listener = options; options = {}; } this._events = Object.create(null); this._handle = 0; this._address = null; this.listening = false; this.maxConnections = 0; this.connections = 0; this._refed = true; if (typeof listener === 'function') this.on('connection', listener); } Server.prototype.on = Socket.prototype.on; Server.prototype.once = Socket.prototype.once; Server.prototype.off = Server.prototype.removeListener = Socket.prototype.off; Server.prototype.emit = Socket.prototype.emit; Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_net_accept(this._handle); if (accepted.indexOf('ok:') !== 0) return; var peer = accepted.split(':'); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; this.connections++; this.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } }); return this; }; Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_net_close_listener(this._handle); this._handle = 0; this.listening = false; queueMicrotask(() => this.emit('close')); return this; }; Server.prototype.closeAllConnections = Server.prototype.closeIdleConnections = function() {}; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }\n\
             module.exports = { isIP: isIP, isIPv4: isIPv4, isIPv6: isIPv6, BlockList: BlockList, SocketAddress: SocketAddress, Socket: Socket, createConnection: createConnection, connect: createConnection, Server: Server, createServer: createServer }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_net_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { var error = new Error(accepted.substring(4)); error.code = 'ECONNABORTED'; server.emit('error', error); if (server._handle) setTimeout(pump, 1); return; } var peer = accepted.split(':'); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); }); server.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
"#,
        )),
        "tls" => Some(concat!(
            "function TLSSocket(socket, options) { if (!(this instanceof TLSSocket)) return new TLSSocket(socket, options); this._events = Object.create(null); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.encrypted = true; this.authorized = false; this.authorizationError = null; this.alpnProtocol = false; this.servername = null; this.bytesRead = 0; this.bytesWritten = 0; this._refed = true; } TLSSocket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; TLSSocket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; TLSSocket.prototype.off = TLSSocket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; TLSSocket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; if (typeof listener === 'function') this.once('secureConnect', listener); var host = String(options.host || 'localhost'), port = Number(options.port), servername = String(options.servername || host); var ca = options.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'); this.connecting = true; var outcome = __thaw_tls_connect(host, port, servername, caHex); this.connecting = false; this.servername = servername; if (outcome.indexOf('ok:') === 0) { this._handle = Number(outcome.substring(3)); this.readable = true; this.writable = true; this.authorized = true; queueMicrotask(() => { this.emit('connect'); this.emit('secureConnect'); }); } else { var error = new Error(outcome.substring(4)); error.code = 'ERR_TLS_CERT_ALTNAME_INVALID'; this.authorizationError = error.message; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; }; TLSSocket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (!this._handle || !this.writable) throw new Error('TLS socket is not writable'); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_tls_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') throw new Error(outcome.substring(4)); this.bytesWritten += buffer.length; if (callback) queueMicrotask(callback); return true; }; TLSSocket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); var outcome = __thaw_tls_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } this.emit('end'); if (callback) callback(); this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); this.emit('close', true); } }); return this; }; TLSSocket.prototype.destroy = function(error) { if (this._handle) __thaw_tls_destroy(this._handle); this._handle = 0; this.destroyed = true; if (error) queueMicrotask(() => this.emit('error', error)); queueMicrotask(() => this.emit('close', Boolean(error))); return this; }; TLSSocket.prototype.setEncoding = function(encoding) { var emit = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return emit.call(this, name, value); }; return this; }; TLSSocket.prototype.getProtocol = function() { return this.authorized ? 'TLSv1.3' : null; }; TLSSocket.prototype.getCipher = function() { return { name: 'TLS_AES_256_GCM_SHA384', standardName: 'TLS_AES_256_GCM_SHA384', version: 'TLSv1.3' }; }; TLSSocket.prototype.getPeerCertificate = function() { return this.authorized ? { subject: {}, issuer: {}, valid_from: '', valid_to: '' } : {}; }; TLSSocket.prototype.getCertificate = function() { return {}; }; TLSSocket.prototype.getFinished = TLSSocket.prototype.getPeerFinished = function() { return undefined; }; TLSSocket.prototype.isSessionReused = function() { return false; }; TLSSocket.prototype.renegotiate = function(options, callback) { if (callback) queueMicrotask(function() { callback(new Error('TLS renegotiation is not supported')); }); return false; }; TLSSocket.prototype.setMaxSendFragment = function() { return true; }; TLSSocket.prototype.enableTrace = function() {}; TLSSocket.prototype.ref = function() { this._refed = true; return this; }; TLSSocket.prototype.unref = function() { this._refed = false; return this; }; function connect(options, listener) { return new TLSSocket().connect(options, listener); } function createSecureContext(options) { return { context: options || {}, options: options || {} }; } function checkServerIdentity() { return undefined; } function getCiphers() { return ['tls_aes_128_gcm_sha256', 'tls_aes_256_gcm_sha384', 'tls_chacha20_poly1305_sha256']; } module.exports = { TLSSocket: TLSSocket, connect: connect, createSecureContext: createSecureContext, checkServerIdentity: checkServerIdentity, getCiphers: getCiphers, rootCertificates: [], DEFAULT_MIN_VERSION: 'TLSv1.2', DEFAULT_MAX_VERSION: 'TLSv1.3', CLIENT_RENEG_LIMIT: 3, CLIENT_RENEG_WINDOW: 600 }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); this._events = Object.create(null); this._options = options || {}; this._handle = 0; this._address = null; this.listening = false; this.connections = 0; this.maxConnections = 0; this._refed = true; if (typeof listener === 'function') this.on('secureConnection', listener); }
             Server.prototype.on = TLSSocket.prototype.on; Server.prototype.once = TLSSocket.prototype.once; Server.prototype.off = Server.prototype.removeListener = TLSSocket.prototype.off; Server.prototype.emit = TLSSocket.prototype.emit;
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var outcome = __thaw_tls_server_listen(hostname, Number(options.port), Buffer.from(cert).toString('hex'), Buffer.from(key).toString('hex')); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_tls_server_accept(this._handle); if (accepted.indexOf('ok:') !== 0) { this.emit('tlsClientError', new Error(accepted.substring(4))); return; } var peer = accepted.split(':'); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; this.connections++; this.emit('secureConnection', socket); var incoming = __thaw_tls_server_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else socket.emit('error', new Error(incoming.substring(4))); }); return this; };
             Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_tls_server_close(this._handle); this._handle = 0; this.listening = false; queueMicrotask(() => this.emit('close')); return this; }; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var connectWithoutIdentity = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function(options, listener) { options = options || {}; if (options.cert === undefined && options.key === undefined) return connectWithoutIdentity.call(this, options, listener); var cert = options.cert, key = options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; var nativeConnect = __thaw_tls_connect; __thaw_tls_connect = function(host, port, servername, ca) { return __thaw_tls_connect_with_identity(host, port, servername, ca, cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex')); }; try { return connectWithoutIdentity.call(this, options, listener); } finally { __thaw_tls_connect = nativeConnect; } };
             var listenWithoutClientAuth = Server.prototype.listen; Server.prototype.listen = function() { if (!this._options.requestCert) return listenWithoutClientAuth.apply(this, arguments); var ca = this._options.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'); var rejectUnauthorized = this._options.rejectUnauthorized !== false; var nativeListen = __thaw_tls_server_listen; __thaw_tls_server_listen = function(host, port, cert, key) { return __thaw_tls_server_listen_with_ca(host, port, cert, key, caHex, rejectUnauthorized); }; try { return listenWithoutClientAuth.apply(this, arguments); } finally { __thaw_tls_server_listen = nativeListen; } };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key, ca = this._options.ca; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (Array.isArray(ca)) ca = ca[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var certHex = Buffer.from(cert).toString('hex'), keyHex = Buffer.from(key).toString('hex'); var outcome = this._options.requestCert ? __thaw_tls_server_listen_with_ca(hostname, Number(options.port), certHex, keyHex, ca === undefined ? '' : Buffer.from(ca).toString('hex'), this._options.rejectUnauthorized !== false) : __thaw_tls_server_listen(hostname, Number(options.port), certHex, keyHex); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_tls_server_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { server.emit('tlsClientError', new Error(accepted.substring(4))); if (server._handle) setTimeout(pump, 1); return; } var peer = accepted.split(':'); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); }); server.emit('secureConnection', socket); var incoming = __thaw_tls_server_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else socket.emit('error', new Error(incoming.substring(4))); if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
             function alpnSpec(value) { if (value === undefined) return ''; var values = Array.isArray(value) ? value : [value]; return values.map(function(protocol) { return Buffer.from(protocol).toString('hex'); }).join(','); }
             TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; if (typeof listener === 'function') this.once('secureConnect', listener); var host = String(options.host || 'localhost'), port = Number(options.port), servername = String(options.servername || host), ca = options.ca, cert = options.cert, key = options.key; if (Array.isArray(ca)) ca = ca[0]; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; this.connecting = true; var tlsOptions = (options.rejectUnauthorized === false ? '0|' : '1|') + alpnSpec(options.ALPNProtocols); var outcome = __thaw_tls_connect_with_options(host, port, servername, ca === undefined ? '' : Buffer.from(ca).toString('hex'), cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex'), tlsOptions); this.connecting = false; this.servername = servername; if (outcome.indexOf('ok:') === 0) { var fields = outcome.split(':'); this._handle = Number(fields[1]); this.alpnProtocol = fields[2] ? Buffer.from(fields[2], 'hex').toString() : false; this.readable = true; this.writable = true; this.authorized = options.rejectUnauthorized !== false; this.authorizationError = this.authorized ? null : 'UNABLE_TO_VERIFY_LEAF_SIGNATURE'; queueMicrotask(() => { this.emit('connect'); this.emit('secureConnect'); }); } else { var error = new Error(outcome.substring(4)); error.code = 'ERR_TLS_CERT_ALTNAME_INVALID'; this.authorizationError = error.message; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; };
             var listenWithoutAlpn = Server.prototype.listen; Server.prototype.listen = function() { var serverOptions = this._options, protocols = alpnSpec(serverOptions.ALPNProtocols), ca = serverOptions.ca; if (Array.isArray(ca)) ca = ca[0]; var caHex = ca === undefined ? '' : Buffer.from(ca).toString('hex'), flags = (serverOptions.requestCert ? 1 : 0) | (serverOptions.rejectUnauthorized !== false ? 2 : 0); var basic = __thaw_tls_server_listen, withCa = __thaw_tls_server_listen_with_ca; __thaw_tls_server_listen = __thaw_tls_server_listen_with_ca = function(host, port, cert, key) { return __thaw_tls_server_listen_with_options(host, port, cert, key, caHex, flags, protocols); }; try { return listenWithoutAlpn.apply(this, arguments); } finally { __thaw_tls_server_listen = basic; __thaw_tls_server_listen_with_ca = withCa; } };
             var emitWithoutAlpn = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection' && arguments[1]) { var protocol = __thaw_tls_alpn(arguments[1]._handle); arguments[1].alpnProtocol = protocol ? Buffer.from(protocol, 'hex').toString() : false; } return emitWithoutAlpn.apply(this, arguments); };
             function rememberCertificates(socket) { if (!socket || !socket._handle) return; var peer = __thaw_tls_peer_certificate(socket._handle), local = __thaw_tls_local_certificate(socket._handle); socket._peerCertificateRaw = peer ? Buffer.from(peer, 'hex') : undefined; socket._localCertificateRaw = local ? Buffer.from(local, 'hex') : undefined; socket._peerCertificateMetadata = JSON.parse(__thaw_tls_peer_certificate_metadata(socket._handle)); socket._localCertificateMetadata = JSON.parse(__thaw_tls_local_certificate_metadata(socket._handle)); }
             function certificateInfo(raw, metadata) { if (!raw) return {}; var digest = __thaw_crypto_hash_hex('sha256', raw.toString('hex')).toUpperCase().replace(/(..)(?=.)/g, '$1:'); return Object.assign({ raw: Buffer.from(raw), fingerprint256: digest }, metadata || {}); }
             var connectWithoutCertificates = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function() { var result = connectWithoutCertificates.apply(this, arguments); rememberCertificates(this); return result; }; TLSSocket.prototype.getPeerCertificate = function() { return certificateInfo(this._peerCertificateRaw, this._peerCertificateMetadata); }; TLSSocket.prototype.getCertificate = function() { return certificateInfo(this._localCertificateRaw, this._localCertificateMetadata); };
             var emitWithoutCertificates = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection') rememberCertificates(arguments[1]); return emitWithoutCertificates.apply(this, arguments); };
             module.exports.Server = Server; module.exports.createServer = createServer;
"#,
        )),
        "console" => Some(
            "module.exports = globalThis.console; module.exports.Console = globalThis.Console; module.exports.console = globalThis.console; module.exports.default = globalThis.console; module.exports.__esModule = true;\n",
        ),
        "constants" => Some(
            "var constants = globalThis.__thaw_fs_constants || (globalThis.__thaw_fs_constants = { F_OK: 0, X_OK: 1, W_OK: 2, R_OK: 4, O_RDONLY: 0, O_WRONLY: 1, O_RDWR: 2, O_CREAT: 64, O_EXCL: 128, O_NOCTTY: 256, O_TRUNC: 512, O_APPEND: 1024, O_DIRECTORY: 65536, O_NOFOLLOW: 131072, O_SYNC: 1052672, S_IFMT: 61440, S_IFREG: 32768, S_IFDIR: 16384, S_IFCHR: 8192, S_IFBLK: 24576, S_IFIFO: 4096, S_IFLNK: 40960, S_IFSOCK: 49152, COPYFILE_EXCL: 1, COPYFILE_FICLONE: 2, COPYFILE_FICLONE_FORCE: 4 }); module.exports = constants; module.exports.default = constants; module.exports.__esModule = true;\n",
        ),
        "crypto" => Some(
            "module.exports = globalThis.__thaw_crypto_module; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "perf_hooks" => Some(
            "function Histogram() { this.enabled = false; this.min = 0; this.max = 0; this.mean = 0; this.stddev = 0; this.exceeds = 0; this.count = 0; this.percentiles = new Map([[0, 0], [50, 0], [75, 0], [90, 0], [99, 0], [100, 0]]); }\n\
             Histogram.prototype.enable = function() { this.enabled = true; return true; }; Histogram.prototype.disable = function() { this.enabled = false; return true; }; Histogram.prototype.reset = function() { this.min = this.max = this.mean = this.stddev = this.exceeds = this.count = 0; }; Histogram.prototype.percentile = function() { return 0; }; Histogram.prototype.percentileBigInt = function() { return 0n; };\n\
             function monitorEventLoopDelay() { return new Histogram(); } function createHistogram() { return new Histogram(); }\n\
             var started = globalThis.performance.timeOrigin; globalThis.performance.nodeTiming = globalThis.performance.nodeTiming || { name: 'node', entryType: 'node', startTime: 0, duration: globalThis.performance.now(), nodeStart: 0, v8Start: 0, bootstrapComplete: 0, environment: 0, loopStart: 0, loopExit: -1, idleTime: 0 };\n\
             globalThis.performance.eventLoopUtilization = globalThis.performance.eventLoopUtilization || function(previous) { var active = globalThis.performance.now(); if (previous) active = Math.max(0, active - Number(previous.active || 0)); return { idle: 0, active: active, utilization: active === 0 ? 0 : 1 }; };\n\
             module.exports = { performance: globalThis.performance, PerformanceEntry: globalThis.PerformanceEntry, PerformanceMark: globalThis.PerformanceMark, PerformanceMeasure: globalThis.PerformanceMeasure, PerformanceObserver: globalThis.PerformanceObserver, PerformanceObserverEntryList: globalThis.PerformanceObserverEntryList, monitorEventLoopDelay: monitorEventLoopDelay, createHistogram: createHistogram, constants: { NODE_PERFORMANCE_GC_MAJOR: 4, NODE_PERFORMANCE_GC_MINOR: 1, NODE_PERFORMANCE_GC_INCREMENTAL: 8, NODE_PERFORMANCE_GC_WEAKCB: 16 }, timerify: globalThis.performance.timerify };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "v8" => Some(
            "function encode(root) { var seen = new Map(); var nodes = []; function visit(value) { if (value === undefined) return { t: 'u' }; if (typeof value === 'bigint') return { t: 'i', v: String(value) }; if (typeof value === 'number' && !Number.isFinite(value)) return { t: 'n', v: String(value) }; if (value === null || typeof value !== 'object') return { t: 'p', v: value }; if (seen.has(value)) return { t: 'r', v: seen.get(value) }; var id = nodes.length; seen.set(value, id); nodes.push(null); var node; if (Buffer.isBuffer(value)) node = { k: 'b', v: value.toString('base64') }; else if (value instanceof Date) node = { k: 'd', v: value.toISOString() }; else if (value instanceof RegExp) node = { k: 'x', v: value.source, f: value.flags, l: value.lastIndex }; else if (value instanceof Map) node = { k: 'm', v: Array.from(value, function(entry) { return [visit(entry[0]), visit(entry[1])]; }) }; else if (value instanceof Set) node = { k: 's', v: Array.from(value, visit) }; else if (Array.isArray(value)) node = { k: 'a', v: value.map(visit) }; else node = { k: 'o', v: Object.keys(value).map(function(key) { return [key, visit(value[key])]; }) }; nodes[id] = node; return { t: 'r', v: id }; } return JSON.stringify({ root: visit(root), nodes: nodes }); }\n\
             function decode(text) { var graph = JSON.parse(text); var values = new Array(graph.nodes.length); graph.nodes.forEach(function(node, index) { if (node.k === 'b') values[index] = Buffer.from(node.v, 'base64'); else if (node.k === 'd') values[index] = new Date(node.v); else if (node.k === 'x') values[index] = new RegExp(node.v, node.f); else if (node.k === 'm') values[index] = new Map(); else if (node.k === 's') values[index] = new Set(); else if (node.k === 'a') values[index] = []; else values[index] = {}; }); function read(value) { if (value.t === 'u') return undefined; if (value.t === 'i') return BigInt(value.v); if (value.t === 'n') return Number(value.v); if (value.t === 'p') return value.v; return values[value.v]; } graph.nodes.forEach(function(node, index) { var target = values[index]; if (node.k === 'm') node.v.forEach(function(entry) { target.set(read(entry[0]), read(entry[1])); }); else if (node.k === 's') node.v.forEach(function(entry) { target.add(read(entry)); }); else if (node.k === 'a') node.v.forEach(function(entry) { target.push(read(entry)); }); else if (node.k === 'o') node.v.forEach(function(entry) { target[entry[0]] = read(entry[1]); }); else if (node.k === 'x') target.lastIndex = node.l; }); return read(graph.root); }\n\
             function serialize(value) { return Buffer.from(encode(value), 'utf8'); } function deserialize(value) { return decode(Buffer.from(value).toString('utf8')); }\n\
             function getHeapStatistics() { return { total_heap_size: 0, total_heap_size_executable: 0, total_physical_size: 0, total_available_size: 0, used_heap_size: 0, heap_size_limit: Number.MAX_SAFE_INTEGER, malloced_memory: 0, peak_malloced_memory: 0, does_zap_garbage: 0, number_of_native_contexts: 1, number_of_detached_contexts: 0, total_global_handles_size: 0, used_global_handles_size: 0, external_memory: 0 }; }\n\
             function getHeapSpaceStatistics() { return []; } function getHeapCodeStatistics() { return { code_and_metadata_size: 0, bytecode_and_metadata_size: 0, external_script_source_size: 0, cpu_profiler_metadata_size: 0 }; } function cachedDataVersionTag() { return 0; } function setFlagsFromString() {}\n\
             module.exports = { serialize: serialize, deserialize: deserialize, getHeapStatistics: getHeapStatistics, getHeapSpaceStatistics: getHeapSpaceStatistics, getHeapCodeStatistics: getHeapCodeStatistics, cachedDataVersionTag: cachedDataVersionTag, setFlagsFromString: setFlagsFromString }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "vm" => Some(
            "var contexts = globalThis.__thaw_vm_contexts || (globalThis.__thaw_vm_contexts = new WeakSet()); function createContext(object, options) { var context = object === undefined ? {} : object; if ((typeof context !== 'object' && typeof context !== 'function') || context === null) throw new TypeError('contextObject must be an object'); contexts.add(context); if (!Object.prototype.hasOwnProperty.call(context, 'globalThis')) Object.defineProperty(context, 'globalThis', { value: context, configurable: true }); if (!Object.prototype.hasOwnProperty.call(context, 'global')) Object.defineProperty(context, 'global', { value: context, configurable: true }); return context; } function isContext(value) { return (typeof value === 'object' || typeof value === 'function') && value !== null && contexts.has(value); } function scopeFor(context) { return new Proxy(context, { has: function(target, key) { return key !== 'scope' && key !== 'source'; }, get: function(target, key) { if (key === Symbol.unscopables) return undefined; return key in target ? target[key] : globalThis[key]; }, set: function(target, key, value) { target[key] = value; return true; } }); } function runInContext(code, context, options) { if (!isContext(context)) throw new TypeError('contextifiedObject must be a vm.Context'); return Function('scope', 'source', 'with (scope) { return eval(source); }')(scopeFor(context), String(code)); } function runInNewContext(code, context, options) { return runInContext(code, createContext(context === undefined ? {} : context), options); } function runInThisContext(code, options) { return (0, eval)(String(code)); }\n\
             function Script(code, options) { if (!(this instanceof Script)) return new Script(code, options); this.code = String(code); this.filename = options && options.filename ? String(options.filename) : 'evalmachine.<anonymous>'; this.cachedDataRejected = false; this.sourceMapURL = undefined; Function(this.code); } Script.prototype.runInContext = function(context, options) { return runInContext(this.code, context, options); }; Script.prototype.runInNewContext = function(context, options) { return runInNewContext(this.code, context, options); }; Script.prototype.runInThisContext = function(options) { return runInThisContext(this.code, options); }; Script.prototype.createCachedData = function() { return Buffer.from(this.code, 'utf8'); }; function compileFunction(code, params, options) { params = params || []; var fn = Function.apply(null, params.concat(String(code))); if (options && options.filename) Object.defineProperty(fn, 'filename', { value: String(options.filename) }); return fn; } function measureMemory(options) { return Promise.resolve({ total: { jsMemoryEstimate: 0, jsMemoryRange: [0, 0] }, current: { jsMemoryEstimate: 0, jsMemoryRange: [0, 0] }, other: [] }); } function getDefaultContext() { return globalThis; }\n\
             module.exports = { Script: Script, createScript: function(code, options) { return new Script(code, options); }, createContext: createContext, isContext: isContext, runInContext: runInContext, runInNewContext: runInNewContext, runInThisContext: runInThisContext, compileFunction: compileFunction, measureMemory: measureMemory, constants: { USE_MAIN_CONTEXT_DEFAULT_LOADER: Symbol.for('vm_dynamic_import_main_context_default'), DONT_CONTEXTIFY: Symbol.for('vm_context_no_contextify') }, getDefaultContext: getDefaultContext }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "zlib" => Some(
            "var stream = require('node:stream'); function input(value) { return Buffer.isBuffer(value) ? value : Buffer.from(value); } function run(operation, format, value) { return Buffer.from(__thaw_zlib_hex(operation, format, input(value).toString('hex')), 'hex'); } function sync(operation, format) { return function(value) { return run(operation, format, value); }; } function async(syncFunction) { return function(value, options, callback) { if (typeof options === 'function') callback = options; try { var result = syncFunction(value, options); queueMicrotask(function() { callback(null, result); }); } catch (error) { queueMicrotask(function() { callback(error); }); } }; }\n\
             var gzipSync = sync('compress', 'gzip'), gunzipSync = sync('decompress', 'gzip'), deflateSync = sync('compress', 'deflate'), inflateSync = sync('decompress', 'deflate'), deflateRawSync = sync('compress', 'deflateRaw'), inflateRawSync = sync('decompress', 'deflateRaw'), brotliCompressSync = sync('compress', 'brotli'), brotliDecompressSync = sync('decompress', 'brotli');\n\
             function ZlibTransform(operation, format, options) { if (!(this instanceof ZlibTransform)) return new ZlibTransform(operation, format, options); var chunks = [], self = this; stream.Transform.call(this, { transform: function(chunk, encoding, callback) { self.bytesWritten += chunk.length; chunks.push(Buffer.from(chunk)); callback(); }, flush: function(callback) { try { callback(null, run(operation, format, Buffer.concat(chunks))); } catch (error) { callback(error); } } }); this.bytesWritten = 0; this._operation = operation; this._format = format; } ZlibTransform.prototype = Object.create(stream.Transform.prototype); ZlibTransform.prototype.constructor = ZlibTransform; ZlibTransform.prototype.flush = function(kind, callback) { if (typeof kind === 'function') callback = kind; if (callback) queueMicrotask(callback); }; ZlibTransform.prototype.close = function(callback) { this.destroy(); if (callback) queueMicrotask(callback); }; ZlibTransform.prototype.reset = function() { if (this.writableEnded) throw new Error('Cannot reset a finished zlib stream'); return this; };\n\
             function type(name, operation, format) { var Constructor = function(options) { if (!(this instanceof Constructor)) return new Constructor(options); ZlibTransform.call(this, operation, format, options); }; Constructor.prototype = Object.create(ZlibTransform.prototype); Constructor.prototype.constructor = Constructor; Object.defineProperty(Constructor, 'name', { value: name }); return Constructor; } var Gzip = type('Gzip', 'compress', 'gzip'), Gunzip = type('Gunzip', 'decompress', 'gzip'), Deflate = type('Deflate', 'compress', 'deflate'), Inflate = type('Inflate', 'decompress', 'deflate'), DeflateRaw = type('DeflateRaw', 'compress', 'deflateRaw'), InflateRaw = type('InflateRaw', 'decompress', 'deflateRaw'), BrotliCompress = type('BrotliCompress', 'compress', 'brotli'), BrotliDecompress = type('BrotliDecompress', 'decompress', 'brotli'); function factory(Constructor) { return function(options) { return new Constructor(options); }; }\n\
             module.exports = { gzipSync: gzipSync, gunzipSync: gunzipSync, deflateSync: deflateSync, inflateSync: inflateSync, deflateRawSync: deflateRawSync, inflateRawSync: inflateRawSync, brotliCompressSync: brotliCompressSync, brotliDecompressSync: brotliDecompressSync, gzip: async(gzipSync), gunzip: async(gunzipSync), deflate: async(deflateSync), inflate: async(inflateSync), deflateRaw: async(deflateRawSync), inflateRaw: async(inflateRawSync), brotliCompress: async(brotliCompressSync), brotliDecompress: async(brotliDecompressSync), Gzip: Gzip, Gunzip: Gunzip, Deflate: Deflate, Inflate: Inflate, DeflateRaw: DeflateRaw, InflateRaw: InflateRaw, BrotliCompress: BrotliCompress, BrotliDecompress: BrotliDecompress, createGzip: factory(Gzip), createGunzip: factory(Gunzip), createDeflate: factory(Deflate), createInflate: factory(Inflate), createDeflateRaw: factory(DeflateRaw), createInflateRaw: factory(InflateRaw), createBrotliCompress: factory(BrotliCompress), createBrotliDecompress: factory(BrotliDecompress), constants: { Z_OK: 0, Z_STREAM_END: 1, Z_DEFAULT_COMPRESSION: -1, Z_NO_FLUSH: 0, Z_SYNC_FLUSH: 2, Z_FINISH: 4, BROTLI_OPERATION_PROCESS: 0, BROTLI_OPERATION_FLUSH: 1, BROTLI_OPERATION_FINISH: 2 } }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "worker_threads" => Some(concat!(
            "var workerStream = require('node:stream'), WorkerReadable = workerStream.Readable, WorkerWritable = workerStream.Writable; var environmentData = globalThis.__thaw_worker_environment_data || (globalThis.__thaw_worker_environment_data = new Map()); var SHARE_ENV = Symbol.for('nodejs.worker_threads.SHARE_ENV');\n\
             function receiveMessageOnPort(port) { if (!port || !Array.isArray(port.__thawQueue)) throw new TypeError('port must be a MessagePort or BroadcastChannel'); var record = port.__thawQueue.shift(); return record ? { message: record.data } : undefined; }\n\
             function setEnvironmentData(key, value) { environmentData.set(key, structuredClone(value)); } function getEnvironmentData(key) { var value = environmentData.get(key); return value === undefined ? undefined : structuredClone(value); }\n\
             function moveMessagePortToContext(port, context) { if (!(port instanceof MessagePort)) throw new TypeError('port must be a MessagePort'); var contexts = globalThis.__thaw_vm_contexts; if (!contexts || !contexts.has(context)) throw new TypeError('contextifiedSandbox must be a vm.Context'); return port.__thawTransfer(); } var untransferable = globalThis.__thaw_untransferable_objects || (globalThis.__thaw_untransferable_objects = new WeakSet()), uncloneable = globalThis.__thaw_uncloneable_objects || (globalThis.__thaw_uncloneable_objects = new WeakSet()); function markAsUntransferable(value) { untransferable.add(value); } function markAsUncloneable(value) { uncloneable.add(value); } function isMarkedAsUntransferable(value) { return ((typeof value === 'object' && value !== null) || typeof value === 'function') && untransferable.has(value); } function cloneError(message) { return new DOMException(message, 'DataCloneError'); } function assertTransferList(transfer) { for (var value of transfer || []) if (isMarkedAsUntransferable(value)) throw cloneError('Object is marked as untransferable'); } function assertCloneable(value, seen) { if ((typeof value !== 'object' && typeof value !== 'function') || value === null) return; if (uncloneable.has(value)) throw cloneError('Object is marked as uncloneable'); if (seen.has(value)) return; seen.add(value); if (value instanceof Map) value.forEach(function(entry, key) { assertCloneable(key, seen); assertCloneable(entry, seen); }); else if (value instanceof Set) value.forEach(function(entry) { assertCloneable(entry, seen); }); else for (var key of Object.keys(value)) assertCloneable(value[key], seen); } if (!globalThis.__thaw_worker_clone_guards) { globalThis.__thaw_worker_clone_guards = true; var cloneWithoutWorkerGuards = globalThis.structuredClone; globalThis.structuredClone = function(value, options) { assertTransferList(options && options.transfer); assertCloneable(value, new WeakSet()); return cloneWithoutWorkerGuards(value, options); }; var postWithoutWorkerGuards = MessagePort.prototype.postMessage; MessagePort.prototype.postMessage = function(value, transfer) { assertTransferList(transfer); assertCloneable(value, new WeakSet()); return postWithoutWorkerGuards.call(this, value, transfer); }; }\n\
             var threadReceivers = globalThis.__thaw_worker_thread_receivers || (globalThis.__thaw_worker_thread_receivers = new Map()); function threadMessagingError(code, message, cause) { var error = new Error(message); error.code = code; if (cause !== undefined) error.cause = cause; return error; } function dispatchThreadMessage(source, target, value, transferList, timeout) { source = Number(source); target = Number(target); if (source === target) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot send a message to the same thread')); var receiver = threadReceivers.get(target); if (!receiver) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); var cloned; try { cloned = structuredClone(value, { transfer: transferList || [] }); } catch (error) { return Promise.reject(error); } return new Promise(function(resolve, reject) { var settled = false, timer; if (timeout !== undefined && Number(timeout) >= 0) timer = setTimeout(function() { if (!settled) { settled = true; reject(threadMessagingError('ERR_WORKER_MESSAGING_TIMEOUT', 'The destination thread did not process the message')); } }, Number(timeout)); queueMicrotask(function() { if (settled) return; try { if (!receiver(cloned, source)) throw threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread has no workerMessage listener'); settled = true; if (timer) clearTimeout(timer); resolve(); } catch (error) { settled = true; if (timer) clearTimeout(timer); reject(error.code ? error : threadMessagingError('ERR_WORKER_MESSAGING_ERRORED', 'The destination thread listener threw', error)); } }); }); } function postMessageToThread(target, value, transferList, timeout) { return dispatchThreadMessage(0, target, value, transferList, timeout); } threadReceivers.set(0, function(value, source) { var process = globalThis.process; if (!process || !process.listenerCount || process.listenerCount('workerMessage') === 0) return false; process.emit('workerMessage', value, source); return true; });\n\
             module.exports = { isMainThread: true, threadId: 0, threadName: '', workerData: null, parentPort: null, resourceLimits: {}, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, receiveMessageOnPort: receiveMessageOnPort, setEnvironmentData: setEnvironmentData, getEnvironmentData: getEnvironmentData, postMessageToThread: postMessageToThread, moveMessagePortToContext: moveMessagePortToContext, markAsUntransferable: markAsUntransferable, markAsUncloneable: markAsUncloneable, isMarkedAsUntransferable: isMarkedAsUntransferable, SHARE_ENV: SHARE_ENV }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             var nextWorkerId = globalThis.__thaw_next_worker_id || 1; function runWorkerSource(source, context) { context.globalThis = context; context.global = context; var intrinsicNames = new Set(['Error','TypeError','RangeError','ReferenceError','SyntaxError','URIError','EvalError','AggregateError','DOMException','Object','Function','Array','Number','String','Boolean','BigInt','Symbol','Date','RegExp','Map','Set','WeakMap','WeakSet','Promise','Proxy','Reflect','JSON','Math','Intl','ArrayBuffer','SharedArrayBuffer','DataView','Uint8Array','Uint16Array','Uint32Array','Int8Array','Int16Array','Int32Array','Float32Array','Float64Array','BigInt64Array','BigUint64Array','Atomics','URL','URLSearchParams','TextEncoder','TextDecoder']); var scope = new Proxy(context, { has: function(target, key) { return key !== 'scope' && key !== 'source'; }, get: function(target, key) { if (key === Symbol.unscopables) return undefined; return key in target ? target[key] : intrinsicNames.has(key) ? globalThis[key] : undefined; }, set: function(target, key, value) { target[key] = value; return true; } }); return Function('scope', 'source', 'with (scope) { return eval(source); }')(scope, String(source)); }
             function createWorkerProcess(parent) { var process = Object.create(parent), listeners = new Map(); function entries(name) { name = String(name); var list = listeners.get(name); if (!list) { list = []; listeners.set(name, list); } return list; } process.on = process.addListener = function(name, listener) { entries(name).push({ listener: listener, once: false }); return process; }; process.once = function(name, listener) { entries(name).push({ listener: listener, once: true }); return process; }; process.off = process.removeListener = function(name, listener) { listeners.set(String(name), entries(name).filter(function(entry) { return entry.listener !== listener; })); return process; }; process.emit = function(name) { var key = String(name), values = entries(key).slice(), args = Array.prototype.slice.call(arguments, 1); values.forEach(function(entry) { if (entry.once) process.off(key, entry.listener); entry.listener.apply(process, args); }); return values.length > 0; }; process.listenerCount = function(name) { return entries(name).length; }; return process; }
             var nativeWorkers = globalThis.__thaw_native_workers || (globalThis.__thaw_native_workers = new Map()), nativeWorkersByThread = globalThis.__thaw_native_workers_by_thread || (globalThis.__thaw_native_workers_by_thread = new Map()), nativeDirectPending = new Map(), nativeDirectRequest = 1, nativePorts = new Map(), nativePortSequence = 1, nativeDecodeWorker = null; function configureNativePortBridge(port, entry) { port.__thawSchedule = function() { while (port.__thawQueue.length) { var record = port.__thawQueue.shift(); if (entry.worker) __thaw_worker_port(entry.worker._nativeHandle, entry.id, __thaw_worker_encode(record.data)); } }; } function prepareNativePorts(transfer) { var entries = []; for (var port of transfer || []) { if (!(port instanceof MessagePort) || port.__thawHostPortId) continue; var id = 'p:' + (nativePortSequence++), moved = port.__thawTransfer(), entry = { id: id, receiver: moved.__thawPeer, worker: null }; port.__thawHostPortId = id; moved.__thawHostPortId = id; configureNativePortBridge(moved, entry); nativePorts.set(id, entry); entries.push(entry); } return entries; } function bindNativePorts(entries, worker) { entries.forEach(function(entry) { entry.worker = worker; }); } function decodeNativeWorkerValue(worker, payload) { var previous = nativeDecodeWorker; nativeDecodeWorker = worker; try { return __thaw_worker_decode(payload); } finally { nativeDecodeWorker = previous; } } globalThis.__thaw_create_worker_port = function(id) { id = String(id); var existing = nativePorts.get(id); if (existing) return existing.receiver; if (!nativeDecodeWorker) throw new DOMException('MessagePort has no Worker owner', 'DataCloneError'); var port = new MessagePort(), entry = { id: id, receiver: port, worker: nativeDecodeWorker }; port.__thawHostPortId = id; port.postMessage = function(value, transfer) { var entries = prepareNativePorts(transfer), payload = __thaw_worker_encode(value); detachNativeTransfers(transfer); bindNativePorts(entries, entry.worker); __thaw_worker_port(entry.worker._nativeHandle, id, payload); }; nativePorts.set(id, entry); return port; }; function enableNativeSharedEnvironment(parentProcess) { if (parentProcess.__thawSharedEnvEnabled) return; __thaw_shared_env_init(JSON.stringify(parentProcess.env || {})); var proxy = new Proxy({}, { get: function(target, key) { return typeof key === 'symbol' ? target[key] : __thaw_shared_env_get(String(key)); }, set: function(target, key, value) { if (typeof key === 'symbol') target[key] = value; else __thaw_shared_env_set(String(key), String(value)); return true; }, deleteProperty: function(target, key) { return typeof key === 'symbol' ? delete target[key] : __thaw_shared_env_delete(String(key)); }, ownKeys: function() { return JSON.parse(__thaw_shared_env_keys()); }, getOwnPropertyDescriptor: function(target, key) { if (typeof key === 'symbol') return Object.getOwnPropertyDescriptor(target, key); var value = __thaw_shared_env_get(String(key)); return value === undefined ? undefined : { value: value, writable: true, enumerable: true, configurable: true }; } }); parentProcess.env = proxy; Object.defineProperty(parentProcess, '__thawSharedEnvEnabled', { value: true, configurable: true }); } function nativeTransferListSupported(transfer) { return Array.from(transfer || []).every(function(value) { return value instanceof ArrayBuffer || value instanceof MessagePort; }); } function canUseNativeWorker(source, options) { return typeof __thaw_worker_spawn === 'function' && nativeTransferListSupported(options.transferList); } function detachNativeTransfers(transfer) { assertTransferList(transfer); var seen = new Set(); for (var value of transfer || []) { if (seen.has(value)) throw new DOMException('transfer list contains duplicate values', 'DataCloneError'); seen.add(value); if (value instanceof ArrayBuffer) __thaw_detach_array_buffer(value); } } function startNativeWorker(worker, source, options) { if (!canUseNativeWorker(source, options)) return false; worker._terminated = false; worker._exited = false; var parentProcess = globalThis.process || {}; if (options.env === SHARE_ENV) enableNativeSharedEnvironment(parentProcess); var config = { threadName: options.name === undefined ? '' : String(options.name), resourceLimits: Object.assign({}, options.resourceLimits || {}), env: Object.assign({}, (options.env === undefined || options.env === SHARE_ENV) ? (parentProcess.env || {}) : options.env), shareEnv: options.env === SHARE_ENV, argv: Array.from(parentProcess.argv || []).concat(Array.from(options.argv || [], String)), execArgv: options.execArgv === undefined ? Array.from(parentProcess.execArgv || []) : Array.from(options.execArgv || [], String), stdin: !!options.stdin, stdout: !!options.stdout, stderr: !!options.stderr }, portEntries = prepareNativePorts(options.transferList), workerDataPayload = __thaw_worker_encode(options.workerData === undefined ? null : options.workerData); detachNativeTransfers(options.transferList); worker._nativeHandle = __thaw_worker_spawn(String(globalThis.__thaw_worker_bundle_source || ''), String(source), workerDataPayload, JSON.stringify(config), worker.threadId); bindNativePorts(portEntries, worker); worker.stdin = options.stdin ? new WorkerWritable({ write: function(chunk, encoding, callback) { __thaw_worker_stdin(worker._nativeHandle, String(chunk), false); callback(); }, final: function(callback) { __thaw_worker_stdin(worker._nativeHandle, '', true); callback(); } }) : null; worker.stdout = options.stdout ? new WorkerReadable() : null; worker.stderr = options.stderr ? new WorkerReadable() : null; nativeWorkers.set(worker._nativeHandle, worker); nativeWorkersByThread.set(worker.threadId, worker); return true; } function directErrorText(code, message) { return String(code) + ':' + String(message); } function settleNativeDirect(request, errorText) { var pending = nativeDirectPending.get(Number(request)); if (!pending) return; nativeDirectPending.delete(Number(request)); if (pending.timer) clearTimeout(pending.timer); if (!errorText) pending.resolve(); else { var separator = errorText.indexOf(':'), error = new Error(separator < 0 ? errorText : errorText.slice(separator + 1)); error.code = separator < 0 ? errorText : errorText.slice(0, separator); pending.reject(error); } } function routeNativeDirect(event) { if (event.target === 0) { var errorText = ''; try { if (!process.listenerCount || process.listenerCount('workerMessage') === 0) errorText = directErrorText('ERR_WORKER_MESSAGING_FAILED', 'The destination thread has no workerMessage listener'); else process.emit('workerMessage', __thaw_worker_decode(event.payload), event.source); } catch (error) { errorText = directErrorText('ERR_WORKER_MESSAGING_ERRORED', error.message); } __thaw_worker_direct_result(event.handle, event.request, errorText); return; } var target = nativeWorkersByThread.get(event.target); if (!target || !__thaw_worker_route_direct(event.handle, target._nativeHandle, event.payload, event.source, event.request)) __thaw_worker_direct_result(event.handle, event.request, directErrorText('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); } function nativePostMessageToThread(target, value, transferList, timeout) { target = Number(target); if (target === 0) return Promise.reject(threadMessagingError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot send a message to the same thread')); var worker = nativeWorkersByThread.get(target); if (!worker) return dispatchThreadMessage(0, target, value, transferList, timeout); var cloned; try { cloned = structuredClone(value, { transfer: transferList || [] }); } catch (error) { return Promise.reject(error); } var request = nativeDirectRequest++, payload = __thaw_worker_encode(cloned); return new Promise(function(resolve, reject) { var timer; if (timeout !== undefined && Number(timeout) >= 0) timer = setTimeout(function() { if (nativeDirectPending.delete(request)) reject(threadMessagingError('ERR_WORKER_MESSAGING_TIMEOUT', 'The destination thread did not process the message')); }, Number(timeout)); nativeDirectPending.set(request, { resolve: resolve, reject: reject, timer: timer }); if (!__thaw_worker_parent_direct(worker._nativeHandle, payload, request)) { nativeDirectPending.delete(request); if (timer) clearTimeout(timer); reject(threadMessagingError('ERR_WORKER_MESSAGING_FAILED', 'The destination thread is not available')); } }); } postMessageToThread = nativePostMessageToThread; if (!globalThis.__thaw_native_worker_poll_installed) { globalThis.__thaw_native_worker_poll_installed = true; var previousPlatformPoll = globalThis.__thaw_poll_platform_events; globalThis.__thaw_poll_platform_events = function() { if (previousPlatformPoll) previousPlatformPoll(); var events = JSON.parse(__thaw_worker_poll()); events.forEach(function(event) { var worker = nativeWorkers.get(event.handle); if (!worker) return; if (event.type === 'online') worker.emit('online'); else if (event.type === 'message') worker.emit('message', decodeNativeWorkerValue(worker, event.payload)); else if (event.type === 'stdout' && worker.stdout) worker.stdout.push(event.payload); else if (event.type === 'stderr' && worker.stderr) worker.stderr.push(event.payload); else if (event.type === 'port') { var portEntry = nativePorts.get(String(event.port)); if (portEntry) { portEntry.receiver.__thawQueue.push({ data: decodeNativeWorkerValue(worker, event.payload), ports: [] }); portEntry.receiver.__thawSchedule(); } } else if (event.type === 'direct') routeNativeDirect(event); else if (event.type === 'directResult') settleNativeDirect(event.request, event.error || ''); else if (event.type === 'error') worker.emit('error', new Error(event.error)); else if (event.type === 'exit') { nativeWorkers.delete(event.handle); nativeWorkersByThread.delete(worker.threadId); worker._finish(event.code); } }); }; }
             function Worker(filename, options) { if (!(this instanceof Worker)) return new Worker(filename, options); options = options || {}; if (!options.eval) throw new Error('Thaw Worker currently requires { eval: true }'); this._events = Object.create(null); this.threadId = nextWorkerId++; this.threadName = options.name === undefined ? '' : String(options.name); globalThis.__thaw_next_worker_id = nextWorkerId; this.resourceLimits = Object.assign({}, options.resourceLimits || {}); this.performance = { eventLoopUtilization: function() { return { idle: 0, active: 0, utilization: 0 }; } }; var childStdin = new WorkerReadable(), worker = this; this.stdin = options.stdin ? new WorkerWritable({ write: function(chunk, encoding, callback) { childStdin.push(chunk); callback(); }, final: function(callback) { childStdin.push(null); callback(); } }) : null; this.stdout = options.stdout ? new WorkerReadable() : null; this.stderr = options.stderr ? new WorkerReadable() : null; this._terminated = false; this._exited = false; var channel = new MessageChannel(); this._port = channel.port1; this._workerPort = channel.port2; this._port.on('message', function(value) { worker.emit('message', value); }); this._port.on('messageerror', function(error) { worker.emit('messageerror', error); }); this._workerPort.on('close', function() { worker._finish(0); }); var data = options.workerData === undefined ? undefined : structuredClone(options.workerData, { transfer: options.transferList || [] }); queueMicrotask(function() { if (worker._terminated) return; worker.emit('online'); var childModule = { isMainThread: false, threadId: worker.threadId, threadName: worker.threadName, workerData: data, parentPort: worker._workerPort, resourceLimits: worker.resourceLimits, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, receiveMessageOnPort: receiveMessageOnPort, setEnvironmentData: setEnvironmentData, getEnvironmentData: getEnvironmentData, SHARE_ENV: SHARE_ENV }; var parentProcess = globalThis.process || {}, workerProcess = createWorkerProcess(parentProcess), parentEnvironment = parentProcess.env || {}; workerProcess.env = options.env === SHARE_ENV ? parentEnvironment : Object.assign({}, options.env === undefined ? parentEnvironment : options.env); workerProcess.argv = Array.from(parentProcess.argv || []).concat(Array.from(options.argv || [], String)); workerProcess.execArgv = options.execArgv === undefined ? Array.from(parentProcess.execArgv || []) : Array.from(options.execArgv || [], String); childModule.postMessageToThread = function(target, value, transferList, timeout) { return dispatchThreadMessage(worker.threadId, target, value, transferList, timeout); }; threadReceivers.set(worker.threadId, function(value, source) { if (workerProcess.listenerCount('workerMessage') === 0) return false; workerProcess.emit('workerMessage', value, source); return true; }); workerProcess.stdin = childStdin; workerProcess.stdout = worker.stdout ? new WorkerWritable({ write: function(chunk, encoding, callback) { worker.stdout.push(chunk); callback(); } }) : parentProcess.stdout; workerProcess.stderr = worker.stderr ? new WorkerWritable({ write: function(chunk, encoding, callback) { worker.stderr.push(chunk); callback(); } }) : parentProcess.stderr; var workerConsole = Object.create(console); function writeConsole(stream, args) { if (stream && stream.write) stream.write(Array.prototype.map.call(args, String).join(' ') + '\n'); } if (worker.stdout) workerConsole.log = workerConsole.info = function() { writeConsole(workerProcess.stdout, arguments); }; if (worker.stderr) workerConsole.warn = workerConsole.error = function() { writeConsole(workerProcess.stderr, arguments); }; var scriptModule = { exports: {} }; var context = { eval: eval, console: workerConsole, Buffer: Buffer, structuredClone: structuredClone, MessageChannel: MessageChannel, MessagePort: MessagePort, BroadcastChannel: globalThis.BroadcastChannel, setTimeout: setTimeout, clearTimeout: clearTimeout, setInterval: setInterval, clearInterval: clearInterval, queueMicrotask: queueMicrotask, process: workerProcess, module: scriptModule, exports: scriptModule.exports, __thaw_bundle_create_require: globalThis.__thaw_bundle_create_require, __thaw_worker_module: childModule, require: function(name) { if (name === 'worker_threads' || name === 'node:worker_threads') return childModule; return require(name); } }; try { runWorkerSource(String(filename), context); if (!(worker._workerPort.__thawNodeListeners.get('message') || []).length && workerProcess.listenerCount('workerMessage') === 0) worker._workerPort.close(); } catch (error) { worker.emit('error', error); worker._finish(1); } }); }
             Worker.prototype.on = Worker.prototype.addListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; Worker.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; Worker.prototype.prependListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).unshift({ listener: listener, once: false }); return this; }; Worker.prototype.prependOnceListener = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).unshift({ listener: listener, once: true }); return this; }; Worker.prototype.off = Worker.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Worker.prototype.removeAllListeners = function(name) { if (name === undefined) this._events = Object.create(null); else delete this._events[String(name)]; return this; }; Worker.prototype.listeners = Worker.prototype.rawListeners = function(name) { return (this._events[String(name)] || []).map(function(entry) { return entry.listener; }); }; Worker.prototype.listenerCount = function(name, listener) { return (this._events[String(name)] || []).filter(function(entry) { return listener === undefined || entry.listener === listener; }).length; }; Worker.prototype.eventNames = function() { return Object.keys(this._events).filter(function(name) { return this._events[name].length; }, this); }; Worker.prototype.setMaxListeners = function(count) { this._maxListeners = Number(count); return this; }; Worker.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? 10 : this._maxListeners; }; Worker.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; Worker.prototype.postMessage = function(value, transfer) { if (!this._terminated) this._port.postMessage(value, transfer); }; Worker.prototype._finish = function(code) { if (this._exited) return; this._exited = true; if (this.stdout) this.stdout.push(null); if (this.stderr) this.stderr.push(null); if (this.stdin && !this.stdin.writableEnded) this.stdin.end(); var worker = this; queueMicrotask(function() { worker.emit('exit', Number(code)); }); }; Worker.prototype.terminate = function() { if (!this._terminated) { this._terminated = true; this._workerPort.close(); this._port.close(); this._finish(1); } return Promise.resolve(1); }; Worker.prototype.ref = function() { this._port.ref(); return this; }; Worker.prototype.unref = function() { this._port.unref(); return this; }; Worker.prototype.getHeapSnapshot = function() { return Promise.resolve({}); };
             function exitCompletion(worker) { if (!worker._exitPromise) worker._exitPromise = new Promise(function(resolve) { worker._resolveExit = resolve; }); return worker._exitPromise; } var finishWithoutCompletion = Worker.prototype._finish; Worker.prototype._finish = function(code) { if (this._exited) return exitCompletion(this); var completion = exitCompletion(this), worker = this, exitCode = Number(code); threadReceivers.delete(worker.threadId); finishWithoutCompletion.call(this, exitCode); queueMicrotask(function() { worker._resolveExit(exitCode); }); return completion; }; Worker.prototype.terminate = function() { var completion = exitCompletion(this); if (!this._terminated && !this._exited) { this._terminated = true; this._finish(1); this._workerPort.close(); this._port.close(); } return completion; }; if (Symbol.asyncDispose) Worker.prototype[Symbol.asyncDispose] = Worker.prototype.terminate;
             var inProcessPostMessage = Worker.prototype.postMessage, inProcessTerminate = Worker.prototype.terminate, inProcessRef = Worker.prototype.ref, inProcessUnref = Worker.prototype.unref; Worker.prototype.postMessage = function(value, transfer) { if (!this._nativeHandle) return inProcessPostMessage.call(this, value, transfer); if (!nativeTransferListSupported(transfer)) throw new DOMException('value is not transferable', 'DataCloneError'); if (!this._terminated) { var portEntries = prepareNativePorts(transfer), payload = __thaw_worker_encode(value); detachNativeTransfers(transfer); bindNativePorts(portEntries, this); __thaw_worker_send(this._nativeHandle, payload); } }; Worker.prototype.terminate = function() { if (!this._nativeHandle) return inProcessTerminate.call(this); var completion = exitCompletion(this); if (!this._terminated && !this._exited) { this._terminated = true; __thaw_worker_terminate(this._nativeHandle); } return completion; }; Worker.prototype.ref = function() { return this._nativeHandle ? this : inProcessRef.call(this); }; Worker.prototype.unref = function() { return this._nativeHandle ? this : inProcessUnref.call(this); }; if (Symbol.asyncDispose) Worker.prototype[Symbol.asyncDispose] = Worker.prototype.terminate;
             function workerNotRunning() { return threadMessagingError('ERR_WORKER_NOT_RUNNING', 'Worker instance is not running'); } Worker.prototype.cpuUsage = function(previous) { if (this._exited) return Promise.reject(workerNotRunning()); var current = { user: 0, system: 0 }; if (previous) { current.user = Math.max(0, current.user - Number(previous.user || 0)); current.system = Math.max(0, current.system - Number(previous.system || 0)); } return Promise.resolve(current); }; Worker.prototype.getHeapStatistics = function() { if (this._exited) return Promise.reject(workerNotRunning()); return Promise.resolve({ total_heap_size: 0, total_heap_size_executable: 0, total_physical_size: 0, total_available_size: 0, used_heap_size: 0, heap_size_limit: Number.MAX_SAFE_INTEGER, malloced_memory: 0, peak_malloced_memory: 0, does_zap_garbage: 0, number_of_native_contexts: 1, number_of_detached_contexts: 0, total_global_handles_size: 0, used_global_handles_size: 0, external_memory: 0 }); }; Worker.prototype.getHeapSnapshot = function() { if (this._exited) return Promise.reject(workerNotRunning()); var snapshot = new WorkerReadable(); queueMicrotask(function() { snapshot.push(JSON.stringify({ snapshot: { meta: {}, node_count: 0, edge_count: 0 }, nodes: [], edges: [], strings: [] })); snapshot.push(null); }); return Promise.resolve(snapshot); }; Worker.prototype.startCpuProfile = function() { if (this._exited) return Promise.reject(workerNotRunning()); var stopped = false; return Promise.resolve({ stop: function() { if (stopped) return Promise.reject(new Error('CPU profile has already been stopped')); stopped = true; return Promise.resolve({ nodes: [], startTime: 0, endTime: 0, samples: [], timeDeltas: [] }); } }); };
             var EvalWorker = Worker; function createNativeWorker(source, options) { if (!canUseNativeWorker(source, options)) return null; var worker = Object.create(EvalWorker.prototype); worker._events = Object.create(null); worker.threadId = nextWorkerId++; worker.threadName = options.name === undefined ? '' : String(options.name); globalThis.__thaw_next_worker_id = nextWorkerId; worker.resourceLimits = Object.assign({}, options.resourceLimits || {}); worker.performance = { eventLoopUtilization: function() { return { idle: 0, active: 0, utilization: 0 }; } }; return startNativeWorker(worker, source, options) ? worker : null; } function decodeWorkerDataUrl(value) { var text = String(value); if (!text.startsWith('data:')) return null; var comma = text.indexOf(','); if (comma < 0) throw new TypeError('Invalid Worker data URL'); var metadata = text.slice(5, comma).toLowerCase(), payload = text.slice(comma + 1), parts = metadata.split(';'), mediaType = parts[0] || 'text/plain'; if (mediaType !== 'text/javascript' && mediaType !== 'application/javascript') throw new TypeError('Worker data URL must contain JavaScript'); try { return parts.indexOf('base64') >= 0 ? Buffer.from(payload, 'base64').toString('utf8') : decodeURIComponent(payload); } catch (error) { throw new TypeError('Invalid Worker data URL payload'); } } function readRuntimeWorkerFile(value) { var filename = String(value); if (filename.startsWith('file:')) { try { filename = decodeURIComponent(new URL(filename).pathname); } catch (error) { throw new TypeError('Invalid Worker file URL'); } } var source = __thaw_worker_read_source(filename), slash = filename.lastIndexOf('/'), dirname = slash < 0 ? '.' : filename.slice(0, slash), prefix = '(function(){ var __thawBuiltinRequire = require, __thawRuntimeCache = Object.create(null); function __thawNormalize(path) { var absolute = path.charAt(0) === "/", output = []; path.split("/").forEach(function(part) { if (!part || part === ".") return; if (part === "..") output.pop(); else output.push(part); }); return (absolute ? "/" : "") + output.join("/"); } function __thawLoad(base, request) { if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") return __thawBuiltinRequire(request); var target = __thawNormalize(request.charAt(0) === "/" ? request : base + "/" + request), candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".json", target + "/index.js", target + "/index.json"], loaded, filename; for (var index = 0; index < candidates.length; index++) { try { loaded = __thaw_worker_read_source(candidates[index]); filename = candidates[index]; break; } catch (error) {} } if (filename === undefined) throw new Error("Cannot find module \'" + request + "\'"); if (__thawRuntimeCache[filename]) return __thawRuntimeCache[filename].exports; var module = __thawRuntimeCache[filename] = { exports: {} }, slash = filename.lastIndexOf("/"), dirname = slash < 0 ? "." : filename.slice(0, slash), localRequire = function(name) { return __thawLoad(dirname, String(name)); }; if (/\\.json$/.test(filename)) module.exports = JSON.parse(loaded); else Function("module", "exports", "require", "__filename", "__dirname", loaded)(module, module.exports, localRequire, filename, dirname); return module.exports; } require = function(request) { return __thawLoad(__dirname, String(request)); }; })();\n'; return 'var __filename = ' + JSON.stringify(filename) + '; var __dirname = ' + JSON.stringify(dirname) + ';\n' + prefix + source; } Worker = function Worker(filename, options) { options = options || {}; var source = options.eval ? String(filename) : decodeWorkerDataUrl(filename); if (source === null) source = readRuntimeWorkerFile(filename); var nativeWorker = createNativeWorker(source, options); if (nativeWorker) return nativeWorker; var workerOptions = options.eval ? options : Object.assign({}, options, { eval: true }); return new EvalWorker(source, workerOptions); }; Worker.prototype = EvalWorker.prototype;
             var readRuntimeWorkerFileWithoutDirectoryResolution = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutDirectoryResolution(value), basicCandidates = 'candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".json", target + "/index.js", target + "/index.json"], loaded, filename;', nodeCandidates = 'candidates = /\\.[^/]+$/.test(target) ? [target] : [target, target + ".js", target + ".cjs", target + ".json", target + "/index.js", target + "/index.cjs", target + "/index.json"], loaded, filename; if (!/\\.[^/]+$/.test(target)) { try { var manifest = JSON.parse(__thaw_worker_read_source(target + "/package.json")); if (typeof manifest.main === "string" && manifest.main) return __thawLoad(target, "./" + manifest.main); } catch (error) {} }'; return source.replace(basicCandidates, nodeCandidates); };
             var readRuntimeWorkerFileWithoutBareResolution = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutBareResolution(value), builtinOnly = 'if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") return __thawBuiltinRequire(request);', nodeModulesFallback = 'if (request.slice(0, 2) !== "./" && request.slice(0, 3) !== "../" && request.charAt(0) !== "/") { try { return __thawBuiltinRequire(request); } catch (builtinError) { var parts = request.split("/"), packageParts = request.charAt(0) === "@" ? parts.slice(0, 2) : parts.slice(0, 1), packageName = packageParts.join("/"), subpath = parts.slice(packageParts.length).join("/"), directory = base; function exportTarget(value) { if (typeof value === "string") return value; if (Array.isArray(value)) { for (var item of value) { var arrayTarget = exportTarget(item); if (arrayTarget) return arrayTarget; } return null; } if (value && typeof value === "object") { for (var condition of Object.keys(value)) if (condition === "require" || condition === "node" || condition === "default") { var conditionTarget = exportTarget(value[condition]); if (conditionTarget) return conditionTarget; } } return null; } while (directory) { var packageRoot = directory + "/node_modules/" + packageName, packageText; try { packageText = __thaw_worker_read_source(packageRoot + "/package.json"); } catch (lookupError) {} if (packageText !== undefined) { var manifest = JSON.parse(packageText); if (manifest.exports !== undefined) { var exportKey = subpath ? "./" + subpath : ".", exportValue = manifest.exports; if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) exportValue = exportValue[exportKey]; else if (subpath) exportValue = undefined; var resolvedExport = exportTarget(exportValue); if (!resolvedExport) { var pathError = new Error("Package subpath \'" + exportKey + "\' is not defined by exports in " + packageRoot + "/package.json"); pathError.code = "ERR_PACKAGE_PATH_NOT_EXPORTED"; throw pathError; } return __thawLoad(packageRoot, resolvedExport); } return subpath ? __thawLoad(packageRoot, "./" + subpath) : __thawLoad(directory + "/node_modules", "./" + packageName); } var slash = directory.lastIndexOf("/"); if (slash <= 0) break; directory = directory.slice(0, slash); } throw builtinError; } }'; return source.replace(builtinOnly, nodeModulesFallback); };
             var readRuntimeWorkerFileWithoutWildcardExports = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutWildcardExports(value), exactSelection = 'if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) exportValue = exportValue[exportKey]; else if (subpath) exportValue = undefined;', wildcardSelection = 'var wildcardMatch; if (exportValue && typeof exportValue === "object" && !Array.isArray(exportValue) && Object.keys(exportValue).some(function(key) { return key.charAt(0) === "."; })) { var exportMap = exportValue; exportValue = exportMap[exportKey]; if (exportValue === undefined) { var wildcardKeys = Object.keys(exportMap).filter(function(key) { return key.indexOf("*") >= 0; }).sort(function(left, right) { return right.indexOf("*") - left.indexOf("*") || right.length - left.length; }); for (var wildcardKey of wildcardKeys) { var star = wildcardKey.indexOf("*"), prefix = wildcardKey.slice(0, star), suffix = wildcardKey.slice(star + 1); if (exportKey.slice(0, prefix.length) === prefix && exportKey.slice(exportKey.length - suffix.length) === suffix && exportKey.length >= prefix.length + suffix.length) { wildcardMatch = exportKey.slice(prefix.length, exportKey.length - suffix.length); exportValue = exportMap[wildcardKey]; break; } } } } else if (subpath) exportValue = undefined;', exactLoad = 'return __thawLoad(packageRoot, resolvedExport);', wildcardLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); return __thawLoad(packageRoot, resolvedExport);'; return source.replace(exactSelection, wildcardSelection).replace(exactLoad, wildcardLoad); };
             var readRuntimeWorkerFileWithoutPackageImports = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutPackageImports(value), loadStart = 'function __thawLoad(base, request) {', importsResolution = 'function __thawLoad(base, request) { if (request.charCodeAt(0) === 35) { var scope = base; function importTarget(value) { if (typeof value === "string") return value; if (Array.isArray(value)) { for (var item of value) { var arrayTarget = importTarget(item); if (arrayTarget) return arrayTarget; } return null; } if (value && typeof value === "object") { for (var condition of Object.keys(value)) if (condition === "require" || condition === "node" || condition === "default") { var conditionTarget = importTarget(value[condition]); if (conditionTarget) return conditionTarget; } } return null; } while (scope) { var scopeText; try { scopeText = __thaw_worker_read_source(scope + "/package.json"); } catch (scopeError) {} if (scopeText !== undefined) { var scopeManifest = JSON.parse(scopeText), imports = scopeManifest.imports || {}, importValue = imports[request], importMatch; if (importValue === undefined) { var importKeys = Object.keys(imports).filter(function(key) { return key.indexOf("*") >= 0; }).sort(function(left, right) { return right.indexOf("*") - left.indexOf("*") || right.length - left.length; }); for (var importKey of importKeys) { var importStar = importKey.indexOf("*"), importPrefix = importKey.slice(0, importStar), importSuffix = importKey.slice(importStar + 1); if (request.slice(0, importPrefix.length) === importPrefix && request.slice(request.length - importSuffix.length) === importSuffix && request.length >= importPrefix.length + importSuffix.length) { importMatch = request.slice(importPrefix.length, request.length - importSuffix.length); importValue = imports[importKey]; break; } } } var resolvedImport = importTarget(importValue); if (!resolvedImport) { var importError = new Error("Package import specifier \'" + request + "\' is not defined in " + scope + "/package.json"); importError.code = "ERR_PACKAGE_IMPORT_NOT_DEFINED"; throw importError; } if (importMatch !== undefined) resolvedImport = resolvedImport.split("*").join(importMatch); return __thawLoad(scope, resolvedImport); } var scopeSlash = scope.lastIndexOf("/"); if (scopeSlash <= 0) break; scope = scope.slice(0, scopeSlash); } var missingImport = new Error("Package import specifier \'" + request + "\' is not defined"); missingImport.code = "ERR_PACKAGE_IMPORT_NOT_DEFINED"; throw missingImport; }'; return source.replace(loadStart, importsResolution); };
             var readRuntimeWorkerFileWithoutPackageTargetGuards = readRuntimeWorkerFile; readRuntimeWorkerFile = function(value) { var source = readRuntimeWorkerFileWithoutPackageTargetGuards(value), exportLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); return __thawLoad(packageRoot, resolvedExport);', guardedExportLoad = 'if (wildcardMatch !== undefined) resolvedExport = resolvedExport.split("*").join(wildcardMatch); if (resolvedExport.slice(0, 2) !== "./" || resolvedExport.split("/").some(function(part) { return part === ".." || part === "node_modules"; })) { var exportTargetError = new Error("Invalid package target \'" + resolvedExport + "\' in " + packageRoot + "/package.json"); exportTargetError.code = "ERR_INVALID_PACKAGE_TARGET"; throw exportTargetError; } return __thawLoad(packageRoot, resolvedExport);', importLoad = 'return __thawLoad(scope, resolvedImport);', guardedImportLoad = 'if (resolvedImport.charAt(0) === "/" || (resolvedImport.charAt(0) === "." && resolvedImport.slice(0, 2) !== "./") || resolvedImport.split("/").some(function(part) { return part === ".." || part === "node_modules"; })) { var importTargetError = new Error("Invalid package target \'" + resolvedImport + "\' in " + scope + "/package.json"); importTargetError.code = "ERR_INVALID_PACKAGE_TARGET"; throw importTargetError; } return __thawLoad(scope, resolvedImport);'; return source.replace(exportLoad, guardedExportLoad).replace(importLoad, guardedImportLoad); };
             settleNativeDirect = function(request, errorText) { var pending = nativeDirectPending.get(Number(request)); if (!pending) return; nativeDirectPending.delete(Number(request)); if (pending.timer) clearTimeout(pending.timer); if (!errorText) { pending.resolve(); return; } var separator = errorText.indexOf(':'), code = separator < 0 ? errorText : errorText.slice(0, separator), message = separator < 0 ? errorText : errorText.slice(separator + 1), error = new Error(message); error.code = code; if (code === 'ERR_WORKER_MESSAGING_ERRORED') error.cause = new Error(message); pending.reject(error); }; module.exports.Worker = Worker; module.exports.postMessageToThread = postMessageToThread;
"#,
        )),
        _ => None,
    }
}

fn resolve_bare_require(
    node_modules_dir: &Path,
    spec: &str,
) -> Option<(String, String, PathBuf, PathBuf)> {
    let (dep_name, subpath) = split_bare_spec(spec);
    let dep_dir = node_modules_dir.join(dep_name);
    let target = match subpath {
        Some(sub) => read_manifest(&dep_dir)
            .ok()
            .and_then(|manifest| package_subpath_runtime_target(&manifest, sub))
            .unwrap_or_else(|| sub.to_string()),
        None => {
            let manifest = read_manifest(&dep_dir).ok()?;
            package_export_target(&manifest, None, &["require", "import", "default"])
                .or_else(|| manifest.get("main").and_then(|v| v.as_str()))
                .unwrap_or("index.js")
                .to_string()
        }
    };
    let (dep_relative, dep_abs) = resolve_module_path(&dep_dir, &target).ok()?;
    Some((dep_name.to_string(), dep_relative, dep_abs, dep_dir))
}

fn package_subpath_runtime_target(manifest: &serde_json::Value, subpath: &str) -> Option<String> {
    if let Some(target) = package_export_target(
        manifest,
        Some(subpath),
        &["require", "import", "node", "default"],
    ) {
        return Some(target.to_string());
    }
    let exports = manifest.get("exports")?.as_object()?;
    for (key, value) in exports {
        let Some(pattern) = key.strip_prefix("./") else {
            continue;
        };
        let Some(capture) = wildcard_capture(pattern, subpath) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "import", "node", "default"])
        {
            return Some(target.replace('*', capture));
        }
    }
    None
}

fn resolve_package_import(package_dir: &Path, spec: &str) -> Option<(String, PathBuf)> {
    let manifest = read_manifest(package_dir).ok()?;
    let imports = manifest.get("imports")?.as_object()?;
    if let Some(value) = imports.get(spec) {
        let target = select_export_condition(value, &["require", "import", "node", "default"])?;
        return resolve_module_path(package_dir, target).ok();
    }
    for (pattern, value) in imports {
        let Some(capture) = wildcard_capture(pattern, spec) else {
            continue;
        };
        if let Some(target) =
            select_export_condition(value, &["require", "import", "node", "default"])
        {
            return resolve_module_path(package_dir, &target.replace('*', capture)).ok();
        }
    }
    None
}

/// Collapses `.`/`..` segments in a `/`-separated path string (npm
/// `require` specs always use `/`, regardless of host OS). No crate
/// dependency needed for this -- e.g. `"lib/./../lib/parse"` ->
/// `"lib/parse"`.
fn normalize_path_string(path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

fn relative_module_specifier(from_dir: &Path, target: &Path) -> String {
    let from: Vec<_> = from_dir
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let to: Vec<_> = target
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec!["..".to_string(); from.len() - common];
    parts.extend(to[common..].iter().cloned());
    let path = parts.join("/");
    if path.starts_with("../") {
        path
    } else {
        format!("./{path}")
    }
}

/// Renders `modules` into one JS string: a small embedded CommonJS
/// module-system emulation (a factory + a per-module require-spec-to-key
/// map, both precomputed statically -- no runtime path resolution needed
/// in the emitted JS at all), then a final `module.exports = ...` that
/// invokes `main_key`. The result is exactly the kind of single JS
/// string thaw-bridge's `wrap_as_commonjs_module` already knows how to
/// run: it doesn't need to know or care that this is a bundle rather
/// than one file.
///
/// Everything except the final `module.exports = ` assignment is inside
/// an IIFE, deliberately never touching global scope: multiple
/// `--use`'d packages all get `loadScript`'d into the *same* shared
/// QuickJS-NG global context (thaw-bridge's `wrap_as_commonjs_module`,
/// called once per package), so if these helpers were plain globals, a
/// second package's bundle would stomp the first's `__thaw_bundle_cache`/
/// `__thaw_bundle_require`/etc. the moment it loaded. That's invisible
/// for a module that only calls `require` eagerly at load time (already
/// finished and cached by then), but a *lazy* internal require --
/// deferred inside a function body, called only after a later package
/// has overwritten the globals -- would silently resolve against the
/// wrong package's module map. The IIFE's closures keep each package's
/// module system private to itself regardless of what loads after it.
fn prepare_async_modules(modules: &mut [BundledModule]) -> Result<(), String> {
    use std::collections::BTreeSet;

    for module in modules.iter_mut() {
        module.async_module = module.has_top_level_await;
    }
    loop {
        let async_keys: BTreeSet<_> = modules
            .iter()
            .filter(|module| module.async_module)
            .map(|module| module.key.clone())
            .collect();
        let mut changed = false;
        for module in modules.iter_mut().filter(|module| module.has_esm) {
            if module.async_module {
                continue;
            }
            if module.static_esm_specs.iter().any(|specifier| {
                module
                    .requires
                    .iter()
                    .any(|(source, target)| source == specifier && async_keys.contains(target))
            }) {
                module.async_module = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    fn visit(
        key: &str,
        modules: &[BundledModule],
        visiting: &mut Vec<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if let Some(index) = visiting.iter().position(|item| item == key) {
            let mut cycle = visiting[index..].to_vec();
            cycle.push(key.to_string());
            return Err(format!(
                "top-level await module cycle is not supported: {}",
                cycle.join(" -> ")
            ));
        }
        if !visited.insert(key.to_string()) {
            return Ok(());
        }
        let Some(module) = modules.iter().find(|module| module.key == key) else {
            return Ok(());
        };
        visiting.push(key.to_string());
        for (specifier, target) in &module.requires {
            if module.static_esm_specs.contains(specifier)
                && modules
                    .iter()
                    .any(|candidate| candidate.key == *target && candidate.async_module)
            {
                visit(target, modules, visiting, visited)?;
            }
        }
        visiting.pop();
        Ok(())
    }

    let mut visited = BTreeSet::new();
    for module in modules.iter().filter(|module| module.async_module) {
        visit(&module.key, modules, &mut Vec::new(), &mut visited)?;
    }

    for module in modules.iter_mut() {
        let source = rewrite_esm_to_commonjs_mode(&module.source, module.async_module)
            .unwrap_or_else(|| module.source.clone());
        module.source = source;
    }
    Ok(())
}

fn render_bundle(main_key: &str, modules: &[BundledModule]) -> String {
    let mut out = String::from("module.exports = (function() {\n");

    out.push_str("var __thaw_bundle_cache = {};\n");
    out.push_str("var __thaw_bundle_factories = {\n");
    for module in modules {
        let asynchronous = if module.async_module { "async " } else { "" };
        out.push_str(&format!(
            "{}: {asynchronous}function(module, exports, require, requireAsync, __filename, __dirname) {{\n{}\n}},\n",
            js_string_literal(&module.key),
            module.source
        ));
    }
    out.push_str("};\n");

    out.push_str("var __thaw_bundle_require_maps = {\n");
    for module in modules {
        out.push_str(&format!("{}: {{", js_string_literal(&module.key)));
        for (spec, target) in &module.requires {
            out.push_str(&format!(
                "{}: {}, ",
                js_string_literal(spec),
                js_string_literal(target)
            ));
        }
        out.push_str("},\n");
    }
    out.push_str("};\n");

    out.push_str(
        "function __thaw_bundle_target(map, spec) {\n\
         \x20\x20if (Object.prototype.hasOwnProperty.call(map, spec)) return { key: map[spec], factory: map[spec] };\n\
         \x20\x20var query = spec.indexOf('?');\n\
         \x20\x20var fragment = spec.indexOf('#', 1);\n\
         \x20\x20var suffixAt = query < 0 ? fragment : (fragment < 0 ? query : Math.min(query, fragment));\n\
         \x20\x20if (suffixAt < 0) return null;\n\
         \x20\x20var base = spec.slice(0, suffixAt);\n\
         \x20\x20if (!Object.prototype.hasOwnProperty.call(map, base)) return null;\n\
         \x20\x20var factory = map[base];\n\
         \x20\x20return { key: factory + spec.slice(suffixAt), factory: factory };\n\
         }\n\
         function __thaw_bundle_require(key, factoryKey) {\n\
         \x20\x20if (!(key in __thaw_bundle_cache)) {\n\
         \x20\x20\x20\x20factoryKey = factoryKey || key;\n\
         \x20\x20\x20\x20var mod = { exports: {} };\n\
         \x20\x20\x20\x20__thaw_bundle_cache[key] = mod;\n\
         \x20\x20\x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20\x20\x20var localRequire = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(map, spec);\n\
         \x20\x20\x20\x20\x20\x20if (target) return __thaw_bundle_require(target.key, target.factory);\n\
         \x20\x20\x20\x20\x20\x20return require(spec);\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20var localRequireAsync = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(map, spec);\n\
         \x20\x20\x20\x20\x20\x20if (!target) return Promise.resolve().then(function() { return require(spec); });\n\
         \x20\x20\x20\x20\x20\x20var value = __thaw_bundle_require(target.key, target.factory);\n\
         \x20\x20\x20\x20\x20\x20return __thaw_bundle_cache[target.key].ready.then(function() { return value; });\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20var filename = '/thaw_modules/' + factoryKey, slash = filename.lastIndexOf('/'), dirname = slash < 0 ? '.' : filename.slice(0, slash);\n\
         \x20\x20\x20\x20var initialized = __thaw_bundle_factories[factoryKey](mod, mod.exports, localRequire, localRequireAsync, filename, dirname);\n\
         \x20\x20\x20\x20mod.ready = Promise.resolve(initialized).then(function() { return mod.exports; });\n\
         \x20\x20}\n\
         \x20\x20return __thaw_bundle_cache[key].exports;\n\
         }\n\
         function __thaw_bundle_create_require(base) {\n\
         \x20\x20var text = String(base || '');\n\
         \x20\x20var keys = Object.keys(__thaw_bundle_require_maps);\n\
         \x20\x20var factoryKey = keys.indexOf(text) >= 0 ? text : keys.find(function(key) { return text.endsWith('/' + key) || text.endsWith(key); });\n\
         \x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20var created = function(spec) { var target = __thaw_bundle_target(map, String(spec)); if (target) return __thaw_bundle_require(target.key, target.factory); return require(String(spec)); };\n\
         \x20\x20created.resolve = function(spec) { var target = __thaw_bundle_target(map, String(spec)); return target ? target.key : String(spec); };\n\
         \x20\x20created.cache = __thaw_bundle_cache; return created;\n\
         }\n\
         globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\n\
         globalThis.__thaw_worker_bundle_source =\n\
         \x20\x20'var __thaw_bundle_cache = {};\\nvar __thaw_bundle_factories = {' +\n\
         \x20\x20Object.keys(__thaw_bundle_factories).map(function(key) { return JSON.stringify(key) + ': ' + __thaw_bundle_factories[key].toString(); }).join(',\\n') +\n\
         \x20\x20'};\\nvar __thaw_bundle_require_maps = ' + JSON.stringify(__thaw_bundle_require_maps) + ';\\n' +\n\
         \x20\x20__thaw_bundle_target.toString() + '\\n' + __thaw_bundle_require.toString() + '\\n' + __thaw_bundle_create_require.toString() + '\\n' +\n\
         \x20\x20'globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\\n';\n",
    );

    out.push_str(&format!(
        "var __thaw_bundle_entry_key = {};\n\
         var __thaw_bundle_entry = __thaw_bundle_require(__thaw_bundle_entry_key);\n\
         globalThis.__thaw_module_ready = __thaw_bundle_cache[__thaw_bundle_entry_key].ready;\n\
         return __thaw_bundle_entry;\n",
        js_string_literal(main_key)
    ));
    out.push_str("})();\n");

    out
}

/// A double-quoted JS string literal for `s` -- used for module-map keys
/// and require specs, which in practice are always simple path-like
/// strings, but escaped properly regardless.
fn js_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_package_exports_conditions_for_runtime_and_types() {
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{
                "main": "legacy.js",
                "types": "legacy.d.ts",
                "exports": {
                    ".": {
                        "types": "./dist/index.d.ts",
                        "import": "./dist/index.mjs",
                        "require": "./dist/index.cjs",
                        "default": "./dist/index.js"
                    },
                    "./feature": {
                        "types": "./dist/feature.d.ts",
                        "require": "./dist/feature.cjs"
                    }
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            package_export_target(&manifest, None, &["require", "import", "default"]),
            Some("./dist/index.cjs")
        );
        assert_eq!(
            package_export_target(&manifest, None, &["types"]),
            Some("./dist/index.d.ts")
        );
        assert_eq!(
            package_export_target(&manifest, Some("feature"), &["types"]),
            Some("./dist/feature.d.ts")
        );

        let array: serde_json::Value = serde_json::from_str(
            r#"{"exports":{".":[null,{"types":"./fallback.d.ts","require":"./fallback.cjs"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            package_export_target(&array, None, &["types"]),
            Some("./fallback.d.ts")
        );
        assert_eq!(
            package_export_target(&array, None, &["require", "default"]),
            Some("./fallback.cjs")
        );
    }

    #[test]
    fn expands_wildcard_package_exports_from_type_files() {
        let package_dir = temp_registry("wildcard-exports");
        fs::create_dir_all(package_dir.join("dist/features")).unwrap();
        fs::write(package_dir.join("dist/features/alpha.d.ts"), "").unwrap();
        fs::write(package_dir.join("dist/features/alpha.cjs"), "").unwrap();
        fs::write(package_dir.join("dist/features/beta.d.ts"), "").unwrap();
        fs::write(package_dir.join("dist/features/beta.cjs"), "").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"exports":{"./features/*":{"types":"./dist/features/*.d.ts","require":"./dist/features/*.cjs"}}}"#,
        )
        .unwrap();
        assert_eq!(
            package_subpath_exports(&manifest, &package_dir).unwrap(),
            vec![
                PackageSubpathExport {
                    subpath: "features/alpha".to_string(),
                    runtime_entry: "./dist/features/alpha.cjs".to_string(),
                    types_entry: "./dist/features/alpha.d.ts".to_string(),
                },
                PackageSubpathExport {
                    subpath: "features/beta".to_string(),
                    runtime_entry: "./dist/features/beta.cjs".to_string(),
                    types_entry: "./dist/features/beta.d.ts".to_string(),
                },
            ]
        );
        let _ = fs::remove_dir_all(package_dir);
    }

    #[test]
    fn installed_npm_layout_registers_wildcard_subpath_artifacts() {
        let scratch = temp_registry("installed-wildcard-scratch");
        let registry = temp_registry("installed-wildcard-registry");
        let package = scratch.join("node_modules/feature-kit");
        fs::create_dir_all(package.join("dist/features")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
        )
        .unwrap();
        fs::write(
            package.join("index.d.ts"),
            "export declare function root(): number;",
        )
        .unwrap();
        fs::write(
            package.join("index.js"),
            "module.exports = { root: function() { return 1; } };",
        )
        .unwrap();
        fs::write(
            package.join("dist/features/double.d.ts"),
            "export default function double(value: number): number;",
        )
        .unwrap();
        fs::write(
            package.join("dist/features/double.js"),
            "module.exports = function(value) { return value * 2; };",
        )
        .unwrap();

        let added = add_installed(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
        assert_eq!(added.resolved_version, "1.2.3");
        let subpath = resolve(&registry, "feature-kit/features/double").unwrap();
        assert!(subpath.dts_source.contains("double"));
        assert!(subpath.bundle_js.unwrap().contains("value * 2"));
        let _ = fs::remove_dir_all(scratch);
        let _ = fs::remove_dir_all(registry);
    }

    #[test]
    fn resolves_an_installed_package_subpath() {
        let registry = temp_registry("subpath");
        let dir = registry.join("math-kit/subpaths/advanced");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("package.d.ts"),
            "export declare function square(value: number): number;",
        )
        .unwrap();
        fs::write(
            dir.join("bundle.js"),
            "module.exports = { square: function(value) { return value * value; } };",
        )
        .unwrap();

        let package = resolve(&registry, "math-kit/advanced").unwrap();
        assert_eq!(package.name, "math-kit/advanced");
        assert!(package.dts_source.contains("square"));
        assert!(package.bundle_js.unwrap().contains("value * value"));
        let _ = fs::remove_dir_all(registry);
    }

    fn temp_registry(test_name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "thaw-registry-test-{test_name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_a_package_with_all_runtime_backends() {
        let registry = temp_registry("full");
        let pkg_dir = registry.join("left-pad");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.d.ts"),
            "export declare function pad(s: string): string;",
        )
        .unwrap();
        fs::write(pkg_dir.join("native.a"), b"fake archive").unwrap();
        fs::write(pkg_dir.join("native.node"), b"fake addon").unwrap();
        fs::write(pkg_dir.join("bundle.js"), "function pad(s){return s;}").unwrap();
        fs::write(pkg_dir.join("version.txt"), "1.3.0").unwrap();

        let resolved = resolve(&registry, "left-pad").unwrap();
        assert_eq!(resolved.name, "left-pad");
        assert!(resolved.dts_source.contains("declare function pad"));
        assert_eq!(resolved.native_lib, Some(pkg_dir.join("native.a")));
        assert_eq!(resolved.native_addon, Some(pkg_dir.join("native.node")));
        assert_eq!(
            resolved.bundle_js.as_deref(),
            Some("function pad(s){return s;}")
        );
        assert_eq!(resolved.version.as_deref(), Some("1.3.0"));

        let _ = fs::remove_dir_all(&registry);
    }

    #[test]
    fn selects_the_current_targets_bundled_node_prebuild() {
        let package = temp_registry("select_native_prebuild");
        let (platform, arch, libc) = target_prebuild_components();
        let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
        fs::create_dir_all(&target).unwrap();
        let filename = if libc == "musl" {
            "binding.musl.node"
        } else {
            "binding.node"
        };
        fs::write(target.join(filename), b"native bytes").unwrap();
        // The opposite Linux libc must not be selected accidentally.
        if platform == "linux" {
            let opposite = if libc == "musl" {
                "binding.node"
            } else {
                "binding.musl.node"
            };
            fs::write(target.join(opposite), b"wrong libc").unwrap();
        }

        let selected = select_prebuilt_addon(&package).unwrap().unwrap();
        assert_eq!(selected.path, target.join(filename));
        assert_eq!(selected.platform, platform);
        assert_eq!(selected.arch, arch);
        assert_eq!(selected.libc, libc);
        let _ = fs::remove_dir_all(package);
    }

    #[test]
    fn builds_prebuild_install_github_asset_for_the_current_target() {
        let manifest = serde_json::json!({
            "name": "sqlite3",
            "version": "5.1.7",
            "repository": {
                "type": "git",
                "url": "git+https://github.com/TryGhost/node-sqlite3.git"
            },
            "binary": { "napi_versions": [3, 6, 99] }
        });
        let (url, asset, platform, arch) = prebuild_install_asset(&manifest).unwrap();
        let (target_platform, target_arch, libc) = target_prebuild_components();
        let asset_platform = if target_platform == "linux" && libc == "musl" {
            "linuxmusl"
        } else {
            target_platform
        };
        assert_eq!(
            asset,
            format!("sqlite3-v5.1.7-napi-v6-{asset_platform}-{target_arch}.tar.gz")
        );
        assert_eq!(
            url,
            format!("https://github.com/TryGhost/node-sqlite3/releases/download/v5.1.7/{asset}")
        );
        assert_eq!(platform, asset_platform);
        assert_eq!(arch, target_arch);
    }

    #[test]
    fn reports_available_targets_when_no_prebuild_matches() {
        let package = temp_registry("mismatched_native_prebuild");
        let target = package.join("prebuilds/imaginary-other");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("binding.node"), b"native bytes").unwrap();
        let diagnostic = select_prebuilt_addon(&package).unwrap_err();
        assert!(diagnostic.contains("no bundled native addon matches"));
        assert!(diagnostic.contains("imaginary-other"));
        let _ = fs::remove_dir_all(package);
    }

    #[test]
    fn selects_a_platform_optional_dependency_node_addon() {
        let node_modules = temp_registry("optional_native_prebuild");
        let (platform, arch, libc) = target_prebuild_components();
        let dependency = if platform == "linux" {
            format!("@example/addon-{platform}-{arch}-{libc}")
        } else {
            format!("@example/addon-{platform}-{arch}")
        };
        let dependency_dir = node_modules.join(&dependency);
        fs::create_dir_all(&dependency_dir).unwrap();
        fs::write(
            dependency_dir.join("package.json"),
            format!(r#"{{"name":"{dependency}","main":"binding.node"}}"#),
        )
        .unwrap();
        fs::write(dependency_dir.join("binding.node"), b"native bytes").unwrap();
        let manifest = serde_json::json!({
            "optionalDependencies": { dependency.clone(): "1.0.0" }
        });
        let selected = select_optional_dependency_addon(&node_modules, &manifest)
            .unwrap()
            .unwrap();
        assert_eq!(selected.path, dependency_dir.join("binding.node"));
        assert_eq!(selected.source, format!("{dependency}/binding.node"));
        assert_eq!(selected.platform, platform);
        assert_eq!(selected.arch, arch);
        assert_eq!(selected.libc, libc);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn resolves_a_packages_lock_json_when_present() {
        let registry = temp_registry("with_lock");
        let pkg_dir = registry.join("qs");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.d.ts"),
            "export declare function stringify(x: string): string;",
        )
        .unwrap();
        fs::write(
            pkg_dir.join("bundle.js"),
            "function stringify(x){return x;}",
        )
        .unwrap();
        fs::write(pkg_dir.join("version.txt"), "6.11.0").unwrap();
        fs::write(
            pkg_dir.join("lock.json"),
            r#"{"qs": "6.11.0", "side-channel": "1.0.4"}"#,
        )
        .unwrap();

        let resolved = resolve(&registry, "qs").unwrap();
        let deps = resolved.dependency_versions.expect("lock.json was written");
        assert_eq!(deps.get("qs").map(String::as_str), Some("6.11.0"));
        assert_eq!(deps.get("side-channel").map(String::as_str), Some("1.0.4"));

        let _ = fs::remove_dir_all(&registry);
    }

    #[test]
    fn resolves_a_package_with_only_the_required_dts() {
        let registry = temp_registry("dts_only");
        let pkg_dir = registry.join("is-odd");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("package.d.ts"),
            "export declare function isOdd(n: number): boolean;",
        )
        .unwrap();

        let resolved = resolve(&registry, "is-odd").unwrap();
        assert!(resolved.native_lib.is_none());
        assert!(resolved.bundle_js.is_none());
        // A hand-curated package (or one `add`ed before `version.txt`
        // existed) has no version on record -- not an error, just unknown.
        assert!(resolved.version.is_none());

        let _ = fs::remove_dir_all(&registry);
    }

    #[test]
    fn errors_when_package_dts_is_missing() {
        let registry = temp_registry("missing_dts");
        let pkg_dir = registry.join("ghost");
        fs::create_dir_all(&pkg_dir).unwrap();

        let err = resolve(&registry, "ghost").unwrap_err();
        assert!(err.contains("ghost"));
        assert!(err.contains("package.d.ts"));

        let _ = fs::remove_dir_all(&registry);
    }

    #[test]
    fn errors_when_package_directory_does_not_exist() {
        let registry = temp_registry("no_such_pkg");
        let err = resolve(&registry, "nonexistent").unwrap_err();
        assert!(err.contains("nonexistent"));

        let _ = fs::remove_dir_all(&registry);
    }

    /// `add`'s actual `npm install` step needs network access and isn't
    /// exercised by the automated suite (consistent with this project's
    /// other network-touching work, which was validated manually rather
    /// than in `cargo test` -- see docs/design/registry.md). `find_own_dts`/
    /// `types_package_name` are the pieces of `add` with real decision
    /// logic and no network dependency, so they get full offline coverage
    /// here.
    #[test]
    fn finds_dts_from_types_field() {
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"types": "dist/index.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_types_field");
        let (rel, abs) = find_own_dts(&manifest, &dir).unwrap();
        assert_eq!(rel, "dist/index.d.ts");
        assert_eq!(abs, dir.join("dist/index.d.ts"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_dts_from_typings_field_when_types_is_absent() {
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_typings_field");
        let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
        assert_eq!(rel, "index.d.ts");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefers_types_field_over_typings_field() {
        let manifest: serde_json::Value =
            serde_json::from_str(r#"{"types": "a.d.ts", "typings": "b.d.ts"}"#).unwrap();
        let dir = temp_registry("dts_prefers_types");
        let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
        assert_eq!(rel, "a.d.ts");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_index_d_ts_when_no_field_is_present() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
        let dir = temp_registry("dts_index_fallback");
        fs::write(dir.join("index.d.ts"), "declare function f(): void;").unwrap();
        let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
        assert_eq!(rel, "index.d.ts");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_no_dts_when_nothing_is_bundled() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
        let dir = temp_registry("dts_none");
        assert!(find_own_dts(&manifest, &dir).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn types_package_name_for_an_unscoped_package() {
        assert_eq!(types_package_name("left-pad"), "@types/left-pad");
    }

    #[test]
    fn types_package_name_for_a_scoped_package() {
        assert_eq!(types_package_name("@babel/core"), "@types/babel__core");
    }

    /// Found by running a real npm package (`ms`, `"main": "./index"`)
    /// through `add`: reading the literal `main` string as a path fails
    /// since Node resolves the missing `.js` extension at require-time,
    /// which we don't get for free.
    #[test]
    fn resolves_main_field_missing_its_extension() {
        let dir = temp_registry("main_no_extension");
        fs::write(dir.join("index.js"), "module.exports = 1;").unwrap();
        let (rel, abs) = resolve_module_path(&dir, "./index").unwrap();
        assert_eq!(rel, "./index.js");
        assert_eq!(abs, dir.join("index.js"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_main_field_pointing_at_a_directory() {
        let dir = temp_registry("main_directory");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("lib/index.js"), "module.exports = 1;").unwrap();
        let (rel, abs) = resolve_module_path(&dir, "./lib").unwrap();
        assert_eq!(rel, "./lib/index.js");
        assert_eq!(abs, dir.join("lib/index.js"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_a_required_directory_through_its_own_package_manifest() {
        let dir = temp_registry("nested_directory_manifest");
        let feature = dir.join("feature");
        fs::create_dir_all(feature.join("dist")).unwrap();
        fs::write(
            feature.join("package.json"),
            r#"{"exports":{".":{"require":"./dist/index.cjs"}}}"#,
        )
        .unwrap();
        fs::write(feature.join("dist/index.cjs"), "module.exports = 42;").unwrap();
        let (relative, absolute) = resolve_module_path(&dir, "./feature").unwrap();
        assert_eq!(relative, "feature/dist/index.cjs");
        assert_eq!(absolute, feature.join("dist/index.cjs"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_main_field_written_exactly() {
        let dir = temp_registry("main_exact");
        fs::write(dir.join("main.js"), "module.exports = 1;").unwrap();
        let (rel, abs) = resolve_module_path(&dir, "main.js").unwrap();
        assert_eq!(rel, "main.js");
        assert_eq!(abs, dir.join("main.js"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_with_all_tried_candidates_when_main_cannot_be_resolved() {
        let dir = temp_registry("main_missing");
        let err = resolve_module_path(&dir, "./index").unwrap_err();
        assert!(err.contains("./index"));
        assert!(err.contains("./index.js"));
        assert!(err.contains("./index/index.js"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_single_and_double_quoted_relative_requires() {
        let specs: Vec<_> =
            find_module_specs(r#"var a = require('./a'); var b = require("../lib/b");"#)
                .into_iter()
                .filter(|spec| spec.starts_with('.'))
                .collect();
        assert_eq!(specs, vec!["./a".to_string(), "../lib/b".to_string()]);
    }

    #[test]
    fn finds_literal_dependencies_called_through_create_require_aliases() {
        assert_eq!(
            find_module_specs(
                "import { createRequire as makeRequire } from 'node:module';\n\
                 import * as Module from 'module';\n\
                 const local = makeRequire('pkg/index.js');\n\
                 const other = Module.createRequire('pkg/index.js');\n\
                 local('./dependency'); other(`./other`);"
            ),
            vec!["./dependency", "./other", "node:module", "module"]
        );
    }

    #[test]
    fn parser_collects_esm_reexports_and_dynamic_imports_without_false_positives() {
        let specs = find_module_specs(
            r#"
                import main from './main.js';
                export { value } from "./value.js";
                export * from './all.js';
                const package = import(`external-package`);
                const feature = import(("external-" + "feature"));
                const later = import('./later.js');
                const text = "require('./not-real.js')";
                // require('./also-not-real.js')
                object.require('./member.js');
                require(variable);
            "#,
        );
        assert_eq!(
            specs,
            vec![
                "external-package",
                "external-feature",
                "./later.js",
                "./main.js",
                "./value.js",
                "./all.js"
            ]
        );
    }

    #[test]
    fn dynamic_import_candidate_expansion_is_bounded() {
        let analysis = analyze_module(
            "import((a ? 'a' : 'b') + (b ? 'a' : 'b') + (c ? 'a' : 'b') + (d ? 'a' : 'b') + (e ? 'a' : 'b') + (f ? 'a' : 'b') + (g ? 'a' : 'b'));",
        );
        assert!(analysis.has_nonliteral_dynamic_import);
        assert!(analysis.specs.is_empty());
    }

    #[test]
    fn parser_identifies_commonjs_export_assignments() {
        let analysis = analyze_module(
            r#"
                exports.alpha = 1;
                module.exports.beta = 2;
                module.exports["gamma"] = 3;
                module.exports = function () {};
                object.exports.nope = 4;
            "#,
        );
        assert_eq!(
            analysis._commonjs_exports,
            vec!["alpha", "beta", "default", "gamma"]
        );
    }

    #[test]
    fn rewrites_literal_dynamic_import_to_an_async_bundle_require() {
        let rewritten =
            rewrite_esm_to_commonjs("function load() { return import('./feature.js'); }").unwrap();
        assert!(rewritten.contains("requireAsync(String('./feature.js'))"));
        assert!(!rewritten.contains("import("));
    }

    #[test]
    fn ignores_bare_specifier_requires() {
        let specs: Vec<_> = find_module_specs(r#"var x = require('is-number');"#)
            .into_iter()
            .filter(|spec| spec.starts_with('.'))
            .collect();
        assert!(specs.is_empty());
    }

    #[test]
    fn finds_bare_specifiers_including_scoped_packages() {
        let specs: Vec<_> = find_module_specs(
            r#"var a = require('side-channel'); var b = require('@babel/core'); var c = require('./local');"#,
        )
        .into_iter()
        .filter(|spec| !spec.starts_with('.'))
        .collect();
        assert_eq!(
            specs,
            vec!["side-channel".to_string(), "@babel/core".to_string()]
        );
    }

    #[test]
    fn splits_bare_specs_into_package_and_subpath() {
        assert_eq!(split_bare_spec("lodash"), ("lodash", None));
        assert_eq!(split_bare_spec("lodash/fp"), ("lodash", Some("fp")));
        assert_eq!(
            split_bare_spec("es-errors/type"),
            ("es-errors", Some("type"))
        );
        assert_eq!(split_bare_spec("@babel/core"), ("@babel/core", None));
        assert_eq!(
            split_bare_spec("@babel/core/lib/index"),
            ("@babel/core", Some("lib/index"))
        );
    }

    #[test]
    fn splits_version_specs_from_add_arguments() {
        assert_eq!(split_package_spec("left-pad"), ("left-pad", None));
        assert_eq!(
            split_package_spec("left-pad@1.3.0"),
            ("left-pad", Some("1.3.0"))
        );
        assert_eq!(
            split_package_spec("left-pad@^1.2.0"),
            ("left-pad", Some("^1.2.0"))
        );
        assert_eq!(
            split_package_spec("left-pad@next"),
            ("left-pad", Some("next"))
        );
        // A scoped package's leading `@scope/` is never mistaken for a
        // version separator -- only an `@` after the scope's own `/`
        // starts one.
        assert_eq!(split_package_spec("@hapi/hoek"), ("@hapi/hoek", None));
        assert_eq!(
            split_package_spec("@hapi/hoek@9.0.0"),
            ("@hapi/hoek", Some("9.0.0"))
        );
    }

    /// The exact shape found in `qs`'s own real transitive dependency
    /// chain: `require('es-errors/type')`, a "deep import" subpath into
    /// another package, resolved directly against that package's root
    /// (not through its `main` field).
    #[test]
    fn resolves_a_deep_import_subpath_into_a_dependency() {
        let node_modules = temp_registry("deep_import_node_modules");
        fs::create_dir_all(node_modules.join("es-errors")).unwrap();
        fs::write(
            node_modules.join("es-errors/package.json"),
            r#"{"main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            node_modules.join("es-errors/index.js"),
            "module.exports = {};",
        )
        .unwrap();
        fs::write(
            node_modules.join("es-errors/type.js"),
            "module.exports = TypeError;",
        )
        .unwrap();

        let (name, relative, abs, dir) =
            resolve_bare_require(&node_modules, "es-errors/type").unwrap();
        assert_eq!(name, "es-errors");
        assert_eq!(relative, "type.js");
        assert_eq!(abs, node_modules.join("es-errors/type.js"));
        assert_eq!(dir, node_modules.join("es-errors"));

        let _ = fs::remove_dir_all(&node_modules);
    }

    #[test]
    fn resolves_a_deep_import_into_a_scoped_package() {
        let node_modules = temp_registry("deep_import_scoped_node_modules");
        fs::create_dir_all(node_modules.join("@scope/pkg/lib")).unwrap();
        fs::write(
            node_modules.join("@scope/pkg/lib/util.js"),
            "module.exports = 1;",
        )
        .unwrap();

        let (name, relative, ..) =
            resolve_bare_require(&node_modules, "@scope/pkg/lib/util").unwrap();
        assert_eq!(name, "@scope/pkg");
        assert_eq!(relative, "lib/util.js");

        let _ = fs::remove_dir_all(&node_modules);
    }

    #[test]
    fn ignores_dynamic_and_malformed_require_calls() {
        // `require(name)` (a variable, not a literal) and a stray
        // "require" that isn't actually a call must not confuse the scan
        // -- and must not stop it from still finding a real one after.
        let specs: Vec<_> = find_module_specs(
            "var x = require(name); var note = 'requirements'; var y = require('./y');",
        )
        .into_iter()
        .filter(|spec| spec.starts_with('.'))
        .collect();
        assert_eq!(specs, vec!["./y".to_string()]);
    }

    #[test]
    fn normalizes_dot_and_dot_dot_segments() {
        assert_eq!(normalize_path_string("lib/./stringify"), "lib/stringify");
        assert_eq!(normalize_path_string("lib/../parse"), "parse");
        assert_eq!(normalize_path_string("a/b/../../c"), "c");
    }

    /// The exact shape found in a real npm package (`qs`): `main` requires
    /// two sibling files by relative path, each with no further requires
    /// of their own.
    #[test]
    fn bundles_a_multi_file_package_reachable_from_main() {
        let dir = temp_registry("bundle_multi_file");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            "var parse = require('./parse');\nvar stringify = require('./stringify');\n\
             module.exports = { parse: parse, stringify: stringify };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/parse.js"),
            "module.exports = function parse(s) { return s; };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/stringify.js"),
            "module.exports = function stringify(s) { return s; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_multi_file_node_modules");
        let (bundle, main_key, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(main_key, "pkg/lib/index.js");
        assert_eq!(file_count, 3, "main + parse.js + stringify.js");
        assert!(bundle.contains("pkg/lib/parse.js"));
        assert!(bundle.contains("pkg/lib/stringify.js"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn bundled_modules_receive_node_filename_and_dirname() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("bundle_module_paths");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            "var child = require('./child'); module.exports = function() { return [__filename, __dirname, child]; };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/child.js"),
            "module.exports = [__filename, __dirname];",
        )
        .unwrap();
        let node_modules = temp_registry("bundle_module_paths_node_modules");
        let (bundle, _, _, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "lib/index.js").unwrap();
        let script = CString::new(format!(
            "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.readModulePaths = module.exports;"
        ))
        .unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("readModulePaths").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["/thaw_modules/pkg/lib/index.js","/thaw_modules/pkg/lib",["/thaw_modules/pkg/lib/child.js","/thaw_modules/pkg/lib"]]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    /// `bundle_commonjs_package`'s version-recording half (the other half
    /// of `add`'s 17章 `version.txt` work, extended to cover every
    /// package the bundle actually reaches, not just the root one --
    /// `AddedPackage::dependency_versions`/`lock.json`). Both the root
    /// package (`pkg`) and its one real dependency (`left-pad-ish`) have
    /// their own `package.json` with a `version` field here, mirroring
    /// what `npm install` actually leaves on disk.
    #[test]
    fn bundle_commonjs_package_records_every_reached_packages_version() {
        let dir = temp_registry("bundle_versions_root");
        fs::write(
            dir.join("package.json"),
            r#"{"name": "pkg", "version": "2.5.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "var dep = require('left-pad-ish');\nmodule.exports = dep;",
        )
        .unwrap();

        let node_modules = temp_registry("bundle_versions_node_modules");
        fs::create_dir_all(node_modules.join("left-pad-ish")).unwrap();
        fs::write(
            node_modules.join("left-pad-ish/package.json"),
            r#"{"name": "left-pad-ish", "version": "1.3.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            node_modules.join("left-pad-ish/index.js"),
            "module.exports = function () { return 'padded'; };",
        )
        .unwrap();

        let (_, _, _, dependency_versions) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();

        assert_eq!(
            dependency_versions.get("pkg").map(String::as_str),
            Some("2.5.0")
        );
        assert_eq!(
            dependency_versions.get("left-pad-ish").map(String::as_str),
            Some("1.3.0")
        );
        assert_eq!(dependency_versions.len(), 2, "no extra/missing entries");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules);
    }

    /// A package with no dependencies still gets exactly one entry (its
    /// own) -- `fetch_and_copy` uses this len-1 case to decide *not* to
    /// write a redundant `lock.json` next to `version.txt`.
    #[test]
    fn bundle_commonjs_package_with_no_dependencies_records_only_itself() {
        let dir = temp_registry("bundle_versions_solo");
        fs::write(
            dir.join("package.json"),
            r#"{"name": "solo-pkg", "version": "0.1.0", "main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "module.exports = function () { return 1; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_versions_solo_node_modules");
        let (_, _, _, dependency_versions) =
            bundle_commonjs_package(&empty_node_modules, "solo-pkg", &dir, "index.js").unwrap();

        assert_eq!(dependency_versions.len(), 1);
        assert_eq!(
            dependency_versions.get("solo-pkg").map(String::as_str),
            Some("0.1.0")
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn bundle_of_a_single_file_package_still_has_one_module() {
        let dir = temp_registry("bundle_single_file");
        fs::write(
            dir.join("index.js"),
            "module.exports = function f() { return 1; };",
        )
        .unwrap();

        let empty_node_modules = temp_registry("bundle_single_file_node_modules");
        let (_, main_key, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(main_key, "pkg/index.js");
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// An unresolvable relative require (here: a `.json` target, which
    /// `resolve_module_path`'s candidates don't cover) must not abort
    /// bundling the rest of the package -- it's just left for the
    /// external-require stub to report clearly if actually called.
    #[test]
    fn unresolvable_relative_require_does_not_abort_bundling() {
        let dir = temp_registry("bundle_unresolvable_require");
        fs::write(
            dir.join("index.js"),
            "var pkg = require('./package.json');\nmodule.exports = function f() { return 1; };",
        )
        .unwrap();
        // Deliberately no package.json written -- this require can never
        // resolve via resolve_module_path's .js/index.js candidates.

        let empty_node_modules = temp_registry("bundle_unresolvable_require_node_modules");
        let (_, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 1);

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The bundle isn't just plausible-looking text -- it must actually
    /// run correctly through the real QuickJS-NG engine, including a
    /// same-package relative require resolving to a sibling module *and*
    /// a bare-specifier require resolving to a real dependency package
    /// under `node_modules` (the actual new capability: `qs`'s own
    /// dependency on `side-channel`, reproduced in miniature). A third,
    /// genuinely external require (something not present under
    /// `node_modules` at all, mirroring a Node core builtin or a
    /// dependency `npm install` didn't fetch) is left inside a function
    /// that's never called -- were it eager and reached, it would throw
    /// immediately, same as `is-odd`'s real `require('is-number')`
    /// (already covered by this session's end-to-end verification); this
    /// test is specifically about what *does* resolve.
    #[test]
    fn bundle_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("bundle_runs_through_quickjs");
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(
            dir.join("lib/index.js"),
            "var double = require('./double');\n\
             var triple = require('triple-dep');\n\
             function unused() { return require('a-package-that-was-never-installed'); }\n\
             module.exports = function run(n) { return double(triple(n)); };",
        )
        .unwrap();
        fs::write(
            dir.join("lib/double.js"),
            "module.exports = function (n) { return n * 2; };",
        )
        .unwrap();

        let node_modules_dir = temp_registry("bundle_runs_through_quickjs_node_modules");
        fs::create_dir_all(node_modules_dir.join("triple-dep")).unwrap();
        fs::write(
            node_modules_dir.join("triple-dep/package.json"),
            r#"{"main": "index.js"}"#,
        )
        .unwrap();
        fs::write(
            node_modules_dir.join("triple-dep/index.js"),
            "module.exports = function (n) { return n * 3; };",
        )
        .unwrap();

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "lib/index.js").unwrap();
        assert_eq!(
            file_count, 3,
            "pkg's index.js + double.js + triple-dep's index.js"
        );

        // Same environment thaw-bridge's `wrap_as_commonjs_module` sets
        // up: global `module`/`exports`/`require` before running the
        // source, then bind the default export by name afterward.
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports;\n"
        );

        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("run").unwrap();
        let args = CString::new("[7]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            result, "42",
            "relative require (double) and cross-package bare require (triple-dep) must both resolve correctly"
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules_dir);
    }

    /// Real `--use pkg-a --use pkg-b` loads each package's bundle into the
    /// *same* shared thread-local QuickJS-NG context, one `loadScript`
    /// call per package (`wrap_as_commonjs_module`, called once per
    /// package). Before wrapping each bundle's module-system helpers in
    /// an IIFE, they were plain globals (`__thaw_bundle_cache` etc.), so
    /// loading package B would silently overwrite package A's -- invisible
    /// for a require resolved eagerly at load time (already finished and
    /// cached by then), but package A's *lazy* internal require (deferred
    /// inside a function body, only actually called after B has loaded)
    /// would then resolve against B's module map instead of its own.
    #[test]
    fn multiple_bundled_packages_dont_stomp_each_others_module_state() {
        use std::ffi::{CStr, CString};

        let node_modules_dir = temp_registry("multi_pkg_node_modules");

        let dir_a = temp_registry("multi_pkg_a");
        fs::write(
            dir_a.join("index.js"),
            "module.exports = function getLazy() { return require('./lazy')(); };",
        )
        .unwrap();
        fs::write(
            dir_a.join("lazy.js"),
            "module.exports = function () { return 'from lazy'; };",
        )
        .unwrap();
        let (bundle_a, _, _, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg-a", &dir_a, "index.js").unwrap();

        let dir_b = temp_registry("multi_pkg_b");
        fs::write(
            dir_b.join("index.js"),
            "module.exports = function () { return 'b'; };",
        )
        .unwrap();
        let (bundle_b, _, _, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg-b", &dir_b, "index.js").unwrap();

        let wrap = |bundle: &str, bind_as: &str| {
            format!(
                "globalThis.module = {{ exports: {{}} }};\n\
                 globalThis.exports = globalThis.module.exports;\n\
                 globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
                 {bundle}\n\
                 globalThis.{bind_as} = module.exports;\n"
            )
        };

        let source_a = CString::new(wrap(&bundle_a, "getLazy")).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source_a.as_ptr()),
            1,
            "package A failed to load"
        );

        // Loaded into the same shared global context *after* A.
        let source_b = CString::new(wrap(&bundle_b, "pkgB")).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source_b.as_ptr()),
            1,
            "package B failed to load"
        );

        // Call A's lazily-requiring function *after* B has loaded.
        let func = CString::new("getLazy").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            result, "\"from lazy\"",
            "package A's lazy internal require must still resolve against its own module map, \
             not package B's, after B has loaded into the shared global context"
        );

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        let _ = fs::remove_dir_all(&node_modules_dir);
    }

    #[test]
    fn bundle_exports_self_contained_worker_runtime_source() {
        use std::ffi::CString;

        let node_modules_dir = temp_registry("worker_runtime_source_node_modules");
        let dir = temp_registry("worker_runtime_source");
        fs::write(
            dir.join("index.js"),
            "module.exports = function () { return require('./value')(); };",
        )
        .unwrap();
        fs::write(
            dir.join("value.js"),
            "module.exports = function () { return 42; };",
        )
        .unwrap();
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules_dir, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let source = CString::new(format!(
            "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle}"
        ))
        .unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);

        let result = thaw_quickjs::eval_json(
            "Function('require', __thaw_worker_bundle_source + \"return __thaw_bundle_create_require('pkg/index.js')('./value')();\")(function(name) { throw new Error(name); })",
        )
        .unwrap();
        assert_eq!(result.as_deref(), Some("42"));

        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules_dir);
    }

    /// The exact shape found in `qs`'s real dependency chain:
    /// `object-inspect` (pulled in transitively) does
    /// `require('util').inspect.custom` unconditionally at load time,
    /// with no matching `node_modules/util` -- must resolve via the
    /// `util` builtin polyfill instead of falling through to the
    /// external-require stub.
    #[test]
    fn bundles_the_util_builtin_polyfill_when_required() {
        let dir = temp_registry("builtin_util");
        fs::write(
            dir.join("index.js"),
            "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_node_modules");

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2, "pkg's index.js + the util polyfill");
        assert!(bundle.contains("node:util"));

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The exact pattern found in a real ESM npm package (`has-flag`):
    /// `import process from 'process'`, then reading `process.argv`.
    #[test]
    fn process_builtin_polyfill_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_process");
        fs::write(
            dir.join("index.js"),
            "import process from 'process';\n\
             export default function getPlatform() { return process.platform; }",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_process_node_modules");

        let (bundle, _, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.getPlatform = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "failed to load"
        );

        let func = CString::new("getPlatform").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"linux\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn querystring_builtin_runs_through_bundled_commonjs_require() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_querystring");
        fs::write(
            dir.join("index.js"),
            "var querystring = require('querystring');\n\
             module.exports = function () {\n\
             \x20 return querystring.stringify({ a: [1, 2], space: 'two words' }) + ':' + JSON.stringify(querystring.parse('x=1&x=2'));\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_querystring_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseQuerystring = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseQuerystring").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#""a=1&a=2&space=two%20words:{\"x\":[\"1\",\"2\"]}""#
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn events_builtin_runs_event_emitter_through_commonjs_require() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_events");
        fs::write(
            dir.join("index.js"),
            "var EventEmitter = require('events');\n\
             function Child() { EventEmitter.call(this); }\n\
             Child.prototype = Object.create(EventEmitter.prototype);\n\
             Child.prototype.constructor = Child;\n\
             module.exports = function () {\n\
             \x20 var emitter = new Child(); var seen = [];\n\
             \x20 function regular(value) { seen.push('regular:' + value); }\n\
             \x20 emitter.on('value', regular);\n\
             \x20 emitter.prependOnceListener('value', function(value) { seen.push('once:' + value); });\n\
             \x20 var first = emitter.emit('value', 1); var second = emitter.emit('value', 2);\n\
             \x20 emitter.off('value', regular); var third = emitter.emit('value', 3);\n\
             \x20 return [seen.join(','), first, second, third, emitter.listenerCount('value'), emitter.eventNames().length];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_events_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseEvents = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseEvents").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["once:1,regular:1,regular:2",true,true,false,0,0]"#
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn events_builtin_supports_symbols_async_iteration_and_abort() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_events_async");
        fs::write(
            dir.join("index.js"),
            "var events = require('node:events'); module.exports = async function () { var emitter = new events.EventEmitter(), symbol = Symbol('value'), observed = []; emitter.on(symbol, function(value) { observed.push(value); }); emitter.emit(symbol, 1); var iterator = events.on(emitter, 'data'); emitter.emit('data', 'first', 2); emitter.emit('data', 'second', 3); var first = await iterator.next(), second = await iterator.next(), returned = await iterator.return(); var controller = new AbortController(), aborted = events.on(emitter, 'wait', { signal: controller.signal }), error; var pending = aborted.next().catch(function(value) { error = [value.name, value.code, value.cause]; }); controller.abort('reason'); await pending; var onceController = new AbortController(), onceError; var oncePending = events.once(emitter, 'never', { signal: onceController.signal }).catch(function(value) { onceError = [value.name, value.code, value.cause]; }); onceController.abort('once-reason'); await oncePending; events.setMaxListeners(4, emitter); var listener = function() {}; emitter.on('probe', listener); return [observed, first.value, second.value, returned.done, emitter.listenerCount('data'), emitter.eventNames().some(function(value) { return value === symbol; }), events.getEventListeners(emitter, 'probe')[0] === listener, emitter.getMaxListeners(), events.getMaxListeners(emitter), error, onceError, typeof events.errorMonitor, typeof events.captureRejectionSymbol]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_events_async_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseEventsAsync = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseEventsAsync").unwrap().as_ptr(),
            CString::new("[]").unwrap().as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[[1],["first",2],["second",3],true,0,true,true,4,4,["AbortError","ABORT_ERR","reason"],["AbortError","ABORT_ERR","once-reason"],"symbol","symbol"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn assert_builtin_reports_structured_failures_through_commonjs_require() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_assert");
        fs::write(
            dir.join("index.js"),
            "var assert = require('node:assert/strict');\n\
             module.exports = function () {\n\
             \x20 assert.deepStrictEqual({ a: [1, 2], date: new Date(3) }, { date: new Date(3), a: [1, 2] });\n\
             \x20 assert.throws(function() { throw new TypeError('bad value'); }, /bad/);\n\
             \x20 var failure; try { assert.strictEqual(1, 2, 'different'); } catch (error) { failure = [error instanceof assert.AssertionError, error.name, error.code, error.actual, error.expected, error.operator, error.message]; }\n\
             \x20 return failure;\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_assert_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAssert = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseAssert").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,"AssertionError","ERR_ASSERTION",1,2,"strictEqual","different"]"#
        );

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// Not just plausible text -- runs through the real QuickJS-NG
    /// engine, confirming `util.inspect.custom` actually comes back as a
    /// real `Symbol` (what `object-inspect` needs it to be), not merely
    /// present.
    #[test]
    fn util_polyfill_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_util_runs");
        fs::write(
            dir.join("index.js"),
            "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_runs_node_modules");

        let (bundle, _, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.checkInspectCustom = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("checkInspectCustom").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"symbol\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn util_helpers_run_through_bundled_commonjs_require() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_util_helpers");
        fs::write(
            dir.join("index.js"),
            "var util = require('node:util');\n\
             function Base() {} function Child() {} util.inherits(Child, Base);\n\
             module.exports = async function () {\n\
             \x20 var custom = {}; custom[util.inspect.custom] = function() { return 'custom'; };\n\
             \x20 var add = util.promisify(function(a, b, callback) { queueMicrotask(function() { callback(null, a + b); }); });\n\
             \x20 var sum = await add(2, 3);\n\
             \x20 var callbackValue = await new Promise(function(resolve, reject) { util.callbackify(async function(value) { return value * 2; })(4, function(error, value) { if (error) reject(error); else resolve(value); }); });\n\
             \x20 return [util.format('%s:%d:%j:%%', 'value', 4, { ok: true }), util.inspect(custom), Child.super_ === Base, new Child() instanceof Base, util.types.isDate(new Date()), util.types.isRegExp(/x/), util.types.isMap(new Map()), util.types.isTypedArray(new Uint8Array(1)), sum, callbackValue, util.stripVTControlCharacters('\\u001b[31mred\\u001b[0m'), util.toUSVString('x\\ud800y')];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_helpers_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseUtil = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseUtil").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["value:4:{\"ok\":true}:%","custom",true,true,true,true,true,true,5,8,"red","x�y"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn util_parse_args_handles_long_short_multiple_negative_and_tokens() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_util_parse_args");
        fs::write(
            dir.join("index.js"),
            "var util = require('node:util'); module.exports = function () { var result = util.parseArgs({ args: ['-v', '--name=thaw', '--tag', 'one', '--tag=two', '--no-color', 'input.ts', '--', '-literal'], options: { verbose: { type: 'boolean', short: 'v' }, name: { type: 'string' }, tag: { type: 'string', multiple: true }, color: { type: 'boolean', default: true } }, allowNegative: true, allowPositionals: true, tokens: true }); var unknown; try { util.parseArgs({ args: ['--missing'], options: {} }); } catch (error) { unknown = error.code; } return [result.values.verbose, result.values.name, result.values.tag, result.values.color, result.positionals, result.tokens.map(function(token) { return token.kind; }), unknown, new util.TextEncoder().encode('ok').length, new util.TextDecoder().decode(Uint8Array.of(111, 107))]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_util_parse_args_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilParseArgs = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseUtilParseArgs").unwrap().as_ptr(),
            CString::new("[]").unwrap().as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,"thaw",["one","two"],false,["input.ts","-literal"],["option","option","option","option","option","positional","option-terminator","positional"],"ERR_PARSE_ARGS_UNKNOWN_OPTION",2,"ok"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn buffer_builtin_shares_the_global_buffer_implementation() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_buffer");
        fs::write(
            dir.join("index.js"),
            "var buffer = require('node:buffer');\n\
             module.exports = function () {\n\
             \x20 var value = buffer.Buffer.from('雪', 'utf8'); var invalid = buffer.Buffer.from([0xff]);\n\
             \x20 return [buffer.Buffer === globalThis.Buffer, value.toString('hex'), value.toString('base64'), buffer.byteLength('雪'), buffer.isUtf8(value), buffer.isUtf8(invalid), buffer.isAscii(buffer.Buffer.from('abc')), buffer.transcode(buffer.Buffer.from('hi'), 'utf8', 'utf16le').toString('hex')];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_buffer_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseBuffer = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseBuffer").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,"e99baa","6Zuq",3,true,false,true,"68006900"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn path_builtin_exposes_posix_and_win32_operations() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_path_operations");
        fs::write(
            dir.join("index.js"),
            "var path = require('node:path');\n\
             module.exports = function () {\n\
             \x20 var parsed = path.parse('/tmp/archive.tar.gz');\n\
             \x20 return [path.normalize('/a//b/../c/'), path.relative('/a/b', '/a/c/d'), path.extname('archive.tar.gz'), path.basename('archive.tar.gz', '.gz'), parsed, path.format(parsed), path.isAbsolute('/a'), path.posix === path, path.win32.normalize('C:\\\\a\\\\..\\\\b'), path.win32.isAbsolute('C:\\\\a'), path.delimiter, path.win32.delimiter];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_path_operations_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePath = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exercisePath").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["/a/c/","../c/d",".gz","archive.tar",{"root":"/","dir":"/tmp","base":"archive.tar.gz","ext":".gz","name":"archive.tar"},"/tmp/archive.tar.gz",true,true,"C:\\b",true,":",";"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn string_decoder_preserves_multibyte_boundaries() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_string_decoder");
        fs::write(
            dir.join("index.js"),
            "var StringDecoder = require('node:string_decoder').StringDecoder;\n\
             module.exports = function () {\n\
             \x20 var utf8 = new StringDecoder('utf8'); var snow = Buffer.from('雪'); var utf8Parts = [utf8.write(snow.subarray(0, 1)), utf8.write(snow.subarray(1, 2)), utf8.write(snow.subarray(2)), utf8.end()];\n\
             \x20 var utf16 = new StringDecoder('utf16le'); var wide = Buffer.from('A雪', 'utf16le'); var utf16Parts = [utf16.write(wide.subarray(0, 3)), utf16.end(wide.subarray(3))];\n\
             \x20 var base64 = new StringDecoder('base64'); var hello = Buffer.from('hello'); var encoded = base64.write(hello.subarray(0, 2)) + base64.write(hello.subarray(2)) + base64.end();\n\
             \x20 var incomplete = new StringDecoder(); var replacement = incomplete.end(Buffer.from([0xe9]));\n\
             \x20 return [utf8Parts, utf16Parts, encoded, replacement, utf8.lastNeed, utf8.encoding];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_string_decoder_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStringDecoder = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseStringDecoder").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["","","雪",""],["A","雪"],"aGVsbG8=","�",0,"utf8"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn timer_modules_share_the_runtime_queue_and_support_abort() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_timers");
        fs::write(
            dir.join("index.js"),
            "var timers = require('node:timers'); var promises = require('node:timers/promises');\n\
             module.exports = async function () {\n\
             \x20 var immediate = await new Promise(function(resolve) { timers.setImmediate(resolve, 'immediate'); });\n\
             \x20 var delayed = await promises.setTimeout(0, 'delayed'); await promises.scheduler.yield();\n\
             \x20 var controller = new AbortController(); controller.abort('cancelled'); var reason; try { await promises.setTimeout(1, 'wrong', { signal: controller.signal }); } catch (error) { reason = error; }\n\
             \x20 var interval = promises.setInterval(0, 'tick'); var first = await interval.next(); var second = await interval.next(); var ended = await interval.return();\n\
             \x20 return [immediate, delayed, reason, first.value, first.done, second.value, ended.done];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_timers_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTimers = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseTimers").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["immediate","delayed","cancelled","tick",false,"tick",true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_read_and_write_streams_integrate_with_node_streams() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_streams");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), stream = require('node:stream'); module.exports = async function (path) { var writer = fs.createWriteStream(path, { highWaterMark: 2 }), writerEvents = []; writer.on('open', function() { writerEvents.push('open'); }); writer.on('ready', function() { writerEvents.push('ready'); }); writer.on('close', function() { writerEvents.push('close'); }); var written = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.write('abc'); writer.end('def'); await written; await new Promise(function(resolve) { queueMicrotask(resolve); }); var appender = fs.createWriteStream(path, { flags: 'a' }), appended = new Promise(function(resolve, reject) { appender.on('error', reject); appender.on('finish', resolve); }); appender.end('!'); await appended; var reader = fs.createReadStream(path, { start: 1, end: 4, highWaterMark: 2 }), chunks = [], readerEvents = []; reader.on('open', function() { readerEvents.push('open'); }); reader.on('ready', function() { readerEvents.push('ready'); }); reader.on('data', function(value) { chunks.push(value.toString()); }); reader.on('close', function() { readerEvents.push('close'); }); var read = new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await read; await new Promise(function(resolve) { queueMicrotask(resolve); }); return [writer instanceof fs.WriteStream, writer instanceof stream.Writable, writer.bytesWritten, writerEvents, reader instanceof fs.ReadStream, reader instanceof stream.Readable, reader.bytesRead, chunks, readerEvents, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_streams_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreams = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("stream.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsStreams").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,true,6,["open","ready","close"],true,true,4,["bc","de"],["open","ready","close"],"abcdef!"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_streams_use_incremental_positioned_host_io() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_incremental_streams");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); var writer = fs.createWriteStream(path, { flags: 'r+', start: 2 }), observed; await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.write('XY', function(error) { if (error) return reject(error); observed = fs.readFileSync(path, 'utf8'); writer.end('Z'); }); }); var beforeRead = fs.readFileSync(path, 'utf8'), chunks = [], reader = fs.createReadStream(path, { highWaterMark: 2 }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('data', function(chunk) { chunks.push(chunk.toString()); if (chunks.length === 1) fs.writeFileSync(path, 'abWXYZ'); }); reader.on('end', resolve); }); return [observed, beforeRead, writer.bytesWritten, chunks, reader.bytesRead, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_incremental_streams_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseIncrementalFsStreams = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("stream.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseIncrementalFsStreams")
                .unwrap()
                .as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["abXYef","abXYZf",3,["ab","WX","YZ"],6,"abWXYZ"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_write_streams_apply_backpressure_and_serialize_callbacks() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_stream_backpressure");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/queued.txt', directPath = root + '/direct.txt', events = [], writer = fs.createWriteStream(path, { highWaterMark: 3 }); writer.on('drain', function() { events.push('drain'); }); writer.on('finish', function() { events.push('finish'); }); var first = writer.write('ab', function() { events.push('first'); }), second = writer.write('cd', function() { events.push('second'); }), beforeDrain = fs.readFileSync(path, 'utf8'); var finished = new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); }); writer.end('ef', function() { events.push('end'); }); await finished; await new Promise(function(resolve) { queueMicrotask(resolve); }); var direct = new fs.WriteStream(directPath, { highWaterMark: 1 }), directResult = direct.write('x'); var directFinished = new Promise(function(resolve, reject) { direct.on('error', reject); direct.on('finish', resolve); }); direct.end(); await directFinished; return [first, second, beforeDrain, writer.writableLength, writer.writableHighWaterMark, events, fs.readFileSync(path, 'utf8'), directResult, direct.writableHighWaterMark, fs.readFileSync(directPath, 'utf8')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_stream_backpressure_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreamBackpressure = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsStreamBackpressure")
                .unwrap()
                .as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,false,"ab",0,3,["first","second","drain","finish","end"],"abcdef",false,1,"x"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn stream_builtin_pipes_transforms_and_finishes() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_stream");
        fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); var streamPromises = require('node:stream/promises');\n\
             module.exports = async function () {\n\
             \x20 var output = []; var source = stream.Readable.from(['a', Buffer.from('b')]);\n\
             \x20 var upper = new stream.Transform({ transform: function(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); } });\n\
             \x20 var sink = new stream.Writable({ write: function(chunk, encoding, callback) { output.push(chunk.toString()); callback(); } });\n\
             \x20 await new Promise(function(resolve, reject) { stream.pipeline(source, upper, sink, function(error) { if (error) reject(error); else resolve(); }); });\n\
             \x20 var queued = new stream.Readable(); queued.push('left'); queued.push('right'); queued.push(null); var combined = queued.read().toString();\n\
             \x20 var pass = new stream.PassThrough(); var passed = []; pass.on('data', function(chunk) { passed.push(chunk.toString()); }); var completion = streamPromises.finished(pass); pass.end('pass'); await completion;\n\
             \x20 var promiseOutput = []; await streamPromises.pipeline(stream.Readable.from(['promise']), new stream.Writable({ write: function(chunk, encoding, callback) { promiseOutput.push(chunk.toString()); callback(); } }));\n\
             \x20 var controller = new AbortController(); var aborted = new stream.Readable(); var reason; aborted.on('error', function(error) { reason = error; }); stream.addAbortSignal(controller.signal, aborted); controller.abort('stop');\n\
             \x20 return [output.join(''), sink.writableFinished, source.readableEnded, combined, passed.join(''), pass.readableEnded, promiseOutput.join(''), aborted.destroyed, reason];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_stream_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStream = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseStream").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["AB",true,true,"leftright","pass",true,"promise",true,"stop"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn stream_writables_serialize_async_work_and_signal_backpressure() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_stream_backpressure");
        fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'); module.exports = async function () { var events = [], sink = new stream.Writable({ highWaterMark: 3, write: function(chunk, encoding, callback) { var value = chunk.toString(); events.push('start:' + value); setTimeout(function() { events.push('end:' + value); callback(); }, 1); } }); sink.on('drain', function() { events.push('drain'); }); sink.on('finish', function() { events.push('finish'); }); var first = sink.write('a', function() { events.push('callback:a'); }), second = sink.write('bb', function() { events.push('callback:bb'); }), completion = new Promise(function(resolve, reject) { sink.on('error', reject); sink.on('finish', resolve); }); sink.end('c', function() { events.push('end-callback'); }); await completion; var transformed = [], transform = new stream.Transform({ highWaterMark: 2, transform: function(chunk, encoding, callback) { setTimeout(function() { callback(null, chunk.toString().toUpperCase()); }, 1); } }); transform.on('data', function(chunk) { transformed.push(chunk.toString()); }); var transformFirst = transform.write('x'), transformSecond = transform.write('y'), transformCompletion = new Promise(function(resolve, reject) { transform.on('error', reject); transform.on('finish', resolve); }); transform.end('z'); await transformCompletion; return [first, second, sink.writableLength, sink.writableFinished, events, transformFirst, transformSecond, transformed.join(''), transform.readableEnded]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_stream_backpressure_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamBackpressure = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseStreamBackpressure").unwrap().as_ptr(),
            CString::new("[]").unwrap().as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,false,0,true,["start:a","end:a","callback:a","start:bb","end:bb","callback:bb","drain","start:c","end:c","finish","end-callback"],true,false,"XYZ",true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn diagnostics_channels_publish_bind_stores_and_trace() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_diagnostics_channel");
        fs::write(
            dir.join("index.js"),
            "var diagnostics = require('node:diagnostics_channel');\n\
             module.exports = async function () {\n\
             \x20 var events = []; var work = diagnostics.channel('work'); var same = work === diagnostics.channel('work');\n\
             \x20 function subscriber(data, name) { events.push(name + ':' + data.value); } diagnostics.subscribe('work', subscriber); diagnostics.subscribe('work', subscriber);\n\
             \x20 var store = { run: function(value, callback) { events.push('store:' + value); return callback(); } }; work.bindStore(store, function(data) { return data.value * 2; });\n\
             \x20 var storeResult = work.runStores({ value: 2 }, function(left, right) { return left + right; }, null, 3, 4); work.publish({ value: 5 }); var removed = diagnostics.unsubscribe('work', subscriber); work.publish({ value: 6 }); work.unbindStore(store);\n\
             \x20 var trace = diagnostics.tracingChannel('operation'); ['start', 'end', 'asyncStart', 'asyncEnd', 'error'].forEach(function(name) { trace[name].subscribe(function(context) { events.push(name + ':' + (context.result || context.error && context.error.message || '')); }); });\n\
             \x20 var sync = trace.traceSync(function(value) { return value + 1; }, {}, null, 4); var promised = await trace.tracePromise(async function(value) { return value * 2; }, {}, null, 3); var failed; try { trace.traceSync(function() { throw new Error('bad'); }, {}); } catch (error) { failed = error.message; }\n\
             \x20 return [same, storeResult, removed, diagnostics.hasSubscribers('work'), sync, promised, failed, events];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_diagnostics_channel_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseDiagnostics = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseDiagnostics").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,7,true,false,5,6,"bad",["store:4","store:10","work:5","store:12","start:","end:5","start:","asyncStart:6","asyncEnd:6","end:6","start:","error:bad","end:bad"]]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn async_hooks_preserve_storage_and_resource_scope() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_async_hooks");
        fs::write(
            dir.join("index.js"),
            "var hooks = require('node:async_hooks');\n\
             module.exports = async function () {\n\
             \x20 var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default', name: 'request' }); var events = [storage.getStore()]; var bound; var snapshot;\n\
             \x20 var result = await storage.run('outer', async function(value) { events.push(storage.getStore() + ':' + value); bound = hooks.AsyncLocalStorage.bind(function(suffix) { return storage.getStore() + suffix; }); snapshot = hooks.AsyncLocalStorage.snapshot(); await Promise.resolve(); events.push(storage.getStore()); var nested = storage.run('inner', function() { return storage.getStore(); }); events.push(nested + ':' + storage.getStore()); var exited = storage.exit(function() { return storage.getStore(); }); events.push(String(exited) + ':' + storage.getStore()); return 'done'; }, 4);\n\
             \x20 storage.enterWith('changed'); var rebound = bound('!'); var snapped = snapshot(function() { return storage.getStore(); });\n\
             \x20 var resource = new hooks.AsyncResource('work'); var outside = hooks.executionAsyncId(); var inside = resource.runInAsyncScope(function(left, right) { return [hooks.executionAsyncId(), hooks.triggerAsyncId(), hooks.executionAsyncResource() === resource, left + right]; }, null, 2, 3); var reboundResource = resource.bind(function() { return hooks.executionAsyncId(); })(); resource.emitDestroy();\n\
             \x20 storage.disable(); return [events, result, rebound, snapped, storage.getStore() === undefined, outside, inside, reboundResource, resource.asyncId(), resource.triggerAsyncId(), resource._destroyed, hooks.createHook({}).enable().disable().callbacks];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_async_hooks_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAsyncHooks = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseAsyncHooks").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["default","outer:4","outer","inner:outer","undefined:outer"],"done","outer!","outer",true,1,[2,1,true,5],2,2,1,true,{}]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn tty_builtin_reports_capabilities_and_emits_ansi_sequences() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_tty");
        fs::write(
            dir.join("index.js"),
            "var tty = require('node:tty');\n\
             module.exports = async function () {\n\
             \x20 var input = new tty.ReadStream(0); var same = input.setRawMode(true) === input; input.setRawMode(false);\n\
             \x20 var output = new tty.WriteStream(1); var callbacks = []; output.write('text'); output.cursorTo(2, 3, function() { callbacks.push('cursor'); }); output.moveCursor(-1, 2, function() { callbacks.push('move'); }); output.clearLine(0); output.clearScreenDown(); await Promise.resolve();\n\
             \x20 return [tty.isatty(1), input.isTTY, input.isRaw, same, output.getWindowSize(), output.getColorDepth({ FORCE_COLOR: '3' }), output.getColorDepth({ TERM: 'xterm-256color' }), output.hasColors(256, { TERM: 'xterm-256color' }), output.hasColors(16, {}), output._output, callbacks.sort().join(',')];\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_tty_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTty = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseTty").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            "[false,true,false,true,[80,24],24,8,true,false,\"text\\u001b[4;3H\\u001b[1D\\u001b[2B\\u001b[2K\\u001b[0J\",\"cursor,move\"]"
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn module_create_require_loads_relative_and_builtin_dependencies() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_module_create_require");
        fs::write(
            dir.join("index.js"),
            "import { createRequire, isBuiltin, builtinModules, registerHooks } from 'node:module';\n\
             const localRequire = createRequire('pkg/index.js');\n\
             export default function () {\n\
             \x20 const dependency = localRequire('./dependency'); const path = localRequire('node:path'); const hooks = registerHooks({}); hooks.deregister();\n\
             \x20 return [dependency.value, path.basename('/tmp/file.txt'), localRequire.resolve('./dependency'), Object.keys(localRequire.cache).length >= 3, isBuiltin('node:path'), isBuiltin('missing'), builtinModules.includes('stream'), hooks.active];\n\
             }",
        )
        .unwrap();
        fs::write(dir.join("dependency.js"), "module.exports = { value: 42 };").unwrap();
        let empty_node_modules = temp_registry("builtin_module_create_require_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseModule = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseModule").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[42,"file.txt","pkg/dependency.js",true,true,false,true,false]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn console_builtin_shares_global_console_and_constructor() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_console");
        fs::write(
            dir.join("index.js"),
            "var consoleModule = require('node:console');\n\
             module.exports = function () { var output = []; var instance = new consoleModule.Console({ write: function(value) { output.push(value); } }); instance.log('%s:%d', 'value', 2); instance.warn({ ok: true }); return [consoleModule === globalThis.console, consoleModule.console === globalThis.console, output.join('')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_console_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseConsole = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseConsole").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[true,true,"value:2\n{\"ok\":true}\n"]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn crypto_builtin_shares_native_hash_and_random_implementations() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_crypto");
        fs::write(
            dir.join("index.js"),
            "var crypto = require('node:crypto');\n\
             module.exports = async function () { var callbackLength = await new Promise(function(resolve, reject) { crypto.randomBytes(7, function(error, value) { if (error) reject(error); else resolve(value.length); }); }); var uuid = crypto.randomUUID(); return [crypto === globalThis.__thaw_crypto_module, crypto.createHash('sha256').update('abc').digest('hex'), crypto.createHmac('sha512', 'key').update('value').digest().length, callbackLength, /^[0-9a-f-]{36}$/.test(uuid), crypto.webcrypto === globalThis.crypto, crypto.timingSafeEqual(Buffer.from('x'), Buffer.from('x'))]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_crypto_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseCrypto = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseCrypto").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",64,7,true,true,true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn perf_hooks_builtin_shares_the_performance_timeline() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_perf_hooks");
        fs::write(
            dir.join("index.js"),
            "var hooks = require('node:perf_hooks');\n\
             module.exports = function () { hooks.performance.clearMarks(); hooks.performance.clearMeasures(); hooks.performance.mark('start', { startTime: 2 }); hooks.performance.mark('end', { startTime: 7 }); var measure = hooks.performance.measure('elapsed', 'start', 'end'); var histogram = hooks.monitorEventLoopDelay({ resolution: 10 }); var enabled = histogram.enable(); var disabled = histogram.disable(); var utilization = hooks.performance.eventLoopUtilization(); return [hooks.performance === globalThis.performance, measure.duration, hooks.PerformanceObserver === globalThis.PerformanceObserver, enabled, disabled, histogram.percentile(99), typeof histogram.percentileBigInt(99), utilization.idle, utilization.active >= 0, utilization.utilization >= 0, hooks.performance.nodeTiming.name, hooks.constants.NODE_PERFORMANCE_GC_MAJOR]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_perf_hooks_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePerformanceHooks = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exercisePerformanceHooks").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,5,true,true,true,0,"bigint",0,true,true,"node",4]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn v8_builtin_serializes_graphs_and_exposes_runtime_statistics() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_v8");
        fs::write(
            dir.join("index.js"),
            "var v8 = require('node:v8');\n\
             module.exports = function () { var source = { bigint: 42n, bytes: Buffer.from('thaw'), map: new Map([['answer', 42]]), set: new Set(['x']), missing: undefined }; source.self = source; var encoded = v8.serialize(source); var copy = v8.deserialize(encoded); var heap = v8.getHeapStatistics(); var code = v8.getHeapCodeStatistics(); return [Buffer.isBuffer(encoded), copy !== source, copy.self === copy, copy.bigint === 42n, copy.bytes.toString(), copy.map.get('answer'), copy.set.has('x'), Object.prototype.hasOwnProperty.call(copy, 'missing'), copy.missing === undefined, heap.number_of_native_contexts, heap.heap_size_limit > 0, v8.getHeapSpaceStatistics().length, code.code_and_metadata_size, v8.cachedDataVersionTag()]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_v8_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseV8 = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let func = CString::new("exerciseV8").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,true,true,true,"thaw",42,true,true,true,1,true,0,0,0]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn zlib_builtin_compresses_sync_and_callback_values() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_zlib");
        fs::write(dir.join("index.js"), "var zlib = require('node:zlib'), stream = require('node:stream'); module.exports = async function () { var source = Buffer.from('thaw compression '.repeat(8)); var gzip = zlib.gzipSync(source); var raw = zlib.deflateRawSync(source); var brotli = zlib.brotliCompressSync(source); var callback = await new Promise(function(resolve, reject) { zlib.gunzip(gzip, function(error, value) { error ? reject(error) : resolve(value); }); }); var brotliCallback = await new Promise(function(resolve, reject) { zlib.brotliDecompress(brotli, function(error, value) { error ? reject(error) : resolve(value); }); }); var compressor = zlib.createGzip(), compressed = []; compressor.on('data', function(value) { compressed.push(value); }); var streamDone = new Promise(function(resolve, reject) { compressor.on('error', reject); compressor.on('end', resolve); }); compressor.write(source.subarray(0, 7)); compressor.end(source.subarray(7)); await streamDone; var streamed = Buffer.concat(compressed), decompressor = zlib.createGunzip(), restored = []; decompressor.on('data', function(value) { restored.push(value); }); var restoreDone = new Promise(function(resolve, reject) { decompressor.on('error', reject); decompressor.on('end', resolve); }); stream.Readable.from([streamed.subarray(0, 5), streamed.subarray(5)]).pipe(decompressor); await restoreDone; var brotliStream = zlib.createBrotliCompress(), brotliChunks = []; brotliStream.on('data', function(value) { brotliChunks.push(value); }); var brotliDone = new Promise(function(resolve, reject) { brotliStream.on('error', reject); brotliStream.on('end', resolve); }); brotliStream.end(source); await brotliDone; var flushed = await new Promise(function(resolve) { compressor.flush(resolve); }); return [gzip[0], gzip[1], zlib.gunzipSync(gzip).toString() === source.toString(), zlib.inflateRawSync(raw).toString() === source.toString(), callback.toString() === source.toString(), zlib.constants.Z_OK, zlib.constants.Z_SYNC_FLUSH, compressor instanceof zlib.Gzip, compressor instanceof stream.Transform, compressor.bytesWritten, Buffer.concat(restored).toString() === source.toString(), flushed, zlib.brotliDecompressSync(brotli).toString() === source.toString(), brotliCallback.toString() === source.toString(), brotliStream instanceof zlib.BrotliCompress, zlib.brotliDecompressSync(Buffer.concat(brotliChunks)).toString() === source.toString(), zlib.constants.BROTLI_OPERATION_FINISH]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_zlib_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseZlib = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseZlib").unwrap().as_ptr(),
            CString::new("[]").unwrap().as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[31,139,true,true,true,0,2,true,true,136,true,null,true,true,true,true,2]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn worker_threads_builtin_exchanges_cloned_messages() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_threads");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var original = { value: 7 }; channel.port1.postMessage(original); original.value = 9; var received = workers.receiveMessageOnPort(channel.port2); var pending = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port1.postMessage(new Map([['answer', 42]])); var asynchronous = await pending; workers.setEnvironmentData('config', { enabled: true }); var environment = workers.getEnvironmentData('config'); environment.enabled = false; var freshEnvironment = workers.getEnvironmentData('config'); var protectedBuffer = new ArrayBuffer(4); workers.markAsUntransferable(protectedBuffer); var transferRejected = false; try { channel.port1.postMessage('x', [protectedBuffer]); } catch (error) { transferRejected = error.name === 'DataCloneError'; } var protectedObject = { value: 1 }; workers.markAsUncloneable(protectedObject); var cloneRejected = false; try { structuredClone({ nested: protectedObject }); } catch (error) { cloneRejected = error.name === 'DataCloneError'; } var extra = new workers.MessageChannel(); channel.port1.postMessage({ port: extra.port1 }, [extra.port1]); var transferred = workers.receiveMessageOnPort(channel.port2); transferred.message.port.postMessage('through'); var through = workers.receiveMessageOnPort(extra.port2).message; var broadcastSender = new workers.BroadcastChannel('sync-receive'), broadcastReceiver = new workers.BroadcastChannel('sync-receive'); broadcastSender.postMessage({ value: 5 }); var broadcast = workers.receiveMessageOnPort(broadcastReceiver); broadcastSender.close(); broadcastReceiver.close(); extra.port1.close(); extra.port2.close(); channel.port1.unref(); var refed = channel.port1.hasRef(); channel.port1.ref(); channel.port1.close(); channel.port2.close(); return [workers.isMainThread, workers.threadId, workers.parentPort, received.message.value, asynchronous.get('answer'), freshEnvironment.enabled, refed, channel.port1.hasRef(), workers.SHARE_ENV === Symbol.for('nodejs.worker_threads.SHARE_ENV'), workers.receiveMessageOnPort(channel.port2) === undefined, workers.isMarkedAsUntransferable(protectedBuffer), transferRejected, cloneRejected, broadcast.message.value, through]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_worker_threads_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkers = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseWorkers").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,0,null,7,42,true,false,true,true,true,true,true,true,5,"through"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn worker_threads_eval_worker_isolates_state_and_exchanges_messages() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_eval");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var events = []; delete process.env.THAW_WORKER_TEST; var source = \"var wt = require('node:worker_threads'); globalThis.workerOnly = 99; wt.parentPort.on('message', function(value) { var args = process.argv.slice(-2).join(','); var environment = process.env.THAW_WORKER_TEST; process.env.THAW_WORKER_TEST = 'mutated'; wt.parentPort.postMessage({ answer: wt.workerData.base + value, main: wt.isMainThread, threadId: wt.threadId, threadName: wt.threadName, isolated: globalThis.workerOnly, args: args, execArgv: process.execArgv.join(','), environment: environment, native: typeof __thaw_worker_spawn }); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, name: 'alpha', workerData: { base: 40 }, argv: ['one', 2], execArgv: ['--trace-warnings'], env: { THAW_WORKER_TEST: 'child' }, resourceLimits: { maxOldGenerationSizeMb: 64 } }); var listenerOrder = []; function regular() { listenerOrder.push('regular'); } worker.addListener('probe', regular).prependOnceListener('probe', function() { listenerOrder.push('first'); }); var listenerCount = worker.listenerCount('probe'); var hasProbe = worker.eventNames().indexOf('probe') >= 0; worker.emit('probe'); worker.emit('probe'); worker.removeListener('probe', regular).setMaxListeners(20); var completed = new Promise(function(resolve) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.answer + ':' + value.main + ':' + (value.threadId > 0) + ':' + value.threadName + ':' + value.isolated + ':' + value.args + ':' + value.execArgv + ':' + value.environment + ':' + value.native); }); worker.on('error', function(error) { events.push('error:' + error.stack); resolve(); }); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); worker.postMessage(2); await completed; delete process.env.THAW_WORKER_SHARED; var shared = new workers.Worker(\"var wt = require('node:worker_threads'); process.env.THAW_WORKER_SHARED = 'shared'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); await new Promise(function(resolve, reject) { shared.on('error', reject); shared.on('exit', resolve); }); var sharedValue = process.env.THAW_WORKER_SHARED; delete process.env.THAW_WORKER_SHARED; return [events, worker.threadName, listenerOrder, listenerCount, hasProbe, worker.listenerCount('probe'), worker.getMaxListeners(), typeof globalThis.workerOnly, process.env.THAW_WORKER_TEST, sharedValue, worker.resourceLimits.maxOldGenerationSizeMb, worker.threadId > 0, worker.ref() === worker, worker.unref() === worker, await worker.terminate()]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_worker_eval_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorker = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseWorker").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["online","message:42:false:true:alpha:99:one,2:--trace-warnings:child:function","exit:0"],"alpha",["first","regular","regular"],2,true,0,20,"undefined",null,"shared",64,true,true,true,0]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn worker_threads_share_environment_across_native_workers() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_native_share_env");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } function result(worker) { return new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); } module.exports = async function () { delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; var first = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() { var parent = process.env.THAW_PARENT_SHARED; process.env.THAW_WORKER_SHARED = 'worker'; wt.parentPort.postMessage(parent); wt.parentPort.close(); });\", { eval: true, env: workers.SHARE_ENV }); await online(first); process.env.THAW_PARENT_SHARED = 'parent'; var firstResult = result(first), firstExit = new Promise(function(resolve) { first.on('exit', resolve); }); first.postMessage('go'); var parentSeen = await firstResult; await firstExit; var second = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.postMessage(process.env.THAW_WORKER_SHARED); process.env.THAW_SECOND_SHARED = 'second'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); var secondResult = result(second), secondExit = new Promise(function(resolve) { second.on('exit', resolve); }); var workerSeen = await secondResult; await secondExit; var secondSeen = process.env.THAW_SECOND_SHARED, native = [typeof first._nativeHandle, typeof second._nativeHandle]; delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; return [parentSeen, workerSeen, secondSeen, native]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_native_share_env_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeShareEnv = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseNativeShareEnv").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["parent","worker","second",["number","number"]]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_moves_message_ports_to_vm_contexts() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_move_port_context");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); var vm = require('node:vm'); module.exports = function () { var channel = new workers.MessageChannel(); var context = vm.createContext({}); var moved = workers.moveMessagePortToContext(channel.port1, context); moved.postMessage('moved'); var received = workers.receiveMessageOnPort(channel.port2); var invalidContext = false; try { workers.moveMessagePortToContext(channel.port2, {}); } catch (error) { invalidContext = error instanceof TypeError; } return [received.message, channel.port1.__thawClosed, moved !== channel.port1, invalidContext]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_move_port_context_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseMovePortToContext = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseMovePortToContext").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, r#"["moved",true,true,true]"#);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_transfers_worker_data_message_ports() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_data_transfer");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var source = \"var wt = require('node:worker_threads'); wt.workerData.port.postMessage('from-worker'); wt.workerData.port.on('message', function(value) { wt.workerData.port.postMessage(value.toUpperCase()); wt.workerData.port.close(); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, workerData: { port: channel.port1 }, transferList: [channel.port1] }); var first = new Promise(function(resolve) { channel.port2.once('message', resolve); }); var detached = channel.port1.__thawClosed && channel.port1.__thawPeer === null; var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var initial = await first, second = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port2.postMessage('through'); return [initial, await second, await exit, detached, typeof worker._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_data_transfer_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDataTransfer = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseWorkerDataTransfer").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, r#"["from-worker","THROUGH",0,true,"number"]"#);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_transfer_message_ports_from_native_workers() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_native_outbound_port");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'), channel = new wt.MessageChannel(); channel.port2.on('message', function(value) { channel.port2.postMessage(value.toUpperCase()); channel.port2.close(); wt.parentPort.close(); }); wt.parentPort.postMessage({ port: channel.port1 }, [channel.port1]);\"; var worker = new Worker(source, { eval: true }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var transferred = await new Promise(function(resolve) { worker.once('message', function(value) { resolve(value.port); }); }); var response = new Promise(function(resolve) { transferred.once('message', resolve); }); transferred.postMessage('worker-port'); var value = await response, code = await exit; transferred.close(); return [value, code, typeof worker._nativeHandle, transferred.__thawHostPortId.startsWith('w:')]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_native_outbound_port_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeOutboundPort = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseNativeOutboundPort").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, r#"["WORKER-PORT",0,"number",true]"#);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_redirects_stdin_stdout_and_stderr() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_stdio");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); process.stdin.setEncoding('utf8'); process.stdin.on('data', function(value) { console.log('stdout', value); console.error('stderr', value); wt.parentPort.postMessage(value.toUpperCase()); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, stdin: true, stdout: true, stderr: true }); worker.stdout.setEncoding('utf8'); worker.stderr.setEncoding('utf8'); var output = '', errors = '', message; worker.stdout.on('data', function(value) { output += value; }); worker.stderr.on('data', function(value) { errors += value; }); worker.on('message', function(value) { message = value; }); var exited = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); worker.stdin.end('hello'); var code = await exited; return [message, output, errors, code, worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded, typeof worker._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_stdio_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerStdio = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseWorkerStdio").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["HELLO","stdout hello\n","stderr hello\n",0,true,true,true,"number"]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_terminate_resolves_with_one_shared_exit_code() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_terminate");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); var exits = []; worker.on('exit', function(code) { exits.push(code); }); var first = worker.terminate(); var second = worker.terminate(); var code = await first; return [first === second, code, await second, await worker.terminate(), exits, Symbol.asyncDispose ? worker[Symbol.asyncDispose] === worker.terminate : true]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_terminate_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerTerminate = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseWorkerTerminate").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, "[true,1,1,1,[1],true]");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_diagnostics_follow_running_state() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_diagnostics");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var cpu = await worker.cpuUsage(); var heap = await worker.getHeapStatistics(); var snapshot = await worker.getHeapSnapshot(); snapshot.setEncoding('utf8'); var text = ''; await new Promise(function(resolve, reject) { snapshot.on('data', function(chunk) { text += chunk; }); snapshot.on('end', resolve); snapshot.on('error', reject); }); var profile = await worker.startCpuProfile(); var stopped = await profile.stop(); await worker.terminate(); var stoppedError; try { await worker.cpuUsage(); } catch (error) { stoppedError = error.code; } return [cpu, heap.number_of_native_contexts, JSON.parse(text).snapshot.node_count, stopped.nodes.length, stopped.samples.length, stoppedError]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_diagnostics_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDiagnostics = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseWorkerDiagnostics").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[{"user":0,"system":0},1,0,0,0,"ERR_WORKER_NOT_RUNNING"]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_routes_messages_by_thread_id() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_direct_messages");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var fromWorker = []; function mainListener(value, source) { fromWorker.push([value.kind, source]); } process.on('workerMessage', mainListener); var source = \"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage(['direct:' + value.kind, source]); wt.parentPort.close(); }); wt.postMessageToThread(0, { kind: 'hello' }).then(function() { wt.parentPort.postMessage(['ready', wt.threadId]); });\"; var worker = new workers.Worker(source, { eval: true }); var messages = []; var direct; var completed = new Promise(function(resolve, reject) { worker.on('message', function(value) { messages.push(value); if (value[0] === 'ready') workers.postMessageToThread(worker.threadId, { kind: 'ping' }).catch(reject); else direct = value; }); worker.on('error', reject); worker.on('exit', resolve); }); var exitCode = await completed; process.off('workerMessage', mainListener); var same, missing; try { await workers.postMessageToThread(0, 'x'); } catch (error) { same = error.code; } try { await workers.postMessageToThread(999999, 'x'); } catch (error) { missing = error.code; } var silent = new workers.Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await online(silent); var noListener; try { await workers.postMessageToThread(silent.threadId, 'x'); } catch (error) { noListener = error.code; } await silent.terminate(); var throwing = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function() { throw new Error('listener failed'); });\", { eval: true }); await online(throwing); var listenerError; try { await workers.postMessageToThread(throwing.threadId, 'x'); } catch (error) { listenerError = [error.code, error.cause.message]; } var native = [typeof worker._nativeHandle, typeof silent._nativeHandle, typeof throwing._nativeHandle]; await throwing.terminate(); return [fromWorker, direct, exitCode, same, missing, noListener, listenerError, native]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_direct_messages_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDirectWorkerMessages = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseDirectWorkerMessages").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[[["hello",1]],["direct:ping",0],0,"ERR_WORKER_MESSAGING_SAME_THREAD","ERR_WORKER_MESSAGING_FAILED","ERR_WORKER_MESSAGING_FAILED",["ERR_WORKER_MESSAGING_ERRORED","listener failed"],["number","number","number"]]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_route_direct_messages_between_native_workers() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_direct_between_workers");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var receiver = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage([value.answer, source]); wt.parentPort.close(); });\", { eval: true }); await online(receiver); var sender = new workers.Worker(\"var wt = require('node:worker_threads'); wt.postMessageToThread(wt.workerData.target, { answer: 42 }).then(function() { wt.parentPort.postMessage('sent'); wt.parentPort.close(); });\", { eval: true, workerData: { target: receiver.threadId } }); var received = new Promise(function(resolve, reject) { receiver.on('message', resolve); receiver.on('error', reject); }); var sent = new Promise(function(resolve, reject) { sender.on('message', resolve); sender.on('error', reject); }); var receiverExit = new Promise(function(resolve) { receiver.on('exit', resolve); }), senderExit = new Promise(function(resolve) { sender.on('exit', resolve); }); var values = await Promise.all([received, sent, receiverExit, senderExit]); return [values, typeof receiver._nativeHandle, typeof sender._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_direct_between_workers_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRouting = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseNativeWorkerRouting").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, r#"[[[42,2],"sent",0,0],"number","number"]"#);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_data_url_worker_decodes_javascript() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_data_url");
        fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var parentThread = globalThis.__thaw_os_thread_token; var source = \"var wt = require('node:worker_threads'); wt.parentPort.postMessage({ value: wt.workerData.value, main: wt.isMainThread, native: typeof __thaw_worker_spawn, thread: globalThis.__thaw_os_thread_token }); wt.parentPort.close();\"; var worker = new workers.Worker('data:text/javascript,' + encodeURIComponent(source), { workerData: { value: 17 } }); var events = []; await new Promise(function(resolve, reject) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.value + ':' + value.main + ':' + value.native + ':' + (value.thread !== parentThread)); }); worker.on('error', reject); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); var rejected = false; try { new workers.Worker('data:text/plain,not-javascript'); } catch (error) { rejected = error instanceof TypeError; } return [events, rejected]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_worker_data_url_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDataWorker = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseDataWorker").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["online","message:17:false:function:true","exit:0"],true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn worker_threads_load_runtime_computed_absolute_paths() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_runtime_path");
        let worker_dir = dir.join("workers/nested");
        fs::create_dir_all(&worker_dir).unwrap();
        fs::create_dir_all(dir.join("internal/features")).unwrap();
        fs::write(
            dir.join("package.json"),
            r##"{"imports":{"#worker-tool":{"node":"./internal/tool.cjs","default":"./internal/wrong.cjs"},"#features/*":{"require":"./internal/features/*.js"},"#escape":"../outside.js"}}"##,
        )
        .unwrap();
        fs::write(dir.join("internal/tool.cjs"), "module.exports = 11;").unwrap();
        fs::write(
            dir.join("internal/features/math.js"),
            "module.exports = 12;",
        )
        .unwrap();
        let worker_path = worker_dir.join("dynamic-worker.js");
        fs::write(
            worker_dir.join("worker-value.js"),
            "module.exports = require('./worker-package') + 1;",
        )
        .unwrap();
        fs::create_dir_all(worker_dir.join("worker-package/lib")).unwrap();
        fs::write(
            worker_dir.join("worker-package/package.json"),
            r#"{"main":"lib/value"}"#,
        )
        .unwrap();
        fs::write(
            worker_dir.join("worker-package/lib/value.cjs"),
            "module.exports = require('../worker-data.json').value;",
        )
        .unwrap();
        fs::write(
            worker_dir.join("worker-package/worker-data.json"),
            r#"{"value":41}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency")).unwrap();
        fs::write(
            dir.join("node_modules/runtime-worker-dependency/package.json"),
            r#"{"main":"wrong.cjs","exports":{".":{"node":"./entry.cjs","default":"./wrong.cjs"},"./feature":{"require":"./feature.js"},"./features/*":{"require":"./dist/*.cjs"},"./escape":"../outside.js"}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("node_modules/runtime-worker-dependency/entry.cjs"),
            "module.exports = 8;",
        )
        .unwrap();
        fs::write(
            dir.join("node_modules/runtime-worker-dependency/feature.js"),
            "module.exports = 9;",
        )
        .unwrap();
        fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency/dist")).unwrap();
        fs::write(
            dir.join("node_modules/runtime-worker-dependency/dist/math.cjs"),
            "module.exports = 10;",
        )
        .unwrap();
        fs::write(
            &worker_path,
            "var wt = require('node:worker_threads'); var hiddenCode; try { require('runtime-worker-dependency/hidden'); } catch (error) { hiddenCode = error.code; } var missingImportCode; try { require('#missing'); } catch (error) { missingImportCode = error.code; } var exportEscapeCode; try { require('runtime-worker-dependency/escape'); } catch (error) { exportEscapeCode = error.code; } var importEscapeCode; try { require('#escape'); } catch (error) { importEscapeCode = error.code; } wt.parentPort.postMessage([wt.workerData, __filename, __dirname, typeof __thaw_worker_spawn, require('./worker-value'), require('runtime-worker-dependency'), require('runtime-worker-dependency/feature'), require('runtime-worker-dependency/features/math'), hiddenCode, require('#worker-tool'), require('#features/math'), missingImportCode, exportEscapeCode, importEscapeCode]); wt.parentPort.close();",
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function (runtimePath) { var worker = new Worker(runtimePath, { workerData: 23 }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var message = await new Promise(function(resolve) { worker.on('message', resolve); }); return [message, await exit, typeof worker._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_runtime_path_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRuntimeWorkerPath = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseRuntimeWorkerPath").unwrap();
        let path = worker_path.to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&vec![&path]).unwrap()).unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        let expected = serde_json::json!([
            [
                23,
                path,
                worker_dir.to_string_lossy(),
                "function",
                42,
                8,
                9,
                10,
                "ERR_PACKAGE_PATH_NOT_EXPORTED",
                11,
                12,
                "ERR_PACKAGE_IMPORT_NOT_DEFINED",
                "ERR_INVALID_PACKAGE_TARGET",
                "ERR_INVALID_PACKAGE_TARGET"
            ],
            0,
            "number"
        ])
        .to_string();
        assert_eq!(result, expected);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_native_runtime_round_trips_parent_messages() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_native_round_trip");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage([value * wt.workerData, globalThis.__thaw_os_thread_token]); wt.parentPort.close(); });\"; var parentThread = globalThis.__thaw_os_thread_token; var worker = new Worker(source, { eval: true, workerData: 7 }); var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var result = await new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); worker.postMessage(6); }); var code = await exit; return [result[0], result[1] !== parentThread, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_native_round_trip_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRoundTrip = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseNativeWorkerRoundTrip").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(result, r#"[42,true,0,"number"]"#);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn worker_threads_native_runtime_transfers_structured_array_buffers() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_native_structured_transfer");
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = new ArrayBuffer(3), second = new ArrayBuffer(2); new Uint8Array(first).set([1, 2, 3]); new Uint8Array(second).set([4, 5]); var data = { buffer: first, map: new Map([['answer', 42]]) }; data.self = data; var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { var reply = { first: Array.from(new Uint8Array(wt.workerData.buffer)), second: Array.from(new Uint8Array(value.buffer)), answer: wt.workerData.map.get('answer'), workerCycle: wt.workerData.self === wt.workerData, messageCycle: value.self === value }; reply.self = reply; wt.parentPort.postMessage(reply); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, workerData: data, transferList: [first] }); var message = { buffer: second }; message.self = message; var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var reply = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); worker.postMessage(message, [second]); var value = await reply, code = await exit; return [first.byteLength, second.byteLength, value.first, value.second, value.answer, value.workerCycle, value.messageCycle, value.self === value, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
        let node_modules = temp_registry("builtin_worker_native_structured_transfer_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerTransfer = module.exports;")).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
        let function = CString::new("exerciseNativeWorkerTransfer").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[0,0,[1,2,3],[4,5],42,true,true,true,0,"number"]"#
        );
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
    }

    #[test]
    fn bundler_embeds_static_file_url_worker_sources() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_worker_file_url");
        fs::write(
            dir.join("double.js"),
            "module.exports = function(value) { return value * 2; };",
        )
        .unwrap();
        fs::write(
            dir.join("worker.js"),
            "import { parentPort, workerData } from 'node:worker_threads'; var double = require('./double'); parentPort.postMessage(double(workerData)); parentPort.close();",
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); const workerFile = './' + 'worker.js'; const joinedWorkerFile = path.join(__dirname, 'worker.js'); function run(worker, events) { return new Promise(function(resolve, reject) { worker.on('message', function(value) { events.push(value); }); worker.on('error', reject); worker.on('exit', function(code) { events.push(code); resolve(); }); }); } module.exports = async function () { var events = []; await run(new Worker(new URL('./worker.js', import.meta.url), { workerData: 21 }), events); await run(new Worker('./worker.js', { workerData: 11 }), events); await run(new Worker(`./${'worker'}.js`, { workerData: 5 }), events); await run(new Worker(workerFile, { workerData: 3 }), events); await run(new Worker(joinedWorkerFile, { workerData: 2 }), events); return events; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_worker_file_url_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 6);
        fs::remove_dir_all(&dir).unwrap();
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFileWorker = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseFileWorker").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, "[42,0,22,0,10,0,6,0,4,0]");
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn worker_url_rewrite_ignores_unrelated_worker_bindings() {
        let dir = temp_registry("unrelated_worker_url");
        let source = "class Worker {}\nnew Worker(new URL('./missing.js', import.meta.url));";
        let rewritten = rewrite_static_worker_urls(source, &dir.join("index.js"), "pkg", &dir)
            .expect("an unrelated Worker must not attempt to read its URL");
        assert_eq!(rewritten, source);
        let eval_source = "var Worker = require('node:worker_threads').Worker; new Worker('./missing.js', { eval: true });";
        let rewritten = rewrite_static_worker_urls(eval_source, &dir.join("index.js"), "pkg", &dir)
            .expect("eval Worker source must not be treated as a file");
        assert_eq!(rewritten, eval_source);
        let shadowed_source = "var Worker = require('node:worker_threads').Worker; const workerFile = './missing.js'; function start(workerFile) { return new Worker(workerFile); }";
        let rewritten =
            rewrite_static_worker_urls(shadowed_source, &dir.join("index.js"), "pkg", &dir)
                .expect("a shadowed constant Worker path must remain dynamic");
        assert_eq!(rewritten, shadowed_source);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn worker_threads_broadcast_channel_clones_between_matching_names() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_broadcast_channel");
        fs::write(
            dir.join("index.js"),
            "var BroadcastChannel = require('node:worker_threads').BroadcastChannel; module.exports = async function () { var sender = new BroadcastChannel('room'); var first = new BroadcastChannel('room'); var second = new BroadcastChannel('room'); var isolated = new BroadcastChannel('elsewhere'); var source = { nested: { value: 4 } }; var firstMessage = new Promise(function(resolve) { first.onmessage = function(event) { event.data.nested.value = 8; resolve(event.data.nested.value); }; }); var secondMessage = new Promise(function(resolve) { second.addEventListener('message', function(event) { resolve(event.data.nested.value); }, { once: true }); }); var isolatedCalled = false; isolated.onmessage = function() { isolatedCalled = true; }; sender.postMessage(source); source.nested.value = 9; var values = await Promise.all([firstMessage, secondMessage]); first.close(); var closedError = false; try { first.postMessage('x'); } catch (error) { closedError = error.name === 'InvalidStateError'; } sender.close(); second.close(); isolated.close(); return [values[0], values[1], isolatedCalled, closedError, sender.name, sender.ref() === sender, sender.unref() === sender]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_broadcast_channel_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseBroadcast = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseBroadcast").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[8,4,false,true,"room",true,true]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_sync_and_promise_apis_operate_on_the_host_filesystem() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_promises");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); var promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/work', original = work + '/value.txt', renamed = work + '/renamed.txt'; fs.mkdirSync(work, { recursive: true }); await new Promise(function(resolve, reject) { fs.writeFile(original, 'one', function(error) { error ? reject(error) : resolve(); }); }); await new Promise(function(resolve, reject) { fs.appendFile(original, Buffer.from('two'), function(error) { error ? reject(error) : resolve(); }); }); var text = await promises.readFile(original, 'utf8'), callbackText = await new Promise(function(resolve, reject) { fs.readFile(original, 'utf8', function(error, value) { error ? reject(error) : resolve(value); }); }), stat = await new Promise(function(resolve, reject) { fs.stat(original, function(error, value) { error ? reject(error) : resolve(value); }); }), entries = fs.readdirSync(work, { withFileTypes: true }); await promises.rename(original, renamed); var renamedExists = fs.existsSync(renamed), missingCode; try { await promises.readFile(work + '/missing'); } catch (error) { missingCode = [error.code, error.path, error.syscall]; } await promises.unlink(renamed); await promises.rm(work, { recursive: true }); return [text, callbackText, stat.isFile(), stat.isDirectory(), stat.size, entries[0].name, entries[0].isFile(), renamedExists, fs.existsSync(renamed), fs.existsSync(work), missingCode]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_promises_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromises = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseFsPromises").unwrap();
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            format!(
                r#"["onetwo","onetwo",true,false,6,"value.txt",true,true,false,false,["ENOENT","{}/work/missing","read"]]"#,
                dir.to_string_lossy()
            )
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_copy_realpath_and_mkdtemp_work_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_paths");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', copied = root + '/copied.txt', prefix = root + '/temporary-'; fs.writeFileSync(source, 'copy-value'); fs.copyFileSync(source, copied); var copiedValue = fs.readFileSync(copied, 'utf8'), resolved = await promises.realpath(copied); var temporary = await new Promise(function(resolve, reject) { fs.mkdtemp(prefix, function(error, path) { error ? reject(error) : resolve(path); }); }); var callbackPath = await new Promise(function(resolve, reject) { fs.realpath.native(source, function(error, path) { error ? reject(error) : resolve(path); }); }); await promises.unlink(source); await promises.unlink(copied); await promises.rmdir(temporary); return [copiedValue, resolved.slice(-10), callbackPath.slice(-10), temporary.indexOf(prefix) === 0, fs.existsSync(temporary), typeof fs.realpathSync.native]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_paths_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPaths = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsPaths").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["copy-value","copied.txt","source.txt",true,false,"function"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_disposable_temporary_directories_remove_nested_contents_once() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_disposable_mkdtemp");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = fs.mkdtempDisposableSync(root + '/sync-'); fs.mkdirSync(sync.path + '/nested'); fs.writeFileSync(sync.path + '/nested/value.txt', 'sync'); var syncPath = sync.path, syncSymbol = Symbol.dispose ? sync[Symbol.dispose] === sync.remove : true; sync.remove(); sync.remove(); var disposable = await promises.mkdtempDisposable(root + '/async-', { encoding: 'buffer' }), asyncPath = disposable.path.toString(), asyncSymbol = Symbol.asyncDispose ? disposable[Symbol.asyncDispose] === disposable.remove : true; fs.mkdirSync(asyncPath + '/nested'); fs.writeFileSync(asyncPath + '/nested/value.txt', 'async'); var first = disposable.remove(), sameRemoval = first === disposable.remove(); await first; await disposable.remove(); return [syncPath.indexOf(root + '/sync-') === 0, fs.existsSync(syncPath), syncSymbol, Buffer.isBuffer(disposable.path), asyncPath.indexOf(root + '/async-') === 0, fs.existsSync(asyncPath), asyncSymbol, sameRemoval]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_disposable_mkdtemp_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDisposableMkdtemp = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseDisposableMkdtemp").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[true,false,true,true,true,false,true,true]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_open_as_blob_exposes_blob_reads_and_detects_file_changes() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_open_as_blob");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/blob.txt'; fs.writeFileSync(path, 'abcdef'); var blob = await fs.openAsBlob(path, { type: 'Text/Plain' }), slice = blob.slice(1, 4, 'APPLICATION/X'), bytes = Array.from(await blob.bytes()), buffer = Array.from(new Uint8Array(await blob.arrayBuffer())), reader = blob.stream().getReader(), streamed = await reader.read(); reader.releaseLock(); var missingCode; try { await fs.openAsBlob(root + '/missing'); } catch (error) { missingCode = error.code; } fs.writeFileSync(path, 'changed-value'); var changedName; try { await blob.text(); } catch (error) { changedName = error.name; } var sliceChangedName; try { await slice.text(); } catch (error) { sliceChangedName = error.name; } return [blob instanceof Blob, blob.size, blob.type, Buffer.from(bytes).toString(), buffer.length, Buffer.from(streamed.value).toString(), streamed.done, slice.size, slice.type, missingCode, changedName, sliceChangedName]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_open_as_blob_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOpenAsBlob = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseOpenAsBlob").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,6,"text/plain","abcdef",6,"abcdef",false,3,"application/x","ENOENT","NotReadableError","NotReadableError"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_cp_recursively_copies_directories_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_cp");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', asyncPath = root + '/async', callbackPath = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/nested/value.txt', 'copied'); fs.cpSync(source, sync, { recursive: true }); await promises.cp(source, asyncPath, { recursive: true }); await new Promise(function(resolve, reject) { fs.cp(source, callbackPath, { recursive: true }, function(error) { error ? reject(error) : resolve(); }); }); fs.writeFileSync(sync + '/nested/value.txt', 'kept'); fs.cpSync(source, sync, { recursive: true, force: false }); var existingCode; try { fs.cpSync(source, sync, { recursive: true, force: false, errorOnExist: true }); } catch (error) { existingCode = error.code; } return [fs.readFileSync(sync + '/nested/value.txt', 'utf8'), fs.readFileSync(asyncPath + '/nested/value.txt', 'utf8'), fs.readFileSync(callbackPath + '/nested/value.txt', 'utf8'), existingCode]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_cp_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCp = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsCp").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"["kept","copied","copied","EEXIST"]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_cp_honors_filters_links_and_timestamp_options() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_cp_options");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', promised = root + '/promised', callback = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/keep.txt', 'keep'); fs.writeFileSync(source + '/skip.txt', 'skip'); fs.writeFileSync(source + '/nested/value.txt', 'nested'); fs.symlinkSync('keep.txt', source + '/link'); fs.utimesSync(source + '/keep.txt', new Date(1000000), new Date(2000000)); var syncSeen = []; fs.cpSync(source, sync, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true, filter: function(src) { syncSeen.push(src); return !src.endsWith('/skip.txt'); } }); var asyncSeen = []; await promises.cp(source, promised, { recursive: true, dereference: true, filter: async function(src) { asyncSeen.push(src); await Promise.resolve(); return !src.endsWith('/nested'); } }); var callbackSeen = []; await new Promise(function(resolve, reject) { fs.cp(source, callback, { recursive: true, filter: function(src) { callbackSeen.push(src); return Promise.resolve(!src.endsWith('/skip.txt')); } }, function(error) { error ? reject(error) : resolve(); }); }); var directoryCode; try { fs.cpSync(source, root + '/not-recursive'); } catch (error) { directoryCode = error.code; } var asyncFilterError; try { fs.cpSync(source + '/keep.txt', root + '/invalid-filter', { filter: function() { return Promise.resolve(true); } }); } catch (error) { asyncFilterError = error.name; } return [fs.existsSync(sync + '/keep.txt'), fs.existsSync(sync + '/skip.txt'), fs.readlinkSync(sync + '/link'), Math.abs(fs.statSync(sync + '/keep.txt').mtimeMs - 2000000) < 2, syncSeen.length, fs.readFileSync(promised + '/link', 'utf8'), fs.existsSync(promised + '/nested'), asyncSeen.length, fs.existsSync(callback + '/keep.txt'), fs.existsSync(callback + '/skip.txt'), callbackSeen.length, directoryCode, asyncFilterError]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_cp_options_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCpOptions = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsCpOptions").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,false,"keep.txt",true,6,"keep",false,5,true,false,6,"ERR_FS_EISDIR","TypeError"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_promise_file_handles_manage_repeated_operations_and_streams() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_file_handle");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'), stream = require('node:stream'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'), originalFd = handle.fd; await handle.writeFile('abcdef'); await handle.appendFile('ghi'); var before = await handle.stat(); await handle.truncate(5); var value = await handle.readFile('utf8'), reader = handle.createReadStream({ highWaterMark: 2 }), chunks = []; reader.on('data', function(chunk) { chunks.push(chunk.toString()); }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await handle.close(); var closedCode; try { await handle.readFile(); } catch (error) { closedCode = error.code; } return [originalFd >= 10, before.size, value, chunks, reader instanceof fs.ReadStream, reader instanceof stream.Readable, handle.fd, handle.closed, closedCode, Symbol.asyncDispose ? typeof handle[Symbol.asyncDispose] : 'unavailable']; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_file_handle_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandle = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("handle.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsFileHandle").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,9,"abcde",["ab","cd","e"],true,true,-1,true,"EBADF","function"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_file_handles_support_positioned_buffer_reads_and_writes() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_file_handle_positions");
        fs::write(
            dir.join("index.js"),
            "var promises = require('node:fs/promises'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'); await handle.writeFile('abcdef'); var first = Buffer.alloc(4, 46), read = await handle.read(first, 1, 2, 2); var written = await handle.write(Buffer.from('XYZ'), 1, 2, 4); var textWrite = await handle.write('!', 1, 'utf8'); var sequential = Buffer.alloc(3), sequentialRead = await handle.read(sequential, 0, 3, null); var value = await handle.readFile('utf8'); await handle.close(); return [read.bytesRead, read.buffer.toString(), written.bytesWritten, written.buffer.toString(), textWrite.bytesWritten, textWrite.buffer, sequentialRead.bytesRead, sequential.toString(), value]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_file_handle_positions_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandlePositions = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("positioned.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsFileHandlePositions")
                .unwrap()
                .as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[2,".cd.",2,"XYZ",1,"!",3,"a!c","a!cdYZ"]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_directory_handles_support_reads_callbacks_and_async_iteration() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_directory_handles");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/entries'; fs.mkdirSync(work); fs.writeFileSync(work + '/a.txt', 'a'); fs.mkdirSync(work + '/nested'); var sync = fs.opendirSync(work), first = sync.readSync(), second = sync.readSync(); sync.closeSync(); var closedCode; try { sync.readSync(); } catch (error) { closedCode = error.code; } var callbackName = await new Promise(function(resolve, reject) { fs.opendir(work, function(error, opened) { if (error) return reject(error); opened.read(function(readError, entry) { if (readError) return reject(readError); opened.close(function(closeError) { closeError ? reject(closeError) : resolve(entry.name); }); }); }); }); var opened = await promises.opendir(work), iterated = []; for await (var entry of opened) iterated.push([entry.name, entry.isFile(), entry.isDirectory()]); iterated.sort(function(left, right) { return left[0].localeCompare(right[0]); }); return [[first.name, second.name].sort(), sync instanceof fs.Dir, sync.closed, closedCode, typeof callbackName, iterated, opened.closed]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_directory_handles_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsDirectoryHandles = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsDirectoryHandles").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["a.txt","nested"],true,true,"ERR_DIR_CLOSED","string",[["a.txt",true,false],["nested",false,true]],true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_links_and_permissions_use_host_filesystem_semantics() {
        use std::ffi::{CStr, CString};
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_registry("builtin_fs_links");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', hard = root + '/hard.txt', symbolic = root + '/symbolic.txt'; fs.writeFileSync(source, 'linked'); fs.linkSync(source, hard); await promises.symlink(source, symbolic); var target = await new Promise(function(resolve, reject) { fs.readlink(symbolic, function(error, value) { error ? reject(error) : resolve(value); }); }); await promises.chmod(source, 384); return [fs.readFileSync(hard, 'utf8'), fs.readFileSync(symbolic, 'utf8'), target, fs.readlinkSync(symbolic, 'buffer').toString(), fs.existsSync(hard), fs.existsSync(symbolic)]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_links_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLinks = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsLinks").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let source_path = dir.join("source.txt").to_string_lossy().into_owned();
        assert_eq!(
            result,
            format!(r#"["linked","linked","{0}","{0}",true,true]"#, source_path)
        );
        assert_eq!(
            fs::metadata(dir.join("source.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_stats_and_utimes_use_host_timestamps_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_timestamps");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'time'); fs.utimesSync(path, 100, 200); var sync = fs.statSync(path); await new Promise(function(resolve, reject) { fs.utimes(path, new Date(300000), new Date(400000), function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.stat(path); await promises.utimes(path, 500, 600); var handle = await promises.open(path, 'r+'); await handle.utimes(700, 800); var finalStats = await handle.stat(); await handle.close(); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), sync.atime.getTime(), sync.mtime.getTime(), sync.ctimeMs > 0, (sync.mode & 32768) !== 0, Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(finalStats.atimeMs), Math.round(finalStats.mtimeMs), finalStats.atime.getTime(), finalStats.mtime.getTime()]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_timestamps_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTimestamps = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("timestamps.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsTimestamps").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[100000,200000,100000,200000,true,true,300000,400000,700000,800000,700000,800000]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_ownership_apis_use_host_uid_and_gid() {
        use std::ffi::{CStr, CString};
        use std::os::unix::fs::MetadataExt;
        let dir = temp_registry("builtin_fs_ownership");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root, uid, gid) { var source = root + '/owned.txt', link = root + '/owned-link'; fs.writeFileSync(source, 'owned'); fs.symlinkSync(source, link); fs.chownSync(source, uid, gid); await promises.chown(source, uid, gid); await new Promise(function(resolve, reject) { fs.lchown(link, uid, gid, function(error) { error ? reject(error) : resolve(); }); }); var handle = await promises.open(source, 'r+'); await handle.chown(uid, gid); var stats = await handle.stat(); await handle.close(); return [stats.uid, stats.gid, fs.statSync(source).uid, fs.statSync(source).gid]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_ownership_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOwnership = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let owner = fs::metadata(&dir).unwrap();
        let uid = owner.uid();
        let gid = owner.gid();
        let arguments = CString::new(
            serde_json::to_string(&[
                serde_json::json!(dir.to_string_lossy()),
                serde_json::json!(uid),
                serde_json::json!(gid),
            ])
            .unwrap(),
        )
        .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsOwnership").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, format!("[{uid},{gid},{uid},{gid}]"));
        let link_metadata = fs::symlink_metadata(dir.join("owned-link")).unwrap();
        assert_eq!((link_metadata.uid(), link_metadata.gid()), (uid, gid));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_watch_file_reports_host_changes_and_stops_cleanly() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_watch_file");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'one'); var ignoredCalls = 0, ignored = function() { ignoredCalls++; }, watcher; var changed = new Promise(function(resolve) { watcher = fs.watchFile(path, { interval: 1, persistent: false }, function(current, previous) { fs.unwatchFile(path); resolve([previous.size, current.size, current.isFile(), watcher.hasRef(), ignoredCalls]); }); fs.watchFile(path, { interval: 1 }, ignored); fs.unwatchFile(path, ignored); }); setTimeout(function() { fs.writeFileSync(path, 'changed-value'); }, 3); var result = await changed; return [result, watcher._timer, watcher._listeners.length]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_watch_file_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatchFile = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("watched.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsWatchFile").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[[3,13,true,false,0],null,0]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_watch_reports_directory_rename_and_change_events() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_watch");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var events = [], closeCount = 0, watcher; var completed = new Promise(function(resolve) { watcher = fs.watch(work, { interval: 1, persistent: false }, function(type, name) { events.push([type, name]); if (events.length === 2) { watcher.close(); resolve(); } }); watcher.once('close', function() { closeCount++; }); }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'a'); }, 3); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'expanded'); }, 12); await completed; var controller = new AbortController(), aborted = fs.watch(work, { interval: 1, signal: controller.signal, encoding: 'buffer' }); controller.abort(); return [watcher instanceof fs.FSWatcher, events, watcher.closed, watcher._timer, watcher.hasRef(), closeCount, aborted.closed, aborted._timer]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_watch_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatch = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsWatch").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,[["rename","value.txt"],["change","value.txt"]],true,null,false,1,true,null]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_glob_supports_patterns_exclusions_and_async_iteration() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_glob");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { fs.mkdirSync(root + '/src/nested', { recursive: true }); fs.writeFileSync(root + '/root.txt', 'root'); fs.writeFileSync(root + '/src/main.js', 'js'); fs.writeFileSync(root + '/src/nested/value.txt', 'txt'); fs.writeFileSync(root + '/src/nested/skip.txt', 'skip'); var sync = fs.globSync('**/*.txt', { cwd: root, exclude: '**/skip.txt' }).sort(); var callback = await new Promise(function(resolve, reject) { fs.glob(['src/*.js', 'root.?xt'], { cwd: root }, function(error, values) { error ? reject(error) : resolve(values.sort()); }); }); var asyncValues = []; for await (var value of promises.glob('src/**', { cwd: root })) asyncValues.push(value); asyncValues.sort(); var dirents = fs.globSync('src/*', { cwd: root, withFileTypes: true }); return [sync, callback, asyncValues, dirents.map(function(entry) { return [entry.name, entry.isFile(), entry.isDirectory()]; }).sort()]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_glob_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsGlob = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsGlob").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["root.txt","src/nested/value.txt"],["root.txt","src/main.js"],["src/main.js","src/nested","src/nested/skip.txt","src/nested/value.txt"],[["main.js",true,false],["nested",false,true]]]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_access_checks_host_permissions_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_access");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/access.txt', missing = root + '/missing.txt'; fs.writeFileSync(path, 'access'); fs.accessSync(path, fs.constants.R_OK | fs.constants.W_OK); await promises.access(root, fs.constants.X_OK); var callback = await new Promise(function(resolve, reject) { fs.access(path, fs.constants.F_OK, function(error) { error ? reject(error) : resolve(true); }); }); var syncError, callbackError, promiseError; try { fs.accessSync(missing); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.access(missing, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.access(missing, fs.constants.R_OK); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } return [callback, syncError, callbackError, promiseError]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_access_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsAccess = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsAccess").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let missing = dir.join("missing.txt").to_string_lossy().into_owned();
        assert_eq!(
            result,
            format!(
                r#"[true,["ENOENT","{0}","access"],["ENOENT","{0}","access"],["ENOENT","{0}","access"]]"#,
                missing
            )
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_numeric_descriptors_support_sync_and_callback_io() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_numeric_descriptors");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), source = Buffer.from('abcdef'), firstWrite = fs.writeSync(fd, source, 1, 3, 0), secondWrite = fs.writeSync(fd, '!', 3, 'utf8'); fs.fsyncSync(fd); fs.fdatasyncSync(fd); var buffer = Buffer.alloc(6, 46), firstRead = fs.readSync(fd, buffer, 1, 4, 0); fs.ftruncateSync(fd, 3); var size = fs.fstatSync(fd).size; fs.closeSync(fd); var closedCode; try { fs.fstatSync(fd); } catch (error) { closedCode = error.code; } var callbackResult = await new Promise(function(resolve, reject) { fs.open(path, 'r+', function(openError, callbackFd) { if (openError) return reject(openError); fs.write(callbackFd, Buffer.from('XYZ'), 0, 3, 0, function(writeError, written, original) { if (writeError) return reject(writeError); var target = Buffer.alloc(3); fs.read(callbackFd, target, 0, 3, 0, function(readError, read, returned) { if (readError) return reject(readError); fs.fstat(callbackFd, function(statError, stats) { if (statError) return reject(statError); fs.close(callbackFd, function(closeError) { closeError ? reject(closeError) : resolve([written, original.toString(), read, returned === target, target.toString(), stats.size]); }); }); }); }); }); }); return [firstWrite, secondWrite, firstRead, buffer.toString(), size, closedCode, callbackResult, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_numeric_descriptors_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsNumericDescriptors = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("descriptor.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsNumericDescriptors")
                .unwrap()
                .as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[3,1,4,".bcd!.",3,"EBADF",[3,"XYZ",3,true,"XYZ",3],"XYZ"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_scatter_gather_io_works_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_scatter_gather");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), written = fs.writevSync(fd, [Buffer.from('ab'), Buffer.from('cd')], 0), first = Buffer.alloc(2), second = Buffer.alloc(3), read = fs.readvSync(fd, [first, second], 0); var callback = await new Promise(function(resolve, reject) { var output = [Buffer.from('e'), Buffer.from('f')]; fs.writev(fd, output, 4, function(writeError, bytesWritten, returnedWrite) { if (writeError) return reject(writeError); var input = [Buffer.alloc(3), Buffer.alloc(3)]; fs.readv(fd, input, 0, function(readError, bytesRead, returnedRead) { readError ? reject(readError) : resolve([bytesWritten, returnedWrite === output, bytesRead, returnedRead === input, input[0].toString(), input[1].toString()]); }); }); }); fs.closeSync(fd); var handle = await promises.open(path, 'r+'), handleWriteBuffers = [Buffer.from('X'), Buffer.from('Y')], handleWrite = await handle.writev(handleWriteBuffers, 1), handleReadBuffers = [Buffer.alloc(3), Buffer.alloc(3)], handleRead = await handle.readv(handleReadBuffers, 0); await handle.close(); return [written, read, first.toString(), second.subarray(0, 2).toString(), callback, handleWrite.bytesWritten, handleWrite.buffers === handleWriteBuffers, handleRead.bytesRead, handleRead.buffers === handleReadBuffers, handleReadBuffers[0].toString(), handleReadBuffers[1].toString(), fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_scatter_gather_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsScatterGather = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("vectors.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsScatterGather").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[4,4,"ab","cd",[2,true,6,true,"abc","def"],2,true,6,true,"aXY","def","aXYdef"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_statfs_reports_host_capacity_across_api_styles() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_statfs");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var sync = fs.statfsSync(path), bigint = fs.statfsSync(path, { bigint: true }); var callback = await new Promise(function(resolve, reject) { fs.statfs(path, function(error, value) { error ? reject(error) : resolve(value); }); }); var promised = await promises.statfs(path), handle = await promises.open(path + '/value.txt', 'w+'), handled = await handle.statfs({ bigint: true }); await handle.close(); return [sync instanceof fs.StatFs, sync.bsize > 0, sync.blocks > 0, sync.bfree >= sync.bavail, sync.files >= sync.ffree, typeof bigint.bsize, bigint.bsize.toString() === String(sync.bsize), callback.bsize, promised.bsize, typeof handled.blocks, handled.blocks.toString() === String(sync.blocks)]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_statfs_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStatfs = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsStatfs").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let values = parsed.as_array().unwrap();
        assert_eq!(
            &values[..7],
            &[
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!(true),
                serde_json::json!("bigint"),
                serde_json::json!(true)
            ]
        );
        assert_eq!(values[7], values[8]);
        assert_eq!(
            &values[9..],
            &[serde_json::json!("bigint"), serde_json::json!(true)]
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_lstat_and_dirents_identify_symbolic_links() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_lstat_symlinks");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); var followed = fs.statSync(link), own = fs.lstatSync(link), callback = await new Promise(function(resolve, reject) { fs.lstat(link, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(link), entry = fs.readdirSync(root, { withFileTypes: true }).filter(function(value) { return value.name === 'target-link'; })[0]; return [followed.isFile(), followed.isSymbolicLink(), own.isFile(), own.isSymbolicLink(), (own.mode & fs.constants.S_IFMT) === fs.constants.S_IFLNK, own.size === target.length, callback.isSymbolicLink(), promised.isSymbolicLink(), entry.isFile(), entry.isSymbolicLink()]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_lstat_symlinks_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLstatSymlinks = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsLstatSymlinks").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,false,false,true,true,true,true,true,false,true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_lutimes_updates_links_without_touching_targets() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_lutimes");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); fs.utimesSync(target, 100, 200); fs.lutimesSync(link, 300, 400); var sync = fs.lstatSync(link); await new Promise(function(resolve, reject) { fs.lutimes(link, 500, 600, function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.lstat(link); await promises.lutimes(link, new Date(700000), new Date(800000)); var promised = await promises.lstat(link), followed = await promises.stat(link); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(promised.atimeMs), Math.round(promised.mtimeMs), Math.round(followed.atimeMs), Math.round(followed.mtimeMs)]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_lutimes_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLutimes = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsLutimes").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[300000,400000,500000,600000,700000,800000,100000,200000]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_copy_exclusive_and_forced_removal_match_node_options() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_operation_options");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', destination = root + '/destination.txt', missing = root + '/missing'; fs.writeFileSync(source, 'source'); fs.writeFileSync(destination, 'existing'); var syncError, callbackError, promiseError, missingError; try { fs.copyFileSync(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.copyFile(source, destination, fs.constants.COPYFILE_EXCL, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.copyFile(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } fs.rmSync(missing, { force: true }); await new Promise(function(resolve, reject) { fs.rm(missing, { force: true }, function(error) { error ? reject(error) : resolve(); }); }); await promises.rm(missing, { force: true }); try { fs.rmSync(missing); } catch (error) { missingError = error.code; } return [syncError, callbackError, promiseError, fs.readFileSync(destination, 'utf8'), missingError]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_operation_options_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOperationOptions = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsOperationOptions").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let destination = dir.join("destination.txt").to_string_lossy().into_owned();
        assert_eq!(
            result,
            format!(
                r#"[["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],"existing","ENOENT"]"#,
                destination
            )
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_readdir_supports_recursive_and_buffer_results() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_recursive_readdir");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/tree'; fs.mkdirSync(work + '/nested/deep', { recursive: true }); fs.writeFileSync(work + '/root.txt', 'root'); fs.writeFileSync(work + '/nested/value.txt', 'value'); fs.writeFileSync(work + '/nested/deep/end.txt', 'end'); fs.symlinkSync(work + '/nested', work + '/linked'); var sync = fs.readdirSync(work, { recursive: true }).sort(); var callback = await new Promise(function(resolve, reject) { fs.readdir(work, { recursive: true, withFileTypes: true }, function(error, entries) { if (error) return reject(error); resolve(entries.map(function(entry) { return [entry.name, entry.parentPath.slice(work.length), entry.isFile(), entry.isDirectory(), entry.isSymbolicLink()]; }).sort()); }); }); var buffers = (await promises.readdir(work, { recursive: true, encoding: 'buffer' })).map(function(value) { return [Buffer.isBuffer(value), value.toString()]; }).sort(function(left, right) { return left[1].localeCompare(right[1]); }); return [sync, callback, buffers]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_recursive_readdir_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsRecursiveReaddir = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsRecursiveReaddir").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let values = parsed.as_array().unwrap();
        assert_eq!(
            values[0],
            serde_json::json!([
                "linked",
                "nested",
                "nested/deep",
                "nested/deep/end.txt",
                "nested/value.txt",
                "root.txt"
            ])
        );
        assert_eq!(values[1].as_array().unwrap().len(), 6);
        assert_eq!(
            values[1][0],
            serde_json::json!(["deep", "/nested", false, true, false])
        );
        assert_eq!(
            values[1][1],
            serde_json::json!(["end.txt", "/nested/deep", true, false, false])
        );
        assert_eq!(
            values[1][2],
            serde_json::json!(["linked", "", false, false, true])
        );
        assert!(values[2]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry[0] == serde_json::json!(true)));
        assert_eq!(values[2].as_array().unwrap().len(), 6);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_write_flags_honor_append_and_exclusive_creation() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_write_flags");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/flags.txt', exclusive = root + '/exclusive.txt', opened = root + '/opened.txt'; fs.writeFileSync(path, 'one'); fs.writeFileSync(path, '-two', { flag: 'a' }); await new Promise(function(resolve, reject) { fs.writeFile(path, '-three', { flag: 'a' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(path, '-four', { flag: 'a' }); fs.appendFileSync(exclusive, 'created', { flag: 'ax' }); var errors = []; try { fs.writeFileSync(path, 'lost', { flag: 'wx' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } await new Promise(function(resolve) { fs.appendFile(exclusive, 'lost', { flag: 'ax' }, function(error) { errors.push([error.code, error.path, error.syscall]); resolve(); }); }); try { await promises.writeFile(path, 'lost', { flag: 'ax' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { fs.openSync(path, 'wx'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { await promises.open(path, 'ax'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } var fd = fs.openSync(opened, 'wx'); fs.closeSync(fd); return [fs.readFileSync(path, 'utf8'), fs.readFileSync(exclusive, 'utf8'), fs.existsSync(opened), fs.statSync(opened).size, errors]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_write_flags_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWriteFlags = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsWriteFlags").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        let path = dir.join("flags.txt").to_string_lossy().into_owned();
        let exclusive = dir.join("exclusive.txt").to_string_lossy().into_owned();
        assert_eq!(
            result,
            format!(
                r#"["one-two-three-four","created",true,0,[["EEXIST","{0}","open"],["EEXIST","{1}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"]]]"#,
                path, exclusive
            )
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_creation_apis_honor_requested_modes() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_creation_modes");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = root + '/sync.txt', callback = root + '/callback.txt', promised = root + '/promised.txt', opened = root + '/opened.txt', asyncOpened = root + '/async-opened.txt', streamed = root + '/streamed.txt', directory = root + '/directory'; fs.writeFileSync(sync, 'sync', { mode: 384 }); await new Promise(function(resolve, reject) { fs.appendFile(callback, 'callback', { mode: '600' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(promised, 'promised', { mode: 384 }); var fd = fs.openSync(opened, 'w', 384); fs.closeSync(fd); var handle = await promises.open(asyncOpened, 'w', 384); await handle.close(); fs.mkdirSync(directory, { mode: 448 }); var writer = fs.createWriteStream(streamed, { mode: 384 }); await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.end('stream'); }); return [sync, callback, promised, opened, asyncOpened, streamed, directory].map(function(path) { return fs.statSync(path).mode & 511; }); };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_creation_modes_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCreationModes = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsCreationModes").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[384,384,384,384,384,384,448]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_truncate_and_file_handle_sync_methods_work() {
        use std::ffi::{CStr, CString};
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_registry("builtin_fs_truncate_sync");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); fs.truncateSync(path, 3); await new Promise(function(resolve, reject) { fs.truncate(path, 5, function(error) { error ? reject(error) : resolve(); }); }); await promises.truncate(path, 4); var handle = await promises.open(path, 'r+'); await handle.chmod(384); await handle.sync(); await handle.datasync(); await handle.close(); var closedCode; try { await handle.sync(); } catch (error) { closedCode = error.code; } return [fs.statSync(path).size, fs.readFileSync(path).toString('hex'), closedCode]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_truncate_sync_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTruncateSync = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("truncate.txt");
        let arguments =
            CString::new(serde_json::to_string(&[path.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsTruncateSync").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[4,"61626300","EBADF"]"#);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_bigint_stats_include_host_identity_and_nanoseconds() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_bigint_stats");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'bigint'); var sync = fs.statSync(path, { bigint: true }), callback = await new Promise(function(resolve, reject) { fs.stat(path, { bigint: true }, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(path, { bigint: true }), fd = fs.openSync(path, 'r'), descriptor = fs.fstatSync(fd, { bigint: true }); fs.closeSync(fd); var handle = await promises.open(path, 'r'), handled = await handle.stat({ bigint: true }); await handle.close(); function summarize(value) { return [typeof value.size, value.size.toString(), typeof value.ino, value.ino > 0n, typeof value.blocks, typeof value.atimeMs, typeof value.atimeNs, value.atimeNs / 1000000n === value.atimeMs, value.atime instanceof Date]; } return [summarize(sync), callback.size === sync.size, promised.ino === sync.ino, descriptor.dev === sync.dev, handled.nlink === sync.nlink]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_bigint_stats_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsBigintStats = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let path = dir.join("bigint.txt").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsBigintStats").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["bigint","6","bigint",true,"bigint","bigint","bigint",true,true],true,true,true,true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn fs_promises_watch_iterates_changes_and_honors_abort() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_fs_promises_watch");
        fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var iterator = promises.watch(work, { interval: 1, encoding: 'buffer' }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'value'); }, 3); var event = await iterator.next(), returned = await iterator.return(), after = await iterator.next(); var controller = new AbortController(), aborted = promises.watch(work, { interval: 1, signal: controller.signal }), pending = aborted.next(), abortName, abortCause; controller.abort('reason'); try { await pending; } catch (error) { abortName = error.name; abortCause = error.cause; } return [event.done, event.value.eventType, Buffer.isBuffer(event.value.filename), event.value.filename.toString(), returned.done, after.done, abortName, abortCause]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_fs_promises_watch_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromisesWatch = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(
            CString::new("exerciseFsPromisesWatch").unwrap().as_ptr(),
            arguments.as_ptr(),
        );
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[false,"rename",true,"value.txt",true,true,"AbortError","reason"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn util_types_identifies_standard_and_typed_objects() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_util_types");
        fs::write(dir.join("index.js"), "var types = require('node:util/types'); module.exports = async function () { return [types.isDate(new Date()), types.isRegExp(/x/), types.isMap(new Map()), types.isSet(new Set()), types.isWeakMap(new WeakMap()), types.isPromise(Promise.resolve()), types.isArrayBuffer(new ArrayBuffer(2)), types.isDataView(new DataView(new ArrayBuffer(2))), types.isTypedArray(new Uint8Array(2)), types.isUint8Array(Buffer.from('x')), types.isNativeError(new TypeError('x')), types.isBoxedPrimitive(Object(4)), types.isAsyncFunction(async function() {}), types.isGeneratorFunction(function*() {}), types.isProxy(new Proxy({}, {}))]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_util_types_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilTypes = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseUtilTypes").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,false]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn node_constants_are_shared_with_filesystem_constants() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_constants");
        fs::write(dir.join("index.js"), "var constants = require('node:constants'); var fs = require('node:fs'); module.exports = function () { return [constants === fs.constants, constants.F_OK, constants.R_OK, constants.W_OK, constants.X_OK, constants.O_CREAT, constants.O_APPEND, constants.S_IFREG, constants.S_IFDIR, constants.COPYFILE_FICLONE_FORCE]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_constants_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConstants = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseConstants").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"[true,0,4,2,1,64,1024,32768,16384,4]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn readline_splits_lines_and_supports_callback_and_promise_questions() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_readline");
        fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promiseReadline = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var rl = readline.createInterface({ output: output, historySize: 2, removeHistoryDuplicates: true }); var lines = []; rl.on('line', function(line) { lines.push(line); }); var callbackAnswer = new Promise(function(resolve) { rl.question('name? ', resolve); }); rl.write('alice\\nnext\\r\\n'); var answer = await callbackAnswer; rl.setPrompt('ready> '); rl.prompt(); rl.pause(); rl.resume(); readline.cursorTo(output, 2, 3); readline.clearLine(output, 0); rl.close(); var prl = promiseReadline.createInterface({ output: output }); var promised = prl.question('age? '); prl.write('42\\n'); var age = await promised; var iterator = prl[Symbol.asyncIterator](); var next = iterator.next(); prl.write('tail\\n'); var iterated = await next; prl.close(); return [answer, lines, rl.history, rl.closed, age, iterated.value, writes, readline.ReadLine === readline.Interface, prl instanceof promiseReadline.Interface]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_readline_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadline = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseReadline").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["alice",["alice","next"],["next","alice"],true,"42","42",["\u001b[4;3H","\u001b[2K"],true,true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn readline_questions_write_to_configured_output() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_readline_output");
        fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promises = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var input = { on: function() {}, off: function() {} }; var callbackInterface = readline.createInterface({ input: input, output: output }); var callbackAnswer = new Promise(function(resolve) { callbackInterface.question('first? ', resolve); }); callbackInterface.write('yes\\n'); var first = await callbackAnswer; var promiseInterface = promises.createInterface({ input: input, output: output }); var secondPromise = promiseInterface.question('second? '); promiseInterface.write('ok\\n'); var second = await secondPromise; callbackInterface.close(); promiseInterface.close(); return [first, second, writes]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_readline_output_node_modules");
        let (bundle, _, _, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadlineOutput = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseReadlineOutput").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, r#"["yes","ok",["first? ","second? "]]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn dns_modules_resolve_local_names_and_report_not_found() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_dns");
        fs::write(dir.join("index.js"), "var dns = require('node:dns'); var promises = require('node:dns/promises'); module.exports = async function () { var callbackLookup = await new Promise(function(resolve, reject) { dns.lookup('localhost', { family: 4 }, function(error, address, family) { error ? reject(error) : resolve([address, family]); }); }); var all = await dns.promises.lookup('localhost', { all: true }); var ipv6 = await promises.resolve6('localhost'); var reverse = await promises.reverse('127.0.0.1'); var errorCode; try { await promises.lookup('does-not-exist.invalid'); } catch (error) { errorCode = [error.code, error.syscall, error.hostname]; } dns.setDefaultResultOrder('ipv4first'); var resolver = new promises.Resolver(); resolver.setServers(['127.0.0.1']); return [callbackLookup, all, ipv6, reverse, errorCode, promises.getDefaultResultOrder(), resolver.getServers()]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_dns_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDns = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseDns").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["127.0.0.1",4],[{"address":"127.0.0.1","family":4},{"address":"::1","family":6}],["::1"],["localhost"],["ENOTFOUND","getaddrinfo","does-not-exist.invalid"],"ipv4first",["127.0.0.1"]]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn net_builtin_validates_addresses_and_block_lists() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_net");
        fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = function () { var block = new net.BlockList(); block.addAddress('127.0.0.1'); block.addRange('10.0.0.2', '10.0.0.5'); block.addSubnet('192.168.1.0', 24); block.addAddress('::1', 'ipv6'); var ipv4 = new net.SocketAddress({ address: '127.0.0.1', port: 8080 }); var ipv6 = new net.SocketAddress({ address: '::1', family: 'ipv6' }); var invalidPort = false; try { new net.SocketAddress({ address: '127.0.0.1', port: 70000 }); } catch (error) { invalidPort = error instanceof RangeError; } return [net.isIP('127.0.0.1'), net.isIP('2001:db8::1'), net.isIP('999.0.0.1'), net.isIPv4('01.2.3.4'), net.isIPv6('::ffff:192.0.2.1'), block.check('127.0.0.1'), block.check('10.0.0.4'), block.check('10.0.0.9'), block.check('192.168.1.88'), block.check('::1'), ipv4.toJSON(), ipv6.family, invalidPort]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_net_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNet = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseNet").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[4,6,0,false,true,true,true,false,true,true,{"address":"127.0.0.1","port":8080,"family":"ipv4","flowlabel":0},"ipv6",true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn net_socket_exchanges_bytes_with_a_real_tcp_peer() {
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            assert_eq!(request, b"ping");
            stream.write_all(b"pong").unwrap();
        });

        let dir = temp_registry("builtin_net_socket");
        fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = []; var socket = new net.Socket(); socket.on('connect', function() { events.push('connect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var completed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect(port, '127.0.0.1'); socket.end('ping'); await completed; return [events, socket.bytesWritten, socket.bytesRead, socket.destroyed, socket.remotePort, socket.remoteFamily]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_net_socket_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetSocket = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseNetSocket").unwrap();
        let arguments = CString::new(format!("[{port}]")).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            format!(r#"[["connect","data:pong","end","close:false"],4,4,true,{port},"IPv4"]"#)
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn http_client_requests_and_parses_a_real_chunked_response() {
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /items?q=thaw HTTP/1.1\r\n"));
            assert!(request.to_ascii_lowercase().contains("x-thaw: enabled\r\n"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-Reply: yes\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\n4\r\nthaw\r\n3\r\n-ok\r\n0\r\n\r\n",
                )
                .unwrap();
        });

        let dir = temp_registry("builtin_http_client");
        fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { return await new Promise(function(resolve, reject) { var request = http.get({ hostname: '127.0.0.1', port: port, path: '/items?q=thaw', headers: { 'X-Thaw': 'enabled' } }, function(response) { var chunks = []; response.setEncoding('utf8'); response.on('data', function(chunk) { chunks.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.statusMessage, response.httpVersion, response.headers['x-reply'], response.headers['set-cookie'], response.rawHeaders.length, chunks.join(''), response.complete, request.finished, request.destroyed, http.METHODS.indexOf('GET') >= 0, http.STATUS_CODES[200]]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_http_client_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpClient = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseHttpClient").unwrap();
        let arguments = CString::new(format!("[{port}]")).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[200,"OK","1.1","yes",["a=1","b=2"],10,"thaw-ok",true,true,false,true,"OK"]"#
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn http_agents_and_header_validators_share_across_https() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_http_agents");
        fs::write(
            dir.join("index.js"),
            "var http = require('node:http'), https = require('node:https'); module.exports = function () { var agent = new http.Agent({ keepAlive: true, maxSockets: 7, maxTotalSockets: 9 }); var secureAgent = new https.Agent({ keepAliveMsecs: 250 }); var request = http.request({ host: 'example.com', port: 8080, agent: agent }); var secureRequest = https.request({ host: 'example.com', agent: false }); request.setHeader('X-Valid', ['one', 'two']); var errors = []; try { http.validateHeaderName('bad name'); } catch (error) { errors.push(error.code); } try { https.validateHeaderValue('X-Test', 'bad\\nvalue'); } catch (error) { errors.push(error.code); } try { request.setHeader('X-Missing', undefined); } catch (error) { errors.push(error.code); } http.setMaxIdleHTTPParsers(10); return [agent instanceof http.Agent, secureAgent instanceof https.Agent, secureAgent instanceof http.Agent, request.agent === agent, secureRequest.agent, request.getHeader('x-valid'), agent.getName({ host: 'example.com', port: 8080, family: 4 }), agent.keepAlive, agent.maxSockets, agent.maxTotalSockets, agent.totalSocketCount, secureAgent.protocol, secureAgent.defaultPort, secureAgent.keepAliveMsecs, http.globalAgent instanceof http.Agent, https.globalAgent instanceof https.Agent, errors]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_http_agents_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 6);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpAgents = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseHttpAgents").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,true,true,true,null,["one","two"],"example.com:8080::4",true,7,9,0,"https:",443,250,true,true,["ERR_INVALID_HTTP_TOKEN","ERR_INVALID_CHAR","ERR_HTTP_INVALID_HEADER_VALUE"]]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn child_process_sync_apis_execute_and_report_node_shaped_results() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_child_process_sync");
        fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = function (cwd) { var failed = cp.spawnSync('/bin/sh', ['-c', 'printf out; printf err >&2; exit 3'], { encoding: 'utf8', cwd: cwd }); var input = cp.spawnSync('/bin/sh', ['-c', 'cat'], { input: Buffer.from('stdin') }); var environment = cp.execFileSync('/bin/sh', ['-c', 'printf "$VALUE"'], { encoding: 'utf8', env: { VALUE: 'env-ok' } }); var shell = cp.execSync('printf shell-ok', { encoding: 'utf8' }); var missing = cp.spawnSync('/thaw/does-not-exist', []); var thrown; try { cp.execFileSync('/bin/sh', ['-c', 'printf bad >&2; exit 7'], { encoding: 'utf8' }); } catch (error) { thrown = [error.status, error.stderr, error.stdout, error.pid > 0]; } var overflow = cp.spawnSync('/bin/sh', ['-c', 'printf 12345'], { maxBuffer: 4 }); return [failed.status, failed.signal, failed.stdout, failed.stderr, failed.output[1], failed.pid > 0, Buffer.isBuffer(input.stdout), input.stdout.toString(), input.stderr.length, environment, shell, missing.status, missing.error.code, missing.error.path, thrown, overflow.status, overflow.error.code]; };"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_sync_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessSync = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseChildProcessSync").unwrap();
        let arguments =
            CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
                .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[3,null,"out","err","out",true,true,"stdin",0,"env-ok","shell-ok",null,"ENOENT","/thaw/does-not-exist",[7,"bad","",true],null,"ENOBUFS"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn child_process_async_apis_stream_and_emit_lifecycle_events() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_child_process_async");
        fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { var child = cp.spawn('/bin/sh', ['-c', 'read value; printf "out:$value"; printf "err:$value" >&2; exit 4']); var events = [], stdout = [], stderr = []; child.on('spawn', function() { events.push('spawn:' + (child.pid > 0)); }); child.stdout.on('data', function(value) { stdout.push(value.toString()); }); child.stderr.on('data', function(value) { stderr.push(value.toString()); }); var closed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('exit', function(code, signal) { events.push('exit:' + code + ':' + signal); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); child.stdin.end('hello\n'); await closed; var executed = await new Promise(function(resolve) { cp.exec('printf callback', { encoding: 'utf8' }, function(error, out, err) { resolve([error, out, err]); }); }); var failed = await new Promise(function(resolve) { cp.execFile('/bin/sh', ['-c', 'printf failure >&2; exit 6'], { encoding: 'utf8' }, function(error, out, err) { resolve([error.code, error.stderr, out, err]); }); }); var killed = cp.spawn('/bin/sh', ['-c', 'sleep 10']); var killedResult = new Promise(function(resolve, reject) { killed.on('error', reject); killed.on('spawn', function() { killed.kill('SIGINT'); }); killed.on('close', function(code, signal) { resolve([code, signal, killed.killed]); }); }); var missing = cp.spawn('/thaw/missing-async', []), missingResult = new Promise(function(resolve) { var code; missing.on('error', function(error) { code = error.code; }); missing.on('close', function(exitCode, signal) { resolve([code, exitCode, signal]); }); }); return [events, stdout.join(''), stderr.join(''), child.exitCode, child.signalCode, child.stdin.writableEnded, child instanceof cp.ChildProcess, executed, failed, await killedResult, await missingResult]; };"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_async_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessAsync = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseChildProcessAsync").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["spawn:true","exit:4:null","close:4:null"],"out:hello","err:hello",4,null,true,true,[null,"callback",""],[6,"failure","","failure"],[null,"SIGINT",true],["ENOENT",null,null]]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn child_process_spawn_honors_stdio_timeout_abort_and_detached_options() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_child_process_options");
        fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { function close(child) { return new Promise(function(resolve) { child.on('error', function() {}); child.on('close', function(code, signal) { resolve([code, signal]); }); }); } var ignored = cp.spawn('/bin/sh', ['-c', 'printf hidden; printf secret >&2'], { stdio: 'ignore' }); var ignoredResult = await close(ignored); var inheritedOut = '', inheritedErr = '', oldOut = process.stdout, oldErr = process.stderr; process.stdout = { write: function(value) { inheritedOut += Buffer.from(value).toString(); return true; } }; process.stderr = { write: function(value) { inheritedErr += Buffer.from(value).toString(); return true; } }; var inherited = cp.spawn('/bin/sh', ['-c', 'printf visible; printf warning >&2'], { stdio: ['ignore', 'inherit', 'inherit'] }); var inheritedResult = await close(inherited); process.stdout = oldOut; process.stderr = oldErr; var timed = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { timeout: 20 }); var timedResult = await close(timed); var controller = new AbortController(), aborted = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { signal: controller.signal }), abortError; aborted.on('error', function(error) { abortError = [error.name, error.code]; }); var abortedResult = close(aborted); controller.abort('stop'); abortedResult = await abortedResult; var detached = cp.spawn('/bin/sh', ['-c', 'ps -o sid= -p $$'], { detached: true }), detachedOutput = ''; detached.stdout.on('data', function(value) { detachedOutput += value.toString(); }); var detachedResult = await close(detached); var invalid; try { cp.spawn('/bin/true', [], { stdio: 'invalid' }); } catch (error) { invalid = error.code; } return [[ignored.stdin, ignored.stdout, ignored.stderr, ignoredResult], [inherited.stdin, inherited.stdout, inherited.stderr, inheritedOut, inheritedErr, inheritedResult], [timed._timedOut, timedResult], [abortError, abortedResult], [detached.detached, Number(detachedOutput.trim()) === detached.pid, detachedResult], invalid]; };"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_options_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessOptions = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseChildProcessOptions").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[[null,null,null,[0,null]],[null,null,null,"visible","warning",[0,null]],[true,[null,"SIGTERM"]],[["AbortError","ABORT_ERR"],[null,"SIGTERM"]],[true,true,[0,null]],"ERR_INVALID_ARG_VALUE"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn child_process_fork_exchanges_ipc_messages_and_disconnects() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_child_process_fork");
        fs::write(
            dir.join("child.js"),
            "console.log('child-ready'); process.on('message', function(value) { process.send({ answer: value.base + 2, argument: process.argv[2] }, function() { process.disconnect(); }); });",
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function (childPath) { var child = cp.fork(childPath, ['argument'], { silent: true, execArgv: ['--no-warnings'] }), events = [], output = '', callbackCode; child.stdout.on('data', function(value) { output += value.toString(); }); var completed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('spawn', function() { events.push('spawn'); child.send({ base: 40 }, function(error) { callbackCode = error && error.code || null; }); }); child.on('message', function(value) { events.push('message:' + value.answer + ':' + value.argument); }); child.on('disconnect', function() { events.push('disconnect'); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); await completed; var closed; try { child.send({ late: true }); } catch (error) { closed = error.code; } return [events, output.trim(), callbackCode, child.connected, child.channel, closed, child instanceof cp.ChildProcess]; };"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_fork_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessFork = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseChildProcessFork").unwrap();
        let child_path = dir.join("child.js").to_string_lossy().into_owned();
        let arguments = CString::new(serde_json::to_string(&[child_path]).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["spawn","message:42:argument","disconnect","close:0:null"],"child-ready",null,false,null,"ERR_IPC_CHANNEL_CLOSED",true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn http_server_parses_and_replies_to_a_real_tcp_client() {
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::time::Duration;

        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let client = std::thread::spawn(move || {
            let mut stream = loop {
                match TcpStream::connect(("127.0.0.1", port)) {
                    Ok(stream) => break stream,
                    Err(_) => std::thread::sleep(Duration::from_millis(2)),
                }
            };
            stream
                .write_all(b"POST /submit HTTP/1.1\r\nHost: localhost\r\nX-Client: rust\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping")
                .unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });

        let dir = temp_registry("builtin_http_server");
        fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { var observed = []; var server = http.createServer(function(request, response) { var body = []; observed.push(request.method, request.url, request.headers['x-client'], request.httpVersion); request.on('data', function(chunk) { body.push(chunk.toString()); }); request.on('end', function() { observed.push(body.join('')); response.statusCode = 201; response.statusMessage = 'Stored'; response.setHeader('X-Server', 'thaw'); response.setHeader('Set-Cookie', ['a=1', 'b=2']); response.write('po'); response.end('ng', function() { server.close(); }); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof http.Server]; };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_http_server_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServer = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseHttpServer").unwrap();
        let arguments = CString::new(format!("[{port}]")).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["POST","/submit","rust","1.1","ping"],false,true]"#
        );
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 201 Stored\r\n"));
        assert!(response.contains("X-Server: thaw\r\n"));
        assert!(response.contains("Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"));
        assert!(response.ends_with("\r\n\r\npong"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn https_client_verifies_a_custom_ca_and_parses_http() {
        use rustls::pki_types::ServerName;
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use rustls::{
            ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
            StreamOwned,
        };
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::process::Command;
        use std::sync::Arc;

        let certificate_dir = temp_registry("builtin_https_certificate");
        let key_pem = certificate_dir.join("key.pem");
        let cert_pem = certificate_dir.join("cert.pem");
        let key_der = certificate_dir.join("key.der");
        let cert_der = certificate_dir.join("cert.der");
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature,keyEncipherment",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout",
            ])
            .arg(&key_pem)
            .arg("-out")
            .arg(&cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(&key_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&key_der)
            .status()
            .unwrap()
            .success());
        let certificate = CertificateDer::from(fs::read(&cert_der).unwrap());
        let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
        let config = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], private_key)
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(config).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /secure HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nX-Secure: yes\r\nConnection: close\r\n\r\nsecret")
                .unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
        });

        let dir = temp_registry("builtin_https_client");
        fs::write(
            dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, ca) { return await new Promise(function(resolve, reject) { var request = https.get({ hostname: '127.0.0.1', port: port, path: '/secure', ca: ca }, function(response) { var body = []; response.setEncoding('utf8'); response.on('data', function(chunk) { body.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.headers['x-secure'], body.join(''), request.protocol, request.socket.encrypted, request.socket.authorized, https.globalAgent.protocol]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_https_client_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 6);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpsClient = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseHttpsClient").unwrap();
        let ca = fs::read_to_string(&cert_pem).unwrap();
        let arguments = CString::new(serde_json::to_string(&(port, ca)).unwrap()).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[200,"yes","secret","https:",true,true,"https:"]"#
        );
        server.join().unwrap();

        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let server_port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
            .unwrap();
        let client_config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let tls_client = std::thread::spawn(move || {
            let socket = loop {
                match TcpStream::connect(("127.0.0.1", server_port)) {
                    Ok(socket) => break socket,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                }
            };
            let connection =
                ClientConnection::new(client_config, ServerName::try_from("localhost").unwrap())
                    .unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            stream
                .write_all(
                    b"GET /from-rust HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        });
        let server_dir = temp_registry("builtin_https_server");
        fs::write(
            server_dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, cert, key) { var observed; var server = https.createServer({ cert: cert, key: key }, function(request, response) { observed = [request.method, request.url, request.socket.encrypted, request.socket.authorized]; response.statusCode = 202; response.setHeader('X-TLS', 'yes'); response.end('secure-server', function() { server.close(); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof https.Server]; };",
        )
        .unwrap();
        let server_node_modules = temp_registry("builtin_https_server_node_modules");
        let (server_bundle, _, server_file_count, _) =
            bundle_commonjs_package(&server_node_modules, "secure-pkg", &server_dir, "index.js")
                .unwrap();
        assert_eq!(server_file_count, 6);
        let server_script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {server_bundle} globalThis.exerciseHttpsServer = module.exports;");
        let server_source = CString::new(server_script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(server_source.as_ptr()), 1);
        let server_function = CString::new("exerciseHttpsServer").unwrap();
        let server_arguments = CString::new(
            serde_json::to_string(&(
                server_port,
                fs::read_to_string(&cert_pem).unwrap(),
                fs::read_to_string(&key_pem).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let server_result =
            thaw_quickjs::thaw_js_call(server_function.as_ptr(), server_arguments.as_ptr());
        let server_result = unsafe { CStr::from_ptr(server_result) }.to_string_lossy();
        assert_eq!(
            server_result,
            r#"[["GET","/from-rust",true,true],false,true]"#
        );
        let response = tls_client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
        assert!(response.contains("X-TLS: yes\r\n"));
        assert!(response.ends_with("\r\n\r\nsecure-server"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        let _ = fs::remove_dir_all(&server_dir);
        let _ = fs::remove_dir_all(&server_node_modules);
        let _ = fs::remove_dir_all(&certificate_dir);
    }

    #[test]
    fn net_server_accepts_and_replies_to_a_real_tcp_client() {
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::{Shutdown, TcpListener, TcpStream};
        use std::time::Duration;

        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let client = std::thread::spawn(move || {
            (0..3)
                .map(|index| {
                    let mut stream = loop {
                        match TcpStream::connect(("127.0.0.1", port)) {
                            Ok(stream) => break stream,
                            Err(_) => std::thread::sleep(Duration::from_millis(5)),
                        }
                    };
                    stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                    stream.shutdown(Shutdown::Write).unwrap();
                    let mut response = String::new();
                    stream.read_to_string(&mut response).unwrap();
                    response
                })
                .collect::<Vec<_>>()
        });

        let dir = temp_registry("builtin_net_server");
        fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = [], handled = 0; var server; var closed = new Promise(function(resolve, reject) { server = net.createServer(function(socket) { events.push('connection:' + socket.remoteFamily); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong' + handled, function() { if (handled === 3) server.close(); }); }); }); server.on('error', reject); server.on('listening', function() { var address = server.address(); events.push('listening:' + address.address + ':' + address.port); }); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections, server.ref() === server, server.unref() === server]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_net_server_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetServer = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseNetServer").unwrap();
        let arguments = CString::new(format!("[{port}]")).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            format!(
                r#"[["listening:127.0.0.1:{port}","connection:IPv4","data:ping0","connection:IPv4","data:ping1","connection:IPv4","data:ping2","close"],false,{port},0,true,true]"#
            )
        );
        assert_eq!(client.join().unwrap(), ["pong1", "pong2", "pong3"]);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn vm_builtin_runs_scripts_in_contexts_and_compiles_functions() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_vm");
        fs::write(dir.join("index.js"), "var vm = require('node:vm'); module.exports = async function () { var sandbox = { value: 3 }; var context = vm.createContext(sandbox); var script = new vm.Script('value *= 4; created = value + 1; value', { filename: 'sample.js' }); var contextual = script.runInContext(context); var fresh = vm.runInNewContext('input + 2', { input: 5 }); var current = vm.runInThisContext('6 * 7'); var add = vm.compileFunction('return left + right;', ['left', 'right'], { filename: 'add.js' }); var memory = await vm.measureMemory(); var cached = script.createCachedData(); return [contextual, sandbox.value, sandbox.created, fresh, current, add(8, 9), vm.isContext(context), vm.isContext({}), cached.toString().includes('value *= 4'), script.cachedDataRejected, memory.total.jsMemoryEstimate, vm.getDefaultContext() === globalThis, typeof vm.constants.DONT_CONTEXTIFY]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_vm_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseVm = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseVm").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[12,12,13,7,42,17,true,false,true,false,0,true,"symbol"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn punycode_builtin_converts_unicode_labels_and_code_points() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_punycode");
        fs::write(dir.join("index.js"), "var punycode = require('node:punycode'); module.exports = function () { var encoded = punycode.encode('mañana'); var snowman = punycode.encode('☃-⌘'); var points = punycode.ucs2.decode('A😀Z'); return [encoded, punycode.decode(encoded), snowman, punycode.decode(snowman), punycode.toASCII('mañana.com'), punycode.toUnicode('xn--bcher-kva.example'), points, punycode.ucs2.encode(points), punycode.toASCII('user@bücher.example'), punycode.version]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_punycode_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePunycode = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exercisePunycode").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["maana-pta","mañana","--dqo34k","☃-⌘","xn--maana-pta.com","bücher.example",[65,128512,90],"A😀Z","user@xn--bcher-kva.example","2.1.0"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn dgram_socket_exchanges_real_udp_datagrams() {
        use std::ffi::{CStr, CString};
        use std::net::UdpSocket;
        use std::time::Duration;

        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(30)))
            .unwrap();
        let client = std::thread::spawn(move || {
            let mut response = [0u8; 16];
            loop {
                peer.send_to(b"ping", ("127.0.0.1", port)).unwrap();
                if let Ok((length, _)) = peer.recv_from(&mut response) {
                    return response[..length].to_vec();
                }
            }
        });

        let dir = temp_registry("builtin_dgram");
        fs::write(dir.join("index.js"), "var dgram = require('node:dgram'); module.exports = async function (port) { var events = []; var socket = dgram.createSocket('udp4'); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('listening', function() { var address = socket.address(); events.push('listening:' + address.family + ':' + address.port); }); socket.on('message', function(message, remote) { events.push('message:' + message.toString() + ':' + remote.family + ':' + remote.size); socket.send('pong', remote.port, remote.address, function(error, written) { if (error) reject(error); else { events.push('sent:' + written); socket.close(); } }); }); socket.on('close', function() { events.push('close'); resolve(); }); }); socket.bind(port, '127.0.0.1'); await closed; return [events, socket.hasRef(), socket.ref() === socket, socket.unref() === socket]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_dgram_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDgram = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseDgram").unwrap();
        let arguments = CString::new(format!("[{port}]")).unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            format!(
                r#"[["listening:IPv4:{port}","message:ping:IPv4:4","sent:4","close"],true,true,true]"#
            )
        );
        assert_eq!(client.join().unwrap(), b"pong");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn stream_consumers_collect_streams_and_async_iterables() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_stream_consumers");
        fs::write(dir.join("index.js"), "var stream = require('node:stream'); var consumers = require('node:stream/consumers'); function consume(method, value) { var input = new stream.PassThrough(); var result = consumers[method](input); input.end(value); return result; } module.exports = async function () { var text = await consume('text', 'hello'); var object = await consume('json', '{\"answer\":42}'); var bytes = await consume('buffer', 'abc'); var array = new Uint8Array(await consume('arrayBuffer', 'xy')); var blob = await consume('blob', 'blob'); var iterable = { async *[Symbol.asyncIterator]() { yield 'one'; yield Buffer.from('two'); } }; var joined = await consumers.text(iterable); var sliced = blob.slice(1, 3, 'text/plain'); return [text, object.answer, bytes.toString('hex'), Array.from(array), blob instanceof Blob, blob.size, await blob.text(), sliced.type, await sliced.text(), joined, new File(['x'], 'a.txt', { lastModified: 7 }).name]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_stream_consumers_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConsumers = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseConsumers").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["hello",42,"616263",[120,121],true,4,"blob","text/plain","lo","onetwo","a.txt"]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn stream_web_pipes_transforms_and_exposes_readers_and_writers() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_stream_web");
        fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); module.exports = async function () { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue('one'); controller.enqueue('two'); controller.close(); } }); var upper = new web.TransformStream({ transform: function(chunk, controller) { controller.enqueue(chunk.toUpperCase()); } }); var transformed = await consumers.text(source.pipeThrough(upper)); var writes = []; var writable = new web.WritableStream({ write: function(chunk) { writes.push(chunk); }, close: function() { writes.push('closed'); } }); var writer = writable.getWriter(); await writer.write('value'); await writer.close(); writer.releaseLock(); var encodedSource = new web.ReadableStream({ start: function(controller) { controller.enqueue('hé'); controller.close(); } }); var decoded = await consumers.text(encodedSource.pipeThrough(new web.TextEncoderStream()).pipeThrough(new web.TextDecoderStream())); var readerSource = new web.ReadableStream({ pull: function(controller) { controller.enqueue(7); controller.close(); } }); var reader = readerSource.getReader(); var first = await reader.read(); var done = await reader.read(); reader.releaseLock(); var byteStrategy = new web.ByteLengthQueuingStrategy({ highWaterMark: 8 }); var countStrategy = new web.CountQueuingStrategy({ highWaterMark: 3 }); return [web.ReadableStream === globalThis.ReadableStream, transformed, writes, decoded, first, done.done, readerSource.locked, byteStrategy.highWaterMark, byteStrategy.size(new Uint8Array(4)), countStrategy.highWaterMark, countStrategy.size('x')]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_stream_web_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWeb = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseStreamWeb").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[true,"ONETWO",["value","closed"],"hé",{"value":7,"done":false},true,false,8,4,3,1]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn web_compression_streams_round_trip_supported_formats() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_web_compression");
        fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); async function roundTrip(format) { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('thaw ')); controller.enqueue(new TextEncoder().encode('compression')); controller.close(); } }); return consumers.text(source.pipeThrough(new web.CompressionStream(format)).pipeThrough(new web.DecompressionStream(format)).pipeThrough(new web.TextDecoderStream())); } module.exports = async function () { var gzipSource = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('header')); controller.close(); } }); var gzip = await consumers.buffer(gzipSource.pipeThrough(new CompressionStream('gzip'))); var unsupported = false; try { new CompressionStream('brotli'); } catch (error) { unsupported = error instanceof TypeError; } return [await roundTrip('gzip'), await roundTrip('deflate'), await roundTrip('deflate-raw'), gzip[0], gzip[1], web.CompressionStream === globalThis.CompressionStream, unsupported]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_web_compression_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompressionStreams = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseCompressionStreams").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"["thaw compression","thaw compression","thaw compression",31,139,true,true]"#
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn tls_client_verifies_custom_ca_and_exchanges_encrypted_bytes() {
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use rustls::server::WebPkiClientVerifier;
        use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::process::Command;
        use std::sync::Arc;

        let certificate_dir = temp_registry("builtin_tls_certificate");
        let key_pem = certificate_dir.join("key.pem");
        let cert_pem = certificate_dir.join("cert.pem");
        let key_der = certificate_dir.join("key.der");
        let cert_der = certificate_dir.join("cert.der");
        let client_key_pem = certificate_dir.join("client-key.pem");
        let client_cert_pem = certificate_dir.join("client-cert.pem");
        let client_cert_der = certificate_dir.join("client-cert.der");
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature,keyEncipherment",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout"
            ])
            .arg(&key_pem)
            .arg("-out")
            .arg(&cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=thaw-client",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature",
                "-addext",
                "extendedKeyUsage=clientAuth",
                "-keyout",
            ])
            .arg(&client_key_pem)
            .arg("-out")
            .arg(&client_cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&client_cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&client_cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(&key_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&key_der)
            .status()
            .unwrap()
            .success());
        let certificate_bytes = fs::read(&cert_der).unwrap();
        let certificate = CertificateDer::from(certificate_bytes.clone());
        let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
        let mut client_roots = RootCertStore::empty();
        client_roots
            .add(CertificateDer::from(fs::read(&client_cert_der).unwrap()))
            .unwrap();
        let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
            .build()
            .unwrap();
        let mut config = ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(vec![certificate], private_key)
            .unwrap();
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (socket, _) = listener.accept().unwrap();
                let connection = ServerConnection::new(Arc::clone(&config)).unwrap();
                let mut stream = StreamOwned::new(connection, socket);
                let mut request = Vec::new();
                stream.read_to_end(&mut request).unwrap();
                assert_eq!(
                    request,
                    if index == 0 {
                        b"ping".as_slice()
                    } else {
                        b"insecure".as_slice()
                    }
                );
                stream
                    .write_all(if index == 0 {
                        b"pong".as_slice()
                    } else {
                        b"accepted".as_slice()
                    })
                    .unwrap();
                stream.conn.send_close_notify();
                stream.flush().unwrap();
            }
        });
        let ca_hex = fs::read(&cert_pem)
            .unwrap()
            .iter()
            .fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            });
        let client_cert_hex =
            fs::read(&client_cert_pem)
                .unwrap()
                .iter()
                .fold(String::new(), |mut output, byte| {
                    use std::fmt::Write as _;
                    let _ = write!(output, "{byte:02x}");
                    output
                });
        let client_key_hex =
            fs::read(&client_key_pem)
                .unwrap()
                .iter()
                .fold(String::new(), |mut output, byte| {
                    use std::fmt::Write as _;
                    let _ = write!(output, "{byte:02x}");
                    output
                });

        let dir = temp_registry("builtin_tls");
        fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function (port, caHex, certHex, keyHex) { var events = []; var socket = new tls.TLSSocket(); socket.on('connect', function() { events.push('connect'); }); socket.on('secureConnect', function() { events.push('secureConnect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect({ host: '127.0.0.1', port: port, servername: 'localhost', ca: Buffer.from(caHex, 'hex'), cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ALPNProtocols: ['h2', 'http/1.1'] }); socket.end('ping'); await closed; var peer = socket.getPeerCertificate(), local = socket.getCertificate(); var bypassData = '', bypass = new tls.TLSSocket(); var bypassClosed = new Promise(function(resolve, reject) { bypass.on('error', reject); bypass.on('data', function(chunk) { bypassData += chunk.toString(); }); bypass.on('close', resolve); }); bypass.connect({ host: '127.0.0.1', port: port, servername: 'not-localhost', cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), rejectUnauthorized: false }); bypass.end('insecure'); await bypassClosed; return [events, socket.authorized, socket.authorizationError, socket.encrypted, socket.alpnProtocol, socket.getProtocol(), socket.getCipher().version, socket.bytesWritten, socket.bytesRead, tls.getCiphers().length, tls.DEFAULT_MIN_VERSION, peer.raw.length > 0, local.raw.length > 0, /^(?:[0-9A-F]{2}:){31}[0-9A-F]{2}$/.test(peer.fingerprint256), peer.subject.CN, peer.issuer.CN, local.subject.CN, peer.valid_from.length > 0, peer.valid_to.length > 0, peer.serialNumber.length > 0, bypassData, bypass.authorized, bypass.authorizationError]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_tls_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTls = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseTls").unwrap();
        let arguments = CString::new(format!(
            "[{port},\"{ca_hex}\",\"{client_cert_hex}\",\"{client_key_hex}\"]"
        ))
        .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            r#"[["connect","secureConnect","data:pong","end","close:false"],true,null,true,"h2","TLSv1.3","TLSv1.3",4,4,3,"TLSv1.2",true,true,true,"localhost","localhost","thaw-client",true,true,true,"accepted",false,"UNABLE_TO_VERIFY_LEAF_SIGNATURE"]"#
        );
        server.join().unwrap();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        let _ = fs::remove_dir_all(&certificate_dir);
    }

    #[test]
    fn tls_server_accepts_verified_clients_and_exchanges_encrypted_bytes() {
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
        use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
        use std::ffi::{CStr, CString};
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::process::Command;
        use std::sync::Arc;
        use std::time::Duration;

        let certificate_dir = temp_registry("builtin_tls_server_certificate");
        let key_pem = certificate_dir.join("key.pem");
        let cert_pem = certificate_dir.join("cert.pem");
        let cert_der = certificate_dir.join("cert.der");
        let client_key_pem = certificate_dir.join("client-key.pem");
        let client_cert_pem = certificate_dir.join("client-cert.pem");
        let client_key_der = certificate_dir.join("client-key.der");
        let client_cert_der = certificate_dir.join("client-cert.der");
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature,keyEncipherment",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout",
            ])
            .arg(&key_pem)
            .arg("-out")
            .arg(&cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=thaw-client",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature",
                "-addext",
                "extendedKeyUsage=clientAuth",
                "-keyout",
            ])
            .arg(&client_key_pem)
            .arg("-out")
            .arg(&client_cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&client_cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&client_cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(&client_key_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&client_key_der)
            .status()
            .unwrap()
            .success());
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
            .unwrap();
        let mut config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_client_auth_cert(
                vec![CertificateDer::from(fs::read(&client_cert_der).unwrap())],
                PrivatePkcs8KeyDer::from(fs::read(&client_key_der).unwrap()).into(),
            )
            .unwrap();
        config.alpn_protocols = vec![b"h2".to_vec()];
        let config = Arc::new(config);
        let client = std::thread::spawn(move || {
            (0..3)
                .map(|index| {
                    let socket = (0..50)
                        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                            Ok(socket) => Some(socket),
                            Err(_) => {
                                std::thread::sleep(Duration::from_millis(10));
                                None
                            }
                        })
                        .expect("TLS server did not start");
                    let connection = ClientConnection::new(
                        Arc::clone(&config),
                        ServerName::try_from("localhost".to_string()).unwrap(),
                    )
                    .unwrap();
                    let mut stream = StreamOwned::new(connection, socket);
                    stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                    stream.conn.send_close_notify();
                    stream.flush().unwrap();
                    let mut response = Vec::new();
                    stream.read_to_end(&mut response).unwrap();
                    response
                })
                .collect::<Vec<_>>()
        });
        let to_hex = |value: &[u8]| {
            value.iter().fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            })
        };
        let cert_hex = to_hex(&fs::read(&cert_pem).unwrap());
        let key_hex = to_hex(&fs::read(&key_pem).unwrap());
        let client_ca_hex = to_hex(&fs::read(&client_cert_pem).unwrap());
        let dir = temp_registry("builtin_tls_server");
        fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function(port, certHex, keyHex, clientCaHex) { var events = [], handled = 0, server; var closed = new Promise(function(resolve, reject) { server = tls.createServer({ cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ca: Buffer.from(clientCaHex, 'hex'), requestCert: true, rejectUnauthorized: true, ALPNProtocols: ['h2', 'http/1.1'] }, function(socket) { var peer = socket.getPeerCertificate(), local = socket.getCertificate(); events.push('secureConnection:' + socket.alpnProtocol + ':' + (peer.raw.length > 0) + ':' + (local.raw.length > 0)); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong', function() { if (handled === 3) server.close(); }); }); }); server.on('listening', function() { events.push('listening'); }); server.on('error', reject); server.on('tlsClientError', reject); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections]; };").unwrap();
        let empty_node_modules = temp_registry("builtin_tls_server_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTlsServer = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseTlsServer").unwrap();
        let arguments = CString::new(format!(
            "[{port},\"{cert_hex}\",\"{key_hex}\",\"{client_ca_hex}\"]"
        ))
        .unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(
            result,
            format!(
                r#"[["listening","secureConnection:h2:true:true","data:ping0","secureConnection:h2:true:true","data:ping1","secureConnection:h2:true:true","data:ping2","close"],false,{port},0]"#
            )
        );
        assert_eq!(client.join().unwrap(), vec![b"pong", b"pong", b"pong"]);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        let _ = fs::remove_dir_all(&certificate_dir);
    }

    #[test]
    fn os_builtin_reports_real_host_shapes() {
        use std::ffi::{CStr, CString};
        let dir = temp_registry("builtin_os_info");
        fs::write(dir.join("index.js"), "var os = require('node:os'); module.exports = function () { var cpus = os.cpus(); var interfaces = os.networkInterfaces(); var user = os.userInfo(); return [typeof os.arch() === 'string' && os.arch().length > 0, typeof os.platform() === 'string' && os.platform().length > 0, typeof os.hostname() === 'string' && os.hostname().length > 0, typeof os.homedir() === 'string', typeof os.tmpdir() === 'string', cpus.length > 0, typeof cpus[0].model === 'string', typeof cpus[0].speed === 'number', os.totalmem() >= os.freemem(), os.totalmem() > 0, os.uptime() >= 0, os.loadavg().length === 3, interfaces.lo.length === 2, interfaces.lo[0].internal, typeof user.username === 'string', os.endianness() === 'LE' || os.endianness() === 'BE', os.devNull === '/dev/null', os.constants.signals.SIGTERM === 15, os.constants.errno.ENOENT === 2, os.EOL === '\\n']; };").unwrap();
        let empty_node_modules = temp_registry("builtin_os_info_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOsInfo = module.exports;");
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("exerciseOsInfo").unwrap();
        let arguments = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
        assert_eq!(result, format!("[{}]", vec!["true"; 20].join(",")));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    /// The exact real-world pattern that motivated `path`/`os`/`fs`: a real
    /// native addon package (`bcrypt`, `utf-8-validate`, ...) depends on
    /// `node-gyp-build`, whose real, unmodified source reads
    /// `fs.readdirSync` (always wrapped in its own try/catch expecting
    /// `[]` back on failure), `path.join`/`path.resolve`/`path.dirname`,
    /// and `os.arch`/`os.platform` while hunting for a prebuilt `.node`
    /// binary that this registry never bundles. Confirms all three
    /// polyfills actually run together through real QuickJS-NG and that
    /// `fs.readdirSync` failing is silently absorbed exactly the way real
    /// Node's `ENOENT` would be, rather than crashing the whole load.
    #[test]
    fn path_os_fs_polyfills_actually_run_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_addon_chase");
        fs::write(
            dir.join("index.js"),
            "var fs = require('fs');\n\
             var path = require('path');\n\
             var os = require('os');\n\
             function readdirSync(d) { try { return fs.readdirSync(d); } catch (err) { return []; } }\n\
             module.exports = function locate() {\n\
             \x20\x20var dir = path.resolve(__dirname);\n\
             \x20\x20var release = readdirSync(path.join(dir, 'build/Release'));\n\
             \x20\x20return os.platform() + '/' + os.arch() + '/' + path.dirname(dir) + '/' + release.length;\n\
             };",
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_addon_chase_node_modules");

        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(
            file_count, 5,
            "pkg's index.js + fs/path/os/stream polyfills"
        );

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             globalThis.__dirname = '/thaw_modules/pkg';\n\
             {bundle}\n\
             globalThis.locate = module.exports;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "bundle failed to load"
        );

        let func = CString::new("locate").unwrap();
        let args = CString::new("[]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "\"linux/x64//thaw_modules/0\"");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn plain_commonjs_is_left_untouched() {
        assert!(rewrite_esm_to_commonjs("module.exports = function f() { return 1; };").is_none());
    }

    #[test]
    fn rewrites_default_export_to_module_exports_default() {
        let rewritten =
            rewrite_esm_to_commonjs("export default function greet() { return 'hi'; }").unwrap();
        assert!(rewritten.contains("module.exports.default = function greet() { return 'hi'; }"));
        assert!(rewritten.contains("module.exports.__esModule = true;"));
    }

    #[test]
    fn rewrites_named_export_and_binds_it_too() {
        let rewritten =
            rewrite_esm_to_commonjs("export function add(a, b) { return a + b; }").unwrap();
        assert!(rewritten.contains("function add(a, b) { return a + b; }"));
        assert!(rewritten.contains("get: function() { return add; }"));
    }

    #[test]
    fn rewrites_named_import_to_a_require_call() {
        let rewritten =
            rewrite_esm_to_commonjs("import { add } from './math';\nconsole.log(add(1, 2));")
                .unwrap();
        assert!(rewritten.contains("require(\"./math\")"));
        assert!(rewritten.contains("console.log(__thaw_esm_import_0[\"add\"](1, 2));"));
        assert!(!rewritten.contains("var add ="));
    }

    #[test]
    fn live_import_rewrite_respects_shadowing_and_shorthand_properties() {
        let rewritten = rewrite_esm_to_commonjs(
            "import { value } from './state.js';\n\
             function read() {\n\
               const before = value;\n\
               { let value = 9; if (value !== 9) throw new Error('shadow'); }\n\
               return { value }.value + before;\n\
             }",
        )
        .unwrap();
        assert!(rewritten.contains("const before = __thaw_esm_import_0[\"value\"]"));
        assert!(rewritten.contains("let value = 9; if (value !== 9)"));
        assert!(
            rewritten.contains("return { value: __thaw_esm_import_0[\"value\"] }.value + before")
        );
    }

    /// The bundle isn't just plausible-looking text: an ESM main file
    /// importing from an ESM sibling file must actually run correctly
    /// through the real QuickJS-NG engine, exactly like the equivalent
    /// CommonJS package already does (`bundle_actually_runs_through_quickjs`).
    #[test]
    fn esm_bundle_actually_runs_through_quickjs() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("esm_bundle_runs_through_quickjs");
        fs::write(
            dir.join("index.js"),
            "import { double } from './double.js';\nexport default function run(n) { return double(n); }",
        )
        .unwrap();
        fs::write(
            dir.join("double.js"),
            "export function double(n) { return n * 2; }",
        )
        .unwrap();

        let empty_node_modules = temp_registry("esm_bundle_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2, "index.js + double.js");

        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.run = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(
            thaw_quickjs::thaw_js_load(source.as_ptr()),
            1,
            "ESM bundle failed to load"
        );

        let func = CString::new("run").unwrap();
        let args = CString::new("[21]").unwrap();
        let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
        let result = unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned();
        assert_eq!(result, "42");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn mixed_esm_bundle_supports_live_exports_cycles_imports_json_and_dynamic_import() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("esm_mixed_graph");
        fs::write(
            dir.join("package.json"),
            r##"{"imports":{"#counter":{"node":"./counter.js","default":"./wrong.js"}}}"##,
        )
        .unwrap();
        fs::write(
            dir.join("index.js"),
            "import { increment, value } from '#counter';\n\
             import data from './data.json?payload' with { type: 'json' };\n\
             import { fromA } from './a.js';\n\
             export default async function run() {\n\
               increment();\n\
               const dynamic = await import('./dynamic.js');\n\
               return value + dynamic.extra + data.base + (fromA() === 'b' ? 10 : 0);\n\
             }",
        )
        .unwrap();
        fs::write(
            dir.join("counter.js"),
            "export let value = 1; export function increment() { value++; }",
        )
        .unwrap();
        fs::write(
            dir.join("a.js"),
            "import * as b from './b.js'; export function fromA() { return b.name; } export const name = 'a';",
        )
        .unwrap();
        fs::write(
            dir.join("b.js"),
            "import * as a from './a.js'; export const name = 'b'; export function fromB() { return a.name; }",
        )
        .unwrap();
        fs::write(dir.join("dynamic.js"), "export const extra = 10;").unwrap();
        fs::write(dir.join("data.json"), r#"{"base":20}"#).unwrap();

        let empty_node_modules = temp_registry("esm_mixed_graph_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 6, "{bundle}");
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runMixed = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("runMixed").unwrap();
        let args = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn top_level_await_initializes_dependencies_before_export_binding() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("top_level_await_graph");
        fs::write(
            dir.join("index.js"),
            "import { value } from './value.js'; export default function run() { return value + 2; }",
        )
        .unwrap();
        fs::write(
            dir.join("value.js"),
            "export const value = await new Promise(resolve => setTimeout(() => resolve(40), 1));",
        )
        .unwrap();
        let empty_node_modules = temp_registry("top_level_await_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runTopLevelAwait = function() {{ return module.exports.default(); }};\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("runTopLevelAwait").unwrap();
        let args = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn top_level_await_cycle_is_an_explicit_bundle_error() {
        let dir = temp_registry("top_level_await_cycle");
        fs::write(
            dir.join("a.js"),
            "import { b } from './b.js'; export const a = await Promise.resolve(b);",
        )
        .unwrap();
        fs::write(
            dir.join("b.js"),
            "import { a } from './a.js'; export const b = a;",
        )
        .unwrap();
        let empty_node_modules = temp_registry("top_level_await_cycle_modules");
        let error = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "a.js").unwrap_err();
        assert!(error.contains("top-level await module cycle"), "{error}");
        assert!(error.contains("pkg/a.js"), "{error}");
        assert!(error.contains("pkg/b.js"), "{error}");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn validates_json_import_attributes_and_rejects_unsupported_types() {
        let modern = analyze_module(
            "import data from './data.json' with { type: 'json' }; export default data;",
        );
        assert!(modern.attribute_error.is_none());

        let queried = analyze_module(
            "import data from './data.json?payload' with { type: 'json' }; export default data;",
        );
        assert!(queried.attribute_error.is_none());
        assert_eq!(queried.specs, vec!["./data.json?payload"]);

        let legacy = analyze_module(
            "import data from './data.json' assert { type: 'json' }; export default data;",
        );
        assert!(legacy.attribute_error.is_none());

        let unsupported = analyze_module(
            "import source from './code.js' with { type: 'javascript' }; export default source;",
        );
        assert!(unsupported
            .attribute_error
            .as_deref()
            .is_some_and(|error| error.contains("unsupported import attribute")));

        let dynamic =
            analyze_module("const data = import('./data.json', { with: { type: 'json' } });");
        assert!(dynamic.attribute_error.is_none());
        assert_eq!(dynamic.specs, vec!["./data.json"]);
        let legacy_dynamic =
            analyze_module("const data = import('./data.json', { assert: { type: 'json' } });");
        assert!(legacy_dynamic.attribute_error.is_none());
        let wrong_dynamic =
            analyze_module("const data = import('./data.js', { with: { type: 'json' } });");
        assert!(wrong_dynamic
            .attribute_error
            .as_deref()
            .is_some_and(|error| error.contains("only JSON modules")));
        let unknown_dynamic =
            analyze_module("const data = import('./data.json', { integrity: 'sha256-test' });");
        assert!(unknown_dynamic
            .attribute_error
            .as_deref()
            .is_some_and(|error| error.contains("unsupported dynamic import option")));
        let runtime_attributed =
            analyze_module("const data = import(name, { with: { type: 'json' } });");
        assert!(runtime_attributed
            .attribute_error
            .as_deref()
            .is_some_and(|error| error.contains("finite static specifier set")));

        let rewritten = rewrite_dynamic_imports(
            "const data = import('./data.json', { with: { type: 'json' } });",
        )
        .unwrap();
        assert!(rewritten.contains("requireAsync(String('./data.json'))"));
        assert!(!rewritten.contains("type: 'json'"));
    }

    #[test]
    fn nonliteral_dynamic_import_resolves_candidates_and_reuses_namespace() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("runtime_dynamic_import");
        fs::write(
            dir.join("index.js"),
            "export default async function run() {\n\
               const name = 'feature';\n\
               const first = await import('./' + name + '.js');\n\
               const second = await import(`./${name}.js`);\n\
               return first === second ? first.value : 0;\n\
             }",
        )
        .unwrap();
        fs::write(dir.join("feature.js"), "export const value = 42;").unwrap();
        let empty_node_modules = temp_registry("runtime_dynamic_import_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 2);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runRuntimeImport = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("runRuntimeImport").unwrap();
        let args = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn module_query_and_fragment_are_part_of_cache_identity() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("module_url_identity");
        fs::write(
            dir.join("index.js"),
            "export default async function run() {
               const first = await import('./feature.js?one');
               const again = await import('./feature.js?one');
               const second = await import('./feature.js#two');
               return first === again && first !== second && first.value === 1 && second.value === 2 ? 42 : 0;
             }",
        )
        .unwrap();
        fs::write(
            dir.join("feature.js"),
            "globalThis.__thawModuleIdentity = (globalThis.__thawModuleIdentity || 0) + 1; export const value = globalThis.__thawModuleIdentity;",
        )
        .unwrap();
        let empty_node_modules = temp_registry("module_url_identity_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 3);
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runModuleIdentity = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("runModuleIdentity").unwrap();
        let args = CString::new("[]").unwrap();
        let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "42");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
    }

    #[test]
    fn runtime_dynamic_import_resolves_declared_external_packages() {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("constant_external_dynamic_import");
        fs::write(
            dir.join("index.js"),
            "export default async function run(name) { const dep = await import(name); return dep.value; }",
        )
        .unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"name":"pkg","version":"1.0.0","dependencies":{"dep-a":"1.0.0"},"optionalDependencies":{"dep-b":"1.0.0"}}"#,
        )
        .unwrap();
        let node_modules = temp_registry("constant_external_dynamic_modules");
        for (name, value) in [("dep-a", 41), ("dep-b", 42)] {
            let dependency = node_modules.join(name);
            fs::create_dir_all(&dependency).unwrap();
            let exports = if name == "dep-a" {
                r#", "exports":{".":"./index.js","./feature":"./feature.js","./features/*":"./features/*.js"}"#
            } else {
                ""
            };
            fs::write(
                dependency.join("package.json"),
                format!(r#"{{"name":"{name}","version":"1.0.0","main":"index.js"{exports}}}"#),
            )
            .unwrap();
            fs::write(
                dependency.join("index.js"),
                format!("exports.value = {value};"),
            )
            .unwrap();
            if name == "dep-a" {
                fs::create_dir_all(dependency.join("features")).unwrap();
                fs::write(dependency.join("feature.js"), "exports.value = 43;").unwrap();
                fs::write(dependency.join("features/math.js"), "exports.value = 44;").unwrap();
            } else {
                fs::create_dir_all(dependency.join("lib/tools")).unwrap();
                fs::write(
                    dependency.join("lib/tool.js"),
                    "globalThis.__thawDeepIdentity = (globalThis.__thawDeepIdentity || 44) + 1; exports.value = globalThis.__thawDeepIdentity;",
                )
                .unwrap();
                fs::write(dependency.join("lib/tools/index.js"), "exports.value = 46;").unwrap();
            }
        }
        let (bundle, _, file_count, versions) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 9);
        assert_eq!(versions.get("dep-a").map(String::as_str), Some("1.0.0"));
        assert_eq!(versions.get("dep-b").map(String::as_str), Some("1.0.0"));
        let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(name); }};\n\
             {bundle}\n\
             globalThis.runConstantExternalImport = module.exports.default;\n"
        );
        let source = CString::new(script).unwrap();
        assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
        let function = CString::new("runConstantExternalImport").unwrap();
        for (args, expected) in [
            (r#"["dep-a"]"#, "41"),
            (r#"["dep-b"]"#, "42"),
            (r#"["dep-a/feature"]"#, "43"),
            (r#"["dep-a/features/math"]"#, "44"),
            (r#"["dep-b/lib/tool"]"#, "45"),
            (r#"["dep-b/lib/tool?raw"]"#, "46"),
            (r#"["dep-b/lib/tool?raw"]"#, "46"),
            (r#"["dep-b/lib/tool#part"]"#, "47"),
            (r#"["dep-b/lib/tools"]"#, "46"),
        ] {
            let args = CString::new(args).unwrap();
            let result = thaw_quickjs::thaw_js_call(function.as_ptr(), args.as_ptr());
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_string_lossy(),
                expected
            );
        }
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules);
    }

    #[test]
    fn bare_subpaths_honor_exact_and_wildcard_export_conditions() {
        let node_modules = temp_registry("conditional_subpath_modules");
        let package = node_modules.join("conditional-pkg");
        fs::create_dir_all(package.join("dist/features")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"exports":{"./feature":{"require":"./dist/feature.cjs","default":"./wrong.js"},"./features/*":{"require":"./dist/features/*.cjs"}}}"#,
        )
        .unwrap();
        fs::write(package.join("dist/feature.cjs"), "module.exports = 1;").unwrap();
        fs::write(
            package.join("dist/features/math.cjs"),
            "module.exports = 2;",
        )
        .unwrap();

        let (_, exact, ..) =
            resolve_bare_require(&node_modules, "conditional-pkg/feature").unwrap();
        let (_, wildcard, ..) =
            resolve_bare_require(&node_modules, "conditional-pkg/features/math").unwrap();
        assert_eq!(exact, "./dist/feature.cjs");
        assert_eq!(wildcard, "./dist/features/math.cjs");
        let _ = fs::remove_dir_all(&node_modules);
    }
}
