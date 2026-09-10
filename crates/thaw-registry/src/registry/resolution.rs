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
            "export declare function inspect(value: Json, options?: Json): string;\nexport declare function format(value: Json, ...args: Json[]): string;\nexport declare function formatWithOptions(argsArray: any): any;\nexport declare function inherits(argsArray: any): void;\nexport declare function promisify(callback: (...args: any[]) => void): JsValue;\nexport declare function callbackify(callback: () => Promise<any>): (callback: (error: JsValue, value: any) => void) => void;\nexport declare function callbackify(callback: (arg: any) => Promise<any>): (arg: any, callback: (error: JsValue, value: any) => void) => void;\nexport declare function callbackify(callback: (first: any, second: any) => Promise<any>): (first: any, second: any, callback: (error: JsValue, value: any) => void) => void;\nexport declare function deprecate(argsArray: any): any;\nexport declare function stripVTControlCharacters(value: string): string;\nexport declare function toUSVString(argsArray: any): any;\nexport declare function parseArgs(argsArray: any): any;\nexport declare const TextEncoder: any;\nexport declare const TextDecoder: any;\n"
        }
        "util/types" => {
            "export declare function isDate(value: Json): boolean;\nexport declare function isRegExp(value: Json): boolean;\nexport declare function isMap(value: Json): boolean;\nexport declare function isSet(value: Json): boolean;\nexport declare function isPromise(value: Json): boolean;\nexport declare function isArrayBuffer(value: Json): boolean;\nexport declare function isTypedArray(value: Json): boolean;\nexport declare function isNativeError(value: Json): boolean;\n"
        }
        "path" | "path/posix" | "path/win32" => {
            "export interface ParsedPath { root: string; dir: string; base: string; ext: string; name: string; }\nexport declare function resolve(...paths: string[]): string;\nexport declare function join(...paths: string[]): string;\nexport declare function dirname(path: string): string;\nexport declare function basename(path: string, suffix?: string): string;\nexport declare function extname(path: string): string;\nexport declare function normalize(path: string): string;\nexport declare function relative(from: string, to: string): string;\nexport declare function isAbsolute(path: string): boolean;\nexport declare function parse(path: string): ParsedPath;\nexport declare function format(pathObject: Json): string;\nexport declare function toNamespacedPath(path: string): string;\n"
        }
        "process" => {
            "export declare const argv: any[];\nexport declare function cwd(): string;\nexport declare function chdir(directory: string): void;\nexport declare function uptime(): number;\nexport declare function hrtime(time?: any): any;\nexport declare function memoryUsage(): any;\nexport declare function cpuUsage(previousValue?: any): any;\nexport declare function emitWarning(warning: any, options?: any): void;\n"
        }
        "domain" => {
            "export declare const active: any;\nexport declare class Domain { constructor(); on(event: string, listener: (error: JsValue) => void): Domain; run(callback: () => void): void; enter(): Domain; exit(): Domain; add(emitter: JsValue): Domain; remove(emitter: JsValue): Domain; }\nexport declare function create(): Domain;\nexport declare function createDomain(): Domain;\n"
        }
        "trace_events" => {
            "export declare class Tracing { enabled: boolean; enable(): void; disable(): void; }\nexport declare function createTracing(options: any): Tracing;\nexport declare function getEnabledCategories(): string | undefined;\n"
        }
        "inspector" => {
            "export declare class Session { constructor(); connect(): void; connectToMainThread(): void; disconnect(): void; post(method: string, params: any, callback: (error: Json | null, result: JsValue) => void): void; }\nexport declare function open(argsArray: any): any;\nexport declare function close(argsArray: any): void;\nexport declare function url(argsArray: any): any;\nexport declare function waitForDebugger(argsArray: any): void;\n"
        }
        "inspector/promises" => {
            "export declare class Session { constructor(); connect(): void; connectToMainThread(): void; disconnect(): void; post(method: string, params?: any): Promise<JsValue>; }\nexport declare function open(argsArray: any): any;\nexport declare function close(argsArray: any): void;\nexport declare function url(argsArray: any): any;\nexport declare function waitForDebugger(argsArray: any): void;\n"
        }
        "repl" => {
            "export declare const REPLServer: any;\nexport declare const Recoverable: any;\nexport declare const REPL_MODE_SLOPPY: any;\nexport declare const REPL_MODE_STRICT: any;\nexport declare function start(argsArray: any): any;\nexport declare function writer(argsArray: any): string;\n"
        }
        "cluster" => {
            "export declare const Worker: any;\nexport declare const workers: any;\nexport declare const settings: any;\nexport declare const isPrimary: boolean;\nexport declare const isMaster: boolean;\nexport declare const isWorker: boolean;\nexport declare const worker: any;\nexport declare const SCHED_NONE: number;\nexport declare const SCHED_RR: number;\nexport declare let schedulingPolicy: number;\nexport declare function setupPrimary(argsArray: any): any;\nexport declare function setupMaster(argsArray: any): any;\nexport declare function fork(argsArray: any): any;\nexport declare function disconnect(argsArray: any): any;\n"
        }
        "http2" => {
            "export interface Http2Settings { headerTableSize: number; enablePush: boolean; initialWindowSize: number; maxFrameSize: number; maxHeaderListSize: number; }\nexport declare const Http2Session: any;\nexport declare const ClientHttp2Session: any;\nexport declare const ServerHttp2Session: any;\nexport declare const Http2Stream: any;\nexport declare const ClientHttp2Stream: any;\nexport declare const ServerHttp2Stream: any;\nexport declare const Http2Server: any;\nexport declare const Http2SecureServer: any;\nexport declare const constants: any;\nexport declare function connect(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\nexport declare function createSecureServer(argsArray: any): any;\nexport declare function getDefaultSettings(): Http2Settings;\nexport declare function getPackedSettings(settings: Http2Settings): JsValue;\nexport declare function getUnpackedSettings(buffer: JsValue): Http2Settings;\nexport declare function sensitiveHeaders(argsArray: any): any;\n"
        }
        "test" => {
            "export declare function test(argsArray: any): any;\nexport declare function describe(argsArray: any): any;\nexport declare function suite(argsArray: any): any;\nexport declare function it(argsArray: any): any;\nexport declare function before(argsArray: any): any;\nexport declare function after(argsArray: any): any;\nexport declare function beforeEach(argsArray: any): any;\nexport declare function afterEach(argsArray: any): any;\nexport declare function run(argsArray: any): any;\nexport declare const mock: any;\nexport declare const assert: any;\n"
        }
        "test/reporters" => {
            "export interface ReporterResult { done: boolean; value?: string; }\nexport declare class Reporter { next(): Promise<ReporterResult>; }\nexport declare function dot(source: Json): Reporter;\nexport declare function junit(source: Json): Reporter;\nexport declare function lcov(source: Json): Reporter;\nexport declare function spec(source: Json): Reporter;\nexport declare function tap(source: Json): Reporter;\n"
        }
        "wasi" => {
            "export declare class WASI { constructor(options: any); wasiImport: JsValue; getImportObject(): JsValue; start(instance: JsValue): number | undefined; initialize(instance: JsValue): void; }\n"
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
            "export declare class Buffer { toString(encoding?: string): string; }\nexport declare const ChildProcess: any;\nexport declare function spawn(command: string, args?: string[], options?: any): JsValue;\nexport declare function exec(command: string, callback: (error: Json, stdout: string, stderr: string) => void): JsValue;\nexport declare function exec(command: string, options: any, callback: (error: Json, stdout: string, stderr: string) => void): JsValue;\nexport declare function execFile(file: string, callback: (error: Json, stdout: string, stderr: string) => void): JsValue;\nexport declare function execFile(file: string, args: string[], callback: (error: Json, stdout: string, stderr: string) => void): JsValue;\nexport declare function spawnSync(argsArray: any): any;\nexport declare function execFileSync(file: string, args?: string[]): Buffer;\nexport declare function execSync(argsArray: any): any;\n"
        }
        "punycode" => {
            "export declare function encode(input: string): string;\nexport declare function decode(input: string): string;\nexport declare function toASCII(input: string): string;\nexport declare function toUnicode(input: string): string;\n"
        }
        "buffer" => {
            "export declare class Buffer { static from(value: Json, encoding?: string): Buffer; static alloc(size: number, fill?: Json, encoding?: string): Buffer; static byteLength(value: string, encoding?: string): number; static concat(values: Buffer[]): Buffer; toString(encoding?: string): string; subarray(start?: number, end?: number): Buffer; readUInt16LE(offset?: number): number; readInt32BE(offset?: number): number; writeUInt16LE(value: number, offset?: number): number; writeInt32BE(value: number, offset?: number): number; }\nexport declare const SlowBuffer: any;\nexport declare function byteLength(value: string, encoding?: string): number;\nexport declare function isUtf8(argsArray: any): any;\nexport declare function isAscii(argsArray: any): any;\nexport declare function transcode(argsArray: any): any;\n"
        }
        "string_decoder" => {
            "export declare class StringDecoder { constructor(encoding?: string); write(buffer: Json): string; end(buffer?: Json): string; }\n"
        }
        "timers" => {
            "export declare function setTimeout(callback: () => void, delay?: number): JsValue;\nexport declare function clearTimeout(handle: JsValue): void;\nexport declare function setInterval(callback: () => void, delay?: number): JsValue;\nexport declare function clearInterval(handle: JsValue): void;\nexport declare function setImmediate(callback: () => void): JsValue;\nexport declare function clearImmediate(handle: JsValue): void;\n"
        }
        "timers/promises" => {
            "export declare function setTimeout(delay: number, value: Json, options: any): Promise<Json>;\nexport declare function setTimeout(delay: number, value?: Json): Promise<Json>;\nexport declare function setImmediate(value: Json, options: any): Promise<Json>;\nexport declare function setImmediate(value?: Json): Promise<Json>;\nexport declare function setInterval(argsArray: any): any;\n"
        }
        "stream" => {
            "export declare function Stream(argsArray: any): any;\nexport declare class Readable { constructor(options?: any); static from(value: Json, options?: any): Readable; on(event: string, listener: (value: JsValue) => void): Readable; on(event: string, listener: () => void): Readable; on(event: string, listener: (...args: any[]) => void): Readable; pipe(destination: JsValue): JsValue; pause(): Readable; resume(): Readable; }\nexport interface WritableOptions { write: (chunk: JsValue, encoding: string, callback: () => void) => void; }\nexport interface DuplexOptions { read: () => void; write: (chunk: JsValue, encoding: string, callback: () => void) => void; }\nexport interface TransformOptions { transform: (chunk: JsValue, encoding: string, callback: (error?: any, data?: Json) => void) => void; }\nexport interface TransformFlushOptions { transform: (chunk: JsValue, encoding: string, callback: (error?: any, data?: Json) => void) => void; flush: (callback: (error?: any, data?: Json) => void) => void; }\nexport declare class Writable { constructor(options?: WritableOptions); on(event: string, listener: (value: JsValue) => void): Writable; on(event: string, listener: () => void): Writable; on(event: string, listener: (...args: any[]) => void): Writable; }\nexport declare class Duplex { constructor(options: DuplexOptions); on(event: string, listener: (value: JsValue) => void): Duplex; on(event: string, listener: () => void): Duplex; on(event: string, listener: (...args: any[]) => void): Duplex; push(chunk: Json): boolean; end(chunk?: Json): Duplex; }\nexport declare class Transform { constructor(); constructor(options: TransformOptions); constructor(options: TransformFlushOptions); on(event: string, listener: (value: JsValue) => void): Transform; on(event: string, listener: () => void): Transform; on(event: string, listener: (...args: any[]) => void): Transform; }\nexport declare class PassThrough { constructor(options?: any); on(event: string, listener: (value: JsValue) => void): PassThrough; on(event: string, listener: () => void): PassThrough; on(event: string, listener: (...args: any[]) => void): PassThrough; }\nexport declare function pipeline(source: JsValue, destination: JsValue, callback: (error: any) => void): void;\nexport declare function compose(argsArray: any): any;\nexport declare function finished(stream: JsValue, callback: (error: any) => void): JsValue;\nexport declare function finished(stream: JsValue, options: any, callback: (error: any) => void): JsValue;\nexport declare function addAbortSignal(argsArray: any): any;\nexport declare function isDestroyed(argsArray: any): boolean;\nexport declare function isDisturbed(argsArray: any): boolean;\nexport declare function isErrored(argsArray: any): boolean;\nexport declare function isReadable(argsArray: any): boolean;\nexport declare function isWritable(argsArray: any): boolean;\nexport declare function getDefaultHighWaterMark(argsArray: any): number;\nexport declare function setDefaultHighWaterMark(argsArray: any): void;\nexport declare function duplexPair(argsArray: any): any;\nexport declare function destroy(argsArray: any): void;\nexport declare function _isArrayBufferView(argsArray: any): boolean;\nexport declare function _isUint8Array(argsArray: any): boolean;\nexport declare function _uint8ArrayToBuffer(argsArray: any): any;\nexport declare const promises: any;\n"
        }
        "stream/promises" => {
            "export declare function pipeline(argsArray: any): any;\nexport declare function finished(stream: JsValue, options?: any): JsValue;\n"
        }
        "stream/consumers" => {
            "export declare function arrayBuffer(stream: JsValue): JsValue;\nexport declare function blob(stream: JsValue): JsValue;\nexport declare function buffer(stream: JsValue): JsValue;\nexport declare function json(stream: JsValue): JsValue;\nexport declare function text(stream: JsValue): JsValue;\n"
        }
        "stream/web" => {
            "export declare const ReadableStreamDefaultController: any;\nexport interface UnderlyingSource { start?: (controller: JsValue) => void; }\nexport declare class ReadableStream { constructor(source?: UnderlyingSource); getReader(): JsValue; }\nexport interface UnderlyingSink { write?: (chunk: Json) => void; close?: () => void; }\nexport declare class WritableStream { constructor(sink?: UnderlyingSink); getWriter(): JsValue; }\nexport interface UnderlyingTransformer { transform?: (chunk: Json, controller: JsValue) => void; }\nexport declare class TransformStream { constructor(transformer?: UnderlyingTransformer); readable: JsValue; writable: JsValue; }\nexport declare class TextEncoderStream { constructor(); readable: JsValue; writable: JsValue; }\nexport declare class TextDecoderStream { constructor(); readable: JsValue; writable: JsValue; }\nexport declare class CompressionStream { constructor(format: string); readable: JsValue; writable: JsValue; }\nexport declare class DecompressionStream { constructor(format: string); readable: JsValue; writable: JsValue; }\nexport declare const ReadableByteStreamController: any;\nexport declare const ReadableStreamBYOBReader: any;\nexport declare const ReadableStreamBYOBRequest: any;\nexport declare const WritableStreamDefaultController: any;\nexport declare const TransformStreamDefaultController: any;\n"
        }
        "readline" => {
            "export declare class Interface { constructor(options: any); on(event: string, listener: (line: string) => void): Interface; on(event: string, listener: () => void): Interface; once(event: string, listener: (line: string) => void): Interface; close(): void; pause(): Interface; resume(): Interface; question(query: string, callback: (answer: string) => void): void; }\nexport declare function createInterface(options: any): Interface;\nexport declare function clearLine(argsArray: any): boolean;\nexport declare function clearScreenDown(argsArray: any): boolean;\nexport declare function cursorTo(argsArray: any): boolean;\nexport declare function moveCursor(argsArray: any): boolean;\n"
        }
        "readline/promises" => {
            "export declare class Interface { constructor(options: any); closed: boolean; line: string; close(): void; question(query: string, options?: any): Promise<string>; }\nexport declare function createInterface(options: any): Interface;\n"
        }
        "diagnostics_channel" => {
            "export declare function channel(name: string): JsValue;\nexport declare function hasSubscribers(name: string): boolean;\nexport declare function subscribe(name: string, listener: JsValue): void;\nexport declare function unsubscribe(name: string, listener: JsValue): boolean;\nexport declare function tracingChannel(name: string): JsValue;\n"
        }
        "dns" => {
            "export declare function lookup(hostname: string, callback: (error: unknown, address: string, family: number) => void): void;\nexport declare function lookup(hostname: string, options: any, callback: (error: unknown, address: string, family: number) => void): void;\nexport declare function resolve(argsArray: any): void;\nexport declare function reverse(argsArray: any): void;\nexport declare function getDefaultResultOrder(argsArray: any): string;\nexport declare function setDefaultResultOrder(argsArray: any): void;\n"
        }
        "dns/promises" => {
            "export interface LookupAddress { address: string; family: number; }\nexport declare function lookup(hostname: string, options?: any): Promise<LookupAddress>;\nexport declare function resolve(argsArray: any): any;\nexport declare function reverse(argsArray: any): any;\n"
        }
        "dgram" => {
            "export interface SocketAddressInfo { address: string; family: string; port: number; }\nexport declare class Socket { constructor(type: string, listener?: JsValue); bind(port: number, address: string, callback: () => void): Socket; bind(port: number, callback: () => void): Socket; address(): SocketAddressInfo; close(callback?: () => void): Socket; ref(): Socket; unref(): Socket; hasRef(): boolean; }\nexport declare function createSocket(type: string, listener?: JsValue): Socket;\n"
        }
        "async_hooks" => {
            "export declare class AsyncLocalStorage { constructor(options?: any); disable(): void; getStore(): JsValue; enterWith(store: Json): void; run(store: Json, callback: () => void): void; exit(callback: () => void): void; }\nexport declare function AsyncResource(argsArray: any): any;\nexport declare function createHook(argsArray: any): any;\nexport declare function executionAsyncId(argsArray: any): any;\nexport declare function triggerAsyncId(argsArray: any): any;\nexport declare function executionAsyncResource(argsArray: any): any;\n"
        }
        "tty" => {
            "export declare function isatty(fd: number): boolean;\nexport declare function ReadStream(argsArray: any): any;\nexport declare function WriteStream(argsArray: any): any;\n"
        }
        "tls" => {
            "export declare const DEFAULT_MIN_VERSION: string;\nexport declare const DEFAULT_MAX_VERSION: string;\nexport declare const rootCertificates: string[];\nexport declare function connect(argsArray: any): any;\nexport declare function TLSSocket(argsArray: any): any;\nexport declare function createSecureContext(argsArray: any): any;\nexport declare function checkServerIdentity(argsArray: any): any;\nexport declare function getCiphers(): string[];\nexport declare function Server(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\n"
        }
        "module" => {
            "export declare function createRequire(filename: string): JsValue;\nexport declare function isBuiltin(moduleName: string): boolean;\nexport declare function syncBuiltinESMExports(argsArray: any): void;\nexport declare function findSourceMap(argsArray: any): any;\nexport declare function SourceMap(argsArray: any): any;\nexport declare function register(argsArray: any): any;\nexport declare function registerHooks(argsArray: any): any;\n"
        }
        "net" => {
            "export declare function isIP(input: string): number;\nexport declare function isIPv4(input: string): boolean;\nexport declare function isIPv6(input: string): boolean;\nexport declare function BlockList(argsArray: any): any;\nexport declare function SocketAddress(argsArray: any): any;\nexport declare function Socket(argsArray: any): any;\nexport declare function createConnection(argsArray: any): any;\nexport declare function connect(argsArray: any): any;\nexport declare function Server(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\n"
        }
        "console" => {
            "export declare class Console { constructor(stdout: JsValue, stderr?: JsValue); log(...args: any[]): void; info(...args: any[]): void; warn(...args: any[]): void; error(...args: any[]): void; }\nexport declare function log(argsArray: any): void;\nexport declare function info(argsArray: any): void;\nexport declare function warn(argsArray: any): void;\nexport declare function error(argsArray: any): void;\n"
        }
        "constants" => "export declare const F_OK: number;\nexport declare const R_OK: number;\nexport declare const W_OK: number;\nexport declare const X_OK: number;\nexport declare const O_RDONLY: number;\nexport declare const O_WRONLY: number;\nexport declare const O_RDWR: number;\n",
        "crypto" => {
            "export declare class Buffer { length: number; toString(encoding?: string): string; }\nexport declare class Hash { update(data: string): Hash; digest(encoding: string): string; copy(): Hash; }\nexport declare class Hmac { update(data: string): Hmac; digest(encoding: string): string; }\nexport declare class Cipheriv { update(data: Json, inputEncoding?: string, outputEncoding?: string): JsValue; final(outputEncoding?: string): JsValue; }\nexport declare function createHash(algorithm: string): Hash;\nexport declare function createHmac(algorithm: string, key: Json): Hmac;\nexport declare function createCipheriv(algorithm: string, key: Json, iv: Json): Cipheriv;\nexport declare function createDecipheriv(algorithm: string, key: Json, iv: Json): Cipheriv;\nexport declare function pbkdf2Sync(password: Json, salt: Json, iterations: number, keylen: number, digest: string): Buffer;\nexport declare function scrypt(password: Json, salt: Json, keylen: number, options: any, callback: (error: any, value: Buffer) => void): void;\nexport declare function scrypt(password: Json, salt: Json, keylen: number, callback: (error: any, value: Buffer) => void): void;\nexport declare function scryptSync(password: Json, salt: Json, keylen: number, options?: any): Buffer;\nexport declare function randomBytes(size: number): Buffer;\nexport declare function randomFill(argsArray: any): any;\nexport declare function randomFillSync(buffer: JsValue, offset?: number, size?: number): JsValue;\nexport declare function randomInt(max: number): number;\nexport declare function randomInt(min: number, max: number): number;\nexport declare function randomUUID(options?: Json): string;\nexport declare function timingSafeEqual(left: Json, right: Json): boolean;\nexport declare function getHashes(argsArray: any): any;\n"
        }
        "perf_hooks" => {
            "export declare const performance: any;\nexport declare function monitorEventLoopDelay(argsArray: any): any;\nexport declare function createHistogram(argsArray: any): any;\n"
        }
        "v8" => {
            "export declare function serialize(value: Json): JsValue;\nexport declare function deserialize(value: JsValue): Json;\nexport declare function getHeapStatistics(argsArray: any): any;\nexport declare function getHeapSpaceStatistics(argsArray: any): any;\nexport declare function getHeapCodeStatistics(argsArray: any): any;\nexport declare function cachedDataVersionTag(argsArray: any): number;\nexport declare function setFlagsFromString(argsArray: any): void;\n"
        }
        "vm" => {
            "export declare function Script(argsArray: any): any;\nexport declare function createContext(argsArray: any): any;\nexport declare function isContext(argsArray: any): boolean;\nexport declare function runInContext(argsArray: any): any;\nexport declare function runInNewContext(argsArray: any): any;\nexport declare function runInThisContext(argsArray: any): any;\nexport declare function compileFunction(argsArray: any): any;\nexport declare function measureMemory(argsArray: any): any;\n"
        }
        "zlib" => {
            "export declare class Buffer { length: number; toString(encoding?: string): string; }\nexport declare function gzipSync(value: Json): Buffer;\nexport declare function gunzipSync(value: Json): Buffer;\nexport declare function deflateSync(value: Json): Buffer;\nexport declare function inflateSync(value: Json): Buffer;\nexport declare function deflateRawSync(argsArray: any): any;\nexport declare function inflateRawSync(argsArray: any): any;\nexport declare function brotliCompressSync(value: Json, options?: any): Buffer;\nexport declare function brotliDecompressSync(value: Json, options?: any): Buffer;\nexport declare function gzip(argsArray: any): void;\nexport declare function gunzip(argsArray: any): void;\nexport declare function brotliCompress(argsArray: any): void;\nexport declare function brotliDecompress(argsArray: any): void;\nexport declare function createGzip(argsArray: any): any;\nexport declare function createGunzip(argsArray: any): any;\nexport declare function createDeflate(argsArray: any): any;\nexport declare function createInflate(argsArray: any): any;\nexport declare function createDeflateRaw(argsArray: any): any;\nexport declare function createInflateRaw(argsArray: any): any;\nexport declare function createBrotliCompress(argsArray: any): any;\nexport declare function createBrotliDecompress(argsArray: any): any;\n"
        }
        "worker_threads" => {
            "export declare const isMainThread: boolean;\nexport declare const threadId: number;\nexport declare const threadName: string;\nexport declare const workerData: any;\nexport declare const parentPort: any;\nexport declare class MessageChannel { constructor(); port1: JsValue; port2: JsValue; }\nexport declare const MessagePort: any;\nexport declare function Worker(argsArray: any): any;\nexport declare function receiveMessageOnPort(argsArray: any): any;\nexport declare function setEnvironmentData(argsArray: any): void;\nexport declare function getEnvironmentData(argsArray: any): any;\nexport declare function postMessageToThread(argsArray: any): Promise<void>;\n"
        }
        "os" => {
            "export declare function arch(): string;\nexport declare function platform(): string;\nexport declare function type(): string;\nexport declare function tmpdir(): string;\nexport declare function homedir(): string;\nexport declare function hostname(): string;\nexport declare function cpus(): any;\nexport declare function availableParallelism(): number;\nexport declare function totalmem(): number;\nexport declare function freemem(): number;\nexport declare function uptime(): number;\nexport declare const EOL: string;\n"
        }
        "url" => {
            "export declare class URLSearchParams { constructor(init?: string); append(name: string, value: string): void; set(name: string, value: string): void; get(name: string): string | null; getAll(name: string): string[]; }\nexport declare class URL { constructor(input: string, base?: string); hostname: string; pathname: string; searchParams: URLSearchParams; }\nexport declare function parse(url: string, parseQueryString?: boolean, slashesDenoteHost?: boolean): any;\nexport declare function format(urlObject: any, options?: any): string;\nexport declare function pathToFileURL(path: string, options?: any): JsValue;\nexport declare function fileURLToPath(url: string, options?: any): string;\nexport declare function fileURLToPath(url: JsValue, options?: any): string;\nexport declare function urlToHttpOptions(url: JsValue): any;\n"
        }
        "querystring" => {
            "export declare function stringify(object: Json, separator?: string, assignment?: string, options?: Json): string;\nexport declare function encode(object: Json, separator?: string, assignment?: string, options?: Json): string;\nexport declare function parse(text: string, separator?: string, assignment?: string, options?: Json): Json;\nexport declare function decode(text: string, separator?: string, assignment?: string, options?: Json): Json;\nexport declare function escape(value: Json): string;\nexport declare function unescape(value: Json): string;\n"
        }
        "events" => {
            "export declare class EventEmitter { constructor(options?: any); on(event: any, listener: (...args: any[]) => void): EventEmitter; addListener(event: any, listener: (...args: any[]) => void): EventEmitter; once(event: any, listener: (...args: any[]) => void): EventEmitter; off(event: any, listener: (...args: any[]) => void): EventEmitter; removeListener(event: any, listener: (...args: any[]) => void): EventEmitter; emit(event: any, ...args: any[]): boolean; listenerCount(event: any): number; removeAllListeners(event?: any): EventEmitter; }\nexport default EventEmitter;\nexport declare function once(emitter: EventEmitter, event: any, options?: any): Promise<any[]>;\nexport declare function on(emitter: EventEmitter, event: any, options?: any): AsyncIterable<any[]>;\nexport declare function getEventListeners(emitter: EventEmitter, event: any): any[];\nexport declare function getMaxListeners(emitter: EventEmitter): number;\nexport declare function setMaxListeners(count: number, emitter: EventEmitter): void;\n"
        }
        "assert" | "assert/strict" => {
            "export declare function ok(value: Json, message?: string): void;\nexport declare function equal(actual: Json, expected: Json, message?: string): void;\nexport declare function notEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function strictEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function notStrictEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function deepEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function notDeepEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function deepStrictEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function notDeepStrictEqual(actual: Json, expected: Json, message?: string): void;\nexport declare function fail(message?: string): void;\nexport declare function throws(argsArray: any): any;\nexport declare function doesNotThrow(argsArray: any): void;\n"
        }
        "fs" => {
            "export declare class Stats { size: number; isFile(): boolean; isDirectory(): boolean; isSymbolicLink(): boolean; }\nexport declare function existsSync(path: string): boolean;\nexport declare function readFileSync(path: string, encoding: string): string;\nexport declare function writeFileSync(path: string, data: string): boolean;\nexport declare function appendFileSync(path: string, data: string): any;\nexport declare function mkdirSync(path: string, options?: any): boolean;\nexport declare function readdirSync(path: string): string[];\nexport declare function readdirSync(path: string, options: any): any;\nexport declare function statSync(path: string): Stats;\nexport declare function lstatSync(path: string): Stats;\nexport declare function unlinkSync(path: string): any;\nexport declare function rmSync(path: string, options?: any): any;\nexport declare function rmdirSync(path: string, options?: any): any;\nexport declare function renameSync(path: string, destination: string): any;\nexport declare function copyFileSync(path: string, destination: string): any;\nexport declare function cpSync(path: string, destination: string, options?: any): any;\nexport declare function realpathSync(path: string): any;\nexport declare function mkdtempSync(path: string): any;\nexport declare function mkdtempDisposableSync(path: string, options?: any): any;\nexport declare function openAsBlob(path: string, options?: any): Promise<any>;\nexport declare function linkSync(path: string, destination: string): any;\nexport declare function symlinkSync(path: string, destination: string): any;\nexport declare function readlinkSync(path: string): any;\nexport declare function chmodSync(path: string, mode: number): any;\nexport declare function createReadStream(path: string, options?: any): JsValue;\nexport declare function createWriteStream(path: string, options?: any): JsValue;\n"
        }
        "fs/promises" => {
            "export declare function access(path: string, mode?: number): Promise<void>;\nexport declare function open(argsArray: any): any;\nexport declare function readFile(path: string, encoding: string): Promise<string>;\nexport declare function readdir(path: string): Promise<string[]>;\nexport declare function readdir(path: string, options: any): Promise<any>;\nexport declare function stat(path: string, options?: any): JsValue;\nexport declare function lstat(path: string, options?: any): JsValue;\nexport declare function writeFile(path: string, data: Json, options?: any): Promise<void>;\nexport declare function appendFile(path: string, data: Json, options?: any): Promise<void>;\nexport declare function mkdir(path: string, options?: any): Promise<any>;\nexport declare function unlink(path: string): Promise<void>;\nexport declare function rm(path: string, options?: any): Promise<void>;\nexport declare function rmdir(path: string, options?: any): Promise<void>;\nexport declare function rename(path: string, destination: string): Promise<void>;\nexport declare function copyFile(path: string, destination: string, mode?: number): Promise<void>;\nexport declare function realpath(argsArray: any): any;\nexport declare function mkdtemp(argsArray: any): any;\nexport declare function mkdtempDisposable(argsArray: any): any;\nexport declare function link(argsArray: any): any;\nexport declare function symlink(argsArray: any): any;\nexport declare function readlink(argsArray: any): any;\nexport declare function chmod(argsArray: any): any;\n"
        }
        "http" => {
            "export interface IncomingMessage { method: string; url: string; statusCode: number; on(event: string, callback: (value: JsValue) => void): IncomingMessage; }\nexport interface ServerResponse { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean; }\nexport interface Server { listen: (port: number) => string; __listenWithCallback: (port: number, callback: () => void) => string; listenMany: (port: number, count: number) => string; close: () => boolean; __closeWithCallback: (callback: () => void) => boolean; on: (event: string, callback: () => void) => boolean; __onError: (event: string, callback: (error: { message: string; code: string; syscall: string; address: string; port: number }) => void) => boolean; }\nexport declare function serveOnce(port: number, body: string): string;\nexport declare function serveOnceWith(port: number, callback: (target: string) => string): string;\nexport declare function createServerOnce(port: number, callback: (request: IncomingMessage, response: ServerResponse) => void): string;\nexport declare function createServer(callback: (request: IncomingMessage, response: ServerResponse) => void): Server;\nexport declare const get: JsValue;\nexport declare const request: JsValue;\n"
        }
        "https" => {
            "export declare function request(argsArray: any): any;\nexport declare function get(argsArray: any): any;\nexport declare function createServer(argsArray: any): any;\nexport declare const ClientRequest: any;\nexport declare const IncomingMessage: any;\nexport declare const ServerResponse: any;\nexport declare const Server: any;\nexport declare const globalAgent: any;\n"
        }
        _ => return Err(format!("unsupported Node built-in module `{specifier}`")),
    };
    builtin_module_source(name)
        .ok_or_else(|| format!("unsupported Node built-in module `{specifier}`"))?;
    Ok(ResolvedPackage {
        name: format!("node:{name}"),
        dts_source: format!(
            "{dts_source}{}",
            match name {
                "fs" => "export declare function accessSync(path: string, mode?: number): void;\nexport declare function opendirSync(path: string, options?: any): any;\nexport declare function utimesSync(path: string, atime: any, mtime: any): any;\nexport declare function chownSync(path: string, uid: number, gid: number): any;\nexport declare function lchownSync(path: string, uid: number, gid: number): any;\nexport declare function watchFile(path: string, options: any, listener?: any): any;\nexport declare function unwatchFile(path: string, listener?: any): void;\nexport declare function watch(path: string, options?: any, listener?: any): any;\nexport declare function globSync(pattern: string | string[], options?: any): any[];\nexport declare function glob(pattern: string | string[], options: any, callback?: any): void;\nexport declare function openSync(path: string, flags: string, mode?: number): number;\nexport declare function closeSync(fd: number): void;\nexport declare function readSync(fd: number, buffer: any, offset: number, length: number, position: number | null): number;\nexport declare function writeSync(fd: number, data: any, offset?: any, length?: any, position?: any): number;\nexport declare function fstatSync(fd: number): any;\nexport declare function ftruncateSync(fd: number, length?: number): void;\nexport declare function readvSync(fd: number, buffers: any[], position?: number | null): number;\nexport declare function writevSync(fd: number, buffers: any[], position?: number | null): number;\nexport declare function statfsSync(path: string, options?: any): any;\nexport declare function lutimesSync(path: string, atime: any, mtime: any): void;\nexport declare function truncateSync(path: string, length?: number): void;\n",
                "fs/promises" => "export declare function opendir(argsArray: any): any;\nexport declare function cp(source: string, destination: string, options?: any): Promise<void>;\nexport declare function utimes(argsArray: any): any;\nexport declare function lutimes(argsArray: any): any;\nexport declare function chown(argsArray: any): any;\nexport declare function lchown(argsArray: any): any;\nexport declare function glob(pattern: string | string[], options?: any): AsyncIterable<any>;\nexport declare function watch(path: string, options?: any): AsyncIterable<any>;\nexport declare function statfs(path: string, options?: any): Promise<any>;\nexport declare function truncate(path: string, length?: number): Promise<void>;\n",
                _ => "",
            }
        ),
        native_lib: None,
        native_addon: None,
        native_dependencies: Vec::new(),
        bundle_js: Some(bundle_builtin_module(name)?),
        version: None,
        dependency_versions: None,
    })
}
