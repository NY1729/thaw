// A tiny, hand-maintained polyfill for a Node.js core builtin module --
// *not* a real re-implementation of Node's standard library, just
// enough surface for whatever a real npm package's dependency chain
// has actually been found to touch unconditionally at load time, added
// one module (and one function) at a time the same way every other gap
// in this file was: hit a real error against a real package, fix
// exactly that. A Node builtin has no `package.json`/`node_modules`
// entry at all, so `resolve_bare_require` correctly never finds it;
// this is the fallback checked only after that lookup fails.
//
// `util`: found necessary by `qs`'s real dependency chain --
// `object-inspect` (pulled in via `side-channel`) does `require('util')`
// unconditionally at the top of `util.inspect.js`, only to read
// `.inspect`/`.inspect.custom` off the result (as a fallback/symbol
// source, not for `util.inspect`'s actual pretty-printing behavior,
// which `object-inspect` itself reimplements) -- a real
// `util.inspect`-quality implementation is unnecessary for that.

#[path = "builtins/core.rs"]
mod core;
#[path = "builtins/diagnostics.rs"]
mod diagnostics;
#[path = "builtins/filesystem.rs"]
mod filesystem;
#[path = "builtins/http.rs"]
mod http;
#[path = "builtins/network.rs"]
mod network;
#[path = "builtins/streams.rs"]
mod streams;
#[path = "builtins/system.rs"]
mod system;
#[path = "builtins/testing.rs"]
mod testing;

// One source of truth for node:module builtinModules and bundle resolve metadata.
const NODE_BUILTIN_MODULES_JSON: &str = r#"["_http_agent","_http_client","_http_common","_http_incoming","_http_outgoing","_http_server","_stream_duplex","_stream_passthrough","_stream_readable","_stream_transform","_stream_wrap","_stream_writable","_tls_common","_tls_wrap","assert","assert/strict","async_hooks","buffer","child_process","cluster","console","constants","crypto","dgram","diagnostics_channel","dns","dns/promises","domain","events","fs","fs/promises","http","http2","https","inspector","inspector/promises","module","net","os","path","path/posix","path/win32","perf_hooks","process","punycode","querystring","readline","readline/promises","repl","stream","stream/consumers","stream/promises","stream/web","string_decoder","sys","test","test/reporters","timers","timers/promises","tls","trace_events","tty","url","util","util/types","v8","vm","wasi","worker_threads","zlib"]"#;

fn builtin_module_source(name: &str) -> Option<&'static str> {
    core::source(name)
        .or_else(|| filesystem::source(name))
        .or_else(|| http::source(name))
        .or_else(|| testing::source(name))
        .or_else(|| streams::source(name))
        .or_else(|| diagnostics::source(name))
        .or_else(|| network::source(name))
        .or_else(|| system::source(name))
}
