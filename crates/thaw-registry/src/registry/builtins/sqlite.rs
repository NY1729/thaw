// `node:sqlite`: the JS module over the thaw-js `__thaw_sqlite_*` natives. Not in NODE_BUILTIN_MODULES_JSON
// (Node 22 lists it neither in builtinModules nor as a bare name), so only the `node:` prefix resolves.
pub(super) fn source(name: &str) -> Option<&'static str> {
    (name == "sqlite").then_some(include_str!("sqlite.js"))
}
