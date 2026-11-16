// Node 22 `process` members that are plain JS: `ref`/`unref`, `assert`, `openStdin`, `getBuiltinModule` and the
// `allowedNodeEnvironmentFlags` accessor. Runs in the host realm after `install_process_stdin` (process_members.rs).
(function () {
  var proc = globalThis.process;
  function def(name, fn) {
    Object.defineProperty(proc, name, { value: fn, writable: true, enumerable: true, configurable: true });
  }
  // Node's flag list (`[...process.allowedNodeEnvironmentFlags]` of v22.23.1), frozen here.
  var FLAGS = [
  "--heap-prof", "--no-heap-prof", "--test-coverage-include", "--cpu-prof-dir", "--report-on-signal",
  "--no-report-on-signal", "--heap-prof-interval", "--cpu-prof-interval", "--cpu-prof", "--no-cpu-prof",
  "--tls-max-v1.3", "--no-tls-max-v1.3", "--trace-tls", "--no-trace-tls", "--permission", "--no-permission",
  "--experimental-loader", "--experimental-addon-modules", "--no-experimental-addon-modules",
  "--experimental-report", "--experimental-modules", "--test-skip-pattern", "--use-env-proxy",
  "--no-use-env-proxy", "--test-coverage-functions", "--heapsnapshot-signal", "--test-coverage-exclude",
  "--entry-url", "--no-entry-url", "--trace-atomics-wait", "--no-trace-atomics-wait",
  "--experimental-detect-module", "--no-experimental-detect-module", "--allow-addons", "--no-allow-addons",
  "--experimental-global-webcrypto", "--no-experimental-global-webcrypto", "--http-parser",
  "--experimental-fetch", "--no-experimental-fetch", "--trace-promises", "--no-trace-promises",
  "--experimental-abortcontroller", "--disable-sigusr1", "--no-disable-sigusr1", "--allow-wasi",
  "--no-allow-wasi", "--expose-gc", "--diagnostic-dir", "--force-async-hooks-checks",
  "--no-force-async-hooks-checks", "--experimental-sqlite", "--no-experimental-sqlite", "--allow-fs-read",
  "--preserve-symlinks-main", "--no-preserve-symlinks-main", "--allow-fs-write", "--allow-child-process",
  "--no-allow-child-process", "--experimental-async-context-frame", "--no-experimental-async-context-frame",
  "--experimental-vm-modules", "--no-experimental-vm-modules", "--inspect-publish-uid", "--inspect-brk",
  "--no-inspect-brk", "--inspect", "--no-inspect", "--inspect-port", "--report-uncaught-exception",
  "--no-report-uncaught-exception", "--cpu-prof-name", "--experimental-top-level-await",
  "--track-heap-objects", "--no-track-heap-objects", "--jitless", "--input-type", "--enable-source-maps",
  "--no-enable-source-maps", "--experimental-transform-types", "--no-experimental-transform-types",
  "--inspect-wait", "--no-inspect-wait", "--perf-prof", "--max-old-space-size-percentage", "--test-only",
  "--no-test-only", "--dns-result-order", "--network-family-autoselection",
  "--no-network-family-autoselection", "--throw-deprecation", "--no-throw-deprecation",
  "--trace-deprecation", "--no-trace-deprecation", "--heap-prof-dir", "--experimental-default-type",
  "--trace-env-native-stack", "--no-trace-env-native-stack", "--report-exclude-network",
  "--no-report-exclude-network", "--allow-worker", "--no-allow-worker", "--secure-heap-min",
  "--abort-on-uncaught-exception", "--disable-wasm-trap-handler", "--no-disable-wasm-trap-handler",
  "--snapshot-blob", "--unhandled-rejections", "--watch", "--no-watch", "--experimental-json-modules",
  "--openssl-shared-config", "--no-openssl-shared-config", "--conditions", "--watch-path",
  "--tls-cipher-list", "--disable-proto", "--zero-fill-buffers", "--no-zero-fill-buffers",
  "--experimental-webstorage", "--no-experimental-webstorage", "--openssl-legacy-provider",
  "--no-openssl-legacy-provider", "--v8-pool-size", "--force-fips", "--no-force-fips",
  "--experimental-shadow-realm", "--no-experimental-shadow-realm", "--max-old-space-size",
  "--enable-etw-stack-walking", "--perf-basic-prof", "--preserve-symlinks", "--no-preserve-symlinks",
  "--test-reporter-destination", "--trace-env-js-stack", "--no-trace-env-js-stack",
  "--trace-event-file-pattern", "--trace-event-categories", "--trace-env", "--no-trace-env",
  "--test-coverage-lines", "--experimental-strip-types", "--no-experimental-strip-types", "--napi-modules",
  "--network-family-autoselection-attempt-timeout", "--report-dir", "--addons", "--no-addons",
  "--node-memory-debug", "--experimental-specifier-resolution", "--trace-sync-io", "--no-trace-sync-io",
  "--debug-arraybuffer-allocations", "--no-debug-arraybuffer-allocations", "--report-compact",
  "--no-report-compact", "--disallow-code-generation-from-strings", "--title", "--trace-exit",
  "--no-trace-exit", "--tls-min-v1.3", "--no-tls-min-v1.3", "--experimental-websocket",
  "--no-experimental-websocket", "--enable-fips", "--no-enable-fips", "--report-on-fatalerror",
  "--no-report-on-fatalerror", "--huge-max-old-generation-size", "--node-snapshot", "--no-node-snapshot",
  "--icu-data-dir", "--report-filename", "--interpreted-frames-native-stack", "--trace-require-module",
  "--experimental-import-meta-resolve", "--no-experimental-import-meta-resolve",
  "--experimental-wasm-modules", "--use-openssl-ca", "--no-use-openssl-ca",
  "--perf-basic-prof-only-functions", "--experimental-print-required-tla",
  "--no-experimental-print-required-tla", "--experimental-worker", "--use-bundled-ca", "--no-use-bundled-ca",
  "--test-shard", "--experimental-global-navigator", "--no-experimental-global-navigator", "--tls-min-v1.1",
  "--no-tls-min-v1.1", "--use-system-ca", "--no-use-system-ca", "--warnings", "--no-warnings",
  "--test-name-pattern", "--verify-base-objects", "--no-verify-base-objects", "--use-largepages",
  "--experimental-eventsource", "--no-experimental-eventsource", "--test-reporter", "--secure-heap",
  "--experimental-repl-await", "--no-experimental-repl-await", "--openssl-config", "--trace-uncaught",
  "--no-trace-uncaught", "--trace-warnings", "--no-trace-warnings", "--watch-kill-signal",
  "--global-search-paths", "--no-global-search-paths", "--experimental-global-customevent",
  "--no-experimental-global-customevent", "--test-coverage-branches", "--trace-sigint", "--no-trace-sigint",
  "--import", "--localstorage-file", "--redirect-warnings", "--max-http-header-size",
  "--experimental-require-module", "--no-experimental-require-module", "--heap-prof-name",
  "--frozen-intrinsics", "--no-frozen-intrinsics", "--experimental-wasi-unstable-preview1",
  "--watch-preserve-output", "--no-watch-preserve-output", "--insecure-http-parser",
  "--no-insecure-http-parser", "--force-node-api-uncaught-exceptions-policy",
  "--no-force-node-api-uncaught-exceptions-policy", "--report-exclude-env", "--no-report-exclude-env",
  "--require", "--tls-keylog", "--pending-deprecation", "--no-pending-deprecation", "--tls-min-v1.2",
  "--no-tls-min-v1.2", "--tls-max-v1.2", "--no-tls-max-v1.2", "--stack-trace-limit",
  "--extra-info-on-fatal-exception", "--no-extra-info-on-fatal-exception", "--disable-warning",
  "--heapsnapshot-near-heap-limit", "--max-semi-space-size", "--force-context-aware",
  "--no-force-context-aware", "--perf-prof-unwinding-info", "--tls-min-v1.0", "--no-tls-min-v1.0",
  "--deprecation", "--no-deprecation", "--report-signal", "--prof-process",
  "--enable-network-family-autoselection", "-C", "--loader", "--trace-events-enabled", "--debug-port",
  "--es-module-specifier-resolution", "--report-directory", "-r", "--experimental-permission",
  ];
  class NodeEnvironmentFlagsSet extends Set {
    constructor(list) {
      super();
      for (var i = 0; i < list.length; i++) Set.prototype.add.call(this, list[i]);
    }
    // read-only: mutation is accepted and ignored, as in Node
    add() { return this; }
    delete() { return false; }
    clear() {}
  }
  var flagSet = null;
  Object.defineProperty(proc, 'allowedNodeEnvironmentFlags', {
    get: function () { return flagSet || (flagSet = new NodeEnvironmentFlagsSet(FLAGS)); },
    set: function (value) {
      Object.defineProperty(proc, 'allowedNodeEnvironmentFlags', { value: value, writable: true, enumerable: true, configurable: true });
    },
    enumerable: true,
    configurable: true,
  });

  // The builtin ids `require.resolve` answers for (Node 22 `module.builtinModules`); anything else is not a builtin.
  var BUILTINS = new Set(["_http_agent", "_http_client", "_http_common", "_http_incoming", "_http_outgoing", "_http_server", "_stream_duplex", "_stream_passthrough", "_stream_readable", "_stream_transform", "_stream_wrap", "_stream_writable", "_tls_common", "_tls_wrap", "assert", "assert/strict", "async_hooks", "buffer", "child_process", "cluster", "console", "constants", "crypto", "dgram", "diagnostics_channel", "dns", "dns/promises", "domain", "events", "fs", "fs/promises", "http", "http2", "https", "inspector", "inspector/promises", "module", "net", "os", "path", "path/posix", "path/win32", "perf_hooks", "process", "punycode", "querystring", "readline", "readline/promises", "repl", "stream", "stream/consumers", "stream/promises", "stream/web", "string_decoder", "sys", "timers", "timers/promises", "tls", "trace_events", "tty", "url", "util", "util/types", "v8", "vm", "wasi", "worker_threads", "zlib"]);
  def('getBuiltinModule', function getBuiltinModule(id) {
    var name = String(id).replace(/^node:/, '');
    if (!BUILTINS.has(name) || typeof globalThis.require !== 'function') return undefined;
    try { return globalThis.require(name); } catch (_) { return undefined; }
  });

  // as Node's lib/internal/process/per_thread: a resource's own `ref`/`unref` (or the nodejs symbol method) is called
  def('ref', function ref(maybeRefable) {
    if (maybeRefable == null) return;
    var fn = maybeRefable[Symbol.for('nodejs.ref')] || maybeRefable.ref;
    if (typeof fn === 'function') fn.call(maybeRefable);
  });
  def('unref', function unref(maybeRefable) {
    if (maybeRefable == null) return;
    var fn = maybeRefable[Symbol.for('nodejs.unref')] || maybeRefable.unref;
    if (typeof fn === 'function') fn.call(maybeRefable);
  });

  // `process.assert` is `deprecated` (DEP0100) around `assert`; `prototype.constructor` is the inner `assert`, as in Node.
  var assertWarned = false;
  function assert(value, message) {
    if (value) return;
    if (message instanceof Error) throw message;
    var e = new Error(message === undefined ? 'assertion error' : String(message));
    e.code = 'ERR_ASSERTION';
    throw e;
  }
  def('assert', function deprecated(...args) {
    if (!assertWarned && !proc.noDeprecation) {
      assertWarned = true;
      proc.emitWarning('process.assert() is deprecated. Please use the `assert` module instead.', 'DeprecationWarning', 'DEP0100');
    }
    return assert.apply(undefined, args);
  });
  proc.assert.prototype.constructor = assert;

  def('openStdin', function () {
    proc.stdin.resume();
    return proc.stdin;
  });
})();
