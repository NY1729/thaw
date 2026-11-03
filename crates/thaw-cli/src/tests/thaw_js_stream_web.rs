// node:stream/web is allowlisted for thaw-js routing. Kept separate from the pinned allowlist test:
// `node:stream` itself stays on QuickJS.

#[test]
fn thaw_js_stream_web_is_allowlisted() {
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:stream/web"));
    assert!(!THAW_JS_BUILTIN_MODULES.contains(&"node:stream"));
}
