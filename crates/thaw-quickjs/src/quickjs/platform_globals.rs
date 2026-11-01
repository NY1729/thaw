const PLATFORM_GLOBALS: &str = concat!(
    include_str!("platform_globals/webassembly.js"),
    include_str!("platform_globals/json_errors.js"),
    include_str!("platform_globals/runtime.js"),
    include_str!("platform_globals/async_hooks.js"),
    include_str!("platform_globals/dates.js"),
    include_str!("platform_globals/atomics.js"),
    include_str!("platform_globals/performance.js"),
    include_str!("platform_globals/workers/structured_clone.js"),
    include_str!("platform_globals/workers/url.js"),
    include_str!("platform_globals/workers/blob_events.js"),
    include_str!("platform_globals/workers/fetch_streams.js"),
    include_str!("platform_globals/workers/messaging.js"),
    include_str!("platform_globals/workers/abort_timers.js"),
    include_str!("platform_globals/text_encoding.js"),
    include_str!("platform_globals/buffer_crypto.js"),
    include_str!("platform_globals/intl.js"),
    // Node enumerates only these globals; every other installed one (console, internals) is hidden.
    "\n;(function () {
  var keep = { global: 1, clearImmediate: 1, setImmediate: 1, clearInterval: 1, clearTimeout: 1,
    setInterval: 1, setTimeout: 1, queueMicrotask: 1, structuredClone: 1, atob: 1, btoa: 1,
    performance: 1, fetch: 1, navigator: 1, crypto: 1 };
  Object.keys(globalThis).forEach(function (key) {
    var descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    if (!keep[key] && descriptor && descriptor.configurable) {
      Object.defineProperty(globalThis, key, { enumerable: false });
    }
  });
})();\n",
);
