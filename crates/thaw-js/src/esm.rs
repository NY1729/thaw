//! ESM-only packages: the registry's "native bundle" (`__thaw_register_native_bundle("<json>")`,
//! original ES module sources plus a per-module import map) is rewritten into the CommonJS
//! factory form the loader already runs.
//!
//! Imports become `require` calls whose results are bound once the dependency has finished
//! evaluating; exports become getters on `exports`. Constructs whose semantics that rewrite would
//! change are reported as scan blockers instead of being approximated: a mutable export (`export
//! let` / `export var`: a live binding), an import cycle, `import.meta`, top-level `await`.

use std::collections::{BTreeMap, BTreeSet};

use thaw_parser::ast as a;
use thaw_parser::common::{SourceMapper, Spanned};

use crate::builtins::json_quote;
use crate::host::Native;

/// The JSON payload literal of `globalThis.__thaw_register_native_bundle("<json>")` and whether
/// `source` is the pure native shape (otherwise a mixed bundle: CommonJS factories plus native modules).
pub(crate) fn native_payload(source: &str) -> Option<(String, bool)> {
    const HEAD: &str =
        "module.exports = (function() {\nvar entry = globalThis.__thaw_register_native_bundle(";
    const CALL: &str = "globalThis.__thaw_register_native_bundle(";
    let pure = source.starts_with(HEAD);
    let start = if pure {
        HEAD.len()
    } else {
        source.find(CALL)? + CALL.len()
    };
    string_literal_at(source.get(start..)?).map(|lit| (lit.to_string(), pure))
}

/// The JavaScript string literal at the head of `rest` (quotes included), honouring backslash escapes.
fn string_literal_at(rest: &str) -> Option<&str> {
    let mut chars = rest.char_indices();
    let (_, quote) = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let mut escaped = false;
    for (i, c) in chars {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some(&rest[..=i]);
        }
    }
    None
}

/// Indirect re-exports per module key, from the bundle's `var __thaw_bundle_export_graphs =
/// JSON.parse("...")`: `(exported name, specifier, imported name)`. Namespace (`*`) re-exports are
/// not included.
// ponytail: namespace re-exports (`export * as X`) of an indirect graph are skipped; add when a package needs it.
/// The JS string literal of a registry bundle's `var __thaw_bundle_export_graphs = JSON.parse("...")`
/// (quotes included): the JSON text of every ES module's export graph, keyed by module key.
pub(crate) fn export_graphs_literal(source: &str) -> Option<&str> {
    json_table_literal(source, "__thaw_bundle_export_graphs")
}

/// The string literal (quotes included) of `var <name> = JSON.parse(<literal>)` in a registry bundle.
pub(crate) fn json_table_literal<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let head = format!("var {name} = JSON.parse(");
    let at = source.find(&head)? + head.len();
    string_literal_at(source.get(at..)?)
}

fn indirect_exports(source: &str) -> BTreeMap<String, Vec<(String, String, String)>> {
    let mut out = BTreeMap::new();
    let Some(literal) = export_graphs_literal(source) else {
        return out;
    };
    let Ok(Native::Array(mut wrapped)) = crate::json::parse_native(&format!("[{literal}]")) else {
        return out;
    };
    let Some(Native::Str(json)) = wrapped.pop() else {
        return out;
    };
    let Ok(Native::Object(graphs)) = crate::json::parse_native(&json) else {
        return out;
    };
    for (key, graph) in graphs {
        let Native::Object(graph) = graph else {
            continue;
        };
        let Some(Native::Object(indirect)) = field(&graph, "indirect") else {
            continue;
        };
        let mut names = Vec::new();
        for (name, entry) in indirect {
            if let Native::Array(parts) = entry {
                if let [Native::Str(spec), Native::Str(remote), ..] = parts.as_slice() {
                    if remote != "*" {
                        names.push((name.clone(), spec.clone(), remote.clone()));
                    }
                }
            }
        }
        if !names.is_empty() {
            out.insert(key, names);
        }
    }
    out
}

fn field<'a>(fields: &'a [(String, Native)], name: &str) -> Option<&'a Native> {
    fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn text(v: Option<&Native>) -> Option<&str> {
    match v {
        Some(Native::Str(s)) => Some(s),
        _ => None,
    }
}

fn binding_names(p: &a::Pat, out: &mut Vec<String>) {
    match p {
        a::Pat::Ident(i) => out.push(i.id.sym.to_string()),
        a::Pat::Array(arr) => arr
            .elems
            .iter()
            .flatten()
            .for_each(|e| binding_names(e, out)),
        a::Pat::Rest(r) => binding_names(&r.arg, out),
        a::Pat::Assign(d) => binding_names(&d.left, out),
        a::Pat::Object(o) => {
            for prop in &o.props {
                match prop {
                    a::ObjectPatProp::KeyValue(kv) => binding_names(&kv.value, out),
                    a::ObjectPatProp::Assign(asg) => out.push(asg.key.id.sym.to_string()),
                    a::ObjectPatProp::Rest(r) => binding_names(&r.arg, out),
                }
            }
        }
        _ => {}
    }
}

fn export_name(n: &a::ModuleExportName) -> String {
    match n {
        a::ModuleExportName::Ident(i) => i.sym.to_string(),
        a::ModuleExportName::Str(s) => s.value.to_string_lossy().into_owned(),
    }
}

const ORIGIN_SPEC: &str = "__thaw_private_origin_spec/";

// The getter is non-configurable, so a second define of the same name throws; the first definition wins.
fn getter(exports: &str, name: &str, expr: &str) -> String {
    let mut quoted = String::new();
    json_quote(name, &mut quoted);
    format!(
        "if (!Object.prototype.hasOwnProperty.call({exports}, {quoted})) Object.defineProperty({exports}, {quoted}, {{ enumerable: true, get: function () {{ return {expr}; }} }});\n"
    )
}

/// The identifiers a factory body uses for its CommonJS `require` and `exports`.
struct Names {
    require: &'static str,
    exports: &'static str,
}

const BUNDLE_NAMES: Names = Names {
    require: "require",
    exports: "exports",
};

// An entry's factory takes these parameters, so user code's own `require` and `exports` stay unbound.
const ENTRY_NAMES: Names = Names {
    require: "__thaw_esm_require",
    exports: "__thaw_esm_exports",
};

/// One module's CommonJS factory body, plus the specifiers it imports and the problems found.
struct Converted {
    body: String,
    specs: Vec<String>,
    problems: Vec<&'static str>,
    // mutable exports (`export let x`, reassigned here) and remote names of this module's named imports
    live: Vec<String>,
    imported: Vec<String>,
    // `export * from`: copies every name of its source, so any live export of the bundle may be copied
    star: bool,
}

/// Names assigned or updated anywhere in the module (`x = ..`, `x += ..`, `x++`).
// ponytail: destructuring-assignment targets are not tracked; add when a package needs it.
fn reassigned_names(module: &a::Module) -> BTreeSet<String> {
    use swc_ecma_visit::{Visit, VisitWith};
    #[derive(Default)]
    struct Assigned(BTreeSet<String>);
    impl Visit for Assigned {
        fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
            if let a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(i)) = &n.left {
                self.0.insert(i.id.sym.to_string());
            }
            n.visit_children_with(self);
        }
        fn visit_update_expr(&mut self, n: &a::UpdateExpr) {
            if let a::Expr::Ident(i) = &*n.arg {
                self.0.insert(i.sym.to_string());
            }
            n.visit_children_with(self);
        }
    }
    let mut v = Assigned::default();
    module.visit_with(&mut v);
    v.0
}

/// `indirect`: `(exported name, specifier, imported name)` re-exports that the registry removed from
/// `source` and kept only in the export graph.
fn convert(
    source: &str,
    public_specs: &[String],
    indirect: &[(String, String, String)],
) -> Result<Converted, String> {
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source)?;
    Ok(convert_module(
        &module,
        &cm,
        public_specs,
        indirect,
        &BUNDLE_NAMES,
    ))
}

/// `convert` for a parsed module, with the factory identifiers `names`.
fn convert_module(
    module: &a::Module,
    cm: &thaw_parser::common::SourceMap,
    public_specs: &[String],
    indirect: &[(String, String, String)],
    names: &Names,
) -> Converted {
    let (rq, ex) = (names.require, names.exports);
    let snippet = |span| cm.span_to_snippet(span).unwrap_or_default();
    let mut head =
        format!("\"use strict\";\nObject.defineProperty({ex}, '__esModule', {{ value: true }});\n");
    head.push_str(
        "function __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\n\
         function __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); \
         if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } \
         ns.default = m; return ns; }\n",
    );
    let mut body = String::new();
    let mut specs: Vec<String> = Vec::new();
    // names this source re-exports itself (`export { X } from`); the export graph repeats them as indirect entries
    let mut reexported: BTreeSet<String> = BTreeSet::new();
    let mut problems: Vec<&'static str> = Vec::new();
    let mut live: Vec<String> = Vec::new();
    // remote names this module reads from another module (imports, re-exports, indirect entries)
    let mut pulled: Vec<String> = Vec::new();
    let mut star = false;
    let mut counter = 0usize;
    let mut uses_origin = false;
    let reassigned = reassigned_names(module);
    fn tmp(counter: &mut usize) -> String {
        *counter += 1;
        format!("__thaw_esm_m{counter}")
    }
    for item in &module.body {
        match item {
            a::ModuleItem::Stmt(s) => {
                body.push_str(&snippet(s.span()));
                body.push_str(";\n");
            }
            a::ModuleItem::ModuleDecl(decl) => match decl {
                a::ModuleDecl::Import(i) => {
                    let spec = i.src.value.to_string_lossy().into_owned();
                    if spec.starts_with(ORIGIN_SPEC) {
                        // the registry's per-module origin: `readDefault("node:os")` etc. on literal requires
                        let table: Vec<String> = public_specs
                            .iter()
                            .map(|p| {
                                let mut q = String::new();
                                json_quote(p, &mut q);
                                specs.push(p.clone());
                                format!("{q}: function () {{ return {rq}({q}); }}")
                            })
                            .collect();
                        for sp in &i.specifiers {
                            if let a::ImportSpecifier::Named(n) = sp {
                                body.push_str(&format!(
                                    "var {} = __thaw_esm_origin_obj({{ {} }});\n",
                                    n.local.sym,
                                    table.join(", ")
                                ));
                            }
                        }
                        uses_origin = true;
                        continue;
                    }
                    let mut q = String::new();
                    json_quote(&spec, &mut q);
                    specs.push(spec);
                    let m = tmp(&mut counter);
                    body.push_str(&format!("var {m} = {rq}({q});\n"));
                    for sp in &i.specifiers {
                        match sp {
                            a::ImportSpecifier::Default(d) => body.push_str(&format!(
                                "var {} = __thaw_esm_default({m});\n",
                                d.local.sym
                            )),
                            a::ImportSpecifier::Namespace(n) => body
                                .push_str(&format!("var {} = __thaw_esm_ns({m});\n", n.local.sym)),
                            a::ImportSpecifier::Named(n) => {
                                let imported = n
                                    .imported
                                    .as_ref()
                                    .map(export_name)
                                    .unwrap_or_else(|| n.local.sym.to_string());
                                if imported == "default" {
                                    body.push_str(&format!(
                                        "var {} = __thaw_esm_default({m});\n",
                                        n.local.sym
                                    ));
                                } else {
                                    pulled.push(imported.clone());
                                    let mut qi = String::new();
                                    json_quote(&imported, &mut qi);
                                    body.push_str(&format!("var {} = {m}[{qi}];\n", n.local.sym));
                                }
                            }
                        }
                    }
                }
                a::ModuleDecl::ExportDecl(e) => {
                    let mut names = Vec::new();
                    match &e.decl {
                        a::Decl::Fn(f) => names.push(f.ident.sym.to_string()),
                        a::Decl::Class(c) => names.push(c.ident.sym.to_string()),
                        a::Decl::Var(v) => {
                            for d in &v.decls {
                                binding_names(&d.name, &mut names);
                            }
                            // a mutable export is a live getter; it only goes stale in an importer that copies it
                            if !matches!(v.kind, a::VarDeclKind::Const) {
                                live.extend(
                                    names.iter().filter(|n| reassigned.contains(*n)).cloned(),
                                );
                            }
                        }
                        _ => problems.push("esm-declaration"),
                    }
                    for n in &names {
                        head.push_str(&getter(ex, n, n));
                    }
                    body.push_str(&snippet(e.decl.span()));
                    body.push_str(";\n");
                }
                a::ModuleDecl::ExportDefaultDecl(e) => {
                    let (name, text) = match &e.decl {
                        a::DefaultDecl::Fn(f) => (
                            f.ident.as_ref().map(|i| i.sym.to_string()),
                            snippet(f.function.span()),
                        ),
                        a::DefaultDecl::Class(c) => (
                            c.ident.as_ref().map(|i| i.sym.to_string()),
                            snippet(c.class.span()),
                        ),
                        _ => {
                            problems.push("esm-declaration");
                            continue;
                        }
                    };
                    match name {
                        Some(n) => {
                            // a named declaration: keep it as such and export the binding
                            body.push_str(&text);
                            body.push_str(";\n");
                            head.push_str(&getter(ex, "default", &n));
                        }
                        None => body.push_str(&format!("{ex}.default = ({text});\n")),
                    }
                }
                a::ModuleDecl::ExportDefaultExpr(e) => {
                    body.push_str(&format!("{ex}.default = ({});\n", snippet(e.expr.span())));
                }
                a::ModuleDecl::ExportNamed(e) => {
                    let from = e.src.as_ref().map(|s| {
                        let spec = s.value.to_string_lossy().into_owned();
                        let mut q = String::new();
                        json_quote(&spec, &mut q);
                        specs.push(spec);
                        let m = tmp(&mut counter);
                        body.push_str(&format!("var {m} = {rq}({q});\n"));
                        m
                    });
                    for sp in &e.specifiers {
                        match sp {
                            a::ExportSpecifier::Named(n) => {
                                let local = export_name(&n.orig);
                                if from.is_some() {
                                    pulled.push(local.clone());
                                }
                                let exported = n
                                    .exported
                                    .as_ref()
                                    .map(export_name)
                                    .unwrap_or_else(|| local.clone());
                                let expr = match &from {
                                    Some(m) => {
                                        let mut q = String::new();
                                        json_quote(&local, &mut q);
                                        if local == "default" {
                                            format!("__thaw_esm_default({m})")
                                        } else {
                                            format!("{m}[{q}]")
                                        }
                                    }
                                    None => local,
                                };
                                if from.is_some() {
                                    reexported.insert(exported.clone());
                                }
                                head.push_str(&getter(ex, &exported, &expr));
                            }
                            a::ExportSpecifier::Namespace(n) => {
                                if let Some(m) = &from {
                                    head.push_str(&getter(
                                        ex,
                                        &export_name(&n.name),
                                        &format!("__thaw_esm_ns({m})"),
                                    ));
                                }
                            }
                            a::ExportSpecifier::Default(_) => problems.push("esm-declaration"),
                        }
                    }
                }
                a::ModuleDecl::ExportAll(e) => {
                    star = true;
                    let spec = e.src.value.to_string_lossy().into_owned();
                    let mut q = String::new();
                    json_quote(&spec, &mut q);
                    specs.push(spec);
                    let m = tmp(&mut counter);
                    body.push_str(&format!(
                        "var {m} = {rq}({q});\nObject.keys({m}).forEach(function (k) {{ if (k !== 'default' && k !== '__esModule' && !Object.prototype.hasOwnProperty.call({ex}, k)) Object.defineProperty({ex}, k, {{ enumerable: true, get: function () {{ return {m}[k]; }} }}); }});\n"
                    ));
                }
                _ => problems.push("esm-declaration"),
            },
        }
    }
    // Indirect getters go in the head so an `export *` in the body skips their names.
    for (name, spec, remote) in indirect {
        pulled.push(remote.clone());
        if reexported.contains(name) {
            continue;
        }
        let (mut qs, mut qr) = (String::new(), String::new());
        json_quote(spec, &mut qs);
        json_quote(remote, &mut qr);
        let expr = if remote == "default" {
            format!("__thaw_esm_default({rq}({qs}))")
        } else {
            format!("{rq}({qs})[{qr}]")
        };
        head.push_str(&getter(ex, name, &expr));
    }
    if uses_origin {
        head.push_str(
            "function __thaw_esm_origin_obj(t) { return { readDefault: function (s) { return __thaw_esm_default(t[s]()); }, \
             read: function (s, n) { return t[s]()[n]; }, readNamespace: function (s) { return __thaw_esm_ns(t[s]()); }, \
             readImport: function (s, n) { return t[s]()[n]; }, namespaceImport: function (s) { return t[s](); }, \
             defaultImport: function (s, m) { return __thaw_esm_default(m); } }; }\n",
        );
    }
    Converted {
        body: format!("{head}{body}"),
        specs,
        problems,
        live,
        imported: pulled,
        star,
    }
}

/// The factory text, the unsound-construct feature names, and whether the bundle is the pure native shape.
pub(crate) type NativeBundle = (String, BTreeSet<&'static str>, bool);

/// Rewrite a native bundle into the registry's CommonJS factory bundle text. `Ok(None)`: `source`
/// is not a native bundle. Problems that make the rewrite unsound are returned as feature names.
///
/// For a mixed bundle the result is only the native modules' factories and maps (no entry key): the caller
/// appends it to the original text, where its factories replace the stubs of the same key.
pub(crate) fn native_bundle_to_factories(source: &str) -> Result<Option<NativeBundle>, String> {
    let Some((literal, pure)) = native_payload(source) else {
        return Ok(None);
    };
    // the payload is a JS string literal holding JSON
    let json = match crate::json::parse_native(&format!("[{literal}]")) {
        Ok(Native::Array(mut v)) => match v.pop() {
            Some(Native::Str(s)) => s,
            _ => return Err("native bundle payload is not a string".into()),
        },
        _ => return Err("native bundle payload is not a string".into()),
    };
    let Ok(Native::Object(root)) = crate::json::parse_native(&json) else {
        return Err("native bundle payload is not JSON".into());
    };
    let main = text(field(&root, "main"))
        .ok_or("native bundle has no main module")?
        .to_string();
    let Some(Native::Array(modules)) = field(&root, "modules") else {
        return Err("native bundle has no modules".into());
    };
    // opaque edges name the CommonJS factory behind an import of a non-native module
    let mut opaque: BTreeMap<String, String> = BTreeMap::new();
    if let Some(Native::Array(edges)) = field(&root, "opaqueEdges") {
        for e in edges {
            if let Native::Object(f) = e {
                if let (Some(k), Some(t)) = (text(field(f, "key")), text(field(f, "factory"))) {
                    opaque.insert(k.to_string(), t.to_string());
                }
            }
        }
    }
    let mut factories = String::new();
    let mut maps = String::new();
    let mut problems: BTreeSet<&'static str> = BTreeSet::new();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let (mut live_exports, mut imported_names) = (BTreeSet::new(), BTreeSet::new());
    let mut any_star = false;
    let indirects = indirect_exports(source);
    for m in modules {
        let Native::Object(fields) = m else { continue };
        let key = text(field(fields, "key"))
            .ok_or("native module without key")?
            .to_string();
        let src = text(field(fields, "source")).ok_or("native module without source")?;
        let mut imports: Vec<(String, String)> = Vec::new();
        if let Some(Native::Object(map)) = field(fields, "imports") {
            for (spec, target) in map {
                if let Native::Str(t) = target {
                    if spec.starts_with(ORIGIN_SPEC) || t.starts_with("__thaw_private_origin_key/")
                    {
                        continue;
                    }
                    let t = opaque.get(t).unwrap_or(t);
                    imports.push((spec.clone(), t.clone()));
                }
            }
        }
        let public: Vec<String> = imports.iter().map(|(s, _)| s.clone()).collect();
        let indirect = indirects.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let converted = convert(src, &public, indirect)?;
        problems.extend(converted.problems);
        live_exports.extend(converted.live);
        imported_names.extend(converted.imported);
        any_star |= converted.star;
        graph.insert(
            key.clone(),
            imports
                .iter()
                .filter(|(spec, _)| converted.specs.contains(spec))
                .map(|(_, t)| t.clone())
                .collect(),
        );
        let mut qk = String::new();
        json_quote(&key, &mut qk);
        // `requireAsync` is the module's import(): a bare `import(spec)` in the body calls it (lib.rs)
        factories.push_str(&format!(
            "{qk}: function (module, exports, require, requireAsync) {{\n{}\n}},\n",
            converted.body
        ));
        let entries: Vec<String> = imports
            .iter()
            .map(|(spec, target)| {
                let (mut qs, mut qt) = (String::new(), String::new());
                json_quote(spec, &mut qs);
                json_quote(target, &mut qt);
                format!("{qs}: {qt}")
            })
            .collect();
        maps.push_str(&format!("{qk}: {{ {} }},\n", entries.join(", ")));
    }
    // a mutable export goes stale when a module copies it. Matched by name over every pulled name
    // (imports, re-exports, indirect entries); an `export *` may copy any name, so it blocks too.
    // ponytail: name match over-blocks same-named unrelated bindings; add per-module export tracking when it bites.
    if live_exports.intersection(&imported_names).next().is_some()
        || (any_star && !live_exports.is_empty())
    {
        problems.insert("esm-live-binding");
    }
    // an import cycle would observe half-initialised bindings under the snapshot rewrite
    fn cyclic(
        graph: &BTreeMap<String, Vec<String>>,
        node: &str,
        stack: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> bool {
        if stack.iter().any(|s| s == node) {
            return true;
        }
        if !done.insert(node.to_string()) {
            return false;
        }
        stack.push(node.to_string());
        let found = graph
            .get(node)
            .is_some_and(|next| next.iter().any(|n| cyclic(graph, n, stack, done)));
        stack.pop();
        found
    }
    if cyclic(&graph, &main, &mut Vec::new(), &mut BTreeSet::new()) {
        problems.insert("esm-cycle");
    }
    let mut qm = String::new();
    json_quote(&main, &mut qm);
    let entry = if pure {
        format!("var __thaw_bundle_entry_key = {qm};\n")
    } else {
        String::new()
    };
    let bundle = format!(
        "module.exports = (function (require) {{\nvar __thaw_bundle_factories = {{\n{factories}}};\nvar __thaw_bundle_require_maps = {{\n{maps}}};\n{entry}}})(globalThis.require);\n"
    );
    Ok(Some((bundle, problems, pure)))
}

#[cfg(test)]
mod origin_vocabulary_tests {
    use super::convert;

    // The origin object must carry every method the registry transform emits
    // (module_transform.rs readImport/readDefault/namespaceImport/defaultImport).
    #[test]
    fn origin_object_has_the_registry_vocabulary() {
        let source = "import { __thaw_esm_origin_0 } from '__thaw_private_origin_spec/k';\n\
                      export const x = __thaw_esm_origin_0.readImport('dep', 'a');\n";
        let converted = convert(source, &["dep".to_string()], &[]).expect("convert");
        for method in [
            "readImport",
            "namespaceImport",
            "defaultImport",
            "readDefault",
        ] {
            assert!(
                converted.body.contains(&format!("{method}: function")),
                "origin object missing {method}: {}",
                converted.body
            );
        }
    }

    // `export { EventEmitter } from "./index.js"` in an `.mjs` whose source the registry rewrote:
    // the name exists only as an indirect entry of the export graph, and must still be readable.
    #[test]
    fn indirect_cjs_default_reexport_is_readable_from_native_module() {
        use crate::builtins::json_quote;
        use crate::host::{Native, Package};
        let payload = r#"{"main":"m/e.mjs","modules":[{"key":"m/e.mjs","source":"import \"./index.js\";\nexport {};","imports":{"./index.js":"m/cjs.js"}}],"opaqueEdges":[]}"#;
        let graphs = r#"{"m/e.mjs":{"local":{},"indirect":{"EventEmitter":["./index.js","default",true]},"stars":[]}}"#;
        let (mut literal, mut graph_literal) = (String::new(), String::new());
        json_quote(payload, &mut literal);
        json_quote(graphs, &mut graph_literal);
        let bundle = format!(
            r#"module.exports = (function(require) {{
var __thaw_bundle_factories = {{
"m/index.js": function(module, exports, require) {{ var e = require("./e.mjs"); exports.tag = function () {{ return new (e.EventEmitter)().tag; }}; }},
"m/cjs.js": function(module, exports) {{ module.exports = function EventEmitter() {{ this.tag = "ok"; }}; }},
"m/e.mjs": function() {{ throw new TypeError('Native ESM must be loaded by its registered entry'); }},
}};
var __thaw_bundle_require_maps = {{ "m/index.js": {{"./e.mjs": "m/e.mjs"}}, "m/cjs.js": {{}}, "m/e.mjs": {{}}, }};
var __thaw_bundle_export_graphs = JSON.parse({graph_literal});
var __thaw_bundle_entry_key = "m/index.js";
var __thaw_native_registered = globalThis.__thaw_register_native_bundle({literal});
}})(globalThis.require);"#
        );
        let mut p = Package::load(&bundle).expect("loads");
        assert_eq!(p.call("tag", &[]).unwrap(), Native::Str("ok".into()));
    }
}

#[cfg(test)]
mod export_getter_tests {
    use crate::builtins::json_quote;
    use crate::host::{Native, Package};

    // One native ESM module `m/e.mjs` with `source` (its `./e.js` import served by `m/c.js`, built from
    // `dep_factory`) and the export graph `indirect`; `m/index.js` probes `e.<name>` through `require`.
    fn load_entry(source: &str, indirect: &str, dep_factory: &str, name: &str) -> Package {
        let payload = serde_json::json!({
            "main": "m/e.mjs",
            "modules": [{ "key": "m/e.mjs", "source": source, "imports": { "./e.js": "m/c.js" } }],
            "opaqueEdges": [],
        })
        .to_string();
        let graph = format!(r#"{{"m/e.mjs":{{"local":{{}},"indirect":{indirect},"stars":[]}}}}"#);
        let (mut literal, mut graph_literal) = (String::new(), String::new());
        json_quote(&payload, &mut literal);
        json_quote(&graph, &mut graph_literal);
        let bundle = format!(
            r#"module.exports = (function(require) {{
var __thaw_bundle_factories = {{
"m/index.js": function(module, exports, require) {{ var e = require("./e.mjs"); exports.probe = function () {{ return typeof e.{name}; }}; }},
"m/c.js": function(module, exports) {{ {dep_factory} }},
"m/e.mjs": function() {{ throw new TypeError('Native ESM must be loaded by its registered entry'); }},
}};
var __thaw_bundle_require_maps = {{ "m/index.js": {{"./e.mjs": "m/e.mjs"}}, "m/c.js": {{}}, "m/e.mjs": {{}}, }};
var __thaw_bundle_export_graphs = JSON.parse({graph_literal});
var __thaw_bundle_entry_key = "m/index.js";
var __thaw_native_registered = globalThis.__thaw_register_native_bundle({literal});
}})(globalThis.require);"#
        );
        Package::load(&bundle).expect("loads")
    }

    const DEP: &str = "module.exports = { KyError: function KyError() {}, HTTPError: function HTTPError() {}, TimeoutError: function TimeoutError() {} };";

    // (a) `export { X } from` in the source plus an indirect entry for the same X (the shape the registry
    // leaves behind for a re-export barrel such as ky's `export { KyError } from './errors/KyError.js'`).
    #[test]
    fn re_export_with_matching_indirect_entry_defines_once() {
        let mut p = load_entry(
            "export { KyError } from \"./e.js\";\n",
            r#"{"KyError":["./e.js","KyError",false]}"#,
            DEP,
            "KyError",
        );
        assert_eq!(
            p.call("probe", &[]).unwrap(),
            Native::Str("function".into())
        );
    }

    // (b) a local `export class X {}` in the source plus an indirect entry for the same X.
    #[test]
    fn local_class_with_matching_indirect_entry_defines_once() {
        let mut p = load_entry(
            "export class KyError {}\n",
            r#"{"KyError":["./e.js","KyError",false]}"#,
            DEP,
            "KyError",
        );
        assert_eq!(
            p.call("probe", &[]).unwrap(),
            Native::Str("function".into())
        );
    }

    // (c) ky 2.1.0's distribution/index.js export list (the real re-exports, each also an indirect entry);
    // the module paths are collapsed to the one import this test serves.
    #[test]
    fn ky_shaped_barrel_defines_each_name_once() {
        let source = "const ky = {};\nexport default ky;\n\
export { KyError } from './e.js';\n\
export { HTTPError } from './e.js';\n\
export { SchemaValidationError } from './e.js';\n\
export { NetworkError } from './e.js';\n\
export { TimeoutError } from './e.js';\n\
export { ForceRetryError } from './e.js';\n\
export { isKyError, isHTTPError, isNetworkError, isTimeoutError, isForceRetryError, } from './e.js';\n\
export { replaceOption } from './e.js';\n";
        let names = [
            "KyError",
            "HTTPError",
            "SchemaValidationError",
            "NetworkError",
            "TimeoutError",
            "ForceRetryError",
            "isKyError",
            "isHTTPError",
            "isNetworkError",
            "isTimeoutError",
            "isForceRetryError",
            "replaceOption",
        ];
        let indirect = names
            .iter()
            .map(|n| format!("\"{n}\":[\"./e.js\",\"{n}\",false]"))
            .collect::<Vec<_>>()
            .join(",");
        let dep = names
            .iter()
            .map(|n| format!("{n}: function () {{}}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut p = load_entry(
            source,
            &format!("{{{indirect}}}"),
            &format!("module.exports = {{ {dep} }};"),
            "KyError",
        );
        assert_eq!(
            p.call("probe", &[]).unwrap(),
            Native::Str("function".into())
        );
    }
}

#[cfg(test)]
mod live_binding_tests {
    use crate::builtins::json_quote;
    use crate::host::{HostError, Native, Package};

    // A mixed bundle: the native `modules` (`key`, `source`, `imports`) with main `main`, and the CommonJS
    // entry `m/index.js` whose body is `index` (same shape as export_getter_tests::load_entry).
    fn bundle(main: &str, modules: serde_json::Value, index: &str) -> String {
        let payload =
            serde_json::json!({ "main": main, "modules": modules, "opaqueEdges": [] }).to_string();
        let mut literal = String::new();
        json_quote(&payload, &mut literal);
        format!(
            r#"module.exports = (function(require) {{
var __thaw_bundle_factories = {{
"m/index.js": function(module, exports, require) {{ {index} }},
"m/e.mjs": function() {{ throw new TypeError('Native ESM must be loaded by its registered entry'); }},
}};
var __thaw_bundle_require_maps = {{ "m/index.js": {{"./e.mjs": "m/e.mjs"}}, "m/e.mjs": {{}}, }};
var __thaw_bundle_entry_key = "m/index.js";
var __thaw_native_registered = globalThis.__thaw_register_native_bundle({literal});
}})(globalThis.require);"#
        )
    }

    // (a) no importer copies `value`: the CommonJS consumer reads the mutable export live through its getter.
    #[test]
    fn mutable_export_without_snapshot_importer_is_live_for_commonjs() {
        let modules = serde_json::json!([{
            "key": "m/e.mjs",
            "source": "export let value = 0;\nexport function bump() { value++; }\n",
            "imports": {}
        }]);
        let index = r#"var e = require("./e.mjs"); exports.run = function () { e.bump(); e.bump(); return e.value; };"#;
        let mut p = Package::load(&bundle("m/e.mjs", modules, index)).expect("loads");
        assert_eq!(p.call("run", &[]).unwrap(), Native::Number(2.0));
    }

    // (b) an in-bundle importer copies `value` (`var value = m["value"]`): the blocker stays.
    #[test]
    fn mutable_export_copied_by_in_bundle_importer_still_blocks() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": "export let value = 0;\nexport function bump() { value++; }\n", "imports": {} },
            { "key": "m/i.mjs", "source": "import { value } from \"./e.mjs\";\nexport const seen = value;\n",
              "imports": { "./e.mjs": "m/e.mjs" } }
        ]);
        let err = Package::load(&bundle("m/e.mjs", modules, "")).err();
        assert!(
            matches!(&err, Some(HostError::Unsupported(f)) if f == "esm-live-binding"),
            "{err:?}"
        );
    }

    fn assert_blocked(src: &str) {
        let err = Package::load(src).err();
        assert!(
            matches!(&err, Some(HostError::Unsupported(f)) if f == "esm-live-binding"),
            "{err:?}"
        );
    }

    // (c) renamed re-export chain: B re-exports the mutable `value` as `v`, C copies it (`var v = B["v"]`).
    #[test]
    fn mutable_export_renamed_through_reexport_chain_still_blocks() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": "export let value = 0;\nexport function bump() { value++; }\n", "imports": {} },
            { "key": "m/b.mjs", "source": "export { value as v } from \"./e.mjs\";\n", "imports": { "./e.mjs": "m/e.mjs" } },
            { "key": "m/c.mjs", "source": "import { v } from \"./b.mjs\";\nexport const seen = v;\n", "imports": { "./b.mjs": "m/b.mjs" } }
        ]);
        assert_blocked(&bundle("m/e.mjs", modules, ""));
    }

    // (d) `export *` of a module with a mutable live export: the star re-export copies its names, so it blocks.
    #[test]
    fn mutable_export_through_export_star_still_blocks() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": "export let value = 0;\nexport function bump() { value++; }\n", "imports": {} },
            { "key": "m/s.mjs", "source": "export * from \"./e.mjs\";\n", "imports": { "./e.mjs": "m/e.mjs" } }
        ]);
        assert_blocked(&bundle("m/e.mjs", modules, ""));
    }
}

#[cfg(test)]
mod convert_pin_tests {
    use super::convert;

    // Pins the bundle conversion byte for byte: `convert` (the registry bundle path) must not change
    // when the entry path is added.
    #[test]
    fn bundle_conversion_is_byte_identical() {
        let source = "import { a } from './x.js';\nexport const b = a + 1;\nexport default a;\n";
        let indirect = [("c".to_string(), "./y.js".to_string(), "c".to_string())];
        let converted = convert(source, &[], &indirect).expect("convert");
        let expected = concat!(
            "\"use strict\";\nObject.defineProperty(exports, '__esModule', { value: true });\n",
            "function __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\n",
            "function __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); ",
            "if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } ",
            "ns.default = m; return ns; }\n",
            "if (!Object.prototype.hasOwnProperty.call(exports, \"b\")) Object.defineProperty(exports, \"b\", { enumerable: true, get: function () { return b; } });\n",
            "if (!Object.prototype.hasOwnProperty.call(exports, \"c\")) Object.defineProperty(exports, \"c\", { enumerable: true, get: function () { return require(\"./y.js\")[\"c\"]; } });\n",
            "var __thaw_esm_m1 = require(\"./x.js\");\n",
            "var a = __thaw_esm_m1[\"a\"];\n",
            "const b = a + 1;;\n",
            "exports.default = (a);\n",
        );
        assert_eq!(converted.body, expected);
        assert_eq!(converted.specs, vec!["./x.js".to_string()]);
    }

    // The other bundle shapes the rewrite touches: `export *`, a re-export, a namespace re-export and the origin object.
    #[test]
    fn bundle_reexport_shapes_are_byte_identical() {
        let source = "import { __thaw_esm_origin_0 } from '__thaw_private_origin_spec/m/k';\n\
            export * from './s.js';\nexport { q as r } from './t.js';\nexport * as ns from './u.js';\n\
            export const v = __thaw_esm_origin_0.readDefault('node:os');\n";
        let converted = convert(source, &["node:os".to_string()], &[]).expect("convert");
        assert_eq!(converted.body, "\"use strict\";\nObject.defineProperty(exports, '__esModule', { value: true });\nfunction __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\nfunction __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } ns.default = m; return ns; }\nif (!Object.prototype.hasOwnProperty.call(exports, \"r\")) Object.defineProperty(exports, \"r\", { enumerable: true, get: function () { return __thaw_esm_m2[\"q\"]; } });\nif (!Object.prototype.hasOwnProperty.call(exports, \"ns\")) Object.defineProperty(exports, \"ns\", { enumerable: true, get: function () { return __thaw_esm_ns(__thaw_esm_m3); } });\nif (!Object.prototype.hasOwnProperty.call(exports, \"v\")) Object.defineProperty(exports, \"v\", { enumerable: true, get: function () { return v; } });\nfunction __thaw_esm_origin_obj(t) { return { readDefault: function (s) { return __thaw_esm_default(t[s]()); }, read: function (s, n) { return t[s]()[n]; }, readNamespace: function (s) { return __thaw_esm_ns(t[s]()); }, readImport: function (s, n) { return t[s]()[n]; }, namespaceImport: function (s) { return t[s](); }, defaultImport: function (s, m) { return __thaw_esm_default(m); } }; }\nvar __thaw_esm_origin_0 = __thaw_esm_origin_obj({ \"node:os\": function () { return require(\"node:os\"); } });\nvar __thaw_esm_m1 = require(\"./s.js\");\nObject.keys(__thaw_esm_m1).forEach(function (k) { if (k !== 'default' && k !== '__esModule' && !Object.prototype.hasOwnProperty.call(exports, k)) Object.defineProperty(exports, k, { enumerable: true, get: function () { return __thaw_esm_m1[k]; } }); });\nvar __thaw_esm_m2 = require(\"./t.js\");\nvar __thaw_esm_m3 = require(\"./u.js\");\nconst v = __thaw_esm_origin_0.readDefault('node:os');;\n");
    }
}

/// A runtime ES-module entry as a CommonJS-style body: its helpers are `__thaw_esm_require` and
/// `__thaw_esm_exports`, so user code's `require`/`exports` are never shadowed.
#[derive(Debug, PartialEq)]
pub(crate) struct EntryModule {
    pub code: String,
    pub tla: bool,
    pub imports: Vec<String>,
}

/// Converts a runtime ES-module entry. Imports, re-exports and `export *` all count as static specifiers.
pub(crate) fn convert_entry(source: &str) -> Result<EntryModule, String> {
    let (module, cm) = thaw_parser::parse_javascript_module_located(source).map_err(|f| {
        format!(
            "failed to parse module: {} ({}:{})",
            f.message, f.line, f.col
        )
    })?;
    let converted = convert_module(&module, &cm, &[], &[], &ENTRY_NAMES);
    if let Some(problem) = converted.problems.first() {
        return Err(format!("unsupported module syntax: {problem}"));
    }
    Ok(EntryModule {
        code: converted.body,
        tla: has_top_level_await(&module),
        imports: converted.specs,
    })
}

/// Whether `module` awaits outside every function: a top-level `await` or `for await`.
fn has_top_level_await(module: &a::Module) -> bool {
    use swc_ecma_visit::{Visit, VisitWith};
    #[derive(Default)]
    struct Tla(bool);
    impl Visit for Tla {
        fn visit_await_expr(&mut self, _: &a::AwaitExpr) {
            self.0 = true;
        }
        fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
            self.0 |= n.is_await;
            n.visit_children_with(self);
        }
        fn visit_function(&mut self, _: &a::Function) {}
        fn visit_arrow_expr(&mut self, _: &a::ArrowExpr) {}
    }
    let mut v = Tla::default();
    module.visit_with(&mut v);
    v.0
}

/// `convert_entry` as the JSON text `{"code", "tla", "imports"}` that `__thaw_esm_convert` returns.
pub(crate) fn convert_entry_json(source: &str) -> Result<String, String> {
    let entry = convert_entry(source)?;
    let mut code = String::new();
    json_quote(&entry.code, &mut code);
    let imports: Vec<String> = entry
        .imports
        .iter()
        .map(|s| {
            let mut q = String::new();
            json_quote(s, &mut q);
            q
        })
        .collect();
    Ok(format!(
        "{{\"code\":{code},\"tla\":{},\"imports\":[{}]}}",
        entry.tla,
        imports.join(",")
    ))
}

impl crate::Interp {
    /// Defines the native `__thaw_esm_convert(source)`: `convert_entry_json`, throwing a SyntaxError on failure.
    pub(crate) fn install_esm_convert(&mut self) {
        let convert = self.native(|it, _, a| {
            let source = it.to_str(&crate::arg(a, 0)).to_string();
            match convert_entry_json(&source) {
                Ok(json) => Ok(crate::Value::str(&json)),
                Err(msg) => it.throw_error("SyntaxError", &msg),
            }
        });
        self.global.declare("__thaw_esm_convert", convert, true);
        // the ESM loader's factory text, parsed with the module goal (`import.meta` and `import()` parse there)
        let compile = self.native(|it, _, a| {
            let source = it.to_str(&crate::arg(a, 0)).to_string();
            it.eval_module_function(&source)
        });
        self.global.declare("__thaw_esm_compile", compile, true);
    }
}

#[cfg(test)]
mod entry_convert_tests {
    use super::convert_entry;

    // The generated code must use only the renamed helpers: no bare `require` or `exports` in the output.
    fn assert_renamed(code: &str) {
        assert!(
            !code.replace("__thaw_esm_require", "").contains("require"),
            "{code}"
        );
        assert!(
            !code.replace("__thaw_esm_exports", "").contains("exports"),
            "{code}"
        );
    }

    fn entry(source: &str) -> super::EntryModule {
        let m = convert_entry(source).expect("convert_entry");
        assert_renamed(&m.code);
        m
    }

    #[test]
    fn entry_named_import_export_const_and_tla_use_renamed_helpers() {
        let m = entry("import { a } from 'x';\nexport const b = a + 1;\nawait 0;\n");
        assert!(m.code.contains("__thaw_esm_require(\"x\")"), "{}", m.code);
        assert!(m.code.contains("__thaw_esm_exports"), "{}", m.code);
        assert!(m.tla);
        assert_eq!(m.imports, vec!["x".to_string()]);
    }

    #[test]
    fn entry_export_default_expression_assigns_renamed_exports() {
        let m = entry("export default 1 + 2;\n");
        assert!(
            m.code.contains("__thaw_esm_exports.default = (1 + 2)"),
            "{}",
            m.code
        );
        assert!(m.imports.is_empty());
    }

    #[test]
    fn entry_export_named_alias_defines_the_alias_getter() {
        let m = entry("const a = 3;\nexport { a as b };\n");
        assert!(m.code.contains("\"b\""), "{}", m.code);
        assert!(m.code.contains("return a;"), "{}", m.code);
    }

    #[test]
    fn entry_export_namespace_from_requires_and_wraps_namespace() {
        let m = entry("export * as ns from 'm';\n");
        assert!(m.code.contains("__thaw_esm_require(\"m\")"), "{}", m.code);
        assert!(m.code.contains("\"ns\""), "{}", m.code);
        assert!(m.code.contains("__thaw_esm_ns("), "{}", m.code);
        assert_eq!(m.imports, vec!["m".to_string()]);
    }

    #[test]
    fn entry_export_star_copies_names_through_renamed_require() {
        let m = entry("export * from 'm';\n");
        assert!(m.code.contains("__thaw_esm_require(\"m\")"), "{}", m.code);
        assert!(m.code.contains("Object.keys("), "{}", m.code);
        assert_eq!(m.imports, vec!["m".to_string()]);
    }

    #[test]
    fn entry_export_function_class_and_let_declarations_keep_their_bodies() {
        let m = entry("export function f() { return 1; }\nexport class K {}\nexport let n = 0;\n");
        assert!(m.code.contains("function f() { return 1; }"), "{}", m.code);
        assert!(m.code.contains("class K {}"), "{}", m.code);
        assert!(m.code.contains("let n = 0"), "{}", m.code);
        assert!(
            m.code.contains("\"f\"") && m.code.contains("\"K\"") && m.code.contains("\"n\""),
            "{}",
            m.code
        );
    }

    #[test]
    fn entry_default_namespace_and_side_effect_imports_all_require_their_module() {
        let m = entry("import d, { e as f } from 'p';\nimport * as q from 'r';\nimport 's';\n");
        assert_eq!(
            m.imports,
            vec!["p".to_string(), "r".to_string(), "s".to_string()]
        );
        assert!(m.code.contains("__thaw_esm_default("), "{}", m.code);
        assert!(m.code.contains("__thaw_esm_ns("), "{}", m.code);
    }

    #[test]
    fn entry_tla_is_false_when_await_is_only_inside_functions() {
        let m = entry("export const q = 1;\nasync function g() { await 1; }\nconst h = async () => { await 2; };\n");
        assert!(!m.tla);
    }

    #[test]
    fn entry_for_await_at_top_level_is_tla() {
        assert!(entry("for await (const x of []) {}\n").tla);
    }

    #[test]
    fn entry_imports_are_listed_in_source_order() {
        let m = entry("import './b.js';\nimport { a } from 'a.js';\nexport * from 'c.js';\nexport { z } from 'z.js';\n");
        assert_eq!(
            m.imports,
            vec![
                "./b.js".to_string(),
                "a.js".to_string(),
                "c.js".to_string(),
                "z.js".to_string()
            ]
        );
    }

    #[test]
    fn entry_syntax_error_returns_a_clear_error_string() {
        let Err(err) = convert_entry("const = ;\n") else {
            panic!("a syntax error must not convert");
        };
        assert!(err.starts_with("failed to parse module"), "{err}");
    }

    // Runs the converted body with a fake `__thaw_esm_require` (every module is `{__esModule, default: 5, a: 10}`)
    // and reports the exports as JSON.
    #[test]
    fn entry_body_evaluates_against_fake_require_and_exports() {
        let source = "import d, { a as aa } from 'dep';\nimport * as ns from 'dep';\n\
            export const x = aa + 1;\nexport default d;\nexport function f() { return x * 2; }\n\
            export { x as y };\nexport * as whole from 'dep';\nexport const nsa = ns.a;\nexport class K {}\n";
        let m = convert_entry(source).expect("convert_entry");
        let program = format!(
            "(function () {{ var __thaw_esm_require = function (s) {{ return {{ __esModule: true, default: 5, a: 10 }}; }}; \
             return (function (__thaw_esm_require, __thaw_esm_exports) {{\n{}\n\
             return JSON.stringify([__thaw_esm_exports.x, __thaw_esm_exports.y, __thaw_esm_exports.default, \
             __thaw_esm_exports.f(), __thaw_esm_exports.nsa, __thaw_esm_exports.whole.a, typeof __thaw_esm_exports.K]); \
             }})(__thaw_esm_require, {{}}); }})",
            m.code
        );
        let mut it = crate::Interp::new();
        let f = it.eval_function(&program).expect("compiles");
        let out = it.call(&f, &crate::Value::Undefined, &[]).expect("runs");
        assert_eq!(
            it.to_str(&out).to_string(),
            "[11,11,5,22,10,10,\"function\"]"
        );
    }
}

#[cfg(test)]
mod entry_native_tests {
    use crate::{Interp, Value};

    // `__thaw_esm_convert` returns the JSON of `convert_entry`, and a failed conversion throws a SyntaxError.
    #[test]
    fn native_convert_returns_json_and_throws_syntax_error() {
        let mut it = Interp::new();
        let f = it
            .eval_function(
                "(function (s) { try { return __thaw_esm_convert(s); } catch (e) { return e.name + ':' + e.message; } })",
            )
            .expect("compiles");
        let ok = it
            .call(
                &f,
                &Value::Undefined,
                &[Value::str("import 'a';\nawait 1;\n")],
            )
            .expect("runs");
        let json = it.to_str(&ok).to_string();
        assert!(json.starts_with("{\"code\":\""), "{json}");
        assert!(
            json.ends_with("\"tla\":true,\"imports\":[\"a\"]}"),
            "{json}"
        );
        let bad = it
            .call(&f, &Value::Undefined, &[Value::str("const = ;\n")])
            .expect("runs");
        let err = it.to_str(&bad).to_string();
        assert!(
            err.starts_with("SyntaxError:failed to parse module"),
            "{err}"
        );
    }

    // `__thaw_esm_compile` parses its text with the module goal: `import.meta` reaches the factory's parameter, while
    // `new Function` (the script goal) still rejects it.
    #[test]
    fn compile_accepts_import_meta_and_function_constructor_still_rejects_it() {
        let mut it = Interp::new();
        let f = it
            .eval_function(
                "(function () { var fn = __thaw_esm_compile('(async function (__thaw_esm_meta) { return import.meta.url; })'); \
                 var r = fn.call(undefined, { url: 'u' }); return typeof fn + ':' + typeof r.then; })",
            )
            .expect("compiles");
        let out = it.call(&f, &Value::Undefined, &[]).expect("runs");
        assert_eq!(it.to_str(&out).to_string(), "function:function");
        let g = it
            .eval_function(
                "(function () { try { new Function('import.meta'); return 'no'; } catch (e) { return e.name; } })",
            )
            .expect("compiles");
        let out = it.call(&g, &Value::Undefined, &[]).expect("runs");
        assert_eq!(it.to_str(&out).to_string(), "SyntaxError");
    }
}
