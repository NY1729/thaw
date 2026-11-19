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
#[path = "builtins/sqlite.rs"]
mod sqlite;
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
        .or_else(|| sqlite::source(name))
}

/// Method names the registry's JS builtin modules define: every method of a class or object literal in the
/// source of each Node builtin served to thaw-js (`Channel.prototype.publish = function`, `class X { m() {} }`,
/// `{ m() {} }`, `{ m: function() {} }`). Module-local helper functions and `__thaw_*` internals are not in it.
///
/// The principle: a method name that a served builtin defines is implemented on thaw-js, so a call to it is
/// not a missing feature. A Node method name that no builtin defines stays a blocker in thaw-js's scanner
/// (`scan_bundle_with_known` takes this set as the extra known methods).
pub fn builtin_defined_names() -> &'static std::collections::HashSet<String> {
    static NAMES: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let names: Vec<String> = serde_json::from_str(NODE_BUILTIN_MODULES_JSON).unwrap_or_default();
        names
            .iter()
            .filter_map(|name| builtin_module_source(name))
            .flat_map(builtin_source_method_names)
            .filter(|name| !GENERIC_METHOD_NAMES.contains(&name.as_str()))
            .collect()
    })
}

/// The thaw-js feature scan of a registry bundle as the build runs it (`thaw_js_package_eligible_in`): R6 excuses
/// [`builtin_defined_names`], and a native addon's bundle is scanned in addon mode. `installed` is the registry's
/// installed packages. `thaw-js-scan` calls this too, so its verdict is the build's.
pub fn scan_bundle_for_build(
    bundle_js: &str,
    native_addon: bool,
    installed: &std::collections::HashSet<String>,
) -> Result<thaw_js::scan::Report, String> {
    let ctx = thaw_js::scan::ScanContext {
        known_methods: builtin_defined_names(),
        installed_packages: installed,
    };
    if native_addon {
        thaw_js::scan::scan_addon_bundle_in(bundle_js, &ctx)
    } else {
        thaw_js::scan::scan_bundle_in(bundle_js, &ctx)
    }
}

/// Names too generic to excuse a call: a builtin defines `get`/`on`/`write`/... on its own objects, and a call
/// `x.get(` on a host object thaw lacks would then pass the scan. They stay blockers as before. (The
/// channel-specific names graphql and socket.io need, `publish`/`runStores`/`traceSync`/`unsubscribe`, are not here.)
const GENERIC_METHOD_NAMES: &[&str] = &[
    "add", "bind", "close", "connect", "end", "error", "emit", "format", "get", "has", "listen", "on", "once", "open",
    "pipe", "push", "read", "resolve", "reset", "run", "send", "set", "start", "stop", "subscribe", "write",
];

fn builtin_source_method_names(source: &str) -> Vec<String> {
    let Ok(module) = thaw_parser::parse_javascript(source) else {
        return Vec::new();
    };
    let mut names = BuiltinMethodNames::default();
    swc_ecma_visit::VisitWith::visit_with(&module, &mut names);
    names.0
}

#[derive(Default)]
struct BuiltinMethodNames(Vec<String>);

impl BuiltinMethodNames {
    fn add_key(&mut self, key: &thaw_parser::ast::PropName) {
        use thaw_parser::ast::PropName;
        let name = match key {
            PropName::Ident(i) => Some(i.sym.to_string()),
            PropName::Str(s) => s.value.as_str().map(str::to_string),
            _ => None,
        };
        self.add_name(name);
    }
    fn add_name(&mut self, name: Option<String>) {
        if let Some(name) = name.filter(|n| !n.is_empty() && !n.starts_with("__thaw")) {
            self.0.push(name);
        }
    }
}

fn is_fn_value(e: &thaw_parser::ast::Expr) -> bool {
    use thaw_parser::ast::Expr;
    match e {
        Expr::Fn(_) | Expr::Arrow(_) => true,
        Expr::Paren(p) => is_fn_value(&p.expr),
        _ => false,
    }
}

impl swc_ecma_visit::Visit for BuiltinMethodNames {
    fn visit_class_method(&mut self, n: &thaw_parser::ast::ClassMethod) {
        self.add_key(&n.key);
        swc_ecma_visit::VisitWith::visit_children_with(n, self);
    }
    fn visit_method_prop(&mut self, n: &thaw_parser::ast::MethodProp) {
        self.add_key(&n.key);
        swc_ecma_visit::VisitWith::visit_children_with(n, self);
    }
    fn visit_key_value_prop(&mut self, n: &thaw_parser::ast::KeyValueProp) {
        if is_fn_value(&n.value) {
            self.add_key(&n.key);
        }
        swc_ecma_visit::VisitWith::visit_children_with(n, self);
    }
    fn visit_assign_expr(&mut self, n: &thaw_parser::ast::AssignExpr) {
        use thaw_parser::ast::{AssignTarget, MemberProp, SimpleAssignTarget};
        if let AssignTarget::Simple(SimpleAssignTarget::Member(m)) = &n.left {
            if let MemberProp::Ident(p) = &m.prop {
                if is_fn_value(&n.right) {
                    self.add_name(Some(p.sym.to_string()));
                }
            }
        }
        swc_ecma_visit::VisitWith::visit_children_with(n, self);
    }
}
