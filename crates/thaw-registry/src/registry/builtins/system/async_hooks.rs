pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "async_hooks" => Some("module.exports = globalThis.__thaw_async_hooks_exports;\n"),
        _ => None,
    }
}
