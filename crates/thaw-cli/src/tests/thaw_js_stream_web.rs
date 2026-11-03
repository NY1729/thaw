// node:stream and node:stream/web are both allowlisted for thaw-js routing. Kept separate from the
// pinned allowlist test.

#[test]
fn thaw_js_stream_web_is_allowlisted() {
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:stream/web"));
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:stream"));
}
