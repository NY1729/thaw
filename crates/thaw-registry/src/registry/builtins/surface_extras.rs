/// JS appended to a builtin module's source (members that are state or pure logic, kept out of the giant one-line
/// sources). Order in `builtins.rs::builtin_module_source`: base source, these, then `surface_data` (Node's constants).
pub(super) fn extra(module: &str) -> Option<&'static str> {
    match module {
        "module" => Some(include_str!("surface_extras/module.js")),
        "v8" => Some(include_str!("surface_extras/v8.js")),
        "dns/promises" => Some(include_str!("surface_extras/dns_promises.js")),
        "util" => Some(include_str!("surface_extras/util.js")),
        _ => None,
    }
}
