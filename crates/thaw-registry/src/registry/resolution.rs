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
    pub native_dependencies: Vec<PathBuf>,
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
    let mut native_dependencies = fs::read_dir(dir.join("native-dependencies"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    native_dependencies.sort();

    let bundle_js_path = dir.join("bundle.js");
    let compressed_bundle_path = dir.join("bundle.js.gz");
    let bundle_js = if bundle_js_path.is_file() {
        Some(fs::read_to_string(&bundle_js_path).map_err(|e| {
            format!(
                "registry package `{name}`: failed to read `{}`: {e}",
                bundle_js_path.display()
            )
        })?)
    } else if compressed_bundle_path.is_file() {
        let file = fs::File::open(&compressed_bundle_path).map_err(|error| {
            format!(
                "registry package `{name}`: failed to read `{}`: {error}",
                compressed_bundle_path.display()
            )
        })?;
        let mut source = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut source)
            .map_err(|error| {
                format!(
                    "registry package `{name}`: failed to decompress `{}`: {error}",
                    compressed_bundle_path.display()
                )
            })?;
        Some(source)
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
        native_dependencies,
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
        "util" | "sys" => {
            "export declare function inspect(argsArray: any): any;\nexport declare function format(argsArray: any): any;\nexport declare function formatWithOptions(argsArray: any): any;\nexport declare function inherits(argsArray: any): void;\nexport declare function promisify(argsArray: any): any;\nexport declare function callbackify(argsArray: any): any;\nexport declare function deprecate(argsArray: any): any;\nexport declare function stripVTControlCharacters(argsArray: any): any;\nexport declare function toUSVString(argsArray: any): any;\nexport declare function parseArgs(argsArray: any): any;\nexport declare const TextEncoder: any;\nexport declare const TextDecoder: any;\n"
        }
        "util/types" => {
            "export declare function isDate(argsArray: any): boolean;\nexport declare function isRegExp(argsArray: any): boolean;\nexport declare function isMap(argsArray: any): boolean;\nexport declare function isSet(argsArray: any): boolean;\nexport declare function isPromise(argsArray: any): boolean;\nexport declare function isArrayBuffer(argsArray: any): boolean;\nexport declare function isTypedArray(argsArray: any): boolean;\nexport declare function isNativeError(argsArray: any): boolean;\n"
        }
        "path" | "path/posix" | "path/win32" => {
            "export declare function resolve(argsArray: any): any;\nexport declare function join(argsArray: any): any;\nexport declare function dirname(argsArray: any): any;\nexport declare function basename(argsArray: any): any;\nexport declare function extname(argsArray: any): any;\nexport declare function normalize(argsArray: any): any;\nexport declare function relative(argsArray: any): any;\nexport declare function isAbsolute(argsArray: any): any;\nexport declare function parse(argsArray: any): any;\nexport declare function format(argsArray: any): any;\nexport declare function toNamespacedPath(argsArray: any): any;\n"
        }
        "process" => {
            "export declare function cwd(argsArray: any): any;\nexport declare function chdir(argsArray: any): void;\nexport declare function uptime(argsArray: any): any;\nexport declare function hrtime(argsArray: any): any;\nexport declare function memoryUsage(argsArray: any): any;\nexport declare function cpuUsage(argsArray: any): any;\nexport declare function emitWarning(argsArray: any): void;\n"
        }
        "domain" => {
            "export declare const active: any;\nexport declare const Domain: any;\nexport declare function create(argsArray: any): any;\nexport declare function createDomain(argsArray: any): any;\n"
        }
        "trace_events" => {
            "export declare const Tracing: any;\nexport declare function createTracing(argsArray: any): any;\nexport declare function getEnabledCategories(argsArray: any): string;\n"
        }
        "inspector" | "inspector/promises" => {
            "export declare const Session: any;\nexport declare function open(argsArray: any): any;\nexport declare function close(argsArray: any): void;\nexport declare function url(argsArray: any): any;\nexport declare function waitForDebugger(argsArray: any): void;\n"
        }
        "repl" => {
            "export declare const REPLServer: any;\nexport declare const Recoverable: any;\nexport declare const REPL_MODE_SLOPPY: any;\nexport declare const REPL_MODE_STRICT: any;\nexport declare function start(argsArray: any): any;\nexport declare function writer(argsArray: any): string;\n"
        }
        "cluster" => {
            "export declare const Worker: any;\nexport declare const workers: any;\nexport declare const settings: any;\nexport declare const isPrimary: boolean;\nexport declare const isMaster: boolean;\nexport declare const isWorker: boolean;\nexport declare function setupPrimary(argsArray: any): any;\nexport declare function setupMaster(argsArray: any): any;\nexport declare function fork(argsArray: any): any;\nexport declare function disconnect(argsArray: any): any;\n"
        }
        "http2" => {
            "export declare const Http2Session: any;\nexport declare const ClientHttp2Session: any;\nexport declare const ServerHttp2Session: any;\nexport declare const Http2Stream: any;\nexport declare const ClientHttp2Stream: any;\nexport declare const ServerHttp2Stream: any;\nexport declare const Http2Server: any;\nexport declare const Http2SecureServer: any;\nexport declare const constants: any;\nexport declare function connect(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\nexport declare function createSecureServer(argsArray: any): any;\nexport declare function getDefaultSettings(argsArray: any): any;\nexport declare function getPackedSettings(argsArray: any): any;\nexport declare function getUnpackedSettings(argsArray: any): any;\nexport declare function sensitiveHeaders(argsArray: any): any;\n"
        }
        "test" => {
            "export declare function test(argsArray: any): any;\nexport declare function describe(argsArray: any): any;\nexport declare function suite(argsArray: any): any;\nexport declare function it(argsArray: any): any;\nexport declare function before(argsArray: any): any;\nexport declare function after(argsArray: any): any;\nexport declare function beforeEach(argsArray: any): any;\nexport declare function afterEach(argsArray: any): any;\nexport declare function run(argsArray: any): any;\nexport declare const mock: any;\nexport declare const assert: any;\n"
        }
        "test/reporters" => {
            "export declare function dot(argsArray: any): any;\nexport declare function junit(argsArray: any): any;\nexport declare function lcov(argsArray: any): any;\nexport declare function spec(argsArray: any): any;\nexport declare function tap(argsArray: any): any;\n"
        }
        "wasi" => {
            "export declare const WASI: any;\n"
        }
        "_http_agent" | "_http_client" | "_http_common" | "_http_incoming"
        | "_http_outgoing" | "_http_server" | "_tls_common" | "_tls_wrap" => {
            "export declare const Agent: any;\nexport declare const globalAgent: any;\nexport declare const ClientRequest: any;\nexport declare const IncomingMessage: any;\nexport declare const OutgoingMessage: any;\nexport declare const Server: any;\nexport declare const ServerResponse: any;\nexport declare const HTTPParser: any;\nexport declare const SecureContext: any;\nexport declare const TLSSocket: any;\n"
        }
        "_stream_readable" | "_stream_writable" | "_stream_duplex" | "_stream_transform"
        | "_stream_passthrough" | "_stream_wrap" => {
            "export declare const Stream: any;\nexport declare const Readable: any;\nexport declare const Writable: any;\nexport declare const Duplex: any;\nexport declare const Transform: any;\nexport declare const PassThrough: any;\n"
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
            "export declare function Stream(argsArray: any): any;\nexport declare function Readable(argsArray: any): any;\nexport declare function Writable(argsArray: any): any;\nexport declare function Duplex(argsArray: any): any;\nexport declare function Transform(argsArray: any): any;\nexport declare function PassThrough(argsArray: any): any;\nexport declare function pipeline(argsArray: any): any;\nexport declare function compose(argsArray: any): any;\nexport declare function finished(argsArray: any): any;\nexport declare function addAbortSignal(argsArray: any): any;\nexport declare function isDestroyed(argsArray: any): boolean;\nexport declare function isDisturbed(argsArray: any): boolean;\nexport declare function isErrored(argsArray: any): boolean;\nexport declare function isReadable(argsArray: any): boolean;\nexport declare function isWritable(argsArray: any): boolean;\nexport declare function getDefaultHighWaterMark(argsArray: any): number;\nexport declare function setDefaultHighWaterMark(argsArray: any): void;\nexport declare function duplexPair(argsArray: any): any;\nexport declare function destroy(argsArray: any): void;\nexport declare function _isArrayBufferView(argsArray: any): boolean;\nexport declare function _isUint8Array(argsArray: any): boolean;\nexport declare function _uint8ArrayToBuffer(argsArray: any): any;\nexport declare const promises: any;\n"
        }
        "stream/promises" => {
            "export declare function pipeline(argsArray: any): any;\nexport declare function finished(argsArray: any): any;\n"
        }
        "stream/consumers" => {
            "export declare function arrayBuffer(argsArray: any): any;\nexport declare function blob(argsArray: any): any;\nexport declare function buffer(argsArray: any): any;\nexport declare function json(argsArray: any): any;\nexport declare function text(argsArray: any): any;\n"
        }
        "stream/web" => {
            "export declare const ReadableStream: any;\nexport declare const ReadableByteStreamController: any;\nexport declare const ReadableStreamBYOBReader: any;\nexport declare const ReadableStreamBYOBRequest: any;\nexport declare const WritableStream: any;\nexport declare const WritableStreamDefaultController: any;\nexport declare const TransformStream: any;\nexport declare const TransformStreamDefaultController: any;\nexport declare const TextEncoderStream: any;\nexport declare const TextDecoderStream: any;\nexport declare const CompressionStream: any;\nexport declare const DecompressionStream: any;\n"
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
            "export declare const URL: any;\nexport declare const URLSearchParams: any;\nexport declare function parse(argsArray: any): any;\nexport declare function format(argsArray: any): any;\nexport declare function pathToFileURL(argsArray: any): any;\nexport declare function fileURLToPath(argsArray: any): any;\nexport declare function urlToHttpOptions(argsArray: any): any;\n"
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
            "export interface IncomingMessage { method: string; url: string; }\nexport interface ServerResponse { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean; }\nexport interface Server { listen: (port: number) => string; __listenWithCallback: (port: number, callback: () => void) => string; listenMany: (port: number, count: number) => string; close: () => boolean; __closeWithCallback: (callback: () => void) => boolean; on: (event: string, callback: () => void) => boolean; __onError: (event: string, callback: (error: { message: string; code: string; syscall: string; address: string; port: number }) => void) => boolean; }\nexport declare function serveOnce(port: number, body: string): string;\nexport declare function serveOnceWith(port: number, callback: (target: string) => string): string;\nexport declare function createServerOnce(port: number, callback: (request: IncomingMessage, response: ServerResponse) => boolean): string;\nexport declare function createServer(callback: (request: IncomingMessage, response: ServerResponse) => boolean): Server;\n"
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
        native_dependencies: Vec::new(),
        bundle_js: Some(source.to_string()),
        version: None,
        dependency_versions: None,
    })
}
