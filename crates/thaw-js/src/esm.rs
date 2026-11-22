//! ESM-only packages: the registry's "native bundle" (`__thaw_register_native_bundle("<json>")`,
//! original ES module sources plus a per-module import map) is rewritten into the CommonJS
//! factory form the loader already runs.
//!
//! Imports become `require` calls whose results are bound once the dependency has finished
//! evaluating; exports become getters on `exports`. Constructs whose semantics that rewrite would
//! change are reported as scan blockers instead of being approximated: an import cycle,
//! `import.meta`, top-level `await`. Every reference to an imported name is rewritten (esm_live.rs)
//! into a read of the dependency's export, so imports are live bindings.

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
                match parts.as_slice() {
                    [Native::Str(spec), Native::Str(remote), ..] if remote != "*" => {
                        names.push((name.clone(), spec.clone(), remote.clone()))
                    }
                    // a namespace re-export (`export * as name from spec`): remote `*`
                    [Native::Str(spec), Native::Null | Native::Str(_), ..] => {
                        names.push((name.clone(), spec.clone(), "*".to_string()))
                    }
                    _ => {}
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

/// Prefix of the flag that says an exported lexical binding has been initialised.
const TDZ_FLAG: &str = "__thaw_tdz_";

/// The names a `let`/`const`/`class` declaration binds (none for `var` and functions).
fn lexical_decl_names(decl: &a::Decl) -> Vec<String> {
    let mut names = Vec::new();
    match decl {
        a::Decl::Class(c) => names.push(c.ident.sym.to_string()),
        a::Decl::Var(v) if v.kind != a::VarDeclKind::Var => {
            for d in &v.decls {
                binding_names(&d.name, &mut names);
            }
        }
        _ => {}
    }
    names
}

/// Top-level lexical bindings that the module exports (`export let x`, or `export { x }` of a `let x`).
fn exported_lexicals(module: &a::Module) -> BTreeSet<String> {
    let mut lexical = BTreeSet::new();
    let mut local_exports = BTreeSet::new();
    let mut out = BTreeSet::new();
    for item in &module.body {
        match item {
            a::ModuleItem::Stmt(a::Stmt::Decl(d)) => lexical.extend(lexical_decl_names(d)),
            a::ModuleItem::ModuleDecl(a::ModuleDecl::ExportDecl(e)) => {
                out.extend(lexical_decl_names(&e.decl))
            }
            a::ModuleItem::ModuleDecl(a::ModuleDecl::ExportDefaultExpr(_)) => {
                out.insert(DFLT.to_string());
            }
            a::ModuleItem::ModuleDecl(a::ModuleDecl::ExportDefaultDecl(e)) => {
                if let a::DefaultDecl::Class(c) = &e.decl {
                    out.insert(
                        c.ident
                            .as_ref()
                            .map_or(DFLT.to_string(), |i| i.sym.to_string()),
                    );
                }
            }
            a::ModuleItem::ModuleDecl(a::ModuleDecl::ExportNamed(e)) if e.src.is_none() => {
                for sp in &e.specifiers {
                    if let a::ExportSpecifier::Named(n) = sp {
                        local_exports.insert(export_name(&n.orig));
                    }
                }
            }
            _ => {}
        }
    }
    out.extend(lexical.intersection(&local_exports).cloned());
    out
}

/// The synthetic binding behind `export default <expr>`.
const DFLT: &str = "__thaw_esm_dflt";

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
    // the module awaits outside every function (its factory must be async)
    tla: bool,
}

/// What the link phase knows about one bundled module (see `native_json_to_factories`): which of its specifiers
/// name an ES module of the same bundle (`esm`: specifier -> key) and which `export *` names resolve statically to
/// one binding (`stars`: name, module key, export name there; `None` is a namespace). Such a module's exports exist
/// before it evaluates (`require.peek(key)`), so a cycle peer reads them early, as it does in Node.
#[derive(Default)]
struct Links {
    esm: BTreeMap<String, String>,
    stars: Vec<(String, String, Option<String>)>,
}

/// `indirect`: `(exported name, specifier, imported name)` re-exports that the registry removed from
/// `source` and kept only in the export graph.
#[cfg(test)]
fn convert(
    source: &str,
    public_specs: &[String],
    indirect: &[(String, String, String)],
) -> Result<Converted, String> {
    convert_with(source, public_specs, indirect, None, &Links::default())
}

/// `convert`; `awaits: Some(specs)` makes the factory async: it awaits the modules of `specs` before the body.
fn convert_with(
    source: &str,
    public_specs: &[String],
    indirect: &[(String, String, String)],
    awaits: Option<&BTreeSet<String>>,
    links: &Links,
) -> Result<Converted, String> {
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(source)?;
    let (live, named) = live_import_bindings(&module);
    // Imports are live reads: each reference becomes `__thaw_esm_mK["x"]`, so the module is re-parsed from the
    // rewritten text. Only modules that import something pay for the second parse.
    let rewritten = if live.is_empty() {
        None
    } else {
        crate::esm_live::rewrite_import_references(source, &module, &cm, &live, &named, true)
    };
    let Some(rewritten) = rewritten else {
        return Ok(convert_module(
            &module,
            &cm,
            public_specs,
            indirect,
            &BUNDLE_NAMES,
            awaits,
            &live,
            links,
        ));
    };
    let (module, cm) = thaw_parser::parse_javascript_with_source_map(&rewritten)?;
    Ok(convert_module(
        &module,
        &cm,
        public_specs,
        indirect,
        &BUNDLE_NAMES,
        awaits,
        &live,
        links,
    ))
}

/// `local -> read expression` of every default and named import of `module` (the names `convert_module` no longer
/// copies into a `var`), plus the named ones (a call through them is detached). The `__thaw_esm_mK` numbering
/// follows `convert_module`'s: one per import, `export .. from` and `export *`, in source order.
fn live_import_bindings(module: &a::Module) -> (BTreeMap<String, String>, BTreeSet<String>) {
    let (mut live, mut named) = (BTreeMap::new(), BTreeSet::new());
    let mut counter = 0usize;
    for item in &module.body {
        let a::ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        match decl {
            a::ModuleDecl::Import(i) if !i.src.value.to_string_lossy().starts_with(ORIGIN_SPEC) => {
                counter += 1;
                let m = format!("__thaw_esm_m{counter}");
                for sp in &i.specifiers {
                    match sp {
                        a::ImportSpecifier::Default(d) => {
                            // parenthesized: a bare call would turn `new D()` into `new __thaw_esm_default(m)()`
                            live.insert(
                                d.local.sym.to_string(),
                                format!("(__thaw_esm_default({m}))"),
                            );
                        }
                        a::ImportSpecifier::Named(n) => {
                            let local = n.local.sym.to_string();
                            let imported = n
                                .imported
                                .as_ref()
                                .map(export_name)
                                .unwrap_or_else(|| local.clone());
                            let read = if imported == "default" {
                                format!("(__thaw_esm_default({m}))")
                            } else {
                                let mut qi = String::new();
                                json_quote(&imported, &mut qi);
                                format!("{m}[{qi}]")
                            };
                            named.insert(local.clone());
                            live.insert(local, read);
                        }
                        a::ImportSpecifier::Namespace(_) => {}
                    }
                }
            }
            a::ModuleDecl::ExportNamed(e) if e.src.is_some() => counter += 1,
            a::ModuleDecl::ExportAll(_) => counter += 1,
            _ => {}
        }
    }
    (live, named)
}

/// The request line of an import: an awaited dependency is requested with `requireAsync` (awaited in the body's prologue).
fn request(require: &str, m: &str, q: &str, awaited: bool) -> String {
    let call = if awaited { "requireAsync" } else { require };
    format!("var {m} = {call}({q});\n")
}

/// `convert` for a parsed module, with the factory identifiers `names`.
#[allow(clippy::too_many_arguments)]
fn convert_module(
    module: &a::Module,
    cm: &thaw_parser::common::SourceMap,
    public_specs: &[String],
    indirect: &[(String, String, String)],
    names: &Names,
    awaits: Option<&BTreeSet<String>>,
    live: &BTreeMap<String, String>,
    links: &Links,
) -> Converted {
    let (rq, ex) = (names.require, names.exports);
    // bundle modules (not entries) are the ones a cycle peer can read
    let bundle = ex == BUNDLE_NAMES.exports;
    let snippet = |span| cm.span_to_snippet(span).unwrap_or_default();
    let mut head =
        format!("\"use strict\";\nObject.defineProperty({ex}, '__esModule', {{ value: true, configurable: true }});\nObject.defineProperty({ex}, '__thaw_esm_stars', {{ value: Object.create(null) }});\n");
    head.push_str(
        "function __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\n\
         function __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); \
         if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } \
         ns.default = m; return ns; }\n",
    );
    if bundle && !links.stars.is_empty() {
        // `export *` names that resolve statically: defined before the body, reading the (maybe not yet evaluated) target
        head.push_str(&format!("function __thaw_esm_static(n, m, k) {{ {ex}.__thaw_esm_stars[n] = {{ bind: {{ m: m, k: k }}, amb: false, fixed: true }}; Object.defineProperty({ex}, n, {{ enumerable: true, configurable: true, get: function () {{ return k === null ? m : m[k]; }} }}); }}\n"));
        for (name, key, export) in &links.stars {
            let (mut qn, mut qk, mut qe) = (String::new(), String::new(), String::new());
            json_quote(name, &mut qn);
            json_quote(key, &mut qk);
            match export {
                Some(e) => json_quote(e, &mut qe),
                None => qe.push_str("null"),
            }
            head.push_str(&format!(
                "__thaw_esm_static({qn}, {rq}.peek({qk}), {qe});\n"
            ));
        }
    }
    // a module of this bundle is read through its (maybe not yet evaluated) exports, so a cycle peer sees its names
    let peeked = |spec: &str| {
        links.esm.get(spec).filter(|_| bundle).map(|key| {
            let mut q = String::new();
            json_quote(key, &mut q);
            format!("{rq}.peek({q})")
        })
    };
    // The graph's re-exports of a module of this bundle come first (the registry may have rewritten the source's
    // `export { x } from` into a local binding that is in TDZ while a cycle peer reads it): they read the target.
    for (name, spec, remote) in indirect {
        let Some(target) = peeked(spec) else { continue };
        let mut qr = String::new();
        json_quote(remote, &mut qr);
        let expr = match remote.as_str() {
            "*" => target,
            "default" => format!("__thaw_esm_default({target})"),
            _ => format!("{target}[{qr}]"),
        };
        head.push_str(&getter(ex, name, &expr));
    }
    let mut body = String::new();
    let mut prologue = String::new();
    // after the prologue's requests: the awaits of the async dependencies, in source order
    let mut late = String::new();
    let mut specs: Vec<String> = Vec::new();
    // names this source re-exports itself (`export { X } from`); the export graph repeats them as indirect entries
    let mut reexported: BTreeSet<String> = BTreeSet::new();
    let mut problems: Vec<&'static str> = Vec::new();
    let mut has_star = false;
    let mut counter = 0usize;
    let mut uses_origin = false;
    fn tmp(counter: &mut usize) -> String {
        *counter += 1;
        format!("__thaw_esm_m{counter}")
    }
    // Exported `let`/`const`/`class` bindings: a cycle peer reading one before its declaration ran gets Node's
    // TDZ ReferenceError (the engine has no TDZ of its own), tracked by a flag set right after the declaration.
    let tdz = if bundle {
        exported_lexicals(module)
    } else {
        BTreeSet::new()
    };
    let tdz_flags = |decl: &a::Decl, out: &mut String| {
        for n in lexical_decl_names(decl).iter().filter(|n| tdz.contains(*n)) {
            out.push_str(&format!("{TDZ_FLAG}{n} = true;\n"));
        }
    };
    for n in &tdz {
        head.push_str(&format!("var {TDZ_FLAG}{n} = false;\n"));
    }
    if !tdz.is_empty() {
        head.push_str("function __thaw_esm_tdz(n) { throw new ReferenceError(\"Cannot access '\" + n + \"' before initialization\"); }\n");
    }
    // a local binding read through a getter: guarded when it is a flagged lexical
    let local_read = |n: &str, shown: &str| {
        if tdz.contains(n) {
            format!("({TDZ_FLAG}{n} ? {n} : __thaw_esm_tdz(\"{shown}\"))")
        } else {
            n.to_string()
        }
    };
    for item in &module.body {
        match item {
            a::ModuleItem::Stmt(s) => {
                body.push_str(&snippet(s.span()));
                body.push_str(";\n");
                if let a::Stmt::Decl(d) = s {
                    tdz_flags(d, &mut body);
                }
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
                    let awaited = awaits.is_some_and(|set| set.contains(&spec));
                    specs.push(spec);
                    let m = tmp(&mut counter);
                    // every request runs before the body, as ES module evaluation does
                    prologue.push_str(&request(rq, &m, &q, awaited));
                    if awaited {
                        late.push_str(&format!("{m} = await {m};\n"));
                    }
                    for sp in &i.specifiers {
                        match sp {
                            // a name in `live` is read through `m` at each use, not copied
                            a::ImportSpecifier::Default(d)
                                if !live.contains_key(d.local.sym.as_str()) =>
                            {
                                body.push_str(&format!(
                                    "var {} = __thaw_esm_default({m});\n",
                                    d.local.sym
                                ))
                            }
                            a::ImportSpecifier::Default(_) => {}
                            a::ImportSpecifier::Named(n)
                                if live.contains_key(n.local.sym.as_str()) => {}
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
                            for d in &v.decls {
                                binding_names(&d.name, &mut names);
                            }
                        }
                        _ => problems.push("esm-declaration"),
                    }
                    for n in &names {
                        head.push_str(&getter(ex, n, &local_read(n, n)));
                    }
                    body.push_str(&snippet(e.decl.span()));
                    body.push_str(";\n");
                    tdz_flags(&e.decl, &mut body);
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
                            if tdz.contains(&n) {
                                body.push_str(&format!("{TDZ_FLAG}{n} = true;\n"));
                            }
                            head.push_str(&getter(ex, "default", &local_read(&n, "default")));
                        }
                        // an anonymous function is hoisted (a cycle peer calls it before the body runs); a class is a
                        // `const`, so it stays in TDZ. `{ default: .. }` gives it the name "default".
                        None if bundle && matches!(e.decl, a::DefaultDecl::Fn(_)) => {
                            head.push_str(&format!(
                                "var {DFLT} = ({{ default: ({text}) }}).default;\n"
                            ));
                            head.push_str(&getter(ex, "default", DFLT));
                        }
                        None if bundle => {
                            body.push_str(&format!("const {DFLT} = ({{ default: ({text}) }}).default;\n{TDZ_FLAG}{DFLT} = true;\n"));
                            head.push_str(&getter(ex, "default", &local_read(DFLT, "default")));
                        }
                        None => body.push_str(&format!("{ex}.default = ({text});\n")),
                    }
                }
                a::ModuleDecl::ExportDefaultExpr(e) => {
                    if bundle {
                        // a `const`, like Node's `*default*` binding: a cycle peer reads it in TDZ until it is evaluated
                        body.push_str(&format!(
                            "const {DFLT} = ({{ default: ({}) }}).default;\n{TDZ_FLAG}{DFLT} = true;\n",
                            snippet(e.expr.span())
                        ));
                        head.push_str(&getter(ex, "default", &local_read(DFLT, "default")));
                    } else {
                        body.push_str(&format!("{ex}.default = ({});\n", snippet(e.expr.span())));
                    }
                }
                a::ModuleDecl::ExportNamed(e) => {
                    let from = e.src.as_ref().map(|s| {
                        let spec = s.value.to_string_lossy().into_owned();
                        let mut q = String::new();
                        json_quote(&spec, &mut q);
                        let awaited = awaits.is_some_and(|set| set.contains(&spec));
                        let read = peeked(&spec);
                        specs.push(spec);
                        let m = tmp(&mut counter);
                        prologue.push_str(&request(rq, &m, &q, awaited));
                        if awaited {
                            late.push_str(&format!("{m} = await {m};\n"));
                        }
                        // the module's exports as the getters read them: its own (peeked) when it is in this bundle
                        (m.clone(), read.unwrap_or(m))
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
                                    Some((_, m)) => {
                                        let mut q = String::new();
                                        json_quote(&local, &mut q);
                                        if local == "default" {
                                            format!("__thaw_esm_default({m})")
                                        } else {
                                            format!("{m}[{q}]")
                                        }
                                    }
                                    // an imported name is read through its dependency (it has no local binding)
                                    None => live
                                        .get(&local)
                                        .cloned()
                                        .unwrap_or_else(|| local_read(&local, &local)),
                                };
                                if from.is_some() {
                                    reexported.insert(exported.clone());
                                }
                                head.push_str(&getter(ex, &exported, &expr));
                            }
                            a::ExportSpecifier::Namespace(n) => {
                                if let Some((_, m)) = &from {
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
                    let spec = e.src.value.to_string_lossy().into_owned();
                    let mut q = String::new();
                    json_quote(&spec, &mut q);
                    let awaited = awaits.is_some_and(|set| set.contains(&spec));
                    specs.push(spec);
                    if !has_star {
                        head.push_str(STAR_HELPER);
                        has_star = true;
                    }
                    let m = tmp(&mut counter);
                    if awaited {
                        // the star copies the names once the awaited module is ready
                        prologue.push_str(&request(rq, &m, &q, true));
                        late.push_str(&format!("{m} = await {m};\n__thaw_esm_star({ex}, {m});\n"));
                    } else {
                        prologue.push_str(&format!(
                            "var {m} = {rq}({q});\n__thaw_esm_star({ex}, {m});\n"
                        ));
                    }
                }
                _ => problems.push("esm-declaration"),
            },
        }
    }
    // Indirect getters go in the head so an `export *` in the body skips their names.
    for (name, spec, remote) in indirect {
        if reexported.contains(name) || peeked(spec).is_some() {
            continue;
        }
        let (mut qs, mut qr) = (String::new(), String::new());
        json_quote(spec, &mut qs);
        json_quote(remote, &mut qr);
        // only a module of this bundle has a namespace here; the source keeps any other `export * as`
        if remote == "*" {
            continue;
        }
        let expr = if remote == "default" {
            format!("__thaw_esm_default({rq}({qs}))")
        } else {
            format!("{rq}({qs})[{qr}]")
        };
        head.push_str(&getter(ex, name, &expr));
    }
    if uses_origin {
        head.push_str(
            "function __thaw_esm_origin_obj(t) { var nsCache = Object.create(null); return { readDefault: function (s) { return __thaw_esm_default(t[s]()); }, \
             read: function (s, n) { return t[s]()[n]; }, readNamespace: function (s) { return __thaw_esm_ns(t[s]()); }, \
             readImport: function (s, n) { return t[s]()[n]; }, namespaceImport: function (s) { var m = t[s](); if (m !== null && (typeof m === 'object' || typeof m === 'function') && Object.prototype.hasOwnProperty.call(m, '__thaw_esm_stars')) return __thaw_esm_ns(m); if (!nsCache[s] || !Object.is(nsCache[s].m, m)) nsCache[s] = { m: m, ns: __thaw_esm_cjs_ns(m) }; return nsCache[s].ns; }, \
             defaultImport: function (s, m) { return __thaw_esm_default(m); }, \
             validateRequests: function () {}, validateDynamic: function () {}, completeDynamic: function (s, p) { return p; }, \
             rejectDynamic: function (e) { return Promise.reject(e); } }; }\n\
             function __thaw_esm_cjs_ns(m) { var names = m !== null && (typeof m === 'object' || typeof m === 'function') ? Object.keys(m).filter(function (k) { return k !== 'default'; }) : []; names.push('default'); var ns = {}; names.sort().forEach(function (k) { Object.defineProperty(ns, k, { value: k === 'default' ? m : m[k], enumerable: true, writable: true }); }); Object.defineProperty(ns, Symbol.toStringTag, { value: 'Module' }); return Object.preventExtensions(ns); }\n",
        );
    }
    Converted {
        body: format!("{head}{prologue}{late}{body}"),
        specs,
        problems,
        tla: has_top_level_await(module),
    }
}

// The runtime half of `export *`: copies the names of `m` (the target's own enumerable keys, read once it has
// evaluated) onto `ex` as live getters. An explicit or indirect export wins; `default` is never a star name; a
// name two stars provide through different bindings is ambiguous and absent. `__thaw_esm_stars` (non-enumerable)
// records each star name's canonical binding, so a diamond through a re-exporting module stays one name.
// Every converted module has `__thaw_esm_stars` (the marker that its namespace is ESM-shaped, see the origin).
const STAR_HELPER: &str = r#"function __thaw_esm_star(ex, m) { var own = Object.prototype.hasOwnProperty;
  var table = ex.__thaw_esm_stars;
  var mine = m !== null && (typeof m === 'object' || typeof m === 'function') && own.call(m, '__thaw_esm_stars') ? m.__thaw_esm_stars : null;
  (m === null || m === undefined ? [] : Object.keys(m)).forEach(function (k) {
    if (k === 'default') return;
    var bind = mine && own.call(mine, k) ? mine[k].bind : { m: m, k: k };
    if (own.call(table, k)) {
      if (!table[k].fixed && !table[k].amb && (table[k].bind.m !== bind.m || table[k].bind.k !== bind.k)) {
        table[k].amb = true;
        if (k === '__esModule') Object.defineProperty(ex, k, { value: true, configurable: true }); else delete ex[k];
      }
      return;
    }
    if (k !== '__esModule' && own.call(ex, k)) return;
    table[k] = { bind: bind, amb: false };
    Object.defineProperty(ex, k, { enumerable: true, configurable: true, get: function () { return bind.m[bind.k]; } });
  });
}
"#;

/// The factory text, the unsound-construct feature names, whether the bundle is the pure native shape, and the
/// keys of the converted ES modules (their namespace is their exports object).
pub(crate) type NativeBundle = (
    String,
    BTreeSet<&'static str>,
    bool,
    BTreeSet<String>,
    std::collections::HashMap<String, Vec<String>>,
);

/// A converted native module; its body is re-converted as an async one when it awaits (see `native_bundle_to_factories`).
struct Staged<'a> {
    key: String,
    link: Links,
    src: &'a str,
    public: Vec<String>,
    imports: Vec<(String, String)>,
    indirect: &'a [(String, String, String)],
    specs: Vec<String>,
    tla: bool,
    body: String,
}

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
    native_json_to_factories(&json, source, pure).map(Some)
}

/// A native module of a payload: key, source, and `(specifier, target key)` imports.
type RawModule<'a> = (String, &'a str, Vec<(String, String)>);

/// The [`Links`] of every native module (`modules`: key, source, imports) of a bundle whose registry text `source`
/// holds export graphs; empty without them (a hand-registered payload keeps the copy-at-star-time semantics).
fn static_links(source: &str, modules: &[RawModule]) -> BTreeMap<String, Links> {
    let mut out = BTreeMap::new();
    let Some(graphs) = crate::esm::export_graphs_literal(source)
        .and_then(|l| crate::json::parse_native(&format!("[{l}]")).ok())
        .and_then(|v| match v {
            Native::Array(mut a) => a.pop(),
            _ => None,
        })
        .and_then(|v| match v {
            Native::Str(json) => serde_json::from_str::<serde_json::Value>(&json).ok(),
            _ => None,
        })
        .and_then(|v| v.as_object().cloned())
    else {
        return out;
    };
    let staged: BTreeMap<&str, &Vec<(String, String)>> =
        modules.iter().map(|(k, _, i)| (k.as_str(), i)).collect();
    // the key `spec` names in staged module `from`, when that is a staged module with an export graph
    let target = |from: &str, spec: &str| {
        let (_, _, imports) = modules.iter().find(|(k, _, _)| k == from)?;
        let key = &imports.iter().find(|(s, _)| s == spec)?.1;
        (staged.contains_key(key.as_str()) && graphs.contains_key(key)).then(|| key.clone())
    };
    for (key, _, imports) in modules {
        if !graphs.contains_key(key) {
            continue;
        }
        let mut links = Links::default();
        for (spec, _) in imports {
            if let Some(k) = target(key, spec) {
                links.esm.insert(spec.clone(), k);
            }
        }
        links.stars = crate::esm_link::star_bindings(&graphs, key, target)
            .into_iter()
            .map(|(name, bound)| (name, bound.key, bound.export))
            .collect();
        out.insert(key.clone(), links);
    }
    out
}

/// The factory bundle of the native payload `json` (`__thaw_register_native_bundle`'s argument). `source` is the
/// registry bundle text holding its export graphs (empty for a runtime registration).
pub(crate) fn native_json_to_factories(
    json: &str,
    source: &str,
    pure: bool,
) -> Result<NativeBundle, String> {
    let Ok(Native::Object(root)) = crate::json::parse_native(json) else {
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
    let mut staged: Vec<Staged> = Vec::new();
    let mut problems: BTreeSet<&'static str> = BTreeSet::new();
    let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let indirects = indirect_exports(source);
    let mut raws: Vec<RawModule> = Vec::new();
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
        raws.push((key, src, imports));
    }
    let mut links = static_links(source, &raws);
    for (key, src, imports) in raws {
        let public: Vec<String> = imports.iter().map(|(s, _)| s.clone()).collect();
        let indirect = indirects.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let link = links.remove(&key).unwrap_or_default();
        let converted = convert_with(src, &public, indirect, None, &link)?;
        problems.extend(converted.problems);
        graph.insert(
            key.clone(),
            imports
                .iter()
                .filter(|(spec, _)| converted.specs.contains(spec))
                .map(|(_, t)| t.clone())
                .collect(),
        );
        staged.push(Staged {
            key,
            src,
            public,
            imports,
            indirect,
            link,
            specs: converted.specs,
            tla: converted.tla,
            body: converted.body,
        });
    }
    // An async module: one with top-level await, or one that statically imports an async module (a fixpoint).
    // A cycle through an async module stays blocked (`esm-cycle`), so the fixpoint terminates on acyclic graphs.
    let mut asyncs: BTreeSet<String> = staged
        .iter()
        .filter(|s| s.tla)
        .map(|s| s.key.clone())
        .collect();
    loop {
        let found: Vec<String> = staged
            .iter()
            .filter(|s| {
                !asyncs.contains(&s.key)
                    && graph
                        .get(&s.key)
                        .is_some_and(|deps| deps.iter().any(|d| asyncs.contains(d)))
            })
            .map(|s| s.key.clone())
            .collect();
        if found.is_empty() {
            break;
        }
        asyncs.extend(found);
    }
    let staged_keys: Vec<String> = staged.iter().map(|s| s.key.clone()).collect();
    let mut esm_keys = BTreeSet::new();
    let mut esm_specs = std::collections::HashMap::new();
    for s in staged {
        let is_async = asyncs.contains(&s.key);
        let body = if is_async {
            let awaits: BTreeSet<String> = s
                .imports
                .iter()
                .filter(|(spec, target)| s.specs.contains(spec) && asyncs.contains(target))
                .map(|(spec, _)| spec.clone())
                .collect();
            convert_with(s.src, &s.public, s.indirect, Some(&awaits), &s.link)?.body
        } else {
            s.body
        };
        let mut qk = String::new();
        json_quote(&s.key, &mut qk);
        // `requireAsync` is the module's import(): a bare `import(spec)` in the body calls it (lib.rs)
        factories.push_str(&format!(
            "{qk}: {}function (module, exports, require, requireAsync) {{\n{body}\n}},\n",
            if is_async { "async " } else { "" }
        ));
        let entries: Vec<String> = s
            .imports
            .iter()
            .map(|(spec, target)| {
                let (mut qs, mut qt) = (String::new(), String::new());
                json_quote(spec, &mut qs);
                json_quote(target, &mut qt);
                format!("{qs}: {qt}")
            })
            .collect();
        maps.push_str(&format!("{qk}: {{ {} }},\n", entries.join(", ")));
        esm_specs.insert(s.key.clone(), s.specs);
        esm_keys.insert(s.key);
    }
    // A synchronous cycle links like Node (live reads, TDZ at the read); an async one cannot be evaluated depth-first.
    // Every member of a cycle through an async module is itself async (the fixpoint above), so only those are walked.
    fn cyclic(
        graph: &BTreeMap<String, Vec<String>>,
        asyncs: &BTreeSet<String>,
        node: &str,
        stack: &mut Vec<String>,
        done: &mut BTreeSet<String>,
    ) -> bool {
        if !asyncs.contains(node) {
            return false;
        }
        if stack.iter().any(|s| s == node) {
            return true;
        }
        if !done.insert(node.to_string()) {
            return false;
        }
        stack.push(node.to_string());
        let found = graph
            .get(node)
            .is_some_and(|next| next.iter().any(|n| cyclic(graph, asyncs, n, stack, done)));
        stack.pop();
        found
    }
    if staged_keys
        .iter()
        .any(|k| cyclic(&graph, &asyncs, k, &mut Vec::new(), &mut BTreeSet::new()))
    {
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
    Ok((bundle, problems, pure, esm_keys, esm_specs))
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
            "validateRequests",
            "validateDynamic",
            "completeDynamic",
            "rejectDynamic",
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
    use crate::host::{Native, Package};
    use crate::Value;

    // A mixed bundle: the native `modules` (`key`, `source`, `imports`) with main `main`, and the CommonJS
    // entry `m/index.js` whose body is `index`; every module `m/x.mjs` is requirable as `./x.mjs`.
    fn bundle(main: &str, modules: serde_json::Value, index: &str) -> String {
        bundle_with_graphs(main, modules, index, serde_json::Value::Null)
    }

    // `bundle` plus the registry's export graphs (`graphs`, an object keyed by module key) for the named-export link check.
    fn bundle_with_graphs(
        main: &str,
        modules: serde_json::Value,
        index: &str,
        graphs: serde_json::Value,
    ) -> String {
        let table = if graphs.is_null() {
            String::new()
        } else {
            let mut q = String::new();
            json_quote(&graphs.to_string(), &mut q);
            format!("var __thaw_bundle_export_graphs = JSON.parse({q});\n")
        };
        let keys: Vec<String> = modules
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["key"].as_str().unwrap().to_string())
            .collect();
        let stubs: String = keys
            .iter()
            .map(|k| format!("\"{k}\": function() {{ throw new TypeError('Native ESM must be loaded by its registered entry'); }},\n"))
            .collect();
        let map: String = keys
            .iter()
            .map(|k| format!("\"./{}\": \"{k}\", ", k.trim_start_matches("m/")))
            .collect();
        let empties: String = keys.iter().map(|k| format!("\"{k}\": {{}}, ")).collect();
        let payload =
            serde_json::json!({ "main": main, "modules": modules, "opaqueEdges": [] }).to_string();
        let mut literal = String::new();
        json_quote(&payload, &mut literal);
        format!(
            r#"module.exports = (function(require) {{
var __thaw_bundle_factories = {{
"m/index.js": function(module, exports, require, requireAsync) {{ {index} }},
{stubs}}};
var __thaw_bundle_require_maps = {{ "m/index.js": {{{map}}}, {empties}}};
var __thaw_bundle_entry_key = "m/index.js";
{table}var __thaw_native_registered = globalThis.__thaw_register_native_bundle({literal});
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

    const E: &str = "export let value = 0;\nexport function bump() { value++; }\n";

    // Loads `modules` (main m/e.mjs) and runs `index`'s `exports.run`, returning its result.
    fn run(modules: serde_json::Value, index: &str) -> Native {
        let mut p = Package::load(&bundle("m/e.mjs", modules, index)).expect("loads");
        p.call("run", &[]).unwrap()
    }

    // (b) an in-bundle importer reads `value` live: bump() in the exporter is visible through the import.
    #[test]
    fn mutable_export_copied_by_in_bundle_importer_is_live() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": E, "imports": {} },
            { "key": "m/i.mjs", "source": "import { value } from \"./e.mjs\";\nexport function seen() { return value; }\n",
              "imports": { "./e.mjs": "m/e.mjs" } }
        ]);
        let index = r#"var e = require("./e.mjs"), i = require("./i.mjs"); exports.run = function () { e.bump(); e.bump(); return i.seen(); };"#;
        assert_eq!(run(modules, index), Native::Number(2.0));
    }

    // (c) renamed re-export chain: B re-exports the mutable `value` as `v`, C reads it.
    #[test]
    fn mutable_export_renamed_through_reexport_chain_is_live() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": E, "imports": {} },
            { "key": "m/b.mjs", "source": "export { value as v } from \"./e.mjs\";\n", "imports": { "./e.mjs": "m/e.mjs" } },
            { "key": "m/c.mjs", "source": "import { v } from \"./b.mjs\";\nexport function seen() { return v; }\n", "imports": { "./b.mjs": "m/b.mjs" } }
        ]);
        let index = r#"var e = require("./e.mjs"), c = require("./c.mjs"); exports.run = function () { e.bump(); e.bump(); e.bump(); return c.seen(); };"#;
        assert_eq!(run(modules, index), Native::Number(3.0));
    }

    // (d) `export *` of a module with a mutable export: a reader behind the star sees bump().
    #[test]
    fn mutable_export_through_export_star_is_live() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs", "source": E, "imports": {} },
            { "key": "m/s.mjs", "source": "export * from \"./e.mjs\";\n", "imports": { "./e.mjs": "m/e.mjs" } },
            { "key": "m/c.mjs", "source": "import { value } from \"./s.mjs\";\nexport function seen() { return value; }\n", "imports": { "./s.mjs": "m/s.mjs" } }
        ]);
        let index = r#"var e = require("./e.mjs"), c = require("./c.mjs"); exports.run = function () { e.bump(); e.bump(); return c.seen(); };"#;
        assert_eq!(run(modules, index), Native::Number(2.0));
    }

    // Import reads are scope-safe live member reads: shadowing, shorthand, typeof, call this-value,
    // `export { x }` of an import, default import, and assignment to an import throwing a TypeError.
    // Node 22 expectation of the same sources: "2,p,shadow,2,2,number,undefined,4,2,7,TypeError,TypeError,2" (real `node` 22.23.1)
    #[test]
    fn native_default_import_is_constructible_and_a_member_base() {
        // `new D()` must construct D itself, and `D.prototype` / `new D.Inner()` must see the default export: the live
        // read of a default import is a call expression, so it has to be parenthesized wherever it is a `new` callee
        let modules = serde_json::json!([
            { "key": "m/q.mjs",
              "source": "export default class Queue { constructor() { this.n = 1; } static Inner = class { constructor() { this.k = 2; } }; }\n",
              "imports": {} },
            { "key": "m/u.mjs",
              "source": "import Queue from \"./q.mjs\";\nexport function make() { var q = new Queue(); var i = new Queue.Inner(); return [q.n, i.k, typeof Queue.prototype, q instanceof Queue].join(','); }\n",
              "imports": { "./q.mjs": "m/q.mjs" } }
        ]);
        let index = r#"var u = require("./u.mjs");
exports.run = function () { return u.make(); };"#;
        assert_eq!(run(modules, index), Native::Str("1,2,object,true".into()));
    }

    #[test]
    fn native_import_reads_are_live_scope_safe() {
        let modules = serde_json::json!([
            { "key": "m/e.mjs",
              "source": "export let x = 0;\nexport function bump() { x++; }\nexport function who() { return this === undefined ? 'undefined' : 'obj'; }\nexport default function dflt() { return x; }\nlet d = 1;\nexport function setd(v) { d = v; }\nexport { d };\n",
              "imports": {} },
            { "key": "m/i.mjs",
              "source": "import { x, who, d } from \"./e.mjs\";\nimport dflt, * as ns from \"./e.mjs\";\nexport { x as rx, d as rd };\nexport function plain() { return x; }\nexport function param(x) { return x; }\nexport function local() { var x = 'shadow'; return x; }\nexport function block() { { let x = 'inner'; } return x; }\nexport function short() { return { x }; }\nexport function type() { return typeof x; }\nexport function thisval() { return who(); }\nexport function viaDefault() { return dflt() + ns.x; }\nexport function assign() { try { x = 5; } catch (e) { return e.name; } return 'no throw'; }\nexport function update() { try { x++; } catch (e) { return e.name; } return 'no throw'; }\n",
              "imports": { "./e.mjs": "m/e.mjs" } }
        ]);
        let index = r#"var e = require("./e.mjs"), i = require("./i.mjs");
exports.run = function () {
  e.bump(); e.bump(); e.setd(7);
  return [i.plain(), i.param('p'), i.local(), i.block(), i.short().x, i.type(), i.thisval(), i.viaDefault(), i.rx, i.rd, i.assign(), i.update(), e.x].join(',');
};"#;
        assert_eq!(
            run(modules, index),
            Native::Str("2,p,shadow,2,2,number,undefined,4,2,7,TypeError,TypeError,2".into())
        );
    }

    // Node 22.23.1 (g2-probes/fl): a module that imports an unresolvable specifier fails to LINK with
    // ERR_MODULE_NOT_FOUND before its own body runs.
    #[test]
    fn failed_link_runs_no_body_and_reports_err_module_not_found() {
        let modules = serde_json::json!([
            { "key": "m/bad.mjs", "source": "globalThis.badRuns = (globalThis.badRuns || 0) + 1; import \"./unmapped.mjs\"; export const value = 1;\n", "imports": {} }
        ]);
        let index = r#"exports.run = function () { try { require("./bad.mjs"); } catch (e) { return [e.code, e.message, String(globalThis.badRuns)].join('|'); } return 'no throw'; };"#;
        let mut p = Package::load(&bundle("m/bad.mjs", modules, index)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("ERR_MODULE_NOT_FOUND|Cannot find module '/thaw_modules/m/unmapped.mjs' imported from /thaw_modules/m/bad.mjs|undefined".into())
        );
    }

    // Node 22.23.1 (g2b-probes/anchor): in the cycle i -> m -> r -> i, r reads i's `ns` (re-exported from m, which exports
    // the namespace of p) BEFORE p evaluates: it gets p's final namespace, and p's names appear once p has run.
    #[test]
    fn cycle_peer_reads_a_reexported_namespace_before_the_target_evaluates() {
        let modules = serde_json::json!([
            { "key": "m/i.mjs", "source": "import \"./m.mjs\"; export { ns } from \"./m.mjs\";\n", "imports": { "./m.mjs": "m/m.mjs" } },
            { "key": "m/m.mjs", "source": "import \"./r.mjs\"; export * as ns from \"./p.mjs\";\n", "imports": { "./r.mjs": "m/r.mjs", "./p.mjs": "m/p.mjs" } },
            { "key": "m/r.mjs", "source": "import { ns } from \"./i.mjs\"; globalThis.beforePure = !globalThis.pureRuns; globalThis.earlyNs = ns;\n", "imports": { "./i.mjs": "m/i.mjs" } },
            { "key": "m/p.mjs", "source": "globalThis.pureRuns = (globalThis.pureRuns || 0) + 1; export const value = 17;\n", "imports": {} }
        ]);
        let graphs = serde_json::json!({
            "m/i.mjs": { "local": {}, "indirect": { "ns": ["./m.mjs", "ns", false] }, "stars": [], "requests": [["./m.mjs", "ns"]], "dependencies": ["./m.mjs"] },
            "m/m.mjs": { "local": {}, "indirect": { "ns": ["./p.mjs", null, false] }, "stars": [], "requests": [], "dependencies": ["./r.mjs", "./p.mjs"] },
            "m/r.mjs": { "local": {}, "indirect": {}, "stars": [], "requests": [["./i.mjs", "ns"]], "dependencies": ["./i.mjs"] },
            "m/p.mjs": { "local": { "value": "value" }, "indirect": {}, "stars": [], "requests": [], "dependencies": [] }
        });
        let index = r#"var i = require("./i.mjs"); exports.run = function () { return [globalThis.beforePure, globalThis.earlyNs === i.ns, globalThis.earlyNs.value, globalThis.pureRuns].join('|'); };"#;
        let mut p =
            Package::load(&bundle_with_graphs("m/i.mjs", modules, index, graphs)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("true|true|17|1".into())
        );
    }

    // Node 22.23.1 (g2b-probes/pure_ns): a name two `export *` targets reach through DIFFERENT re-export paths is one
    // binding when both end at the same declaration (here p's namespace), so it is not ambiguous; the registry may also
    // have stripped a namespace re-export from the source, leaving it only in the export graph.
    #[test]
    fn star_diamond_through_reexports_is_one_binding_and_graph_only_namespaces_are_exported() {
        let modules = serde_json::json!([
            { "key": "m/i.mjs", "source": "export * from \"./l.mjs\"; export * from \"./r.mjs\";\n", "imports": { "./l.mjs": "m/l.mjs", "./r.mjs": "m/r.mjs", "./p.mjs": "m/p.mjs" } },
            { "key": "m/l.mjs", "source": "export { ns } from \"./b.mjs\";\n", "imports": { "./b.mjs": "m/b.mjs" } },
            { "key": "m/r.mjs", "source": "export { ns } from \"./b.mjs\";\n", "imports": { "./b.mjs": "m/b.mjs" } },
            { "key": "m/b.mjs", "source": "export * as ns from \"./p.mjs\";\n", "imports": { "./p.mjs": "m/p.mjs" } },
            { "key": "m/p.mjs", "source": "export let value = 17; export function set(v) { value = v; }\n", "imports": {} }
        ]);
        let graphs = serde_json::json!({
            "m/i.mjs": { "local": {}, "indirect": { "pns": ["./p.mjs", null, false] }, "stars": ["./l.mjs", "./r.mjs"], "requests": [], "dependencies": ["./l.mjs", "./r.mjs"] },
            "m/l.mjs": { "local": {}, "indirect": { "ns": ["./b.mjs", "ns", false] }, "stars": [], "requests": [["./b.mjs", "ns"]], "dependencies": ["./b.mjs"] },
            "m/r.mjs": { "local": {}, "indirect": { "ns": ["./b.mjs", "ns", false] }, "stars": [], "requests": [["./b.mjs", "ns"]], "dependencies": ["./b.mjs"] },
            "m/b.mjs": { "local": {}, "indirect": { "ns": ["./p.mjs", null, false] }, "stars": [], "requests": [], "dependencies": ["./p.mjs"] },
            "m/p.mjs": { "local": { "value": "value", "set": "set" }, "indirect": {}, "stars": [], "requests": [], "dependencies": [] }
        });
        let index = r#"var i = require("./i.mjs"), p = require("./p.mjs"); exports.run = function () { i.ns.set(18); return [i.ns === p, i.pns === p, i.ns.value, Object.keys(i).join()].join('|'); };"#;
        let mut p =
            Package::load(&bundle_with_graphs("m/i.mjs", modules, index, graphs)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("true|true|18|ns,pns".into())
        );
    }

    // Node 22.23.1 (g2b-probes/lazy): a module reachable only through `import()` stays out of the entry's link: the entry
    // loads though that module has a missing name, and the import() rejects with the SyntaxError before any body runs.
    // (A mixed module's origin lists its dynamic specifiers too; only the export graph's `dependencies` are static.)
    #[test]
    fn lazy_module_with_a_missing_name_rejects_its_dynamic_import_only() {
        let modules = serde_json::json!([
            { "key": "m/p.mjs", "source": "import { origin as __thaw_esm_origin_0 } from \"__thaw_private_origin_spec/m/p.mjs\";\nexport const ready = 9; export const load = () => import(\"./lazy.mjs\");\n", "imports": { "./lazy.mjs": "m/lazy.mjs" } },
            { "key": "m/lazy.mjs", "source": "import { absent } from \"./mid.mjs\"; export const value = 1;\n", "imports": { "./mid.mjs": "m/mid.mjs" } },
            { "key": "m/mid.mjs", "source": "globalThis.midRuns = (globalThis.midRuns || 0) + 1; export const present = 1;\n", "imports": {} }
        ]);
        let graphs = serde_json::json!({
            "m/p.mjs": { "local": { "ready": "ready", "load": "load" }, "indirect": {}, "stars": [], "requests": [], "dependencies": [] },
            "m/lazy.mjs": { "local": { "value": "value" }, "indirect": {}, "stars": [], "requests": [["./mid.mjs", "absent"]], "dependencies": ["./mid.mjs"] },
            "m/mid.mjs": { "local": { "present": "present" }, "indirect": {}, "stars": [], "requests": [], "dependencies": [] }
        });
        let index = r#"exports.run = function () { var p = require("./p.mjs"); return p.load().then(function () { return 'no throw'; }, function (e) { return [p.ready, e.name, String(globalThis.midRuns)].join('|'); }); };"#;
        let mut p =
            Package::load(&bundle_with_graphs("m/p.mjs", modules, index, graphs)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("9|SyntaxError|undefined".into())
        );
    }

    // Node 22.23.1 (g2b-probes/fl): `import { nope } from './dep.mjs'` is a SyntaxError at link time, before ANY body of the
    // graph runs (`okRuns` and `depRuns` stay undefined); the error is cached, and the unrelated modules load normally.
    #[test]
    fn missing_named_export_fails_the_link_before_any_body_runs_and_is_cached() {
        let modules = serde_json::json!([
            { "key": "m/p.mjs", "source": "import \"./ok.mjs\"; import { nope } from \"./dep.mjs\";\nglobalThis.parentRuns = (globalThis.parentRuns || 0) + 1;\n", "imports": { "./ok.mjs": "m/ok.mjs", "./dep.mjs": "m/dep.mjs" } },
            { "key": "m/ok.mjs", "source": "globalThis.okRuns = (globalThis.okRuns || 0) + 1; export const o = 1;\n", "imports": {} },
            { "key": "m/dep.mjs", "source": "globalThis.depRuns = (globalThis.depRuns || 0) + 1; export const x = 1;\n", "imports": {} }
        ]);
        let graphs = serde_json::json!({
            "m/p.mjs": { "local": {}, "indirect": {}, "stars": [], "requests": [["./dep.mjs", "nope"]] },
            "m/ok.mjs": { "local": { "o": "o" }, "indirect": {}, "stars": [], "requests": [] },
            "m/dep.mjs": { "local": { "x": "x" }, "indirect": {}, "stars": [], "requests": [] }
        });
        let index = r#"var grab = function (spec) { try { require(spec); } catch (e) { return e; } };
exports.run = function () {
  var first = grab("./p.mjs"), again = grab("./p.mjs");
  var ok = require("./ok.mjs"), dep = require("./dep.mjs");
  return [first.name, first.message, first === again, String(globalThis.parentRuns), globalThis.okRuns, globalThis.depRuns, ok.o, dep.x].join('|');
};"#;
        let mut p =
            Package::load(&bundle_with_graphs("m/p.mjs", modules, index, graphs)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("SyntaxError|The requested module './dep.mjs' does not provide an export named 'nope'|true|undefined|1|1|1|1".into())
        );
    }

    // Node 22.23.1 (g2b-probes/lazy2): two different bindings of one name through `export *` are ambiguous at link time.
    #[test]
    fn ambiguous_star_name_fails_the_link_with_a_syntax_error() {
        let modules = serde_json::json!([
            { "key": "m/p.mjs", "source": "import { d } from \"./mid.mjs\";\nglobalThis.parentRuns = 1;\n", "imports": { "./mid.mjs": "m/mid.mjs" } },
            { "key": "m/mid.mjs", "source": "export * from \"./l.mjs\"; export * from \"./r.mjs\";\n", "imports": { "./l.mjs": "m/l.mjs", "./r.mjs": "m/r.mjs" } },
            { "key": "m/l.mjs", "source": "globalThis.sideRuns = (globalThis.sideRuns || 0) + 1; export const d = 1;\n", "imports": {} },
            { "key": "m/r.mjs", "source": "globalThis.sideRuns = (globalThis.sideRuns || 0) + 1; export const d = 2;\n", "imports": {} }
        ]);
        let graphs = serde_json::json!({
            "m/p.mjs": { "local": {}, "indirect": {}, "stars": [], "requests": [["./mid.mjs", "d"]] },
            "m/mid.mjs": { "local": {}, "indirect": {}, "stars": ["./l.mjs", "./r.mjs"], "requests": [] },
            "m/l.mjs": { "local": { "d": "d" }, "indirect": {}, "stars": [], "requests": [] },
            "m/r.mjs": { "local": { "d": "d" }, "indirect": {}, "stars": [], "requests": [] }
        });
        let index = r#"exports.run = function () { try { require("./p.mjs"); } catch (e) { return [e.name, e.message, String(globalThis.sideRuns), String(globalThis.parentRuns)].join('|'); } return 'no throw'; };"#;
        let mut p =
            Package::load(&bundle_with_graphs("m/p.mjs", modules, index, graphs)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("SyntaxError|The requested module './mid.mjs' contains conflicting star exports for name 'd'|undefined|undefined".into())
        );
    }

    // graph p -> {ok, bad}: nothing in ok.mjs ran, every later import of p or bad rejects with the SAME error object,
    // and ok.mjs alone still loads (its counter is 1).
    #[test]
    fn failed_link_is_cached_and_does_not_poison_unrelated_modules() {
        let modules = serde_json::json!([
            { "key": "m/p.mjs", "source": "import \"./ok.mjs\"; import \"./bad.mjs\";\nglobalThis.parentRuns = (globalThis.parentRuns || 0) + 1;\n", "imports": { "./ok.mjs": "m/ok.mjs", "./bad.mjs": "m/bad.mjs" } },
            { "key": "m/ok.mjs", "source": "globalThis.okRuns = (globalThis.okRuns || 0) + 1; export const o = 1;\n", "imports": {} },
            { "key": "m/bad.mjs", "source": "globalThis.badRuns = (globalThis.badRuns || 0) + 1; import \"./unmapped.mjs\"; export const value = 1;\n", "imports": {} }
        ]);
        let index = r#"var grab = function (spec) { try { require(spec); } catch (e) { return e; } };
exports.run = function () {
  var first = grab("./p.mjs"), okBefore = globalThis.okRuns, again = grab("./p.mjs"), bad = grab("./bad.mjs");
  var ok = require("./ok.mjs");
  globalThis.firstError = first;
  return [first.code, String(okBefore), String(globalThis.badRuns), String(globalThis.parentRuns), first === again, first === bad, ok.o, globalThis.okRuns].join('|');
};
exports.again = function () { return requireAsync("./bad.mjs"); };
exports.isFirst = function (e) { return e === globalThis.firstError; };"#;
        let mut p = Package::load(&bundle("m/p.mjs", modules, index)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("ERR_MODULE_NOT_FOUND|undefined|undefined|undefined|true|true|1|1".into())
        );
        // a later import() rejects with the same Error object
        let exports = p.exports();
        let it = p.interp();
        let again = it.get(&exports, "again").unwrap();
        let promise = it.call(&again, &Value::Undefined, &[]).unwrap();
        let Some(Err(reason)) = it.await_settled(&promise).unwrap() else {
            panic!("import() of a failed link must reject")
        };
        let is_first = it.get(&exports, "isFirst").unwrap();
        let same = it.call(&is_first, &Value::Undefined, &[reason]).unwrap();
        assert!(matches!(same, Value::Bool(true)));
    }

    // Node 22.23.1 (probes g2-probes/cyc2): depth-first, dependencies first; the cycle peer sees the hoisted function
    // and TDZ for let/const through a namespace and through a named import, at the read.
    const CYC_A: &str = "import './b.mjs'; import './c.mjs';\n(globalThis.L ||= []).push('eval a'); export function f(){return 'f'} export let n = 1; export const k = 9;\n";
    const CYC_B: &str = "import * as A from './a.mjs'; import { n, f } from './a.mjs';\n(globalThis.L ||= []).push('eval b ' + typeof f + ' ' + typeof A.f);\nfor (const name of ['n','k']) { try { globalThis.L.push('b reads A.' + name + ' ' + A[name]); } catch (e) { globalThis.L.push('b reads A.' + name + ' ' + e.name + ' ' + e.message); } }\ntry { globalThis.L.push(String(n)); } catch (e) { globalThis.L.push('b named n ' + e.name + ' ' + e.message); }\nexport const bk = 1; export function bf(){ return n; }\n";
    const CYC_C: &str =
        "import { bf, bk } from './b.mjs'; (globalThis.L ||= []).push('eval c ' + bk);\n";

    fn cycle_modules() -> serde_json::Value {
        serde_json::json!([
            { "key": "m/a.mjs", "source": CYC_A, "imports": { "./b.mjs": "m/b.mjs", "./c.mjs": "m/c.mjs" } },
            { "key": "m/b.mjs", "source": CYC_B, "imports": { "./a.mjs": "m/a.mjs" } },
            { "key": "m/c.mjs", "source": CYC_C, "imports": { "./b.mjs": "m/b.mjs" } }
        ])
    }

    #[test]
    fn esm_cycle_evaluation_order_matches_node() {
        let index =
            r#"exports.run = function () { require("./a.mjs"); return globalThis.L.join('|'); };"#;
        let mut p =
            Package::load(&bundle("m/a.mjs", cycle_modules(), index)).expect("a sync cycle loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("eval b function function|b reads A.n ReferenceError Cannot access 'n' before initialization|b reads A.k ReferenceError Cannot access 'k' before initialization|b named n ReferenceError Cannot access 'n' before initialization|eval c 1|eval a".into())
        );
    }

    // Entry b first: c runs before b's body and reads `bk` in TDZ; the failure reaches the entry, no body of a or b ran.
    #[test]
    fn esm_cycle_entry_b_first_throws_tdz_in_c() {
        let index = r#"exports.run = function () { try { require("./b.mjs"); } catch (e) { return e.name + ': ' + e.message + ' / ' + (globalThis.L || []).join('|'); } return 'no throw'; };"#;
        let mut p = Package::load(&bundle("m/a.mjs", cycle_modules(), index)).expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("ReferenceError: Cannot access 'bk' before initialization / ".into())
        );
    }

    // `export default <expr>` and `export default class {}` are in TDZ for a cycle peer, `export default function () {}`
    // is hoisted and named "default". (Node 22.23.1, probes g2b-probes/dflt: e:ReferenceError|f:function:default|h:ReferenceError)
    #[test]
    fn esm_default_export_is_tdz_in_a_cycle() {
        let peer = |x: &str| {
            format!("import v from './{x}.mjs'; try {{ (globalThis.R ||= []).push('{x}:' + typeof v + ':' + v.name); }} catch (x) {{ (globalThis.R ||= []).push('{x}:' + x.name); }}\n")
        };
        let mut mods = Vec::new();
        for (x, d) in [
            ("e", "5 + 1;"),
            ("f", "function () { return 1; }"),
            ("h", "class { }"),
        ] {
            mods.push(serde_json::json!({ "key": format!("m/{x}.mjs"), "source": format!("import './g{x}.mjs'; export default {d}\n"), "imports": { format!("./g{x}.mjs"): format!("m/g{x}.mjs") } }));
            mods.push(serde_json::json!({ "key": format!("m/g{x}.mjs"), "source": peer(x), "imports": { format!("./{x}.mjs"): format!("m/{x}.mjs") } }));
        }
        let index = r#"exports.run = function () { require("./e.mjs"); require("./f.mjs"); require("./h.mjs"); return globalThis.R.join('|') + '|' + [require("./e.mjs").default, require("./f.mjs").default(), require("./h.mjs").default.name].join(','); };"#;
        let mut p = Package::load(&bundle("m/e.mjs", serde_json::Value::Array(mods), index))
            .expect("loads");
        assert_eq!(
            p.call("run", &[]).unwrap(),
            Native::Str("e:ReferenceError|f:function:default|h:ReferenceError|6,1,default".into())
        );
    }

    // The export shapes the TDZ guard and the default-export rewrite touch, outside a cycle: constructor / default-import /
    // namespace-import / re-export / star-export (a p-limit style `new Queue()` must keep working).
    // Node 22.23.1 (g2b-probes/shapes): "1,2,default,default,3,5,5,5,7,6,7,5,TypeError"
    #[test]
    fn export_shapes_survive_the_tdz_guard_and_default_rewrite() {
        let modules = serde_json::json!([
            { "key": "m/q.mjs", "source": "export default class { constructor() { this.n = 1; } static make() { return new this(); } }\n", "imports": {} },
            { "key": "m/f.mjs", "source": "export default function () { return 2; }\n", "imports": {} },
            { "key": "m/o.mjs", "source": "export default { k: 3 };\nexport let c = 4; export const d = 5; export class K { constructor() { this.z = 6; } }\nlet r = 7; export { r as renamed, r as default2 };\nexport let bump = () => { c++; };\n", "imports": {} },
            { "key": "m/re.mjs", "source": "export { c as c2, K as K2 } from './o.mjs';\nexport * from './o.mjs';\nexport { default as od } from './o.mjs';\nexport * as ons from './o.mjs';\n", "imports": { "./o.mjs": "m/o.mjs" } },
            { "key": "m/u.mjs", "source": "import Q from './q.mjs';\nimport F from './f.mjs';\nimport * as ns from './re.mjs';\nimport { c2, K2, od } from './re.mjs';\nexport function go() { ns.bump(); return [new Q().n, F(), Q.name, F.name, od.k, ns.c2, c2, ns.d, ns.renamed, new K2().z, ns.ons.default2, ns.ons.c, (() => { try { ns.d = 1; } catch (e) { return e.name; } })()].join(','); }\n",
              "imports": { "./q.mjs": "m/q.mjs", "./f.mjs": "m/f.mjs", "./re.mjs": "m/re.mjs" } }
        ]);
        let index = r#"var u = require("./u.mjs"); exports.run = function () { return u.go(); };"#;
        let mut p = Package::load(&bundle("m/u.mjs", modules, index)).expect("loads");
        let Native::Str(got) = p.call("run", &[]).unwrap() else {
            panic!()
        };
        assert_eq!(got, "1,2,default,default,3,5,5,5,7,6,7,5,TypeError");
    }

    // A cycle through a module with top-level await cannot be evaluated depth-first: still a scan blocker.
    #[test]
    fn esm_cycle_through_top_level_await_stays_blocked() {
        let modules = serde_json::json!([
            { "key": "m/a.mjs", "source": "import './b.mjs'; export const a = 1;\n", "imports": { "./b.mjs": "m/b.mjs" } },
            { "key": "m/b.mjs", "source": "import './a.mjs'; await 0; export const b = 1;\n", "imports": { "./a.mjs": "m/a.mjs" } }
        ]);
        let error = Package::load(&bundle("m/a.mjs", modules, ""))
            .err()
            .expect("blocked");
        assert!(format!("{error:?}").contains("esm-cycle"), "{error:?}");
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
            "\"use strict\";\nObject.defineProperty(exports, '__esModule', { value: true, configurable: true });\nObject.defineProperty(exports, '__thaw_esm_stars', { value: Object.create(null) });\n",
            "function __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\n",
            "function __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); ",
            "if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } ",
            "ns.default = m; return ns; }\n",
            "var __thaw_tdz___thaw_esm_dflt = false;\nvar __thaw_tdz_b = false;\n",
            "function __thaw_esm_tdz(n) { throw new ReferenceError(\"Cannot access '\" + n + \"' before initialization\"); }\n",
            "if (!Object.prototype.hasOwnProperty.call(exports, \"b\")) Object.defineProperty(exports, \"b\", { enumerable: true, get: function () { return (__thaw_tdz_b ? b : __thaw_esm_tdz(\"b\")); } });\n",
            "if (!Object.prototype.hasOwnProperty.call(exports, \"default\")) Object.defineProperty(exports, \"default\", { enumerable: true, get: function () { return (__thaw_tdz___thaw_esm_dflt ? __thaw_esm_dflt : __thaw_esm_tdz(\"default\")); } });\n",
            "if (!Object.prototype.hasOwnProperty.call(exports, \"c\")) Object.defineProperty(exports, \"c\", { enumerable: true, get: function () { return require(\"./y.js\")[\"c\"]; } });\n",
            "var __thaw_esm_m1 = require(\"./x.js\");\n",
            "const b = __thaw_esm_m1[\"a\"] + 1;;\n",
            "__thaw_tdz_b = true;\n",
            "const __thaw_esm_dflt = ({ default: (__thaw_esm_m1[\"a\"]) }).default;\n",
            "__thaw_tdz___thaw_esm_dflt = true;\n",
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
        assert_eq!(converted.body, "\"use strict\";\nObject.defineProperty(exports, '__esModule', { value: true, configurable: true });\nObject.defineProperty(exports, '__thaw_esm_stars', { value: Object.create(null) });\nfunction __thaw_esm_default(m) { return m && m.__esModule ? m.default : m; }\nfunction __thaw_esm_ns(m) { if (m && m.__esModule) return m; var ns = Object.create(null); if (m !== null && (typeof m === 'object' || typeof m === 'function')) { for (var k in m) ns[k] = m[k]; } ns.default = m; return ns; }\nvar __thaw_tdz_v = false;\nfunction __thaw_esm_tdz(n) { throw new ReferenceError(\"Cannot access '\" + n + \"' before initialization\"); }\nfunction __thaw_esm_star(ex, m) { var own = Object.prototype.hasOwnProperty;\n  var table = ex.__thaw_esm_stars;\n  var mine = m !== null && (typeof m === 'object' || typeof m === 'function') && own.call(m, '__thaw_esm_stars') ? m.__thaw_esm_stars : null;\n  (m === null || m === undefined ? [] : Object.keys(m)).forEach(function (k) {\n    if (k === 'default') return;\n    var bind = mine && own.call(mine, k) ? mine[k].bind : { m: m, k: k };\n    if (own.call(table, k)) {\n      if (!table[k].fixed && !table[k].amb && (table[k].bind.m !== bind.m || table[k].bind.k !== bind.k)) {\n        table[k].amb = true;\n        if (k === '__esModule') Object.defineProperty(ex, k, { value: true, configurable: true }); else delete ex[k];\n      }\n      return;\n    }\n    if (k !== '__esModule' && own.call(ex, k)) return;\n    table[k] = { bind: bind, amb: false };\n    Object.defineProperty(ex, k, { enumerable: true, configurable: true, get: function () { return bind.m[bind.k]; } });\n  });\n}\nif (!Object.prototype.hasOwnProperty.call(exports, \"r\")) Object.defineProperty(exports, \"r\", { enumerable: true, get: function () { return __thaw_esm_m2[\"q\"]; } });\nif (!Object.prototype.hasOwnProperty.call(exports, \"ns\")) Object.defineProperty(exports, \"ns\", { enumerable: true, get: function () { return __thaw_esm_ns(__thaw_esm_m3); } });\nif (!Object.prototype.hasOwnProperty.call(exports, \"v\")) Object.defineProperty(exports, \"v\", { enumerable: true, get: function () { return (__thaw_tdz_v ? v : __thaw_esm_tdz(\"v\")); } });\nfunction __thaw_esm_origin_obj(t) { var nsCache = Object.create(null); return { readDefault: function (s) { return __thaw_esm_default(t[s]()); }, read: function (s, n) { return t[s]()[n]; }, readNamespace: function (s) { return __thaw_esm_ns(t[s]()); }, readImport: function (s, n) { return t[s]()[n]; }, namespaceImport: function (s) { var m = t[s](); if (m !== null && (typeof m === 'object' || typeof m === 'function') && Object.prototype.hasOwnProperty.call(m, '__thaw_esm_stars')) return __thaw_esm_ns(m); if (!nsCache[s] || !Object.is(nsCache[s].m, m)) nsCache[s] = { m: m, ns: __thaw_esm_cjs_ns(m) }; return nsCache[s].ns; }, defaultImport: function (s, m) { return __thaw_esm_default(m); }, validateRequests: function () {}, validateDynamic: function () {}, completeDynamic: function (s, p) { return p; }, rejectDynamic: function (e) { return Promise.reject(e); } }; }\nfunction __thaw_esm_cjs_ns(m) { var names = m !== null && (typeof m === 'object' || typeof m === 'function') ? Object.keys(m).filter(function (k) { return k !== 'default'; }) : []; names.push('default'); var ns = {}; names.sort().forEach(function (k) { Object.defineProperty(ns, k, { value: k === 'default' ? m : m[k], enumerable: true, writable: true }); }); Object.defineProperty(ns, Symbol.toStringTag, { value: 'Module' }); return Object.preventExtensions(ns); }\nvar __thaw_esm_m1 = require(\"./s.js\");\n__thaw_esm_star(exports, __thaw_esm_m1);\nvar __thaw_esm_m2 = require(\"./t.js\");\nvar __thaw_esm_m3 = require(\"./u.js\");\nvar __thaw_esm_origin_0 = __thaw_esm_origin_obj({ \"node:os\": function () { return require(\"node:os\"); } });\nconst v = __thaw_esm_origin_0.readDefault('node:os');;\n__thaw_tdz_v = true;\n");
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
    let converted = convert_module(
        &module,
        &cm,
        &[],
        &[],
        &ENTRY_NAMES,
        None,
        &BTreeMap::new(),
        &Links::default(),
    );
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
