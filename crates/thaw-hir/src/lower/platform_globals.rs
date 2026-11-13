//! Standard globals that thaw-hir resolves by name, one table.
//!
//! A bare reference to one of these names (value position, `typeof`, `new`,
//! or a TypeScript annotation) lowers to the realm's real global instead of
//! an "unknown variable" error. `PLATFORM_GLOBALS` is generated from Node 22's
//! own global object (`tools/parity/node_globals.json`, see
//! `tools/parity/README.md`) minus the ECMAScript intrinsics that
//! `is_builtin_prototype_owner` already covers, minus `KNOWN_UNSUPPORTED`
//! and `NODE_ONLY_NOT_GLOBAL`. Whether thaw-js actually provides a name is
//! tracked separately by `ABSENT_IN_THAW_JS` in `crates/thaw-js`.

/// How a global is spelled as a value. Value, namespace, and intrinsic-call
/// names all lower to `getDynamicValue(name)`; `Ctor` names are also valid
/// `new` targets and TypeScript annotations (opaque `JsValue`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalKind {
    /// A function or plain value (`fetch`, `structuredClone`, `process.nextTick`-style).
    Value,
    /// A capitalised constructor (`URL`, `WebSocket`, `Headers`).
    Ctor,
    /// A namespace object (`Math`, `JSON`, `crypto`, `navigator`).
    Namespace,
    /// Also a call-position intrinsic in calls.rs (`atob(x)` / `btoa(x)`);
    /// the bare value form is still `getDynamicValue`.
    IntrinsicCall,
}

/// The lookup behind every standard-global check in thaw-hir.
pub fn platform_global(name: &str) -> Option<GlobalKind> {
    PLATFORM_GLOBALS
        .iter()
        .find_map(|&(global, kind)| (global == name).then_some(kind))
}

/// Interface / alias names with no runtime value (`RequestInit`, `BodyInit`),
/// plus the ECMAScript buffer and typed-array type names. Only consulted for
/// TypeScript annotations (`is_opaque_global_type`).
pub const PLATFORM_GLOBALS: &[(&str, GlobalKind)] = &[
    ("AbortController", GlobalKind::Ctor),
    ("AbortSignal", GlobalKind::Ctor),
    ("Atomics", GlobalKind::Namespace),
    ("Blob", GlobalKind::Ctor),
    ("BroadcastChannel", GlobalKind::Ctor),
    ("Buffer", GlobalKind::Ctor),
    ("ByteLengthQueuingStrategy", GlobalKind::Ctor),
    ("CompressionStream", GlobalKind::Ctor),
    ("CountQueuingStrategy", GlobalKind::Ctor),
    ("Crypto", GlobalKind::Ctor),
    ("CryptoKey", GlobalKind::Ctor),
    ("CustomEvent", GlobalKind::Ctor),
    ("DOMException", GlobalKind::Ctor),
    ("DecompressionStream", GlobalKind::Ctor),
    ("Event", GlobalKind::Ctor),
    ("EventTarget", GlobalKind::Ctor),
    ("File", GlobalKind::Ctor),
    ("FinalizationRegistry", GlobalKind::Ctor),
    ("FormData", GlobalKind::Ctor),
    ("Headers", GlobalKind::Ctor),
    ("Intl", GlobalKind::Namespace),
    ("JSON", GlobalKind::Namespace),
    ("Math", GlobalKind::Namespace),
    ("MessageChannel", GlobalKind::Ctor),
    ("MessageEvent", GlobalKind::Ctor),
    ("MessagePort", GlobalKind::Ctor),
    ("Navigator", GlobalKind::Ctor),
    ("Performance", GlobalKind::Ctor),
    ("PerformanceEntry", GlobalKind::Ctor),
    ("PerformanceMark", GlobalKind::Ctor),
    ("PerformanceMeasure", GlobalKind::Ctor),
    ("PerformanceObserver", GlobalKind::Ctor),
    ("PerformanceObserverEntryList", GlobalKind::Ctor),
    ("PerformanceResourceTiming", GlobalKind::Ctor),
    ("Proxy", GlobalKind::Ctor),
    ("ReadableByteStreamController", GlobalKind::Ctor),
    ("ReadableStream", GlobalKind::Ctor),
    ("ReadableStreamBYOBReader", GlobalKind::Ctor),
    ("ReadableStreamBYOBRequest", GlobalKind::Ctor),
    ("ReadableStreamDefaultController", GlobalKind::Ctor),
    ("ReadableStreamDefaultReader", GlobalKind::Ctor),
    ("Reflect", GlobalKind::Namespace),
    ("Request", GlobalKind::Ctor),
    ("Response", GlobalKind::Ctor),
    ("SubtleCrypto", GlobalKind::Ctor),
    ("TextDecoder", GlobalKind::Ctor),
    ("TextDecoderStream", GlobalKind::Ctor),
    ("TextEncoder", GlobalKind::Ctor),
    ("TextEncoderStream", GlobalKind::Ctor),
    ("TransformStream", GlobalKind::Ctor),
    ("TransformStreamDefaultController", GlobalKind::Ctor),
    ("URL", GlobalKind::Ctor),
    ("URLSearchParams", GlobalKind::Ctor),
    ("WeakRef", GlobalKind::Ctor),
    ("WebAssembly", GlobalKind::Namespace),
    ("WebSocket", GlobalKind::Ctor),
    ("WritableStream", GlobalKind::Ctor),
    ("WritableStreamDefaultController", GlobalKind::Ctor),
    ("WritableStreamDefaultWriter", GlobalKind::Ctor),
    ("atob", GlobalKind::IntrinsicCall),
    ("btoa", GlobalKind::IntrinsicCall),
    ("clearImmediate", GlobalKind::Value),
    ("clearInterval", GlobalKind::Value),
    ("clearTimeout", GlobalKind::Value),
    ("console", GlobalKind::Namespace),
    ("crypto", GlobalKind::Namespace),
    ("decodeURI", GlobalKind::Value),
    ("decodeURIComponent", GlobalKind::Value),
    ("encodeURI", GlobalKind::Value),
    ("encodeURIComponent", GlobalKind::Value),
    ("escape", GlobalKind::Value),
    ("fetch", GlobalKind::Value),
    ("globalThis", GlobalKind::Namespace),
    ("isFinite", GlobalKind::Value),
    ("isNaN", GlobalKind::Value),
    ("navigator", GlobalKind::Namespace),
    ("parseFloat", GlobalKind::Value),
    ("parseInt", GlobalKind::Value),
    ("performance", GlobalKind::Namespace),
    ("process", GlobalKind::Namespace),
    ("queueMicrotask", GlobalKind::Value),
    ("setImmediate", GlobalKind::Value),
    ("setInterval", GlobalKind::Value),
    ("setTimeout", GlobalKind::Value),
    ("structuredClone", GlobalKind::Value),
    ("unescape", GlobalKind::Value),
    ("DisposableStack", GlobalKind::Ctor),
    ("AsyncDisposableStack", GlobalKind::Ctor),
];

pub const TYPE_ONLY_NAMES: &[&str] = &[
    "ArrayBuffer",
    "ArrayBufferLike",
    "ArrayBufferView",
    "BigInt64Array",
    "BigUint64Array",
    "BlobPart",
    "BodyInit",
    "BufferSource",
    "CloseEvent",
    "CryptoKeyPair",
    "DOMHighResTimeStamp",
    "DataView",
    "Document",
    "ErrorEvent",
    "EventInit",
    "FileList",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "FormDataEntryValue",
    "HeadersInit",
    "ImageData",
    "Int16Array",
    "Int32Array",
    "Int8Array",
    "Location",
    "MessageEventInit",
    "ProgressEvent",
    "ReadableStreamReadResult",
    "RequestInfo",
    "RequestInit",
    "RequestRedirect",
    "ResponseInit",
    "ResponseType",
    "SharedArrayBuffer",
    "SharedWorker",
    "Storage",
    "StructuredSerializeOptions",
    "Transferable",
    "Uint16Array",
    "Uint32Array",
    "Uint8ClampedArray",
    "Window",
    "Worker",
    "XMLHttpRequest",
];

pub const NODE_ONLY_NOT_GLOBAL: &[(&str, &str)] = &[
    ("__dirname", "CommonJS module-wrapper binding; a script or ESM program has no such global (thaw: commonjs_global_error)"),
    ("__filename", "CommonJS module-wrapper binding; a script or ESM program has no such global (thaw: commonjs_global_error)"),
    ("exports", "CommonJS module-wrapper binding; a script or ESM program has no such global (thaw: commonjs_global_error)"),
    ("module", "CommonJS module-wrapper binding; a script or ESM program has no such global (thaw: commonjs_global_error)"),
    ("require", "CommonJS module-wrapper binding; a script or ESM program has no such global (thaw: commonjs_global_error)"),
    ("assert", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("async_hooks", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("buffer", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("child_process", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("cluster", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("constants", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("dgram", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("diagnostics_channel", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("dns", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("domain", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("events", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("fs", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("http", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("http2", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("https", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("inspector", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("net", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("os", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("path", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("perf_hooks", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("punycode", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("querystring", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("readline", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("repl", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("stream", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("string_decoder", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("sys", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("timers", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("tls", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("trace_events", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("tty", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("url", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("util", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("v8", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("vm", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("wasi", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("worker_threads", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
    ("zlib", "builtin module exposed only by the node REPL/-e, not a global of a script or ESM program"),
];

/// Names in `node_globals.json` that thaw deliberately does not expose as
/// a global, each with the reason.
pub const KNOWN_UNSUPPORTED: &[(&str, &str)] = &[
    ("eval", "thaw compiles ahead of time and has no runtime code evaluation; a bare `eval` value would be unusable"),
    ("localStorage", "Node 22 has no localStorage global without --experimental-webstorage (ReferenceError); a compile error matches it"),
    ("sessionStorage", "Node 22 has no sessionStorage global without --experimental-webstorage (ReferenceError); a compile error matches it"),
];

/// Lowered and typed like a platform global, but absent from Node 22's own
/// global object, so the `node_globals.json` cross-check does not expect them.
pub const NOT_IN_NODE22: &[(&str, &str)] = &[
    ("DisposableStack", "explicit-resource-management constructor (not in Node 22); kept with the other `new`-position constructors"),
    ("AsyncDisposableStack", "explicit-resource-management constructor (not in Node 22); kept with the other `new`-position constructors"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// `node_globals.json` is `[[name, typeof], ...]`; the names are the
    /// even-indexed strings once the array is split on `"`.
    fn node_global_names() -> Vec<&'static str> {
        let json = include_str!("../../../../tools/parity/node_globals.json");
        let strings: Vec<&str> = json
            .split('"')
            .enumerate()
            .filter(|(index, _)| index % 2 == 1)
            .map(|(_, text)| text)
            .collect();
        strings.chunks(2).map(|pair| pair[0]).collect()
    }

    fn lookup<'a>(table: &[(&'a str, &str)], name: &str) -> Option<&'a str> {
        table.iter().find(|(n, _)| *n == name).map(|(n, _)| *n)
    }

    #[test]
    fn every_node_global_is_classified() {
        let names = node_global_names();
        assert!(
            names.len() > 100,
            "node_globals.json parsed as {} names",
            names.len()
        );
        let mut unclassified = Vec::new();
        for name in &names {
            let classified = platform_global(name).is_some()
                || crate::lower::is_builtin_prototype_owner(name)
                || matches!(*name, "NaN" | "Infinity" | "undefined" | "global")
                || lookup(NODE_ONLY_NOT_GLOBAL, name).is_some()
                || lookup(KNOWN_UNSUPPORTED, name).is_some();
            if !classified {
                unclassified.push(*name);
            }
        }
        assert!(
            unclassified.is_empty(),
            "Node 22 globals neither lowered nor listed as unsupported: {unclassified:?}"
        );
    }

    #[test]
    fn every_table_entry_is_a_node_global_or_documented() {
        let names = node_global_names();
        for (name, _) in PLATFORM_GLOBALS {
            assert!(
                names.contains(name) || lookup(NOT_IN_NODE22, name).is_some(),
                "`{name}` is in PLATFORM_GLOBALS but not in node_globals.json or NOT_IN_NODE22"
            );
        }
    }

    #[test]
    fn exception_lists_do_not_overlap_the_table() {
        for (name, reason) in KNOWN_UNSUPPORTED.iter().chain(NODE_ONLY_NOT_GLOBAL) {
            assert!(
                platform_global(name).is_none(),
                "`{name}` is listed as unsupported and also lowered"
            );
            assert!(!reason.is_empty(), "`{name}` needs a reason");
        }
        for (name, reason) in NOT_IN_NODE22 {
            assert!(
                platform_global(name).is_some(),
                "`{name}` is documented but not in PLATFORM_GLOBALS"
            );
            assert!(!reason.is_empty(), "`{name}` needs a reason");
        }
    }

    #[test]
    fn value_and_intrinsic_kinds_match_their_roles() {
        assert_eq!(platform_global("atob"), Some(GlobalKind::IntrinsicCall));
        assert_eq!(platform_global("btoa"), Some(GlobalKind::IntrinsicCall));
        assert_eq!(platform_global("WebSocket"), Some(GlobalKind::Ctor));
        assert_eq!(platform_global("Math"), Some(GlobalKind::Namespace));
        assert_eq!(platform_global("fetch"), Some(GlobalKind::Value));
        assert_eq!(platform_global("localStorage"), None);
        assert_eq!(platform_global("require"), None);
    }
}
