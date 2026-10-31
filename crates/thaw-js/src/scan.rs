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
}

fn prop_key(p: &a::PropName) -> Option<String> {
    match p {
        a::PropName::Ident(i) => Some(i.sym.to_string()),
        a::PropName::Str(s) => s.value.as_str().map(str::to_string),
        _ => None,
    }
}

/// Extract factories, require maps and the entry key from a registry bundle.
pub fn parse_bundle(source: &str) -> Result<BundleParts, String> {
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
    requires: Vec<Option<String>>,
    /// identifiers probed with `typeof X` (assumed guarded)
    probed: HashSet<String>,
    /// `globalThis.X` members tested only for presence
    probed_members: HashSet<String>,
    probed_methods: HashSet<String>,
}

impl Walker {
    /// `globalThis.X` tested for truthiness / `typeof` is a feature probe, not a use.
    fn probe_member(&mut self, e: &a::Expr) {
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
        n.visit_children_with(self);
    }
    fn visit_class_decl(&mut self, n: &a::ClassDecl) {
        self.declared.insert(n.ident.sym.to_string());
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
        n.visit_children_with(self);
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        if n.is_async {
            self.feature("async");
        }
        if n.is_generator {
            self.feature("generator");
        }
        n.visit_children_with(self);
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
                    // generated source text with a `with (..)` block: `new Function` parses strict code only
                    if v.match_indices("with").any(|(i, _)| {
                        !v[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$')
                            && v[i + 4..].trim_start_matches(' ').starts_with('(')
                    }) {
                        self.feature("dynamic-with-code");
                    }
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
        if matches!(n.op, a::BinaryOp::LogicalAnd | a::BinaryOp::LogicalOr) {
            self.probe_member(&n.left);
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
                    if let (false, a::MemberProp::Ident(p)) = (origin, &m.prop) {
                        self.called.insert(p.sym.to_string());
                    }
                }
                // direct eval sees the caller's local scope, which the global-scope `eval` cannot
                a::Expr::Ident(i) if &*i.sym == "eval" => self.feature("direct-eval"),
                a::Expr::Ident(i) if &*i.sym == "require" => {
                    let lit = match n.args.first().map(|x| &*x.expr) {
                        Some(a::Expr::Lit(a::Lit::Str(s))) => s.value.as_str().map(str::to_string),
                        _ => None,
                    };
                    self.requires.push(lit);
                }
                _ => {}
            }
        }
        n.visit_children_with(self);
    }
}

/// Scan a registry bundle.
pub fn scan_bundle(source: &str) -> Result<Report, String> {
    let parts = parse_bundle(source)?;
    let reach = reachable(&parts);
    let mut report = Report {
        entry: parts.entry.clone(),
        factories_total: parts.factories.len(),
        ..Default::default()
    };
    let interp = crate::Interp::new();
    // `File` exists only so `x instanceof File` is false for non-files; any other use is unsupported.
    let globals: HashSet<String> = interp
        .global_names()
        .into_iter()
        .filter(|n| n != "File")
        .collect();
    let known_methods: HashSet<String> = interp.known_method_names().into_iter().collect();
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
    let add = |report: &mut Report, feature: String, n: usize, key: &str| {
        if crate::SUPPORTED_FEATURES.contains(&feature.as_str()) {
            return;
        }
        let e = report
            .blockers
            .entry(feature)
            .or_insert((0, key.to_string()));
        e.0 += n;
    };
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
            if !w.declared.contains(r)
                && !umd_self
                && !globals.contains(r)
                && !params.contains(&r.as_str())
                && !w.probed.contains(r)
                // service-worker globals Node does not define either: a ReferenceError there, same here
                && r != "addEventListener"
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
            {
                add(&mut report, format!("global:{r}"), 1, key);
            }
        }
        for (g, p) in &w.builtin_members {
            if interp.is_builtin_global(g)
                && g != "File"
                && !w.declared.contains(g)
                && !interp.builtin_member_exists(g, p)
                // the package creates its own `globalThis.x` slot (`globalThis.x ??= {}`)
                && !(g == "globalThis" && (defined.contains(p) || w.probed_members.contains(p)))
            {
                add(&mut report, format!("builtin:{g}.{p}"), 1, key);
            }
        }
        for m in &w.called {
            // `util.styleText` calls `stream.hasColors()` only after `typeof stream.hasColors === 'function'`
            if !known_methods.contains(m)
                && !defined.contains(m)
                && !w.probed_methods.contains(m)
                && m != "hasColors"
            {
                add(&mut report, format!("method:{m}"), 1, key);
            }
        }
        let map = parts.require_maps.get(key);
        for r in &w.requires {
            match r {
                Some(spec) if map.is_some_and(|m| m.contains_key(spec)) => {}
                Some(spec) => add(&mut report, format!("require:{spec}"), 1, key),
                None => add(&mut report, "require:dynamic".into(), 1, key),
            }
        }
    }
    Ok(report)
}
