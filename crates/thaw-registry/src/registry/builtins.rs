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
    thaw_node_builtins::source(name)
        .or_else(|| core::source(name))
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
/// installed packages; `served` are the peers thaw-js serves in this build (`ScanContext::served_peers`, see
/// [`served_peers_of`]; `thaw-js-scan` and the gate pass the same set).
pub fn scan_bundle_for_build(
    bundle_js: &str,
    native_addon: bool,
    installed: &std::collections::HashSet<String>,
    served: &std::collections::HashSet<String>,
) -> Result<thaw_js::scan::Report, String> {
    static MEMO: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, Result<thaw_js::scan::Report, String>>>,
    > = std::sync::OnceLock::new();
    let caching = std::env::var_os("THAW_NO_BUILD_CACHE").is_none_or(|v| v.is_empty() || v == "0");
    if !caching {
        return scan_bundle_for_build_uncached(bundle_js, native_addon, installed, served);
    }
    let key = scan_cache_key(bundle_js, native_addon, installed, served);
    let memo = MEMO.get_or_init(Default::default);
    if let Some(hit) = memo.lock().ok().and_then(|m| m.get(&key).cloned()) {
        return hit;
    }
    let dir = build_cache_dir("scan");
    let result = match dir.as_deref().and_then(|d| scan_cache_load(d, &key)) {
        Some(report) => Ok(report),
        None => {
            let result = scan_bundle_for_build_uncached(bundle_js, native_addon, installed, served);
            if let (Ok(report), Some(dir)) = (&result, dir.as_deref()) {
                scan_cache_store(dir, &key, report);
            }
            result
        }
    };
    if let Ok(mut m) = memo.lock() {
        m.insert(key, result.clone());
    }
    result
}

fn scan_bundle_for_build_uncached(
    bundle_js: &str,
    native_addon: bool,
    installed: &std::collections::HashSet<String>,
    served: &std::collections::HashSet<String>,
) -> Result<thaw_js::scan::Report, String> {
    let ctx = thaw_js::scan::ScanContext {
        known_methods: builtin_defined_names(),
        installed_packages: installed,
        served_peers: served,
    };
    if native_addon {
        thaw_js::scan::scan_addon_bundle_in(bundle_js, &ctx)
    } else {
        thaw_js::scan::scan_bundle_in(bundle_js, &ctx)
    }
}

/// Identity of this compiler binary: any rebuild changes the size or mtime, so a cache entry written by another
/// build (different scanner, builtin sources or interpreter globals) is never read. A copied binary only misses.
fn build_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| fs::metadata(p).ok())
            .map(|m| {
                let nanos = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!("{}:{nanos}", m.len())
            })
            .unwrap_or_default();
        format!("{}:{exe}", env!("CARGO_PKG_VERSION"))
    })
}

/// `$XDG_CACHE_HOME/thaw/build-cache/<kind>` (or `$HOME/.cache/thaw/...`); `None` when neither is set.
fn build_cache_dir(kind: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("thaw").join("build-cache").join(kind))
}

/// Every input of the scan result, hashed: the build id, the bundle text, the addon flag, the installed and served sets.
fn scan_cache_key(
    bundle_js: &str,
    native_addon: bool,
    installed: &std::collections::HashSet<String>,
    served: &std::collections::HashSet<String>,
) -> String {
    let mut hasher = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    field(b"scan-v1");
    field(build_id().as_bytes());
    field(&[native_addon as u8]);
    field(bundle_js.as_bytes());
    for set in [installed, served] {
        let mut names: Vec<&String> = set.iter().collect();
        names.sort();
        field(&(names.len() as u64).to_le_bytes());
        for name in names {
            field(name.as_bytes());
        }
    }
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn scan_cache_load(dir: &Path, key: &str) -> Option<thaw_js::scan::Report> {
    let text = fs::read_to_string(dir.join(format!("{key}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if v.get("key")?.as_str()? != key {
        return None;
    }
    let mut blockers = BTreeMap::new();
    for row in v.get("blockers")?.as_array()? {
        let row = row.as_array()?;
        blockers.insert(
            row.first()?.as_str()?.to_string(),
            (row.get(1)?.as_u64()? as usize, row.get(2)?.as_str()?.to_string()),
        );
    }
    Some(thaw_js::scan::Report {
        entry: v.get("entry")?.as_str()?.to_string(),
        factories_total: v.get("factories_total")?.as_u64()? as usize,
        factories_reachable: v.get("factories_reachable")?.as_u64()? as usize,
        blockers,
    })
}

/// Best effort: any failure just leaves the entry missing. Written to a unique temp file and renamed, so a
/// concurrent build reads a whole entry or none. The directory is kept to a few hundred entries.
fn scan_cache_store(dir: &Path, key: &str, report: &thaw_js::scan::Report) {
    // exhaustive destructuring: a new `Report` field must be added to the cache format too
    let thaw_js::scan::Report { entry, factories_total, factories_reachable, blockers } = report;
    let rows: Vec<serde_json::Value> = blockers
        .iter()
        .map(|(k, (n, m))| serde_json::json!([k, n, m]))
        .collect();
    let text = serde_json::json!({
        "key": key, "entry": entry, "factories_total": factories_total,
        "factories_reachable": factories_reachable, "blockers": rows,
    })
    .to_string();
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{key}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let done = fs::create_dir_all(dir)
        .and_then(|_| fs::write(&tmp, text))
        .and_then(|_| fs::rename(&tmp, dir.join(format!("{key}.json"))));
    if done.is_err() {
        let _ = fs::remove_file(&tmp);
        return;
    }
    const MAX_ENTRIES: usize = 256;
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if entries.len() > MAX_ENTRIES {
        entries.sort();
        for (_, path) in entries.iter().take(entries.len() - MAX_ENTRIES / 2) {
            let _ = fs::remove_file(path);
        }
    }
}

/// The exact package names a bundle requires without mapping them: its peers (`thaw_js::scan::unmapped_peer_requires`).
pub fn bundle_peer_requires(bundle_js: &str) -> Result<std::collections::BTreeSet<String>, String> {
    thaw_js::scan::unmapped_peer_requires(bundle_js)
}

/// The peers a bundle is served with in a build: its exact unmapped requires that are installed packages. The build
/// (`add_implicit_peers`) and the gate (`thaw-js-scan`, blocked_bundles.rs) pass this as `served` to
/// [`scan_bundle_for_build`], so a bundle is judged the way a program that imports it is built.
pub fn served_peers_of(
    bundle_js: &str,
    installed: &std::collections::HashSet<String>,
) -> Result<std::collections::BTreeSet<String>, String> {
    Ok(bundle_peer_requires(bundle_js)?
        .into_iter()
        .filter(|peer| installed.contains(peer))
        .collect())
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
