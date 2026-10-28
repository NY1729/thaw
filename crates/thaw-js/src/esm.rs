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

/// The JSON payload of `globalThis.__thaw_register_native_bundle("<json>")`, if `source` is one.
fn native_payload(source: &str) -> Option<String> {
    // only the pure native shape; a mixed bundle (CommonJS factories plus native modules) is left alone
    const HEAD: &str =
        "module.exports = (function() {\nvar entry = globalThis.__thaw_register_native_bundle(";
    let start = source.starts_with(HEAD).then_some(HEAD.len())?;
    // a JavaScript string literal: find the closing quote, honouring backslash escapes
    let rest = source.get(start..)?;
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
            return Some(rest[..=i].to_string());
        }
    }
    None
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

fn getter(name: &str, expr: &str) -> String {
    let mut quoted = String::new();
    json_quote(name, &mut quoted);
    format!(
        "Object.defineProperty(exports, {quoted}, {{ enumerable: true, get: function () {{ return {expr}; }} }});\n"
    )
}

/// One module's CommonJS factory body, plus the specifiers it imports and the problems found.
struct Converted {
    body: String,
    specs: Vec<String>,
    problems: Vec<&'static str>,
}

fn convert(source: &str) -> Result<Converted, String> {
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source)?;
    let snippet = |span| cm.span_to_snippet(span).unwrap_or_default();
    let mut head = String::from(
        "\"use strict\";\nObject.defineProperty(exports, '__esModule', { value: true });\n\
         function __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\n\
         function __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); \
         if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } \
         ns.default = m; return ns; }\n",
    );
    let mut body = String::new();
    let mut specs: Vec<String> = Vec::new();
    let mut problems: Vec<&'static str> = Vec::new();
    let mut counter = 0usize;
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
                    let mut q = String::new();
                    json_quote(&spec, &mut q);
                    specs.push(spec);
                    let m = tmp(&mut counter);
                    body.push_str(&format!("var {m} = require({q});\n"));
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
                            if !matches!(v.kind, a::VarDeclKind::Const) {
                                problems.push("esm-live-binding");
                            }
                            for d in &v.decls {
                                binding_names(&d.name, &mut names);
                            }
                        }
                        _ => problems.push("esm-declaration"),
                    }
                    for n in &names {
                        head.push_str(&getter(n, n));
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
                            head.push_str(&getter("default", &n));
                        }
                        None => body.push_str(&format!("exports.default = ({text});\n")),
                    }
                }
                a::ModuleDecl::ExportDefaultExpr(e) => {
                    body.push_str(&format!(
                        "exports.default = ({});\n",
                        snippet(e.expr.span())
                    ));
                }
                a::ModuleDecl::ExportNamed(e) => {
                    let from = e.src.as_ref().map(|s| {
                        let spec = s.value.to_string_lossy().into_owned();
                        let mut q = String::new();
                        json_quote(&spec, &mut q);
                        specs.push(spec);
                        let m = tmp(&mut counter);
                        body.push_str(&format!("var {m} = require({q});\n"));
                        m
                    });
                    for sp in &e.specifiers {
                        match sp {
                            a::ExportSpecifier::Named(n) => {
                                let local = export_name(&n.orig);
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
                                head.push_str(&getter(&exported, &expr));
                            }
                            a::ExportSpecifier::Namespace(n) => {
                                if let Some(m) = &from {
                                    head.push_str(&getter(
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
                    let spec = e.src.value.to_string_lossy().into_owned();
                    let mut q = String::new();
                    json_quote(&spec, &mut q);
                    specs.push(spec);
                    let m = tmp(&mut counter);
                    body.push_str(&format!(
                        "var {m} = require({q});\nObject.keys({m}).forEach(function (k) {{ if (k !== 'default' && k !== '__esModule' && !Object.prototype.hasOwnProperty.call(exports, k)) Object.defineProperty(exports, k, {{ enumerable: true, get: function () {{ return {m}[k]; }} }}); }});\n"
                    ));
                }
                _ => problems.push("esm-declaration"),
            },
        }
    }
    Ok(Converted {
        body: format!("{head}{body}"),
        specs,
        problems,
    })
}

/// Rewrite a native bundle into the registry's CommonJS factory bundle text. `Ok(None)`: `source`
/// is not a native bundle. Problems that make the rewrite unsound are returned as feature names.
pub(crate) fn native_bundle_to_factories(
    source: &str,
) -> Result<Option<(String, BTreeSet<&'static str>)>, String> {
    let Some(literal) = native_payload(source) else {
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
    let mut factories = String::new();
    let mut maps = String::new();
    let mut problems: BTreeSet<&'static str> = BTreeSet::new();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in modules {
        let Native::Object(fields) = m else { continue };
        let key = text(field(fields, "key"))
            .ok_or("native module without key")?
            .to_string();
        let src = text(field(fields, "source")).ok_or("native module without source")?;
        let converted = convert(src)?;
        problems.extend(converted.problems);
        let mut imports: Vec<(String, String)> = Vec::new();
        if let Some(Native::Object(map)) = field(fields, "imports") {
            for (spec, target) in map {
                if let Native::Str(t) = target {
                    imports.push((spec.clone(), t.clone()));
                }
            }
        }
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
        factories.push_str(&format!(
            "{qk}: function (module, exports, require) {{\n{}\n}},\n",
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
    let bundle = format!(
        "module.exports = (function (require) {{\nvar __thaw_bundle_factories = {{\n{factories}}};\nvar __thaw_bundle_require_maps = {{\n{maps}}};\nvar __thaw_bundle_entry_key = {qm};\n}})(globalThis.require);\n"
    );
    Ok(Some((bundle, problems)))
}
