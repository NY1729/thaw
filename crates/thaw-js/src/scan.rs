//! M0 feature scan: which constructs of a package bundle can the thaw-js evaluator not run?
//!
//! The scan works on the registry bundle format (`thaw_modules/<pkg>/bundle.js`): it finds
//! `__thaw_bundle_factories` (one CommonJS function per module), `__thaw_bundle_require_maps`
//! and `__thaw_bundle_entry_key`, walks only the factories reachable from the entry, and
//! reports every construct outside [`crate::SUPPORTED_FEATURES`] / the interpreter's builtins.
//! The feature names are the same strings the evaluator puts into `Ctl::Unsupported`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

use crate::scan_names::{
    is_node_global, is_node_method_name, node_global_covered, node_global_has,
};

/// Per-package scan result.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub entry: String,
    pub factories_total: usize,
    pub factories_reachable: usize,
    /// feature -> (occurrences, first module key)
    pub blockers: BTreeMap<String, (usize, String)>,
}

impl Report {
    /// True when the evaluator can be expected to run the whole reachable package.
    pub fn is_clean(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// Parsed bundle pieces needed by the loader and the scan.
pub struct BundleParts {
    /// Constructs of a rewritten ES-module bundle that the CommonJS rewrite cannot keep sound.
    pub esm_problems: BTreeSet<&'static str>,
    /// The text that was parsed (the rewritten text for an ES-module bundle).
    pub source: String,
    pub entry: String,
    pub factories: Vec<(String, a::Function)>,
    pub require_maps: HashMap<String, HashMap<String, String>>,
    /// `__thaw_bundle_realm = {key, member}`: this bundle is a view of the realm `key` (a `node:path`
    /// family spec); `member` is the export path of the view (`posix`, `win32`, or empty for the root).
    pub realm: Option<(String, String)>,
}

fn prop_key(p: &a::PropName) -> Option<String> {
    match p {
        a::PropName::Ident(i) => Some(i.sym.to_string()),
        a::PropName::Str(s) => s.value.as_str().map(str::to_string),
        _ => None,
    }
}

/// A plain CommonJS script (the Node builtin shims: `node:path`, `node:assert`, ...) as a one-module
/// registry bundle, so the scan and the loader treat it like any package. Registry bundles pass through,
/// and so does a pure native ESM bundle (the loader reads it through `native_payload`, not as a script).
pub fn script_as_bundle(source: &str) -> std::borrow::Cow<'_, str> {
    if source.contains("__thaw_bundle_factories")
        || matches!(crate::esm::native_payload(source), Some((_, true)))
    {
        return source.into();
    }
    format!(
        "module.exports = (function(require){{ var __thaw_bundle_entry_key = \"thaw-js:script\"; \
         var __thaw_bundle_factories = {{ \"thaw-js:script\": function(module, exports, require, requireAsync, \
         __filename, __dirname, __thaw_require) {{\n{source}\n}} }}; \
         var __thaw_bundle_require_maps = {{ \"thaw-js:script\": {{}} }}; }})(globalThis.require);"
    )
    .into()
}

/// The thaw-js form of `node:worker_threads`: the main-thread module the thaw-js prelude exports
/// (`web/worker_threads.js`). Package bundles embed the QuickJS form of that builtin (natives that exist
/// only in thaw-quickjs), so [`parse_bundle`] substitutes this one for every such embedded factory.
pub const WORKER_THREADS_THAW_JS_FORM: &str =
    "module.exports = globalThis.__thaw_worker_threads_exports;\n";

/// Extract factories, require maps and the entry key from a registry bundle.
pub fn parse_bundle(source: &str) -> Result<BundleParts, String> {
    let mut parts = parse_bundle_raw(source)?;
    stub_cheerio_undici(&mut parts)?;
    substitute_builtin_factory(
        &mut parts,
        "node:worker_threads",
        WORKER_THREADS_THAW_JS_FORM,
    )?;
    Ok(parts)
}

/// Replace the factory `key` of a bundle (a builtin embedded in a package bundle) with the thaw-js form `body`.
/// `key` must be a plain name (it is written between double quotes). Same technique as [`stub_cheerio_undici`]:
/// the stub is appended and the bundle re-parsed so spans index the kept text; the older duplicate is dropped
/// (keep-last); the builtin's own requires are cleared, so the modules only its QuickJS form pulled in are not reached.
fn substitute_builtin_factory(
    parts: &mut BundleParts,
    key: &str,
    body: &str,
) -> Result<(), String> {
    if !parts.factories.iter().any(|(k, _)| k == key) {
        return Ok(());
    }
    let stub = format!(
        "module.exports = (function(require){{ var __thaw_bundle_factories = {{ \"{key}\": function(module, exports, require) {{ {body} }} }}; }})(globalThis.require);"
    );
    let mut combined = parse_factories(&format!("{}\n{stub}", parts.source))?;
    let mut seen = HashSet::new();
    let mut kept: Vec<_> = combined
        .factories
        .drain(..)
        .rev()
        .filter(|(k, _)| seen.insert(k.clone()))
        .collect();
    kept.reverse();
    combined.factories = kept;
    combined.esm_problems = std::mem::take(&mut parts.esm_problems);
    combined.require_maps = std::mem::take(&mut parts.require_maps);
    combined
        .require_maps
        .insert(key.to_string(), HashMap::new());
    combined.realm = parts.realm.take();
    *parts = combined;
    Ok(())
}

const UNDICI_STUB_KEY: &str = "thaw-js:undici-unavailable";

/// cheerio requires `undici` only for `fromURL` (fetch/WebAssembly llhttp/MessagePort/net stack). thaw-js does
/// not run that stack: cheerio's requires of it resolve to a stub whose members throw a clear error when used,
/// so the rest of cheerio (`load`, selectors, manipulation) runs and the scan no longer sees undici.
// ponytail: fromURL/fromBuffer-over-network is unsupported on thaw-js; a native undici replacement would lift it.
fn stub_cheerio_undici(parts: &mut BundleParts) -> Result<(), String> {
    let mut hit = false;
    for (requirer, map) in parts.require_maps.iter_mut() {
        if !requirer.starts_with("cheerio/") {
            continue;
        }
        for target in map.values_mut() {
            if target.starts_with("undici/") {
                *target = UNDICI_STUB_KEY.to_string();
                hit = true;
            }
        }
    }
    if !hit {
        return Ok(());
    }
    let stub = format!(
        "module.exports = (function(require){{ var __thaw_bundle_factories = {{ \"{UNDICI_STUB_KEY}\": function(module, exports) {{ \
         module.exports = new Proxy({{}}, {{ get: function(_, k) {{ \
         if (k === '__esModule') return true; if (typeof k === 'symbol' || k === 'then') return undefined; \
         return function() {{ throw new Error('undici is not available on the thaw-js runtime (cheerio.fromURL): ' + String(k)); }}; \
         }} }}); }} }}; }})(globalThis.require);"
    );
    // re-parse source + stub so the stub factory's spans index the text the interpreter keeps
    let mut combined = parse_factories(&format!("{}\n{stub}", parts.source))?;
    combined.esm_problems = std::mem::take(&mut parts.esm_problems);
    combined.require_maps = std::mem::take(&mut parts.require_maps);
    combined.realm = parts.realm.take();
    *parts = combined;
    Ok(())
}

fn parse_bundle_raw(source: &str) -> Result<BundleParts, String> {
    if let Some((native, problems, pure)) = crate::esm::native_bundle_to_factories(source)? {
        if pure {
            let mut parts = parse_factories(&native)?;
            parts.esm_problems = problems;
            return Ok(parts);
        }
        // mixed bundle: the converted native modules follow the original text and replace its stubs
        let combined = format!("{source}\n{native}");
        let mut parts = parse_factories(&combined)?;
        let mut seen = HashSet::new();
        let mut kept: Vec<_> = parts
            .factories
            .drain(..)
            .rev()
            .filter(|(k, _)| seen.insert(k.clone()))
            .collect();
        kept.reverse();
        parts.factories = kept;
        parts.esm_problems = problems;
        return Ok(parts);
    }
    parse_factories(source)
}

fn parse_factories(source: &str) -> Result<BundleParts, String> {
    let module = thaw_parser::parse_javascript(source)?;
    let (mut entry, mut factories, mut require_maps) = (String::new(), Vec::new(), HashMap::new());
    let mut realm = None;
    // The wrapper is `module.exports = (function(require){ ... })(globalThis.require);`.
    let mut stmts: Vec<&a::Stmt> = Vec::new();
    fn collect<'a>(e: &'a a::Expr, out: &mut Vec<&'a a::Stmt>) {
        match e {
            a::Expr::Paren(p) => collect(&p.expr, out),
            a::Expr::Assign(asg) => collect(&asg.right, out),
            a::Expr::Call(c) => {
                if let a::Callee::Expr(callee) = &c.callee {
                    collect(callee, out);
                }
            }
            a::Expr::Fn(f) => {
                if let Some(b) = &f.function.body {
                    out.extend(b.stmts.iter());
                }
            }
            _ => {}
        }
    }
    for item in &module.body {
        if let a::ModuleItem::Stmt(a::Stmt::Expr(e)) = item {
            collect(&e.expr, &mut stmts);
        }
    }
    for s in stmts {
        let a::Stmt::Decl(a::Decl::Var(v)) = s else {
            continue;
        };
        for d in &v.decls {
            let (a::Pat::Ident(id), Some(init)) = (&d.name, &d.init) else {
                continue;
            };
            match (&*id.id.sym, &**init) {
                ("__thaw_bundle_entry_key", a::Expr::Lit(a::Lit::Str(s))) => {
                    entry = s.value.as_str().unwrap_or("").to_string()
                }
                ("__thaw_bundle_realm", a::Expr::Object(o)) => {
                    let str_prop = |name: &str| {
                        o.props.iter().find_map(|p| match p {
                            a::PropOrSpread::Prop(p) => match &**p {
                                a::Prop::KeyValue(kv)
                                    if prop_key(&kv.key).as_deref() == Some(name) =>
                                {
                                    match &*kv.value {
                                        a::Expr::Lit(a::Lit::Str(s)) => {
                                            Some(s.value.as_str().unwrap_or("").to_string())
                                        }
                                        _ => None,
                                    }
                                }
                                _ => None,
                            },
                            _ => None,
                        })
                    };
                    if let (Some(key), Some(member)) = (str_prop("key"), str_prop("member")) {
                        realm = Some((key, member));
                    }
                }
                ("__thaw_bundle_factories", a::Expr::Object(o)) => {
                    for p in &o.props {
                        if let a::PropOrSpread::Prop(p) = p {
                            if let a::Prop::KeyValue(kv) = &**p {
                                if let (Some(k), a::Expr::Fn(f)) = (prop_key(&kv.key), &*kv.value) {
                                    factories.push((k, (*f.function).clone()));
                                }
                            }
                        }
                    }
                }
                (
                    "__thaw_bundle_require_maps" | "__thaw_bundle_import_maps",
                    a::Expr::Object(o),
                ) => {
                    for p in &o.props {
                        if let a::PropOrSpread::Prop(p) = p {
                            if let a::Prop::KeyValue(kv) = &**p {
                                let (Some(k), a::Expr::Object(inner)) =
                                    (prop_key(&kv.key), &*kv.value)
                                else {
                                    continue;
                                };
                                let mut map = HashMap::new();
                                for ip in &inner.props {
                                    if let a::PropOrSpread::Prop(ip) = ip {
                                        if let a::Prop::KeyValue(ikv) = &**ip {
                                            if let (Some(sk), a::Expr::Lit(a::Lit::Str(sv))) =
                                                (prop_key(&ikv.key), &*ikv.value)
                                            {
                                                map.insert(
                                                    sk,
                                                    sv.value.as_str().unwrap_or("").to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                                require_maps
                                    .entry(k)
                                    .or_insert_with(HashMap::new)
                                    .extend(map);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if entry.is_empty() || factories.is_empty() {
        return Err("not a registry bundle (no entry key / factories)".into());
    }
    Ok(BundleParts {
        esm_problems: BTreeSet::new(),
        source: source.to_string(),
        entry,
        factories,
        require_maps,
        realm,
    })
}

/// Factory keys reachable from the entry through `require_maps`.
pub fn reachable(parts: &BundleParts) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![parts.entry.clone()];
    while let Some(k) = stack.pop() {
        if !seen.insert(k.clone()) {
            continue;
        }
        if let Some(m) = parts.require_maps.get(&k) {
            stack.extend(m.values().cloned());
        }
    }
    seen
}

/// One `require(..)` call: its literal spec (`None` for a computed one), and whether it is guarded.
struct Require {
    spec: Option<String>,
    guarded: bool,
}

/// Node's builtin module names without the `node:` prefix (the list of thaw-registry's NODE_BUILTIN_MODULES_JSON).
#[rustfmt::skip]
const NODE_BUILTIN_NAMES: &[&str] = &[
    "_http_agent", "_http_client", "_http_common", "_http_incoming", "_http_outgoing", "_http_server",
    "_stream_duplex", "_stream_passthrough", "_stream_readable", "_stream_transform", "_stream_wrap",
    "_stream_writable", "_tls_common", "_tls_wrap", "assert", "assert/strict", "async_hooks", "buffer",
    "child_process", "cluster", "console", "constants", "crypto", "dgram", "diagnostics_channel", "dns",
    "dns/promises", "domain", "events", "fs", "fs/promises", "http", "http2", "https", "inspector",
    "inspector/promises", "module", "net", "os", "path", "path/posix", "path/win32", "perf_hooks", "process",
    "punycode", "querystring", "readline", "readline/promises", "repl", "stream", "stream/consumers",
    "stream/promises", "stream/web", "string_decoder", "sys", "test", "test/reporters", "timers",
    "timers/promises", "tls", "trace_events", "tty", "url", "util", "util/types", "v8", "vm", "wasi",
    "worker_threads", "zlib",
];

/// `require(spec)` names an optional npm package: not a builtin (`node:` specs included), not a relative or
/// absolute file of the package itself, not a native addon. Only such a require may be missing and guarded.
fn is_optional_package_spec(spec: &str) -> bool {
    !spec.is_empty()
        && !spec.starts_with("node:")
        && !spec.starts_with('.')
        && !spec.starts_with('/')
        && !spec.ends_with(".node")
        && !NODE_BUILTIN_NAMES.contains(&spec)
}

/// A literal that is always falsy (`0`, `false`, `null`), possibly parenthesized: `0 && x` and `if (0)` are dead.
fn is_falsy_constant(e: &a::Expr) -> bool {
    match e {
        a::Expr::Paren(p) => is_falsy_constant(&p.expr),
        a::Expr::Lit(a::Lit::Num(n)) => n.value == 0.0,
        a::Expr::Lit(a::Lit::Bool(b)) => !b.value,
        a::Expr::Lit(a::Lit::Null(_)) => true,
        _ => false,
    }
}

/// `null`, `undefined` (possibly parenthesized): the other operand of an equality test against it is a presence probe.
fn is_nullish_literal(e: &a::Expr) -> bool {
    match e {
        a::Expr::Paren(p) => is_nullish_literal(&p.expr),
        a::Expr::Lit(a::Lit::Null(_)) => true,
        a::Expr::Ident(i) => &*i.sym == "undefined",
        _ => false,
    }
}

/// The `CallSite` methods the engine implements (the `siteMethods` object of builtins.rs; a test pins the two lists
/// together). A call of one of these names is not a missing method. Node-parity: the method rule (`method:X`) flags a
/// call only when Node 22 has `X` (`scan_names::is_node_method_name`) and thaw-js does not know it.
pub(crate) const CALLSITE_METHODS: &[&str] = &[
    "getThis",
    "getTypeName",
    "getFunction",
    "getFunctionName",
    "getMethodName",
    "getFileName",
    "getLineNumber",
    "getColumnNumber",
    "getEnclosingLineNumber",
    "getEnclosingColumnNumber",
    "getPosition",
    "getPromiseIndex",
    "getScriptNameOrSourceURL",
    "getScriptHash",
    "getEvalOrigin",
    "isToplevel",
    "isEval",
    "isNative",
    "isConstructor",
    "isAsync",
    "isPromiseAll",
    "toString",
];

/// Finds a `throw` statement: a catch body containing one does not handle the failure it catches.
#[derive(Default)]
struct ThrowFinder {
    found: bool,
}

impl Visit for ThrowFinder {
    fn visit_throw_stmt(&mut self, _: &a::ThrowStmt) {
        self.found = true;
    }
}

#[derive(Default)]
struct Walker {
    features: HashMap<String, usize>,
    declared: HashSet<String>,
    refs: HashSet<String>,
    /// `G.p` reads of builtin objects
    builtin_members: HashSet<(String, String)>,
    /// `x.m(...)` call names
    called: HashSet<String>,
    /// every property/function name the bundle itself defines
    defined: HashSet<String>,
    /// identifiers used only as the right operand of `instanceof`
    instance_refs: HashSet<String>,
    requires: Vec<Require>,
    /// inside a try block whose catch handles the failure, in the same function (see `visit_try_stmt`)
    guarded: bool,
    /// identifiers probed with `typeof X` (assumed guarded)
    probed: HashSet<String>,
    /// `globalThis.X` members tested only for presence
    probed_members: HashSet<String>,
    probed_methods: HashSet<String>,
    /// `this` binding of the innermost class body or method: `Some(true)` in a class without `extends`
    /// (its methods are its own), `Some(false)` in a class that extends something, `None` outside classes
    /// and in plain functions (which rebind `this`)
    this_class: Option<bool>,
    /// the next function visited is a class method, which keeps the class's `this` binding (set by `visit_class_method`)
    method_fn: bool,
}

impl Walker {
    /// `globalThis.X` tested for truthiness / `typeof` is a feature probe, not a use.
    fn probe_member(&mut self, e: &a::Expr) {
        // `!x.m` is the negated presence test of `x.m`
        let mut e = e;
        while let a::Expr::Unary(u) = e {
            if u.op != a::UnaryOp::Bang {
                break;
            }
            e = &u.arg;
        }
        // `if (x.m) x.m()` / `x?.m ? x.m(..) : ..`: a method call guarded by a truthiness test of the same name
        let tested = match e {
            a::Expr::Member(m) => Some(m),
            a::Expr::OptChain(o) => match &*o.base {
                a::OptChainBase::Member(m) => Some(m),
                _ => None,
            },
            _ => None,
        };
        if let Some(a::MemberProp::Ident(p)) = tested.map(|m| &m.prop) {
            self.probed_methods.insert(p.sym.to_string());
        }
        if let a::Expr::Member(m) = e {
            if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (&*m.obj, &m.prop) {
                if &*o.sym == "globalThis" {
                    self.probed_members.insert(p.sym.to_string());
                }
            }
        }
    }
    fn feature(&mut self, name: &str) {
        *self.features.entry(name.to_string()).or_insert(0) += 1;
    }
}

impl Visit for Walker {
    fn visit_class(&mut self, n: &a::Class) {
        self.feature("class");
        // without `extends` the class has no host base, so `this.m()` in its body is its own method (see visit_call_expr)
        let outer = self.this_class.replace(n.super_class.is_none());
        n.visit_children_with(self);
        self.this_class = outer;
    }
    fn visit_class_method(&mut self, n: &a::ClassMethod) {
        self.method_fn = true;
        n.visit_children_with(self);
    }
    fn visit_private_method(&mut self, n: &a::PrivateMethod) {
        self.method_fn = true;
        n.visit_children_with(self);
    }
    fn visit_class_decl(&mut self, n: &a::ClassDecl) {
        self.declared.insert(n.ident.sym.to_string());
        n.class.visit_with(self);
    }
    fn visit_class_expr(&mut self, n: &a::ClassExpr) {
        // a class expression's own name is bound inside its body
        if let Some(i) = &n.ident {
            self.declared.insert(i.sym.to_string());
        }
        n.class.visit_with(self);
    }
    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        self.declared.insert(n.ident.sym.to_string());
        self.defined.insert(n.ident.sym.to_string());
        n.function.visit_with(self);
    }
    fn visit_fn_expr(&mut self, n: &a::FnExpr) {
        if let Some(i) = &n.ident {
            self.declared.insert(i.sym.to_string());
            self.defined.insert(i.sym.to_string());
        }
        n.function.visit_with(self);
    }
    fn visit_function(&mut self, n: &a::Function) {
        // the `globalThis` polyfill idiom: `if (typeof globalThis === 'object') return;` makes the rest dead
        if let Some(a::Stmt::If(i)) = n.body.as_ref().and_then(|b| b.stmts.first()) {
            let is_global_probe = matches!(&*i.test, a::Expr::Bin(b)
                if matches!(&*b.left, a::Expr::Unary(u) if u.op == a::UnaryOp::TypeOf
                    && matches!(&*u.arg, a::Expr::Ident(g) if &*g.sym == "globalThis"))
                && matches!(&*b.right, a::Expr::Lit(a::Lit::Str(l)) if l.value.as_str() == Some("object")));
            if is_global_probe && matches!(&*i.cons, a::Stmt::Return(_)) && i.alt.is_none() {
                return;
            }
        }
        if n.is_async {
            self.feature("async");
        }
        if n.is_generator {
            self.feature("generator");
        }
        // a nested function runs when called, not inside an enclosing try
        let guarded = std::mem::replace(&mut self.guarded, false);
        // a plain function rebinds `this`; a class method keeps its class's binding
        let this_class = self.this_class;
        if !std::mem::take(&mut self.method_fn) {
            self.this_class = None;
        }
        n.visit_children_with(self);
        self.guarded = guarded;
        self.this_class = this_class;
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        if n.is_async {
            self.feature("async");
        }
        if n.is_generator {
            self.feature("generator");
        }
        let guarded = std::mem::replace(&mut self.guarded, false);
        n.visit_children_with(self);
        self.guarded = guarded;
    }
    fn visit_try_stmt(&mut self, n: &a::TryStmt) {
        let outer = self.guarded;
        // a catch that does not rethrow handles the failure of a require in the try block
        let handled = n.handler.as_ref().is_some_and(|h| {
            let mut finder = ThrowFinder::default();
            h.body.visit_with(&mut finder);
            !finder.found
        });
        self.guarded = outer || handled;
        n.block.visit_with(self);
        self.guarded = outer;
        n.handler.visit_with(self);
        n.finalizer.visit_with(self);
    }
    fn visit_await_expr(&mut self, n: &a::AwaitExpr) {
        self.feature("await");
        n.visit_children_with(self);
    }
    fn visit_yield_expr(&mut self, n: &a::YieldExpr) {
        self.feature("yield");
        n.visit_children_with(self);
    }
    fn visit_object_pat(&mut self, n: &a::ObjectPat) {
        self.feature("destructuring");
        n.visit_children_with(self);
    }
    fn visit_array_pat(&mut self, n: &a::ArrayPat) {
        self.feature("destructuring");
        n.visit_children_with(self);
    }
    fn visit_tagged_tpl(&mut self, n: &a::TaggedTpl) {
        self.feature("tagged-template");
        n.visit_children_with(self);
    }
    fn visit_opt_chain_expr(&mut self, n: &a::OptChainExpr) {
        self.feature("optional-chaining");
        n.visit_children_with(self);
    }
    fn visit_meta_prop_expr(&mut self, n: &a::MetaPropExpr) {
        self.feature(if matches!(n.kind, a::MetaPropKind::NewTarget) {
            "new.target"
        } else {
            "import.meta"
        });
    }
    fn visit_super(&mut self, _: &a::Super) {
        self.feature("super");
    }
    fn visit_super_prop_expr(&mut self, n: &a::SuperPropExpr) {
        self.feature("super");
        n.visit_children_with(self);
    }
    fn visit_lit(&mut self, n: &a::Lit) {
        match n {
            a::Lit::Regex(_) => self.feature("regexp"),
            a::Lit::BigInt(_) => self.feature("bigint"),
            // method names listed in string tables (`['year', 'month'].forEach(k => proto[k] = ...)`)
            a::Lit::Str(l) => {
                if let Some(v) = l.value.as_str() {
                    if !v.is_empty()
                        && v.len() <= 40
                        && v.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                    {
                        self.defined.insert(v.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    fn visit_with_stmt(&mut self, n: &a::WithStmt) {
        self.feature("with");
        n.visit_children_with(self);
    }
    fn visit_module_decl(&mut self, n: &a::ModuleDecl) {
        self.feature("esm-syntax");
        n.visit_children_with(self);
    }
    fn visit_getter_prop(&mut self, n: &a::GetterProp) {
        self.feature("accessor");
        n.visit_children_with(self);
    }
    fn visit_setter_prop(&mut self, n: &a::SetterProp) {
        self.feature("accessor");
        n.visit_children_with(self);
    }
    fn visit_private_name(&mut self, _: &a::PrivateName) {
        self.feature("private-name");
    }
    fn visit_static_block(&mut self, n: &a::StaticBlock) {
        self.feature("static-block");
        n.visit_children_with(self);
    }
    fn visit_auto_accessor(&mut self, n: &a::AutoAccessor) {
        self.feature("auto-accessor");
        n.visit_children_with(self);
    }
    fn visit_prop_or_spread(&mut self, n: &a::PropOrSpread) {
        if matches!(n, a::PropOrSpread::Spread(_)) {
            self.feature("object-spread");
        }
        n.visit_children_with(self);
    }
    fn visit_bin_expr(&mut self, n: &a::BinExpr) {
        // `0 && (module.exports = {...})` (esbuild's cjs-lexer annotation): the right operand never runs
        if n.op == a::BinaryOp::LogicalAnd && is_falsy_constant(&n.left) {
            return;
        }
        if matches!(n.op, a::BinaryOp::LogicalAnd | a::BinaryOp::LogicalOr) {
            self.probe_member(&n.left);
        }
        // `x.m == null`, `x.m !== undefined`: a presence test of `x.m`
        if matches!(
            n.op,
            a::BinaryOp::EqEq | a::BinaryOp::NotEq | a::BinaryOp::EqEqEq | a::BinaryOp::NotEqEq
        ) {
            if is_nullish_literal(&n.right) {
                self.probe_member(&n.left);
            }
            if is_nullish_literal(&n.left) {
                self.probe_member(&n.right);
            }
        }
        if let (a::BinaryOp::InstanceOf, a::Expr::Ident(i)) = (n.op, &*n.right) {
            self.instance_refs.insert(i.sym.to_string());
            n.left.visit_with(self);
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_prop(&mut self, n: &a::Prop) {
        if let a::Prop::Shorthand(i) = n {
            self.defined.insert(i.sym.to_string());
        }
        n.visit_children_with(self);
    }
    fn visit_prop_name(&mut self, n: &a::PropName) {
        if let Some(k) = prop_key(n) {
            self.defined.insert(k);
        }
        n.visit_children_with(self);
    }
    fn visit_binding_ident(&mut self, n: &a::BindingIdent) {
        self.declared.insert(n.id.sym.to_string());
    }
    fn visit_labeled_stmt(&mut self, n: &a::LabeledStmt) {
        n.body.visit_with(self);
    }
    fn visit_break_stmt(&mut self, _: &a::BreakStmt) {}
    fn visit_continue_stmt(&mut self, _: &a::ContinueStmt) {}
    fn visit_ident(&mut self, n: &a::Ident) {
        match &*n.sym {
            "arguments" => self.feature("arguments"),
            name => {
                self.refs.insert(name.to_string());
            }
        }
    }
    fn visit_cond_expr(&mut self, n: &a::CondExpr) {
        self.probe_member(&n.test);
        n.visit_children_with(self);
    }
    fn visit_if_stmt(&mut self, n: &a::IfStmt) {
        self.probe_member(&n.test);
        if is_falsy_constant(&n.test) {
            // `if (0) {...} else {...}`: the consequent never runs
            if let Some(alt) = &n.alt {
                alt.visit_with(self);
            }
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_unary_expr(&mut self, n: &a::UnaryExpr) {
        if n.op == a::UnaryOp::TypeOf {
            self.probe_member(&n.arg);
        }
        // `typeof X` is a feature probe, not a use.
        if let (a::UnaryOp::TypeOf, a::Expr::Ident(i)) = (n.op, &*n.arg) {
            self.probed.insert(i.sym.to_string());
            return;
        }
        n.visit_children_with(self);
    }
    fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
        if let a::AssignTarget::Simple(a::SimpleAssignTarget::Member(m)) = &n.left {
            if let a::MemberProp::Ident(p) = &m.prop {
                self.defined.insert(p.sym.to_string());
            }
        }
        n.visit_children_with(self);
    }
    fn visit_member_expr(&mut self, n: &a::MemberExpr) {
        if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (&*n.obj, &n.prop) {
            self.builtin_members
                .insert((o.sym.to_string(), p.sym.to_string()));
        }
        n.visit_children_with(self);
    }
    fn visit_call_expr(&mut self, n: &a::CallExpr) {
        if let a::Callee::Expr(c) = &n.callee {
            match &**c {
                a::Expr::Member(m) => {
                    // the ESM origin is `undefined` under this loader; its calls are guarded and dead
                    let origin = matches!(&*m.obj, a::Expr::Ident(o) if o.sym.starts_with("__thaw_esm_origin_"));
                    // `this.m()` in a class without `extends` calls the class's own method, not a host one
                    let own_method =
                        self.this_class == Some(true) && matches!(&*m.obj, a::Expr::This(_));
                    if let (false, false, a::MemberProp::Ident(p)) = (origin, own_method, &m.prop) {
                        self.called.insert(p.sym.to_string());
                    }
                }
                a::Expr::Ident(i) if &*i.sym == "require" => {
                    let lit = match n.args.first().map(|x| &*x.expr) {
                        Some(a::Expr::Lit(a::Lit::Str(s))) => s.value.as_str().map(str::to_string),
                        _ => None,
                    };
                    self.requires.push(Require {
                        spec: lit,
                        guarded: self.guarded,
                    });
                }
                _ => {}
            }
        }
        n.visit_children_with(self);
    }
}

/// Findings the heuristics cannot tell are dead or guarded. Each entry is a construct that is inert on Node too,
/// so running it on thaw-js behaves the same:
/// - `process.type|browser|__nwjs|allowedNodeEnvironmentFlags`: absent on Node (read as `undefined`) -- `debug` probes them to pick its browser entry;
/// - `debug`'s browser entry is never selected by that probe; its `supports-color` require is try/catch-optional;
/// - `node:assert` inspects with `require(['node','util'].join(':'))` inside a try/catch;
/// - axios: `AxiosHeaders` accessors are installed from a name list (`getContentType`, ...), `zlib.createZstdDecompress`
///   is feature-tested, `postMessage`/`send` sit behind `typeof` / XHR-only paths, `isDeepStrictEqual` is http2 session
///   reuse (http2 needs TLS, which thaw-js does not link), `onSocket` is `agent-base`'s `typeof` check.
///
/// Node-parity (R2/R4/R5): a member or global Node 22 lacks is no finding at all, so the `process.type|browser|__nwjs`
/// entries below are inert now (Node lacks them too); only `allowedNodeEnvironmentFlags` (a Node member) still needs its entry.
fn tolerated(feature: &str, key: &str) -> bool {
    const AXIOS_METHODS: &[&str] = &[
        "method:getContentType",
        "method:getContentLength",
        "method:hasContentLength",
        "method:setContentType",
        "method:setContentLength",
        "method:createZstdDecompress",
        "method:postMessage",
        "method:send",
        "method:isDeepStrictEqual",
    ];
    matches!(
        feature,
        "builtin:process.type"
            | "builtin:process.browser"
            | "builtin:process.__nwjs"
            | "builtin:process.allowedNodeEnvironmentFlags"
            // Electron's probe `isElectronApp() && !process.defaultApp`: isElectronApp is false on Node
            | "builtin:process.defaultApp"
    ) ||
        key.ends_with("debug/src/browser.js")
        || (feature == "require:supports-color" && key.ends_with("debug/src/node.js"))
        || (matches!(feature, "require:dynamic" | "method:inspect") && key == "node:assert")
        || (AXIOS_METHODS.contains(&feature) && key.ends_with("axios/./dist/node/axios.cjs"))
        // iconv-lite calls the caller's optional `options.stripBOM` only after `typeof ... === 'function'`
        || (feature == "method:stripBOM" && key.ends_with("iconv-lite/lib/bom-handling.js"))
        || (feature == "method:onSocket" && key.ends_with("agent-base/dist/src/index.js"))
        // node:domain writes `process.domain` on enter/exit; thaw-js has no such property, the write is inert
        || (feature == "builtin:process.domain" && key == "node:domain")
}

/// Globals host.rs installs for the `node:module` entry (`globalThis.<name>` assignments in its prelude).
pub(crate) const MODULE_LOADER_GLOBALS: [&str; 2] = [
    "__thaw_bundle_builtin_names",
    "__thaw_bundle_create_require",
];

/// The `node:sqlite` natives (sqlite/mod.rs NATIVES). They are installed only under the `sqlite` cargo feature, so
/// the scanner (which builds an Interp without features) names them itself, as it does the loader globals above.
pub(crate) const SQLITE_NATIVE_GLOBALS: [&str; 17] = [
    "__thaw_sqlite_open",
    "__thaw_sqlite_close",
    "__thaw_sqlite_is_open",
    "__thaw_sqlite_is_transaction",
    "__thaw_sqlite_location",
    "__thaw_sqlite_exec",
    "__thaw_sqlite_prepare",
    "__thaw_sqlite_stmt_start",
    "__thaw_sqlite_stmt_step",
    "__thaw_sqlite_stmt_reset",
    "__thaw_sqlite_stmt_run",
    "__thaw_sqlite_stmt_get",
    "__thaw_sqlite_stmt_all",
    "__thaw_sqlite_stmt_columns",
    "__thaw_sqlite_stmt_source_sql",
    "__thaw_sqlite_stmt_expanded_sql",
    "__thaw_sqlite_stmt_set_flag",
];

/// Findings a native-addon package bundle carries on thaw-js by design. Its `.node` is loaded through thaw-js
/// `require` (host.rs `addon_of`; any `*.node` name returns the addon). `__non_webpack_require__` is the node-gyp-build
/// probe, used only when `__webpack_require__` is a function (never on thaw-js). A dynamic `require` is that loader
/// only when the same bundle also requires a `.node` by name (`bindings`, `node-gyp-build`). Any `method:<name>` is
/// accepted too, except `method:dlopen`: the addon's exports are native functions named by the addon, not by the
/// bundle, so the scan cannot list them. `process.dlopen` itself is never served on thaw-js.
fn addon_tolerated(feature: &str, node_require: bool) -> bool {
    (feature.starts_with("require:") && feature.ends_with(".node"))
        || (feature == "require:dynamic" && node_require)
        || feature == "global:__non_webpack_require__"
        || (feature.starts_with("method:") && feature != "method:dlopen")
}

/// A `__`-prefixed name: a host-internal or loader convention (`__thaw_*` natives, `__non_webpack_require__`), never a
/// Node API name. The Node-parity relaxations of R4 (members) and R5 (globals) do not apply to it: it stays a blocker
/// exactly as before the rules. R2 (method names) has no such carve-out: `__emitTokens` is a package-own method guarded
/// by a probe and must stay clean.
fn is_host_internal(name: &str) -> bool {
    name.starts_with("__")
}

/// The R5 test for a free name: Node 22 defines it, or it is host-internal (see [`is_host_internal`]).
fn is_node_or_host_internal(name: &str) -> bool {
    is_node_global(name) || is_host_internal(name)
}

/// Scan a registry bundle.
pub fn scan_bundle(source: &str) -> Result<Report, String> {
    scan_bundle_with(source, false)
}

/// Scan the bundle of a package that ships a native (`.node`) addon: the loader keys of
/// [`addon_tolerated`] are accepted on top of [`scan_bundle`]'s rules. Only addon packages use it.
pub fn scan_addon_bundle(source: &str) -> Result<Report, String> {
    scan_bundle_with(source, true)
}

fn scan_bundle_with(source: &str, addon: bool) -> Result<Report, String> {
    let parts = parse_bundle(source)?;
    let reach = reachable(&parts);
    let mut report = Report {
        entry: parts.entry.clone(),
        factories_total: parts.factories.len(),
        ..Default::default()
    };
    let interp = crate::Interp::new();
    // `File` exists only so `x instanceof File` is false for non-files; any other use is unsupported.
    let mut globals: HashSet<String> = interp
        .global_names()
        .into_iter()
        .filter(|n| n != "File")
        .collect();
    globals.extend(SQLITE_NATIVE_GLOBALS.iter().map(|g| g.to_string()));
    // the loader globals host.rs installs for node:module (see install_module_loader)
    let module_loader =
        parts.entry == "node:module" || parts.factories.iter().any(|(k, _)| k == "node:module");
    if module_loader {
        globals.extend(MODULE_LOADER_GLOBALS.iter().map(|g| g.to_string()));
    }
    let mut known_methods: HashSet<String> = interp.known_method_names().into_iter().collect();
    known_methods.extend(CALLSITE_METHODS.iter().map(|m| m.to_string()));
    let params = [
        "module",
        "exports",
        "require",
        "requireAsync",
        "__filename",
        "__dirname",
        "__thaw_require",
        "undefined",
    ];
    // First pass: names the whole reachable package defines (a method call on them is its own code).
    let mut walkers = Vec::new();
    for (key, func) in &parts.factories {
        if !reach.contains(key) {
            continue;
        }
        report.factories_reachable += 1;
        let mut w = Walker::default();
        func.visit_with(&mut w);
        walkers.push((key.clone(), w));
    }
    let node_require = walkers.iter().any(|(_, w)| {
        w.requires.iter().any(|r| {
            r.spec
                .as_deref()
                .is_some_and(|spec| spec.ends_with(".node"))
        })
    });
    let add = |report: &mut Report, feature: String, n: usize, key: &str| {
        if crate::SUPPORTED_FEATURES.contains(&feature.as_str())
            || tolerated(&feature, key)
            || (addon && addon_tolerated(&feature, node_require))
        {
            return;
        }
        let e = report
            .blockers
            .entry(feature)
            .or_insert((0, key.to_string()));
        e.0 += n;
    };
    for problem in &parts.esm_problems {
        add(&mut report, (*problem).to_string(), 1, &parts.entry);
    }
    let defined: HashSet<String> = walkers
        .iter()
        .flat_map(|(_, w)| w.defined.iter().cloned())
        .collect();
    for (key, w) in &walkers {
        for (f, n) in &w.features {
            add(&mut report, f.clone(), *n, key);
        }
        for r in &w.refs {
            // UMD wrappers fall back to `self` only when `globalThis` is missing
            let umd_self =
                r == "self" && (w.refs.contains("globalThis") || w.probed.contains("globalThis"));
            // Node-parity (R5): a free name Node 22 does not define is a ReferenceError on Node too, so it is no blocker.
            // Carve-out: a `__`-prefixed name is host-internal or a loader convention, never a Node name: still a blocker.
            if !w.declared.contains(r)
                && !umd_self
                && !globals.contains(r)
                && !params.contains(&r.as_str())
                && !w.probed.contains(r)
                // service-worker globals Node does not define either: a ReferenceError there, same here
                && r != "addEventListener"
                && is_node_or_host_internal(r)
            {
                add(&mut report, format!("global:{r}"), 1, key);
            }
        }
        for r in &w.instance_refs {
            let known = globals.contains(r) || r == "File";
            if !known
                && !w.declared.contains(r)
                && !params.contains(&r.as_str())
                && !w.probed.contains(r)
                && is_node_or_host_internal(r)
            {
                add(&mut report, format!("global:{r}"), 1, key);
            }
        }
        for (g, p) in &w.builtin_members {
            // Node-parity (R4): a member Node 22 lacks is undefined on Node too. A root the Node data does not
            // model (`Math`, `Array`, ...) keeps the old flag: the data cannot tell that Node lacks the member.
            // Carve-out: a `__`-prefixed member (`globalThis.__thaw_*`) is host-internal, so it stays a blocker.
            if interp.is_builtin_global(g)
                && g != "File"
                && !w.declared.contains(g)
                && !interp.builtin_member_exists(g, p)
                && (!node_global_covered(g) || node_global_has(g, p) || is_host_internal(p))
                // the package creates its own `globalThis.x` slot (`globalThis.x ??= {}`)
                && !(g == "globalThis" && (defined.contains(p) || w.probed_members.contains(p)))
                && !(module_loader && g == "globalThis" && p == "__thaw_bundle_create_require")
            {
                add(&mut report, format!("builtin:{g}.{p}"), 1, key);
            }
        }
        for m in &w.called {
            // `util.styleText` calls `stream.hasColors()` only after `typeof stream.hasColors === 'function'`
            // `process.dlopen` is exported now (it throws ERR_DLOPEN_FAILED), but no addon loads at runtime, so
            // a bundle that calls it stays a blocker, as it was when the member was missing.
            // Node-parity (R2): a name Node 22 does not have is the package's own method, never a missing one.
            if (!known_methods.contains(m) || m == "dlopen")
                && is_node_method_name(m)
                && !defined.contains(m)
                && !w.probed_methods.contains(m)
                && m != "hasColors"
            {
                add(&mut report, format!("method:{m}"), 1, key);
            }
        }
        let map = parts.require_maps.get(key);
        for r in &w.requires {
            match &r.spec {
                Some(spec) if map.is_some_and(|m| m.contains_key(spec)) => {}
                // an optional package that is not installed: the require throws MODULE_NOT_FOUND inside the
                // handling try, as on Node (host.rs `require_in`), so the program's catch takes over
                Some(spec) if r.guarded && is_optional_package_spec(spec) => {}
                Some(spec) => add(&mut report, format!("require:{spec}"), 1, key),
                None => add(&mut report, "require:dynamic".into(), 1, key),
            }
        }
    }
    Ok(report)
}
