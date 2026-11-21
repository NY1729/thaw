// thaw-js routing (the default JavaScript engine): which packages the Thaw-native JavaScript runtime
// can serve for a given program. A package it cannot serve falls back to the QuickJS path while the
// fallback exists (`THAW_JS_ENGINE=quickjs` opts out of thaw-js entirely).

/// The engine when no env var selects one: thaw-js. `THAW_JS_ENGINE=quickjs` is the temporary opt-out.
const THAW_JS_DEFAULT: bool = true;

/// Whether thaw-js routing is on. `THAW_JS_ENGINE` (`thaw-js` or `quickjs`) wins over
/// `THAW_JS_RUNTIME`, which stays a legacy alias: `1` is thaw-js, `0` is QuickJS.
/// Unset, empty and unknown values fall through to the next rule.
fn thaw_js_engine_from(engine: Option<&str>, runtime: Option<&str>, default_thaw_js: bool) -> bool {
    match engine {
        Some("thaw-js") => true,
        Some("quickjs") => false,
        _ => match runtime {
            Some("1") => true,
            Some("0") => false,
            _ => default_thaw_js,
        },
    }
}

/// Warns once on stderr about a `THAW_JS_ENGINE` value that is neither `thaw-js` nor `quickjs`.
fn warn_unknown_thaw_js_engine(engine: Option<&str>) {
    static WARNED: std::sync::Once = std::sync::Once::new();
    if let Some(other) = engine.filter(|v| !matches!(*v, "" | "thaw-js" | "quickjs")) {
        WARNED.call_once(|| {
            eprintln!("thaw: unknown THAW_JS_ENGINE `{other}`, using the default engine");
        });
    }
}

fn thaw_js_routing_enabled() -> bool {
    let engine = std::env::var("THAW_JS_ENGINE").ok();
    let runtime = std::env::var("THAW_JS_RUNTIME").ok();
    warn_unknown_thaw_js_engine(engine.as_deref());
    thaw_js_engine_from(engine.as_deref(), runtime.as_deref(), THAW_JS_DEFAULT)
}

thread_local! {
    /// Why thaw-js refused a package in the current build (`(package, why)`), for `thaw inspect`.
    static THAW_JS_BLOCKERS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// Packages the LAST shim-generation pass left on QuickJS (`THAW_JS_FALLBACK` warn/error). A pass can refuse a
    /// package that the next pass (with forced fallbacks) accepts, so only the final pass's list counts.
    static THAW_JS_REFUSALS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Replaces the refusal list (`(package, why)`) with the current pass's.
fn set_thaw_js_refusals(refusals: Vec<(String, String)>) {
    THAW_JS_REFUSALS.with(|r| *r.borrow_mut() = refusals);
}

/// The final pass's refusals (cleared).
fn take_thaw_js_refusals() -> Vec<(String, String)> {
    THAW_JS_REFUSALS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Starts a fresh blocker list for a build (a build's routing attempts all record into it).
fn clear_thaw_js_blockers() {
    THAW_JS_BLOCKERS.with(|blockers| blockers.borrow_mut().clear());
    set_thaw_js_refusals(Vec::new());
}

/// Records a blocker; returns whether it was new to this build.
fn record_thaw_js_blocker(package: &str, why: &str) -> bool {
    THAW_JS_BLOCKERS.with(|blockers| {
        let entry = (package.to_string(), why.to_string());
        let mut blockers = blockers.borrow_mut();
        if blockers.contains(&entry) {
            false
        } else {
            blockers.push(entry);
            true
        }
    })
}

/// Every reason recorded for `package` in this build, joined: the text of a refusal error.
fn thaw_js_package_refusal(package: &str) -> String {
    THAW_JS_BLOCKERS.with(|blockers| {
        blockers
            .borrow()
            .iter()
            .filter(|(name, _)| name == package)
            .map(|(_, why)| why.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// Every package refusal recorded in this build as `package `X`: why`, joined (appended to a fallback error).
fn thaw_js_recorded_refusals_text() -> String {
    THAW_JS_BLOCKERS.with(|blockers| {
        blockers
            .borrow()
            .iter()
            .filter(|(name, _)| name != "<build>")
            .map(|(name, why)| format!("package `{name}` refused: {why}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// The `FALLBACK:` line of a routed build that stayed on QuickJS: the error, then the refused packages (sorted, one
/// reason each: its first line, its first term, at most 80 chars; at most 8 packages, then `+N more`).
fn thaw_js_fallback_message(error: &str, refusals: &[(String, String)]) -> String {
    const MAX_PACKAGES: usize = 8;
    const MAX_REASON: usize = 80;
    let mut by_package: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();
    for (package, why) in refusals {
        by_package.entry(package.as_str()).or_insert_with(|| {
            let line = why.lines().next().unwrap_or_default();
            let term = line.rsplit(": ").next().unwrap_or_default().split(", ").next().unwrap_or_default();
            term.trim().chars().take(MAX_REASON).collect()
        });
    }
    if by_package.is_empty() {
        return error.to_string();
    }
    let total = by_package.len();
    let mut items: Vec<String> = by_package
        .iter()
        .take(MAX_PACKAGES)
        .map(|(package, why)| if why.is_empty() { package.to_string() } else { format!("{package} ({why})") })
        .collect();
    if total > MAX_PACKAGES {
        items.push(format!("+{} more", total - MAX_PACKAGES));
    }
    format!("{error}; refused: {}", items.join(", "))
}

/// What happens when thaw-js cannot serve something (`THAW_JS_FALLBACK`): `Silent` keeps the QuickJS
/// fallback quiet, `Warn` prints it as a `FALLBACK:` line, `Error` fails the build instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThawJsFallbackMode {
    Silent,
    Warn,
    Error,
}

/// Pure parser for `THAW_JS_FALLBACK`. Unset, empty and unknown values are `Silent`.
fn thaw_js_fallback_mode_from(value: Option<&str>) -> ThawJsFallbackMode {
    match value {
        Some("warn") => ThawJsFallbackMode::Warn,
        Some("error") => ThawJsFallbackMode::Error,
        _ => ThawJsFallbackMode::Silent,
    }
}

#[cfg(test)]
thread_local! {
    /// Test-only per-thread `THAW_JS_FALLBACK` override, so parallel tests never share the process env.
    static FALLBACK_MODE_OVERRIDE: std::cell::Cell<Option<ThawJsFallbackMode>> = const { std::cell::Cell::new(None) };
}

/// Sets the calling thread's fallback-mode override (tests only; the build reads it on the same thread).
#[cfg(test)]
fn set_thaw_js_fallback_mode_override(mode: Option<ThawJsFallbackMode>) {
    FALLBACK_MODE_OVERRIDE.with(|cell| cell.set(mode));
}

fn thaw_js_fallback_mode() -> ThawJsFallbackMode {
    #[cfg(test)]
    if let Some(mode) = FALLBACK_MODE_OVERRIDE.with(|cell| cell.get()) {
        return mode;
    }
    thaw_js_fallback_mode_from(std::env::var("THAW_JS_FALLBACK").ok().as_deref())
}

/// The manifest's `thaw_js_blockers` value: `[{"package", "why"}]`.
fn thaw_js_blockers_manifest() -> serde_json::Value {
    THAW_JS_BLOCKERS.with(|blockers| {
        serde_json::Value::Array(
            blockers
                .borrow()
                .iter()
                .map(|(package, why)| serde_json::json!({ "package": package, "why": why }))
                .collect(),
        )
    })
}

/// The program's own dynamic code (`new Function`, `eval`) is served on thaw-js by default;
/// `THAW_JS_USER_DYNAMIC=0` opts out and keeps it on QuickJS.
fn thaw_js_user_dynamic_enabled() -> bool {
    thaw_js_user_dynamic_from(std::env::var("THAW_JS_USER_DYNAMIC").ok().as_deref())
}

/// Only an explicit `0` turns user dynamic code off; unset or any other value leaves it on.
fn thaw_js_user_dynamic_from(value: Option<&str>) -> bool {
    value != Some("0")
}

/// Route-all is on by default: every program is attempted on thaw-js, not only packages and dynamic-code
/// programs. A build that still needs QuickJS falls back and records why. `THAW_JS_ROUTE_ALL=0` opts out
/// (only the exact value `0` does, like `THAW_JS_USER_DYNAMIC`). Ignored when thaw-js itself is off.
fn thaw_js_route_all_enabled() -> bool {
    thaw_js_route_all_from(std::env::var("THAW_JS_ROUTE_ALL").ok().as_deref())
}

/// Only an explicit `0` turns route-all off; unset or any other value routes every program.
fn thaw_js_route_all_from(value: Option<&str>) -> bool {
    value != Some("0")
}

/// A routed program whose own source needs the dynamic host, built without `THAW_JS_USER_DYNAMIC=1`,
/// links QuickJS; record that so `thaw inspect` says why.
fn record_thaw_js_user_dynamic_blocker(user_source: &str, routing: bool, user_dynamic: bool) {
    if routing && !user_dynamic && source_uses_quickjs(user_source) {
        record_thaw_js_blocker(
            "<program>",
            "program source uses dynamic code (new Function/eval): needs THAW_JS_USER_DYNAMIC=1",
        );
    }
}

/// A routed build that failed and was rebuilt on QuickJS (`<build>`: no single package).
fn record_thaw_js_fallback_blocker(error: &str) {
    record_thaw_js_blocker(
        "<build>",
        &format!("routed build fell back to QuickJS: {error}"),
    );
}

/// Whether the program text names `name` (the last segment of a dotted path) as a whole
/// identifier (`hash` is not mentioned by `hashSync`).
fn thaw_js_mentions(user_source: &str, name: &str) -> bool {
    let name = name.rsplit('.').next().unwrap_or(name);
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    user_source.match_indices(name).any(|(start, _)| {
        !user_source[..start]
            .chars()
            .next_back()
            .is_some_and(is_word)
            && !user_source[start + name.len()..]
                .chars()
                .next()
                .is_some_and(is_word)
    })
}

/// Whether a program of these module-graph sources may call the realm `fetch`: the rewrite that reads it
/// needs the identifier in source, so a scan of every source is a safe over-approximation (a shadowed local
/// or the word in a comment only costs the extra `node:http` / `node:https` bundles).
fn source_may_call_realm_fetch(sources: &[&str]) -> bool {
    sources
        .iter()
        .any(|source| thaw_js_mentions(source, "fetch"))
}

/// The realm `fetch` transport's bundles, linked by `force_realm_fetch_bundles`.
const REALM_FETCH_TRANSPORT_BUNDLES: [&str; 2] = ["node:http", "node:https"];

/// The realm `fetch` transport bundles the shim must load, imported or not (empty when the program may not call
/// `fetch`). Unlike `force_realm_fetch_bundles`, an imported package stays in this set without losing its declarations.
fn realm_fetch_transport_bundles(sources: &[&str]) -> std::collections::HashSet<String> {
    if source_may_call_realm_fetch(sources) {
        REALM_FETCH_TRANSPORT_BUNDLES
            .iter()
            .map(|name| name.to_string())
            .collect()
    } else {
        std::collections::HashSet::new()
    }
}

/// Adds the realm `fetch` transport bundles to the package lists as if imported, at runtime only. A package
/// the program already imports keeps its typed declarations; one added here goes into `forced`, so the
/// declaration passes emit no names for it (its bundle still loads lazily on the first fetch).
fn force_realm_fetch_bundles(
    runtime_packages: &mut std::collections::HashSet<String>,
    resolved_packages: &mut Vec<String>,
    forced: &mut std::collections::HashSet<String>,
) {
    for builtin in REALM_FETCH_TRANSPORT_BUNDLES {
        runtime_packages.insert(builtin.to_string());
        if !resolved_packages.iter().any(|package| package == builtin) {
            resolved_packages.push(builtin.to_string());
            forced.insert(builtin.to_string());
        }
    }
}

/// The local names the program binds to the default export of the package `pkg_name`: `import p from`,
/// `import { default as p } from`, `import p = require(..)`. Unparseable source binds none.
fn thaw_js_default_import_locals(pkg_name: &str, user_source: &str) -> Vec<String> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem, TsModuleRef};
    let Ok((module, _)) = thaw_parser::parse_typescript_with_source_map_named(
        user_source,
        thaw_parser::common::FileName::Custom("thaw-js routing".into()),
    ) else {
        return Vec::new();
    };
    let same_module = |source: &str| {
        source.trim_start_matches("node:") == pkg_name.trim_start_matches("node:")
    };
    let mut locals = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import))
                if import.src.value.as_str().is_some_and(same_module) =>
            {
                for specifier in &import.specifiers {
                    match specifier {
                        ImportSpecifier::Default(default) => {
                            locals.push(default.local.sym.to_string());
                        }
                        ImportSpecifier::Named(named)
                            if matches!(&named.imported, Some(ModuleExportName::Ident(name)) if &*name.sym == "default") =>
                        {
                            locals.push(named.local.sym.to_string());
                        }
                        _ => {}
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::TsImportEquals(decl)) => {
                if let TsModuleRef::TsExternalModuleRef(external) = &decl.module_ref {
                    if external.expr.value.as_str().is_some_and(same_module) {
                        locals.push(decl.id.sym.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    locals
}

/// Whether the program uses the package's value export `name` as a value: any value but a class and
/// anything but a namespace object whose only uses are calls of the package's own functions
/// (`validator.isEmail(..)`, which the shim resolves to the typed functions) and its import. Such a
/// value is served to the program as a thaw-js handle (`$value$` class operation). The default export
/// is also used under the program's own local name (`import p from 'node:path'`), and so is a CommonJS
/// `export =` value, so those uses count.
fn thaw_js_value_export_needs_handle(pkg: &ResolvedPackage, user_source: &str, name: &str) -> bool {
    if pkg.classes.iter().any(|class| class.name == name) {
        return thaw_js_class_needs_handle(pkg, user_source, name);
    }
    // (a builtin's other value exports -- `path.posix`, `path.win32` -- are objects of their own
    // whose members differ from the module's functions, so `posix.join(..)` needs the value)
    let namespace_like = !pkg.name.starts_with("node:") || pkg.name.ends_with(name);
    if thaw_js_binding_needs_handle(pkg, user_source, name, namespace_like) {
        return true;
    }
    // The `export =` value of a CommonJS package is its default export too: a default import binds it
    // under the program's own local name (`import pc from 'picocolors'`), so those uses count.
    let default_export = name == "default" || pkg.commonjs_export_name.as_deref() == Some(name);
    (default_export
        && thaw_js_default_import_locals(&pkg.name, user_source)
            .iter()
            .any(|local| thaw_js_binding_needs_handle(pkg, user_source, local, true)))
        // A builtin's namespace used as a value is served as its own namespace object (`NAMESPACE_VALUE`).
        || (name == thaw_js::host::NAMESPACE_VALUE && !thaw_js_bare_namespace_locals(pkg, user_source).is_empty())
}

/// The local names the program binds to the whole namespace of the builtin `pkg` (`import * as p
/// from 'node:..'`) and uses as a value rather than only through `p.member(..)` calls. Such a
/// namespace is served as Node's namespace object (`thaw_js::host::NAMESPACE_VALUE`) and `module_graph`
/// binds the namespace to that handle.
fn thaw_js_bare_namespace_locals(pkg: &ResolvedPackage, user_source: &str) -> Vec<String> {
    use thaw_parser::ast::{ImportSpecifier, ModuleDecl, ModuleItem};
    if !pkg.name.starts_with("node:") {
        return Vec::new();
    }
    let Ok((module, _)) = thaw_parser::parse_typescript_with_source_map_named(
        user_source,
        thaw_parser::common::FileName::Custom("thaw-js routing".into()),
    ) else {
        return Vec::new();
    };
    let same_module =
        |source: &str| source.trim_start_matches("node:") == pkg.name.trim_start_matches("node:");
    let mut locals = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if !import.src.value.as_str().is_some_and(same_module) {
            continue;
        }
        for specifier in &import.specifiers {
            if let ImportSpecifier::Namespace(namespace) = specifier {
                let local = namespace.local.sym.to_string();
                if thaw_js_binding_needs_handle(pkg, user_source, &local, true) {
                    locals.push(local);
                }
            }
        }
    }
    locals
}

/// Whether some use of the identifier `name` in the program is a value use rather than an import or a call
/// of one of the package's functions through it (see `thaw_js_value_export_needs_handle`).
fn thaw_js_binding_needs_handle(
    pkg: &ResolvedPackage,
    user_source: &str,
    name: &str,
    namespace_like: bool,
) -> bool {
    if !thaw_js_mentions(user_source, name) {
        return false;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    thaw_js_identifier_uses(user_source, name).into_iter().any(|(_, after)| {
        let member = after.strip_prefix('.').map(|member| {
            let end = member.find(|c: char| !is_word(c)).unwrap_or(member.len());
            (&member[..end], member[end..].trim_start())
        });
        let called_function = member.is_some_and(|(member, tail)| {
            namespace_like
                && tail.starts_with('(')
                && pkg.functions.iter().any(|function| function.name == member)
        }) || (after.trim_start().starts_with('(')
            && pkg.functions.iter().any(|function| {
                function.name == name || function.name == name.rsplit('.').next().unwrap_or(name)
            }));
        !called_function
    })
}

/// The uses of the identifier `name` in the program as `(text before, text after)` pairs. Not an import binding,
/// not part of a longer identifier, not the text of a string (`'node:path'`).
fn thaw_js_identifier_uses<'a>(user_source: &'a str, name: &str) -> Vec<(&'a str, &'a str)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let is_quote = |c: char| matches!(c, '\'' | '"' | '`');
    let mut uses = Vec::new();
    let mut offset = 0;
    while let Some(found) = user_source[offset..].find(name) {
        let start = offset + found;
        let (before, after) = (&user_source[..start], &user_source[start + name.len()..]);
        offset = start + name.len();
        if before.chars().next_back().is_some_and(|c| is_word(c) || is_quote(c))
            || after.chars().next().is_some_and(|c| is_word(c) || is_quote(c))
        {
            continue;
        }
        let import_binding = before.trim_end().ends_with("import")
            || before.trim_end().ends_with("* as")
            || before.trim_end().ends_with(',')
                && before
                    .rfind("import")
                    .is_some_and(|i| !before[i..].contains(';'))
            || after.trim_start().starts_with("from ");
        if !import_binding {
            uses.push((before, after));
        }
    }
    uses
}

/// Whether the program uses the class `name` as a bare value (`thaw_js_value_export_needs_handle`): a read of a
/// member that is not one of the class's methods, getters and properties included (`DateTime.DATE_FULL`), or the
/// class itself in a value position. `new C(..)`, `C.method(..)`, `C.prop = ..` (typed property assignment),
/// `x instanceof C` and type positions (`: C`, `<C>`, `C[]`, `as C`) keep the typed class, so a program that only
/// calls the class's static methods gets no value handle.
fn thaw_js_class_needs_handle(pkg: &ResolvedPackage, user_source: &str, name: &str) -> bool {
    let Some(class) = pkg.classes.iter().find(|class| class.name == name) else {
        return false;
    };
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let ends_with_keyword = |before: &str, keyword: &str| {
        before.strip_suffix(keyword).is_some_and(|rest| !rest.ends_with(|c: char| is_word(c)))
    };
    thaw_js_identifier_uses(user_source, name).into_iter().any(|(before, after)| {
        let before = before.trim_end();
        if let Some(member) = after.strip_prefix('.') {
            let end = member.find(|c: char| !is_word(c)).unwrap_or(member.len());
            let (member, tail) = (&member[..end], member[end..].trim_start());
            let assigned = tail.starts_with('=') && !tail.starts_with("==") && !tail.starts_with("=>");
            // (a static getter such as `DateTime.DATE_FULL` is a method entry of kind `Getter`: not a method call)
            return !assigned
                && !tail.starts_with('(')
                && !class.methods.iter().any(|method| {
                    method.name == member && matches!(method.kind, thaw_bridge::DtsMethodKind::Method)
                });
        }
        // `import { C } from 'pkg'`: the specifier is a binding, not a use (the `,` form is already skipped by
        // `thaw_js_identifier_uses`).
        let first_import_specifier = before.ends_with('{')
            && before.rfind("import").is_some_and(|i| !before[i..].contains(';'));
        let type_or_construct = first_import_specifier
            || before.ends_with([':', '<', '|', '&', '.'])
            || ["new", "instanceof", "typeof", "keyof", "as", "extends", "implements"]
                .iter()
                .any(|keyword| ends_with_keyword(before, keyword))
            || after.trim_start().starts_with(['(', '[', '<']);
        !type_or_construct
    })
}

/// A thaw-js package returns plain data, so a bare alias' opaque `JsValue` result becomes `Json`.
fn thaw_js_retarget_alias(alias: String, thaw_js_package: bool) -> String {
    if thaw_js_package {
        alias.replace("JsValue", "Json")
    } else {
        alias
    }
}

/// Builtins whose typed declarations keep an opaque `JsValue` on thaw-js: node:vm hands the same
/// context object back in later calls, so a plain-data copy (`Json`) would lose its identity.
fn thaw_js_keeps_js_value(package: &str) -> bool {
    package == "node:vm"
}

/// Whether a generated `declare function __thaw_typed_js_*` line only mentions types that cross the
/// thaw-js JSON boundary: `number`, `string`, `boolean`, `Json`, `void`, `null`, `undefined`, its own
/// type parameters, unions/array/object-literal shapes of those, and an opaque `JsValue` (a type the
/// `.d.ts` classifier could not resolve), which the retargeted declaration turns into `Json`.
/// Callbacks and every other type name (classes, `Buffer`, ...) are rejected, except a whole
/// `Uint8Array` result.
#[cfg(test)]
fn thaw_js_declaration_plain(line: &str) -> bool {
    thaw_js_declaration_plain_with(line, false)
}

/// `thaw_js_declaration_plain`. `callback_options`: a constructor's option
/// object may carry callback fields (`new Writable({ write })`), so its `=>` types are accepted.
/// Passed `true` only for `$new$` lines (the class-operation gate).
fn thaw_js_declaration_plain_with(line: &str, callback_options: bool) -> bool {
    let Some(rest) = line.strip_prefix("declare function ") else {
        return false;
    };
    // A directly declared callback type (`f: (x: number) => number`) stays on QuickJS. (The LLVM side
    // can already pass such a parameter to thaw-js, see `thaw_js_callback_type`; accepting it here is
    // a one-line change that flips `thaw_js_routing_falls_back_to_quickjs_when_a_callback_...`.)
    // A callback coerced into a `Json` parameter (lodash's `iteratee: Json`) is routed.
    // A callable RESULT (`ejs.compile`, `nanoid.customAlphabet`) is accepted below, see `callable_result_plain`.
    let name_end = rest.find(['<', '(']).unwrap_or(rest.len());
    let mut generics = Vec::new();
    let mut after_name = &rest[name_end..];
    if let Some(inner) = after_name.strip_prefix('<') {
        let mut depth = 1usize;
        let mut end = inner.len();
        for (index, ch) in inner.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        // Type parameter names: the identifier at the start of each top-level `<...>` entry.
        let mut depth = 0usize;
        let mut at_entry_start = true;
        let mut word = String::new();
        for ch in inner[..end].chars().chain(std::iter::once(',')) {
            if ch.is_alphanumeric() || ch == '_' || ch == '$' {
                if at_entry_start {
                    word.push(ch);
                }
                continue;
            }
            match ch {
                '<' | '(' | '{' | '[' => depth += 1,
                '>' | ')' | '}' | ']' => depth = depth.saturating_sub(1),
                _ => {}
            }
            if !word.is_empty() {
                generics.push(std::mem::take(&mut word));
            }
            if ch == ',' && depth == 0 {
                at_entry_start = true;
            } else if !ch.is_whitespace() {
                at_entry_start = false;
            }
        }
        after_name = &inner[(end + 1).min(inner.len())..];
    }
    let Some(result_start) = after_name.rfind("): ") else {
        return false;
    };
    let (params, result) = (&after_name[..result_start], &after_name[result_start + 3..]);
    // A callback in the name or generics or in a parameter stays rejected (a constructor's option
    // object is the exception, see `callback_options`).
    let header = &rest[..rest.len() - after_name.len()];
    if !callback_options && (header.contains("=>") || params.contains("=>")) {
        return false;
    }
    // A bare `Uint8Array` result (`readFileSync(path): Uint8Array`) crosses as bytes; in params,
    // unions and nested positions it stays rejected.
    let result_text = result.trim().trim_end_matches(';').trim();
    let bytes_result = result_text == "Uint8Array";
    let identifiers_plain = |text: &str| {
        let chars = text.chars().collect::<Vec<_>>();
        let mut index = 0;
        while index < chars.len() {
            let ch = chars[index];
            if ch == '"' || ch == '\'' {
                // String literal type: skip it.
                index += 1;
                while index < chars.len() && chars[index] != ch {
                    index += 1;
                }
                index += 1;
            } else if ch.is_alphabetic() || ch == '_' || ch == '$' {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_alphanumeric()
                        || chars[index] == '_'
                        || chars[index] == '$')
                {
                    index += 1;
                }
                let word = chars[start..index].iter().collect::<String>();
                let mut next = index;
                while next < chars.len() && chars[next] == ' ' {
                    next += 1;
                }
                if chars.get(next) == Some(&'?') {
                    next += 1;
                }
                let is_name = chars.get(next) == Some(&':');
                let allowed = matches!(
                    word.as_str(),
                    "number" | "string" | "boolean" | "Json" | "void" | "null" | "undefined"
                ) || generics.contains(&word)
                    || word == "JsValue";
                if !is_name && !allowed {
                    return false;
                }
            } else {
                index += 1;
            }
        }
        true
    };
    let result_plain = if result.contains("=>") && !callback_options {
        callable_result_plain(result_text, &identifiers_plain)
    } else {
        identifiers_plain(result)
    };
    identifiers_plain(params) && (bytes_result || result_plain)
}

/// A callable result (`(arg0?: Json) => string`, or a `|` union of such members as `ejs.compile`
/// declares) is a live handle on thaw-js, so it is plain when each member's parameters and return
/// are plain data (`Promise<T>` is plain when `T` is). A callable parameter is never this shape.
fn callable_result_plain(text: &str, plain: &impl Fn(&str) -> bool) -> bool {
    let members = thaw_js_top_level_members(text, '|');
    !members.is_empty()
        && members.into_iter().all(|member| {
            let mut member = member.trim();
            // a parenthesised member `((x) => string)`
            if member.len() >= 2
                && member.starts_with('(')
                && thaw_js_matching_paren(&member[1..]) == Some(member.len() - 2)
            {
                member = member[1..member.len() - 1].trim();
            }
            let Some(after_open) = member.strip_prefix('(') else {
                return false;
            };
            let Some(close) = thaw_js_matching_paren(after_open) else {
                return false;
            };
            let (params, after) = (&after_open[..close], after_open[close + 1..].trim_start());
            let Some(ret) = after.strip_prefix("=>").map(str::trim) else {
                return false;
            };
            let ret = ret
                .strip_prefix("Promise<")
                .and_then(|inner| inner.strip_suffix('>'))
                .unwrap_or(ret);
            plain(params) && plain(ret) && !params.contains("=>") && !ret.contains("=>")
        })
}

/// The index of the `)` closing an already-open `(` (`text` starts just after it), if any.
fn thaw_js_matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// The members of a top-level `separator`-joined type (`a | (b) => c` splits at the `|` outside parens and `<>`).
fn thaw_js_top_level_members(text: &str, separator: char) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut previous = '\0';
    for (index, ch) in text.char_indices() {
        match ch {
            '(' | '<' | '{' | '[' => depth += 1,
            // the `>` of an arrow `=>` closes nothing
            '>' if previous == '=' => {}
            ')' | '>' | '}' | ']' => depth = depth.saturating_sub(1),
            ch if ch == separator && depth == 0 => {
                members.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
        previous = ch;
    }
    members.push(&text[start..]);
    members
}

/// An async overload (`...): Promise<string>;` or `string | Promise<string>`, ejs `render`): the typed
/// call runs the event loop and unwraps the settled promise, so it is plain when its resolved type is.
#[cfg(test)]
fn thaw_js_async_declaration_plain(line: &str) -> bool {
    thaw_js_async_declaration_plain_with(line)
}

fn thaw_js_async_declaration_plain_with(line: &str) -> bool {
    let Some(at) = line.rfind("): ") else {
        return false;
    };
    let (params, result) = line.split_at(at);
    // `Promise<Uint8Array>` resolves to the bare bytes result the sync gate accepts
    let resolved = result.replace("Promise<Uint8Array>", "Uint8Array");
    result.contains("Promise<")
        && thaw_js_declaration_plain_with(&format!("{params}{}", resolved.replace("Promise", "")), false)
}

/// Whether `line` declares an operation of `package`'s class `class` (`$new$`, `$method$`,
/// `$staticmethod$`, ... keys carry `package::Class`).
fn thaw_js_class_declaration(line: &str, package: &str, class: &str) -> bool {
    let Some(symbol) = line
        .strip_prefix("declare function __thaw_typed_js_")
        .or_else(|| line.strip_prefix("declare function __thaw_typed_tjs_"))
        .and_then(|rest| rest.split(['(', '<']).next())
    else {
        return false;
    };
    let hex = symbol.split("__").next().unwrap_or(symbol);
    let Some(bytes) = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let Ok(key) = String::from_utf8(bytes) else {
        return false;
    };
    key.starts_with('$') && key.split('$').nth(2) == Some(&format!("{package}::{class}"))
}

/// The decoded operation key of a `__thaw_typed_js_<hex>` / `__thaw_typed_tjs_<hex>` declaration line
/// (`$new$pkg::Class$arity0`, `$method$...`).
fn thaw_js_declaration_key(line: &str) -> Option<String> {
    let symbol = line
        .strip_prefix("declare function __thaw_typed_js_")
        .or_else(|| line.strip_prefix("declare function __thaw_typed_tjs_"))?
        .split(['(', '<'])
        .next()?;
    let hex = symbol.split("__").next()?;
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}

/// The member (`$method$pkg::Class$member$overload0$arity3`, `$staticmethod$...`, `$getter$pkg::Class$prop`,
/// `$setter$...`) a class operation line calls; `None` for a constructor or any other operation.
fn thaw_js_class_member(line: &str) -> Option<String> {
    let key = thaw_js_declaration_key(line)?;
    let mut parts = key.strip_prefix('$')?.split('$');
    let kind = parts.next()?;
    let member_kind = ["method", "staticmethod", "getter", "setter", "staticgetter", "staticsetter"]
        .iter()
        .any(|prefix| kind.starts_with(prefix));
    if !member_kind {
        return None;
    }
    parts.nth(1).map(str::to_string)
}

/// Whether the occurrence of `name` at `start` in `code` is a type word rather than a value. `void` is
/// reserved in JS, so a bare one is always an operator or a type (`: void`, `Promise<void>`); the other
/// TS keyword types count only right after an annotation (`:`, `<`, `|`, `&`). `z.void()` is a member use.
fn thaw_js_is_type_word(code: &str, start: usize, name: &str) -> bool {
    const TYPE_KEYWORDS: &[&str] = &[
        "string", "number", "boolean", "any", "unknown", "never", "object", "symbol", "bigint",
    ];
    let before = code[..start].trim_end();
    if before.ends_with('.') {
        return false;
    }
    name == "void" || (TYPE_KEYWORDS.contains(&name) && before.ends_with([':', '<', '|', '&']))
}

/// Whether `name` (a function export) is used other than as `name.member`, as a type word, or in an
/// import/re-export statement: those uses go through the package's value export, not through a call of
/// `name` itself.
fn thaw_js_uses_function(user_source: &str, name: &str) -> bool {
    let name = name.rsplit('.').next().unwrap_or(name);
    let code = thaw_js_without_module_statements(user_source);
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    code.match_indices(name).any(|(start, _)| {
        let end = start + name.len();
        !code[..start].chars().next_back().is_some_and(is_word)
            && !code[end..].chars().next().is_some_and(is_word)
            && !code[end..].starts_with('.')
            && !thaw_js_is_type_word(&code, start, name)
    })
}

/// The argument count of a call whose argument text starts just after its `(` (`args`), or `None` when the
/// text is not a plain argument list: a spread, a template literal, a `/` (regex or division), or no closing
/// `)`. Strings are skipped, so a `,` or `)` inside one does not count.
fn thaw_js_argument_count(args: &str) -> Option<usize> {
    let mut chars = args.char_indices();
    let mut depth = 0usize;
    let mut count = 0usize;
    let mut segment_has_text = false;
    while let Some((index, ch)) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            ',' if depth == 0 => {
                count += usize::from(segment_has_text);
                segment_has_text = false;
            }
            ')' if depth == 0 => return Some(count + usize::from(segment_has_text)),
            ')' | ']' | '}' => depth = depth.checked_sub(1)?,
            '(' | '[' | '{' => {
                depth += 1;
                segment_has_text = true;
            }
            '`' | '/' => return None,
            '.' if args[index..].starts_with("...") => return None,
            '\'' | '"' => {
                segment_has_text = true;
                loop {
                    match chars.next()? {
                        (_, '\\') => {
                            chars.next()?;
                        }
                        (_, quote) if quote == ch => break,
                        _ => {}
                    }
                }
            }
            _ => segment_has_text = true,
        }
    }
    None
}

/// The largest argument count of the program's calls of `name` (its last dotted segment), or `None` when
/// the program may reach it some other way (a bare value reference, a spread, an unparsed call, or no call
/// at all), in which case every overload is checked. Member uses `name.x` and type words are not calls.
fn thaw_js_call_arities(user_source: &str, name: &str) -> Option<usize> {
    let name = name.rsplit('.').next().unwrap_or(name);
    let code = thaw_js_without_module_statements(user_source);
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut max = None;
    for (start, _) in code.match_indices(name) {
        let end = start + name.len();
        if code[..start].chars().next_back().is_some_and(is_word)
            || code[end..].chars().next().is_some_and(is_word)
            || thaw_js_is_type_word(&code, start, name)
        {
            continue;
        }
        if let Some(args) = code[end..].trim_start().strip_prefix('(') {
            max = Some(max.unwrap_or(0).max(thaw_js_argument_count(args)?));
        } else if !code[end..].starts_with('.') {
            return None;
        }
    }
    max
}

/// Whether an overload declaration line can be called by the program. A `__arity_N` overload above the
/// largest call arity is never selected: a call passes at most that many arguments (an optional-parameter
/// wrapper dispatches to a smaller arity only for trailing `undefined`s). Any other line always stays.
fn thaw_js_overload_within_calls(line: &str, max_call_arity: Option<usize>) -> bool {
    let arity = line
        .split("__arity_")
        .nth(1)
        .map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>())
        .and_then(|digits| digits.parse::<usize>().ok());
    match (max_call_arity, arity) {
        (Some(max), Some(arity)) => arity <= max,
        _ => true,
    }
}

/// The program text with every `import ... from '...'`, `import '...'` and `export ... from '...'`
/// statement blanked out (they may share a line with code). Dynamic `import(...)`, `import.meta` and
/// local `export const ...` declarations are kept.
fn thaw_js_without_module_statements(user_source: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut out = String::with_capacity(user_source.len());
    let mut index = 0;
    while index < user_source.len() {
        let tail = &user_source[index..];
        let preceded_by_word = user_source[..index]
            .chars()
            .next_back()
            .is_some_and(|c| is_word(c) || c == '.');
        let keyword = ["import", "export"].into_iter().find(|kw| {
            tail.starts_with(kw)
                && !preceded_by_word
                && !tail[kw.len()..].chars().next().is_some_and(is_word)
        });
        if let Some(keyword) = keyword {
            if let Some(end) = thaw_js_module_statement_end(user_source, index + keyword.len(), keyword)
            {
                out.push(' ');
                index = end;
                continue;
            }
        }
        let c = tail.chars().next().expect("index is inside the source");
        out.push(c);
        index += c.len_utf8();
    }
    out
}

/// The end of the module statement whose keyword ends at `after_keyword`, or `None` when the keyword
/// does not start an import/re-export (`import(..)`, `import.meta`, `export const x = '..'`, ...).
fn thaw_js_module_statement_end(source: &str, after_keyword: usize, keyword: &str) -> Option<usize> {
    let rest = &source[after_keyword..];
    let quote_at = rest.find(['\'', '"'])?;
    let clause = &rest[..quote_at];
    if clause.contains(['(', ';', '=']) {
        return None;
    }
    let trimmed = clause.trim_end();
    let is_bare_import = keyword == "import" && clause.trim().is_empty();
    let has_from = trimmed.ends_with("from")
        && !trimmed[..trimmed.len() - "from".len()]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$');
    if !is_bare_import && !has_from {
        return None;
    }
    let quote = rest[quote_at..].chars().next()?;
    let close = rest[quote_at + 1..].find(quote)? + quote_at + 1;
    let mut end = after_keyword + close + 1;
    if source[end..].starts_with(';') {
        end += 1;
    }
    Some(end)
}

/// The function name encoded in a `__thaw_typed_js_<hex>[__suffix...]` symbol.
fn thaw_js_declared_function(line: &str, package: &str) -> Option<String> {
    let symbol = line
        .strip_prefix("declare function __thaw_typed_js_")
        .or_else(|| line.strip_prefix("declare function __thaw_typed_tjs_"))?
        .split(['(', '<'])
        .next()?;
    let hex = symbol.split('_').next()?;
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let key = String::from_utf8(bytes).ok()?;
    key.strip_prefix(&format!("{package}::"))
        .map(str::to_string)
}

/// Node builtin modules whose registry bundle and typed operations are validated on thaw-js (their
/// node-compat cases match Node under `THAW_JS_RUNTIME=1`). Any other `node:*` package stays on QuickJS
/// until its own cases pass: plan Q2 flips builtin modules one at a time.
const THAW_JS_BUILTIN_MODULES: &[&str] = &[
    "node:events",
    "node:module",
    "node:path",
    "node:path/posix",
    "node:path/win32",
    "node:assert",
    "node:assert/strict",
    "node:os",
    "node:util",
    "node:constants",
    "node:console",
    "node:url",
    "node:fs",
    "node:timers/promises",
    "node:fs/promises",
    "node:vm",
    "node:v8",
    "node:async_hooks",
    "node:querystring",
    "node:timers",
    "node:worker_threads",
    "node:wasi",
    "node:stream/web",
    "node:stream",
    "node:readline",
    "node:readline/promises",
    "node:dns",
    "node:dns/promises",
    "node:inspector",
    "node:inspector/promises",
    "node:util/types",
    "node:string_decoder",
    "node:tty",
    "node:dgram",
    "node:tls",
    "node:buffer",
    "node:process",
    "node:http",
    "node:child_process",
    "node:perf_hooks",
    "node:zlib",
    "node:net",
    "node:crypto",
    "node:https",
    "node:http2",
    "node:stream/consumers",
    "node:diagnostics_channel",
    "node:trace_events",
    "node:repl",
    "node:punycode",
    "node:domain",
    "node:stream/promises",
    "node:test",
    "node:test/reporters",
    "node:cluster",
    "node:sys",
    "node:sea",
    "node:sqlite",
];


/// The bundle text thaw-js loads for a package: the registered thaw-js form of a path builtin, else the
/// bundle itself (a plain script is wrapped into a one-module registry bundle).
fn thaw_js_bundle_form<'a>(
    forms: &'a std::collections::HashMap<String, String>,
    package: &str,
    bundle_js: &'a str,
) -> std::borrow::Cow<'a, str> {
    match forms.get(package) {
        Some(form) => form.as_str().into(),
        None => thaw_js::scan::script_as_bundle(bundle_js),
    }
}

/// The thaw-js form of `node:worker_threads` (owned by thaw-js: `scan::WORKER_THREADS_THAW_JS_FORM`, which package
/// bundles' embedded factories are substituted with too).
const THAW_JS_WORKER_THREADS_FORM: &str = thaw_js::scan::WORKER_THREADS_THAW_JS_FORM;

/// The thaw-js form of a `node:path` family specifier: the unmodified `node:path` bundle plus the realm marker
/// `thaw_tjs_register` reads (`member` is the export path of the shared realm: `posix`, `win32`, or empty for
/// the root). Every family specifier is then a view of one interpreter, so their values are identical objects.
fn thaw_js_path_bundle(root_bundle: &str, builtin: &str) -> String {
    let member = builtin.strip_prefix("node:path/").unwrap_or("");
    let marker = format!("var __thaw_bundle_realm = {{\"key\":\"node:path\",\"member\":\"{member}\"}};");
    root_bundle.replacen(
        "var __thaw_bundle_entry_key = \"node:path\";",
        &format!("var __thaw_bundle_entry_key = \"node:path\";\n{marker}"),
        1,
    )
}

/// The peer-group key of each routed bundle (`names[i]`; `deps[i]` = the routed bundles `i` requires by name): the
/// least name of its connected component in the peer-require graph, `None` for a singleton (it runs in its own realm).
pub(crate) fn thaw_js_peer_group_keys(names: &[&str], deps: &[Vec<usize>]) -> Vec<Option<String>> {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..deps.len()).collect();
    for (i, requires) in deps.iter().enumerate() {
        for &j in requires {
            let (a, b) = (root(&mut parent, i), root(&mut parent, j));
            parent[a] = b;
        }
    }
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); deps.len()];
    for i in 0..deps.len() {
        let r = root(&mut parent, i);
        members[r].push(i);
    }
    let mut keys = vec![None; deps.len()];
    for group in members.iter().filter(|group| group.len() > 1) {
        let key = group.iter().map(|&i| names[i]).min().expect("a group has members");
        for &i in group {
            keys[i] = Some(key.to_string());
        }
    }
    keys
}

/// `js` with `var __thaw_bundle_group = "<key>";` right after its entry-key statement: the scan reads the marker from
/// the wrapper body, next to the entry key. A bundle without that statement is returned unchanged.
fn thaw_js_with_group_marker(js: &str, key: &str) -> String {
    const ENTRY: &str = "var __thaw_bundle_entry_key = ";
    let Some(start) = js.find(ENTRY) else {
        return js.to_string();
    };
    let Some(semicolon) = js[start..].find(';') else {
        return js.to_string();
    };
    let at = start + semicolon + 1;
    format!("{}\nvar __thaw_bundle_group = \"{key}\";{}", &js[..at], &js[at..])
}

/// A `createHook` program that uses promise forms whose hook trace is not yet Node-identical on thaw-js
/// (`finally` trigger ids, Promise combinators): the module stays on QuickJS for those. `then`, `new Promise`
/// and resolve/reject are served by thaw-js, and so is AsyncLocalStorage (reaction contexts reach native awaits).
/// A plain `createHook` keeps `node:async_hooks` on thaw-js.
fn thaw_js_async_hooks_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    const UNVALIDATED_NAMES: &[&str] = &["finally"];
    const UNVALIDATED_PREFIXES: &[&str] = &["Promise.all", "Promise.race", "Promise.any"];
    (pkg_name == "node:async_hooks"
        && thaw_js_mentions(user_source, "createHook")
        && (UNVALIDATED_NAMES
            .iter()
            .any(|name| thaw_js_mentions(user_source, name))
            || UNVALIDATED_PREFIXES
                .iter()
                .any(|prefix| user_source.contains(prefix))))
    .then_some("createHook with finally or Promise combinators is not supported on thaw-js yet")
}

/// The packages installed in the registry at `registry_dir`: each package directory holding a `package.d.ts`
/// (what `thaw_registry::resolve` loads), by name; a scoped package is `@scope/name` under its scope directory.
pub(crate) fn registry_installed_packages(registry_dir: &std::path::Path) -> std::collections::HashSet<String> {
    let mut installed = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir(registry_dir) else {
        return installed;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if name.starts_with('@') {
            for scoped in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                if scoped.path().join("package.d.ts").is_file() {
                    let scoped_name = scoped.file_name().to_string_lossy().into_owned();
                    installed.insert(format!("{name}/{scoped_name}"));
                }
            }
        } else if path.join("package.d.ts").is_file() {
            installed.insert(name);
        }
    }
    installed
}

/// The implicit peers of the build's packages: every installed package a bundle requires by an unmapped exact name
/// (`pg` for drizzle's `require('pg')`) is compiled in with the program, since the runtime resolves that name to the
/// registered package. Repeats for the peers' own peers. Returns the installed names the bundles require (the
/// candidates for served peers, see `ScanContext::served_peers`).
pub(crate) fn add_implicit_peers(
    registry_dir: &std::path::Path,
    resolved_packages: &mut Vec<String>,
    runtime_packages: &mut std::collections::HashSet<String>,
    static_packages: &mut std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let installed = registry_installed_packages(registry_dir);
    let mut peers = std::collections::HashSet::new();
    let mut pending: Vec<String> = resolved_packages
        .iter()
        .filter(|package| runtime_packages.contains(*package))
        .cloned()
        .collect();
    while let Some(name) = pending.pop() {
        // a builtin or an unknown name does not resolve from the registry: it has no registry peers
        let Ok(package) = thaw_registry::resolve(registry_dir, &name) else {
            continue;
        };
        let Some(bundle_js) = package.bundle_js else {
            continue;
        };
        let Ok(served) = thaw_registry::served_peers_of(&bundle_js, &installed) else {
            continue;
        };
        for peer in served {
            peers.insert(peer.clone());
            if !resolved_packages.contains(&peer) {
                resolved_packages.push(peer.clone());
                runtime_packages.insert(peer.clone());
                static_packages.insert(peer.clone());
                pending.push(peer);
            }
        }
    }
    peers
}

/// [`thaw_js_package_eligible_in`] with no installed packages (every unmapped bare require is a non-blocker).
#[cfg(test)]
fn thaw_js_package_eligible(
    pkg: &ResolvedPackage,
    bundle_js: &str,
    user_source: &str,
    typed_targets: &std::collections::HashMap<(String, String), String>,
    segment: &str,
) -> bool {
    thaw_js_package_eligible_in(
        pkg,
        bundle_js,
        user_source,
        typed_targets,
        segment,
        &std::collections::HashSet::new(),
        &std::collections::HashSet::new(),
    )
}

/// Whether the thaw-js runtime can serve `pkg` for this program: its bundle passes the thaw-js
/// feature scan, the program touches no class of it (those are live JavaScript objects), and
/// every function the program mentions is a typed call whose declaration only carries plain data, or
/// a native FastPath function (which keeps its own declaration and has no typed target).
/// `segment` is the package's slice of the generated shim. `installed` is the registry's installed packages
/// ([`registry_installed_packages`]): a require of one the bundle does not map blocks the package, unless `served`
/// (the peers this build compiles in as thaw-js, see `ScanContext::served_peers`) names it.
fn thaw_js_package_eligible_in(
    pkg: &ResolvedPackage,
    bundle_js: &str,
    user_source: &str,
    typed_targets: &std::collections::HashMap<(String, String), String>,
    segment: &str,
    installed: &std::collections::HashSet<String>,
    served: &std::collections::HashSet<String>,
) -> bool {
    let mentions = |name: &str| thaw_js_mentions(user_source, name);
    let debug = std::env::var("THAW_JS_RUNTIME_DEBUG").as_deref() == Ok("1");
    let reject = |why: String| {
        record_thaw_js_blocker(&pkg.name, &why);
        if debug {
            eprintln!(
                "thaw-js routing: package `{}` stays on QuickJS: {why}",
                pkg.name
            );
        }
        false
    };
    // A native addon is hosted by thaw-napi's thaw-js engine; its bundle is scanned in addon mode below.
    if pkg.name.starts_with("node:") && !THAW_JS_BUILTIN_MODULES.contains(&pkg.name.as_str()) {
        return reject("builtin module not validated on thaw-js".into());
    }
    if let Some(why) = thaw_js_async_hooks_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    // A class the program names is served through thaw-js handles (`new`, instance and static
    // methods); every operation the shim declares for it must be plain data or an instance handle.
    // Anything else the program does with an instance that the typed operations do not cover needs
    // QuickJS, which sends the whole build back to the QuickJS path.
    for class in pkg
        .classes
        .iter()
        .filter(|class| !pkg.type_only_value_names.contains(&class.name) && mentions(&class.name))
    {
        for line in segment.lines() {
            if thaw_js_class_declaration(line, &pkg.name, &class.name)
                // a constructor's option object may carry callbacks (`new Writable({ write })`)
                && !thaw_js_declaration_plain_with(
                    line,
                    thaw_js_declaration_key(line).is_some_and(|key| key.starts_with("$new$")),
                )
                // a method operation is only emitted where the program calls it, and the thaw-js backend
                // fails the build (which retries on QuickJS) for a signature it cannot serve
                && thaw_js_class_member(line).is_none()
            {
                return reject(format!("class `{}`: {line}", class.name));
            }
        }
    }
    // R6 and addon mode: `thaw_registry::scan_bundle_for_build` (shared with `thaw-js-scan`)
    let scanned = thaw_registry::scan_bundle_for_build(
        bundle_js,
        pkg.native_addon.is_some(),
        installed,
        served,
    );
    match scanned {
        Ok(report) if report.is_clean() => {}
        Ok(report) => {
            let blockers = report
                .blockers
                .keys()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return reject(format!("bundle uses unsupported features: {blockers}"));
        }
        Err(error) => return reject(format!("bundle scan failed: {error}")),
    }
    pkg.functions
        .iter()
        .filter(|function| thaw_js_uses_function(user_source, &function.name))
        // A FastPath function is lowered natively and keeps its `generate_shim` declaration; it has no
        // typed target (`typed_targets` only records Fallback functions).
        .filter(|function| {
            !pkg.classifications.iter().any(|(name, class)| {
                name == &function.name && matches!(class, thaw_bridge::Classification::FastPath(_))
            })
        })
        .all(|function| {
            let Some(target) = typed_targets.get(&(pkg.name.clone(), function.name.clone())) else {
                return reject(format!("`{}` has no typed target", function.name));
            };
            // A class-instance result is a Json value carrying a thaw-js handle (adopted at the call).
            if target.starts_with("__thaw_typed_jit_") {
                return true;
            }
            let lines = segment
                .lines()
                .filter(|line| {
                    thaw_js_declared_function(line, &pkg.name).as_deref()
                        == Some(function.name.as_str())
                })
                .collect::<Vec<_>>();
            if lines.is_empty() {
                if debug {
                    for line in segment
                        .lines()
                        .filter(|line| line.contains(function.name.as_str()))
                        .take(4)
                    {
                        eprintln!("thaw-js routing:   shim: {}", &line[..line.len().min(300)]);
                    }
                    eprintln!(
                        "thaw-js routing:   target: {target}; segment has {} lines",
                        segment.lines().count()
                    );
                    let hex = target.rsplit('_').next().unwrap_or("");
                    for line in segment.lines().filter(|line| line.contains(hex)).take(4) {
                        eprintln!(
                            "thaw-js routing:   by-hex: {}",
                            &line[..line.len().min(300)]
                        );
                    }
                }
                return reject(format!("`{}` has no declaration", function.name));
            }
            // Overloads the program cannot call (an arity above its calls) are not checked: only a call
            // reaches a declaration, so one unused callback overload must not send the package to QuickJS.
            let max_call_arity = thaw_js_call_arities(user_source, &function.name);
            lines
                .into_iter()
                .filter(|line| thaw_js_overload_within_calls(line, max_call_arity))
                .all(|line| {
                thaw_js_declaration_plain_with(line, false)
                    || thaw_js_async_declaration_plain_with(line)
                    || reject(format!("`{}`: {line}", function.name))
            })
        })
}

#[cfg(test)]
#[test]
fn thaw_js_mentions_matches_whole_identifiers_only() {
    assert!(!thaw_js_mentions("bcrypt.hashSync(a)", "hash"));
    assert!(thaw_js_mentions("bcrypt.hash(a)", "hash"));
    assert!(thaw_js_mentions("import { $x } from 'p'", "$x"));
    assert!(thaw_js_mentions("ns.sub.hash(1)", "a.b.hash"));
}

#[cfg(test)]
#[test]
fn source_may_call_realm_fetch_is_a_whole_word_scan_of_every_source() {
    assert!(source_may_call_realm_fetch(&["fetch('http://x/')"]));
    assert!(source_may_call_realm_fetch(&["const r = await globalThis.fetch(u);"]));
    assert!(source_may_call_realm_fetch(&["window.fetch(u)"]));
    assert!(!source_may_call_realm_fetch(&["const fetchy = 1; refetch(x);"]));
    assert!(!source_may_call_realm_fetch(&["const x = 1;"]));
    assert!(!source_may_call_realm_fetch(&[]));
    // any module of the graph counts, not only the entry
    assert!(source_may_call_realm_fetch(&["export const x = 1;", "fetch(u);"]));
    // a shadowed local or a word in a comment over-approximates: an extra bundle, never a missing one
    assert!(source_may_call_realm_fetch(&["// fetch later"]));
}

#[cfg(test)]
#[test]
fn realm_fetch_bundles_are_forced_runtime_only_unless_the_program_imports_them() {
    let set = |names: &[&str]| {
        names
            .iter()
            .map(|n| n.to_string())
            .collect::<std::collections::HashSet<_>>()
    };
    // no import: both transport bundles are linked, and neither may contribute declarations
    let mut runtime = set(&["node:fs"]);
    let mut resolved = vec!["node:fs".to_string()];
    let mut forced = std::collections::HashSet::new();
    force_realm_fetch_bundles(&mut runtime, &mut resolved, &mut forced);
    assert_eq!(forced, set(&["node:http", "node:https"]));
    assert!(runtime.contains("node:http") && runtime.contains("node:https"));
    assert!(
        resolved.contains(&"node:http".to_string()) && resolved.contains(&"node:https".to_string())
    );
    let declared: Vec<&String> = resolved.iter().filter(|p| !forced.contains(*p)).collect();
    assert_eq!(declared, vec![&"node:fs".to_string()]);
    // an imported node:http keeps its declarations: only the bundle not already listed is forced
    let mut runtime = set(&["node:fs", "node:http"]);
    let mut resolved = vec!["node:fs".to_string(), "node:http".to_string()];
    let mut forced = std::collections::HashSet::new();
    force_realm_fetch_bundles(&mut runtime, &mut resolved, &mut forced);
    assert_eq!(forced, set(&["node:https"]));
    assert_eq!(resolved.iter().filter(|p| *p == "node:http").count(), 1);
}

#[cfg(test)]
#[test]
fn realm_fetch_transport_bundles_load_even_when_the_program_imports_node_http() {
    // a program that calls the realm fetch loads both transport bundles, imported or not
    let fetch = realm_fetch_transport_bundles(&["fetch('http://127.0.0.1/');"]);
    assert_eq!(
        fetch,
        ["node:http", "node:https"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    assert!(realm_fetch_transport_bundles(&["const x = 1;"]).is_empty());
    // an imported node:http keeps its declarations (not forced) yet still loads through the fetch set
    let mut runtime = ["node:fs", "node:http"]
        .map(String::from)
        .into_iter()
        .collect();
    let mut resolved = vec!["node:fs".to_string(), "node:http".to_string()];
    let mut forced = std::collections::HashSet::new();
    force_realm_fetch_bundles(&mut runtime, &mut resolved, &mut forced);
    assert!(!forced.contains("node:http"));
    assert!(fetch.contains("node:http"));
}

#[cfg(test)]
#[test]
fn thaw_js_async_hooks_create_hook_with_promises_keeps_the_module_on_quickjs() {
    let timers = "import { createHook } from 'node:async_hooks';\nsetTimeout(() => {}, 1);";
    let resource =
        "import { createHook, AsyncResource } from 'node:async_hooks';\nnew AsyncResource('x');";
    let promises = "import { createHook } from 'node:async_hooks';\nawait Promise.resolve();";
    let finally = "import { createHook } from 'node:async_hooks';\nnew Promise((r) => r(1)).finally(() => {});";
    let then = "import { createHook } from 'node:async_hooks';\nnew Promise((r) => r(1)).then((v) => v);";
    let all = "import { createHook } from 'node:async_hooks';\nPromise.all([1]);";
    let als = "import { createHook, AsyncLocalStorage } from 'node:async_hooks';\nnew AsyncLocalStorage();";
    let hooks = "import { createHook } from 'node:async_hooks';\ncreateHook({ init() {} }).enable();";
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", timers).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", resource).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", promises).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", finally).is_some());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", then).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", all).is_some());
    // AsyncLocalStorage is served by thaw-js now (its stores reach native awaits): not a blocker any more.
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", als).is_none());
    let als_import_only = "import { createHook, AsyncLocalStorage } from 'node:async_hooks';\ncreateHook({ init() {} }).enable();";
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", als_import_only).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", hooks).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", "new AsyncLocalStorage();").is_none());
    assert!(thaw_js_async_hooks_blocker("node:path", promises).is_none());
}

#[cfg(test)]
#[test]
fn thaw_js_function_use_ignores_imports_and_member_access() {
    let source = "import { marked } from \"marked\";\nconsole.log(marked.parse(\"x\"));\n";
    assert!(!thaw_js_uses_function(source, "marked"));
    assert!(thaw_js_uses_function(source, "parse"));
    assert!(thaw_js_uses_function("const s = marked(\"x\");", "marked"));
    assert!(!thaw_js_uses_function(
        "const s = markedAll(\"x\");",
        "marked"
    ));
}

#[cfg(test)]
#[test]
fn thaw_js_function_use_sees_code_sharing_a_line_with_an_import() {
    let one_line = "import * as http from 'node:http'; async function main(): Promise<void> { const srv = http.createServer(() => {}); srv.listen(0); await fetch('http://x/'); }";
    assert!(thaw_js_uses_function(one_line, "createServer"));
    let named = "import { createServer } from 'node:http'; createServer(() => {});";
    assert!(thaw_js_uses_function(named, "createServer"));
    let import_only = "import { createServer } from 'node:http';";
    assert!(!thaw_js_uses_function(import_only, "createServer"));
    let side_effect = "import 'node:http'; export * from './x'; createServer(() => {});";
    assert!(thaw_js_uses_function(side_effect, "createServer"));
    let export_const = "export const createServer = 1;";
    assert!(thaw_js_uses_function(export_const, "createServer"));
    let re_export = "export { createServer } from 'node:http'; createServer(() => {});";
    assert!(thaw_js_uses_function(re_export, "createServer"));
    let re_export_only = "export { createServer } from 'node:http';";
    assert!(!thaw_js_uses_function(re_export_only, "createServer"));
}

#[cfg(test)]
#[test]
fn thaw_js_class_member_names_the_method_of_a_class_operation() {
    // "$method$p::C$argument$overload0$arity3"
    let key = "$method$p::C$argument$overload0$arity3";
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    let line = format!("declare function __thaw_typed_js_{hex}(receiver: JsValue): JsValue;");
    assert_eq!(thaw_js_class_member(&line).as_deref(), Some("argument"));
    let key = "$new$p::C$arity0";
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    let line = format!("declare function __thaw_typed_js_{hex}(): JsValue;");
    assert_eq!(thaw_js_class_member(&line), None);
}

#[cfg(test)]
fn thaw_js_test_decl(key: &str, tail: &str) -> String {
    let hex: String = key.bytes().map(|b| format!("{b:02x}")).collect();
    format!("declare function __thaw_typed_js_{hex}{tail}")
}

#[cfg(test)]
#[test]
fn thaw_js_call_arities_are_the_largest_argument_count_of_the_program_calls() {
    // zod-shaped: the program calls `string()`; an overload taking `{ errorMap }` is never selected
    assert_eq!(thaw_js_call_arities("const s = z.string().min(1);", "string"), Some(0));
    assert_eq!(thaw_js_call_arities("const a: string = z.string();", "string"), Some(0));
    assert_eq!(thaw_js_call_arities("z.string({ errorMap: () => 'x' });", "string"), Some(1));
    // a `,` or `)` inside a string literal is not an argument separator
    assert_eq!(thaw_js_call_arities("f(\"a,b\"); g(string(\"x)\"));", "string"), Some(1));
    assert_eq!(thaw_js_call_arities("string(1, 2); string(3);", "string"), Some(2));
    // a bare value reference, a spread, or no call at all: every overload stays checked
    assert_eq!(thaw_js_call_arities("arr.map(string);", "string"), None);
    assert_eq!(thaw_js_call_arities("string(...xs);", "string"), None);
    assert_eq!(thaw_js_call_arities("const x = 1;", "string"), None);
    assert_eq!(thaw_js_call_arities("const s = string;", "string"), None);
    assert_eq!(thaw_js_call_arities("string(a / 2);", "string"), None);
}

#[cfg(test)]
#[test]
fn thaw_js_void_is_a_type_word_not_a_use_of_an_export_named_void() {
    // `: void` and `Promise<void>` are annotations, never a use of zod's `void` export
    assert!(!thaw_js_uses_function("async function main(): Promise<void> { await run(); }", "void"));
    assert!(!thaw_js_uses_function("const f = (): void => {};", "void"));
    assert!(!thaw_js_uses_function("let x: string | void = undefined;", "void"));
    // a member use is a use: `z.void()`
    assert!(thaw_js_uses_function("const v = z.void();", "void"));
    assert_eq!(thaw_js_call_arities("let x: void = 0; z.void();", "void"), Some(0));
    // a keyword type after an annotation is no call of the export named `string`
    assert!(!thaw_js_uses_function("const s: string = 'x';", "string"));
    assert!(thaw_js_uses_function("const s = string('x');", "string"));
}

#[cfg(test)]
#[test]
fn thaw_js_unused_overloads_above_the_program_calls_are_not_checked() {
    let key = "pkg::string";
    // the program only calls `string()`: the `{ errorMap }` overload (arity 1) is pruned, the nullary one stays
    let nullary = thaw_js_test_decl(key, "__arity_0(): JsValue;");
    let callback = thaw_js_test_decl(key, "__arity_1(params: {errorMap: (issue: Json) => string}): JsValue;");
    assert!(!thaw_js_declaration_plain_with(&callback, false));
    assert!(thaw_js_declaration_plain_with(&nullary, false));
    let calls = thaw_js_call_arities("const s = z.string();", "string");
    let kept: Vec<&str> = [nullary.as_str(), callback.as_str()]
        .into_iter()
        .filter(|line| thaw_js_overload_within_calls(line, calls))
        .collect();
    assert_eq!(kept, vec![nullary.as_str()]);
    // a bare reference keeps every overload checked: the callback overload is still rejected
    let calls = thaw_js_call_arities("arr.map(string);", "string");
    assert!(thaw_js_overload_within_calls(&callback, calls));
    // a call with two arguments reaches the arity-1 overload too
    assert!(thaw_js_overload_within_calls(&callback, thaw_js_call_arities("string(a, b);", "string")));
    // a line without an arity suffix always stays
    let plain = thaw_js_test_decl(key, "(): JsValue;");
    assert!(thaw_js_overload_within_calls(&plain, Some(0)));
}

#[cfg(test)]
#[test]
fn thaw_js_class_accessor_lines_are_class_members_like_methods() {
    // "$getter$p::C$size" and "$setter$p::C$size": the accessor is named like a method, so it passes the class check
    let getter = thaw_js_test_decl("$getter$p::C$size", "(receiver: JsValue): number;");
    let setter = thaw_js_test_decl("$setter$p::C$size", "(receiver: JsValue, value: number): number;");
    let static_getter = thaw_js_test_decl("$staticgetter$p::C$count", "(): number;");
    assert_eq!(thaw_js_class_member(&getter).as_deref(), Some("size"));
    assert_eq!(thaw_js_class_member(&setter).as_deref(), Some("size"));
    assert_eq!(thaw_js_class_member(&static_getter).as_deref(), Some("count"));
    // a constructor is not a member
    let ctor = thaw_js_test_decl("$new$p::C$arity0", "(): JsValue;");
    assert_eq!(thaw_js_class_member(&ctor), None);
}

#[cfg(test)]
#[test]
fn thaw_js_peer_pair_gets_one_group_key_and_an_unrelated_package_none() {
    // index 1 (`pg`) is required by index 0 (`drizzle-orm`); index 2 (`zod`) has no peer edge
    let deps = vec![vec![1], vec![], vec![]];
    let keys = thaw_js_peer_group_keys(&["drizzle-orm", "pg", "zod"], &deps);
    assert_eq!(keys, vec![Some("drizzle-orm".to_string()), Some("drizzle-orm".to_string()), None]);
}

#[test]
fn thaw_js_peer_group_key_is_the_least_member_name_whatever_the_order() {
    // the same pair listed the other way round, and a transitive chain a -> b -> c: one key each
    let pair = thaw_js_peer_group_keys(&["pg", "drizzle-orm"], &[vec![1], vec![]]);
    assert_eq!(pair, vec![Some("drizzle-orm".to_string()); 2]);
    let chain = thaw_js_peer_group_keys(&["c", "b", "a"], &[vec![1], vec![2], vec![]]);
    assert_eq!(chain, vec![Some("a".to_string()); 3]);
}

#[test]
fn thaw_js_group_marker_goes_after_the_entry_key_statement() {
    let js = "module.exports = (function(require){ var __thaw_bundle_factories = {\"pg/index.js\": function(module, exports, require) {}}; \
              var __thaw_bundle_require_maps = {\"pg/index.js\": {}}; var __thaw_bundle_entry_key = \"pg/index.js\";\n\
              var x = 1; })(globalThis.require);";
    let marked = thaw_js_with_group_marker(js, "drizzle-orm");
    assert!(
        marked.contains("var __thaw_bundle_entry_key = \"pg/index.js\";\nvar __thaw_bundle_group = \"drizzle-orm\";\nvar x = 1;"),
        "{marked}"
    );
    let parts = thaw_js::scan::parse_bundle(&marked).unwrap();
    assert_eq!(parts.group.as_deref(), Some("drizzle-orm"));
}

