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

/// Starts a fresh blocker list for a build (a build's routing attempts all record into it).
fn clear_thaw_js_blockers() {
    THAW_JS_BLOCKERS.with(|blockers| blockers.borrow_mut().clear());
}

fn record_thaw_js_blocker(package: &str, why: &str) {
    THAW_JS_BLOCKERS.with(|blockers| {
        let entry = (package.to_string(), why.to_string());
        let mut blockers = blockers.borrow_mut();
        if !blockers.contains(&entry) {
            blockers.push(entry);
        }
    });
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

/// `THAW_JS_ROUTE_ALL=1` routes every program through thaw-js, not only packages and dynamic-code
/// programs (a diagnostic opt-in: a build that still needs QuickJS falls back and says why).
fn thaw_js_route_all_enabled() -> bool {
    thaw_js_route_all_from(std::env::var("THAW_JS_ROUTE_ALL").ok().as_deref())
}

/// Only `1` routes every program; unset or any other value keeps the routing rules as they are.
fn thaw_js_route_all_from(value: Option<&str>) -> bool {
    value == Some("1")
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

/// Whether the program uses the package's value export `name` as a value: any value but a class and
/// anything but a namespace object whose only uses are calls of the package's own functions
/// (`validator.isEmail(..)`, which the shim resolves to the typed functions) and its import. Such a
/// value is served to the program as a thaw-js handle (`$value$` class operation).
fn thaw_js_value_export_needs_handle(pkg: &ResolvedPackage, user_source: &str, name: &str) -> bool {
    if !thaw_js_mentions(user_source, name) || pkg.classes.iter().any(|class| class.name == name) {
        return false;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let mut rest = user_source;
    let mut offset = 0;
    while let Some(found) = rest.find(name) {
        let start = offset + found;
        let (before, after) = (&user_source[..start], &user_source[start + name.len()..]);
        offset = start + name.len();
        rest = after;
        if before.chars().next_back().is_some_and(is_word)
            || after.chars().next().is_some_and(is_word)
        {
            continue; // part of a longer identifier
        }
        let import_binding = before.trim_end().ends_with("import")
            || before.trim_end().ends_with("* as")
            || before.trim_end().ends_with(',')
                && before
                    .rfind("import")
                    .is_some_and(|i| !before[i..].contains(';'))
            || after.trim_start().starts_with("from ");
        if import_binding {
            continue;
        }
        let member = after.strip_prefix('.').map(|member| {
            let end = member.find(|c: char| !is_word(c)).unwrap_or(member.len());
            (&member[..end], member[end..].trim_start())
        });
        // (a builtin's other value exports -- `path.posix`, `path.win32` -- are objects of their own
        // whose members differ from the module's functions, so `posix.join(..)` needs the value)
        let namespace_like = !pkg.name.starts_with("node:") || pkg.name.ends_with(name);
        let called_function = member.is_some_and(|(member, tail)| {
            namespace_like
                && tail.starts_with('(')
                && pkg.functions.iter().any(|function| function.name == member)
        }) || (after.trim_start().starts_with('(')
            && pkg.functions.iter().any(|function| {
                function.name == name || function.name == name.rsplit('.').next().unwrap_or(name)
            }));
        if !called_function {
            return true;
        }
    }
    false
}

/// Whether the program names the nested-namespace class `path` (`kdf.OpenSSL`), which only the QuickJS
/// loader can capture at its dotted runtime path. A function or value member (`marked.parse`,
/// `chalk.red`) is served through its package's value export handle or typed function instead.
fn thaw_js_blocking_nested_alias(pkg: &ResolvedPackage, user_source: &str, path: &str) -> bool {
    user_source.contains(path) && pkg.classes.iter().any(|class| class.name == path)
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
    thaw_js_declaration_plain_with(line, true, false)
}

/// `thaw_js_declaration_plain`, with the bare `Uint8Array` result relaxation switched by
/// `bytes_result_ok` (see `thaw_js_bytes_result_allowed`). `callback_options`: a constructor's option
/// object may carry callback fields (`new Writable({ write })`), so its `=>` types are accepted.
/// Passed `true` only for `$new$` lines (the class-operation gate).
fn thaw_js_declaration_plain_with(line: &str, bytes_result_ok: bool, callback_options: bool) -> bool {
    let Some(rest) = line.strip_prefix("declare function ") else {
        return false;
    };
    // A directly declared callback type (`f: (x: number) => number`) stays on QuickJS. (The LLVM side
    // can already pass such a parameter to thaw-js, see `thaw_js_callback_type`; accepting it here is
    // a one-line change that flips `thaw_js_routing_falls_back_to_quickjs_when_a_callback_...`.)
    // A callback coerced into a `Json` parameter (lodash's `iteratee: Json`) is routed.
    if !callback_options && rest.contains("=>") {
        return false;
    }
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
    // A bare `Uint8Array` result (`readFileSync(path): Uint8Array`) crosses as bytes; in params,
    // unions and nested positions it stays rejected.
    let bytes_result =
        bytes_result_ok && result.trim().trim_end_matches(';').trim() == "Uint8Array";
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
    identifiers_plain(params) && (bytes_result || identifiers_plain(result))
}

/// An async overload (`...): Promise<string>;` or `string | Promise<string>`, ejs `render`): the typed
/// call runs the event loop and unwraps the settled promise, so it is plain when its resolved type is.
#[cfg(test)]
fn thaw_js_async_declaration_plain(line: &str) -> bool {
    thaw_js_async_declaration_plain_with(line, true)
}

fn thaw_js_async_declaration_plain_with(line: &str, bytes_result_ok: bool) -> bool {
    let Some(at) = line.rfind("): ") else {
        return false;
    };
    let (params, result) = line.split_at(at);
    // `Promise<Uint8Array>` resolves to the bare bytes result the sync gate accepts
    let resolved = result.replace("Promise<Uint8Array>", "Uint8Array");
    result.contains("Promise<")
        && thaw_js_declaration_plain_with(
            &format!("{params}{}", resolved.replace("Promise", "")),
            bytes_result_ok,
            false,
        )
}

/// Whether a program may get the bare `Uint8Array` result relaxation on thaw-js. A program that
/// touches `node:http`/`node:https`/`node:http2` or `node:tls` (substring match) keeps those results
/// on QuickJS: its thaw-js client path (ky over node:https) hangs with bytes results. Batch 2 fixes it.
fn thaw_js_bytes_result_allowed(user_source: &str) -> bool {
    !(user_source.contains("node:http") || user_source.contains("node:tls"))
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

/// The member (`$method$pkg::Class$member$overload0$arity3`, `$staticmethod$...`) a class operation line
/// calls; `None` for a constructor or any other operation.
fn thaw_js_class_member(line: &str) -> Option<String> {
    let key = thaw_js_declaration_key(line)?;
    let mut parts = key.strip_prefix('$')?.split('$');
    let kind = parts.next()?;
    if !kind.starts_with("method") && !kind.starts_with("staticmethod") {
        return None;
    }
    parts.nth(1).map(str::to_string)
}

/// Whether `name` (a function export) is used other than as `name.member` or in an import/re-export
/// statement: those uses go through the package's value export, not through a call of `name` itself.
fn thaw_js_uses_function(user_source: &str, name: &str) -> bool {
    let name = name.rsplit('.').next().unwrap_or(name);
    let code = thaw_js_without_module_statements(user_source);
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    code.match_indices(name).any(|(start, _)| {
        let end = start + name.len();
        !code[..start].chars().next_back().is_some_and(is_word)
            && !code[end..].chars().next().is_some_and(is_word)
            && !code[end..].starts_with('.')
    })
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
    "node:url",
    "node:fs",
    "node:timers/promises",
    "node:fs/promises",
    "node:vm",
    "node:async_hooks",
    "node:querystring",
    "node:timers",
    "node:worker_threads",
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
];

/// `node:perf_hooks` marks, measures, observers, timerify and entries exist only on QuickJS's performance.js.
fn thaw_js_perf_hooks_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    (pkg_name == "node:perf_hooks"
        && [
            "mark",
            "measure",
            "PerformanceObserver",
            "timerify",
            "getEntries",
            "PerformanceEntry",
            "clearMarks",
        ]
        .iter()
        .any(|name| thaw_js_mentions(user_source, name)))
    .then_some("performance marks/measures/observers are not implemented on thaw-js yet")
}

/// `node:test` mock timers exist only on QuickJS's runtime.js (`__thaw_test_mock_timers_*`), so a program
/// that uses `mock.timers` stays on QuickJS. `mock.fn` and the rest of node:test run on thaw-js.
fn thaw_js_test_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    (pkg_name == "node:test"
        && thaw_js_mentions(user_source, "mock")
        && thaw_js_mentions(user_source, "timers"))
    .then_some("node:test mock.timers exists only on QuickJS yet")
}

/// Native addons thaw-js cannot serve yet: `argon2` needs a live owner for its native exports, and
/// `@parcel/watcher` subscription events are never delivered on thaw-js. Both stay on QuickJS.
fn thaw_js_unready_addon_blocker(pkg_name: &str) -> Option<&'static str> {
    matches!(pkg_name, "argon2" | "@parcel/watcher")
        .then_some("native addon export live-owner / subscription events are not supported on thaw-js yet")
}

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

/// The thaw-js form of `node:worker_threads`: the main-thread module the thaw-js prelude exports
/// (`web/worker_threads.js`). Its QuickJS form needs `node:stream`, which thaw-js now loads too.
const THAW_JS_WORKER_THREADS_FORM: &str =
    "module.exports = globalThis.__thaw_worker_threads_exports;\n";

/// `node:worker_threads` names thaw-js does not serve yet (`Worker` runs on thaw-js; its options and the rest of the
/// module do not). A program that names one keeps the module on QuickJS.
const THAW_JS_WORKER_THREADS_UNSUPPORTED: &[&str] = &[
    "postMessageToThread",
    "SHARE_ENV",
    "moveMessagePortToContext",
    "markAsUntransferable",
    "markAsUncloneable",
    "isMarkedAsUntransferable",
    "resourceLimits",
    "SharedArrayBuffer",
    "Atomics",
    "transferList",
];

/// See `THAW_JS_WORKER_THREADS_UNSUPPORTED`: worker threads stay on QuickJS until thaw-js can run them.
/// `process.stdin` is served only inside a worker: a program that mentions it without constructing a Worker (its
/// main thread has no stdin on thaw-js) stays on QuickJS; one that starts a Worker routes, its eval'd source reads stdin.
fn thaw_js_worker_threads_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    let main_thread_stdin = thaw_js_mentions(user_source, "stdin")
        && !(user_source.contains("new Worker") || user_source.contains("Worker("));
    (pkg_name == "node:worker_threads"
        && (main_thread_stdin
            || THAW_JS_WORKER_THREADS_UNSUPPORTED
                .iter()
                .any(|name| thaw_js_mentions(user_source, name))))
    .then_some("worker threads options (resourceLimits, SharedArrayBuffer, ...) and main-thread process.stdin are not supported on thaw-js yet")
}

/// `node:path/posix` / `node:path/win32` as a registry bundle: the `node:path` bundle plus one factory
/// that exports the selected flavour of the root (the QuickJS form is a script wrapper thaw-js cannot scan).
fn thaw_js_path_bundle(root_bundle: &str, builtin: &str) -> String {
    let Some(flavour) = builtin.strip_prefix("node:path/") else {
        return root_bundle.to_string();
    };
    root_bundle
        .replacen(
            "var __thaw_bundle_factories = {\n",
            &format!(
                "var __thaw_bundle_factories = {{\n\"{builtin}\": function(module, exports, require) {{ module.exports = require(\"node:path\").{flavour}; }},\n"
            ),
            1,
        )
        .replacen(
            "var __thaw_bundle_require_maps = {\n",
            &format!(
                "var __thaw_bundle_require_maps = {{\n\"{builtin}\": {{\"node:path\": \"node:path\"}},\n"
            ),
            1,
        )
        .replacen(
            "var __thaw_bundle_entry_key = \"node:path\";",
            &format!("var __thaw_bundle_entry_key = \"{builtin}\";"),
            1,
        )
}

/// A `createHook` program whose timers or promises would become async resources: thaw-js does not report
/// those resources to hooks yet, so the module stays on QuickJS. A plain `createHook` (no timer or promise
/// words) keeps `node:async_hooks` on thaw-js.
fn thaw_js_async_hooks_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    const RESOURCES: &[&str] = &[
        "setTimeout", "setInterval", "setImmediate", "Promise", "then", "await", "async", "nextTick", "queueMicrotask",
    ];
    (pkg_name == "node:async_hooks"
        && thaw_js_mentions(user_source, "createHook")
        && RESOURCES.iter().any(|name| thaw_js_mentions(user_source, name)))
    .then_some("createHook with timers/promises resources is not supported on thaw-js yet")
}

/// `node:child_process` delivers stdout/exit on the thaw-js loop, which node:http's client has not been
/// shown to share reliably under CI load. Remove once the 3 http+spawn cases pass in CI.
fn thaw_js_child_process_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    (pkg_name == "node:child_process" && thaw_js_mentions(user_source, "node:http"))
        .then_some("node:child_process stays on QuickJS while node:http is in the program")
}

/// A native `createServer` next to the global `fetch` client: the fetch request never completes while the
/// native server runs, so such a program keeps `node:http` on QuickJS. (`get` / `request` run on thaw-js
/// next to a native server, see the Q9 Rust-closure this-entry fallback.)
fn thaw_js_native_server_with_http_client_blocker(pkg_name: &str, user_source: &str) -> Option<&'static str> {
    (pkg_name == "node:http"
        && thaw_js_uses_function(user_source, "createServer")
        && thaw_js_mentions(user_source, "fetch"))
        .then_some("native createServer together with global fetch is not supported on thaw-js yet")
}

/// Whether the thaw-js runtime can serve `pkg` for this program: its bundle passes the thaw-js
/// feature scan, the program touches no class of it (those are live JavaScript objects), and
/// every function the program mentions is a typed call whose declaration only carries plain data, or
/// a native FastPath function (which keeps its own declaration and has no typed target).
/// `segment` is the package's slice of the generated shim.
fn thaw_js_package_eligible(
    pkg: &ResolvedPackage,
    bundle_js: &str,
    user_source: &str,
    typed_targets: &std::collections::HashMap<(String, String), String>,
    segment: &str,
) -> bool {
    let mentions = |name: &str| thaw_js_mentions(user_source, name);
    let bytes_result_ok = thaw_js_bytes_result_allowed(user_source);
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
    if let Some(why) = thaw_js_unready_addon_blocker(&pkg.name) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_async_hooks_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_perf_hooks_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_test_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_worker_threads_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_child_process_blocker(&pkg.name, user_source) {
        return reject(why.into());
    }
    if let Some(why) = thaw_js_native_server_with_http_client_blocker(&pkg.name, user_source) {
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
                    bytes_result_ok,
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
    let scanned = if pkg.native_addon.is_some() {
        thaw_js::scan::scan_addon_bundle(bundle_js)
    } else {
        thaw_js::scan::scan_bundle(bundle_js)
    };
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
            lines.into_iter().all(|line| {
                thaw_js_declaration_plain_with(line, bytes_result_ok, false)
                    || thaw_js_async_declaration_plain_with(line, bytes_result_ok)
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
fn thaw_js_async_hooks_create_hook_with_timers_or_promises_keeps_the_module_on_quickjs() {
    let timers = "import { createHook } from 'node:async_hooks';\nsetTimeout(() => {}, 1);";
    let promises = "import { createHook } from 'node:async_hooks';\nawait Promise.resolve();";
    let hooks = "import { createHook } from 'node:async_hooks';\ncreateHook({ init() {} }).enable();";
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", timers).is_some());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", promises).is_some());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", hooks).is_none());
    assert!(thaw_js_async_hooks_blocker("node:async_hooks", "new AsyncLocalStorage();").is_none());
    assert!(thaw_js_async_hooks_blocker("node:path", timers).is_none());
}

#[cfg(test)]
#[test]
fn thaw_js_child_process_blocker_keeps_child_process_on_quickjs_next_to_http() {
    let http = "import { get } from 'node:http';";
    let spawn = "import { spawn } from 'node:child_process';";
    assert!(thaw_js_child_process_blocker("node:child_process", http).is_some());
    assert!(thaw_js_child_process_blocker("node:child_process", spawn).is_none());
    assert!(thaw_js_child_process_blocker("node:http", http).is_none());
}

#[cfg(test)]
#[test]
fn thaw_js_native_server_with_http_client_keeps_node_http_on_quickjs() {
    let combo = "import * as http from 'node:http';\nconst s = http.createServer(() => {});\nhttp.get('http://x/', () => {});";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", combo).is_none());
    let request = "import * as http from 'node:http';\nhttp.createServer(() => {});\nhttp.request('http://x/', () => {});";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", request).is_none());
    let server_only = "import * as http from 'node:http';\nhttp.createServer(() => {}).listen(0);";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", server_only).is_none());
    let client_only = "import * as http from 'node:http';\nhttp.get('http://x/', () => {});";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", client_only).is_none());
    let fetch = "import * as http from 'node:http';\nhttp.createServer(() => {}).listen(0);\nawait fetch('http://x/');";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", fetch).is_some());
    let fetch_only = "import * as http from 'node:http';\nawait fetch('http://x/');";
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", fetch_only).is_none());
    assert!(thaw_js_native_server_with_http_client_blocker("node:path", combo).is_none());
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
    assert!(thaw_js_native_server_with_http_client_blocker("node:http", one_line).is_some());
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
#[test]
fn thaw_js_test_blocker_keeps_mock_timers_on_quickjs() {
    assert!(thaw_js_test_blocker("node:test", "const { mock } = require('node:test'); mock.timers.enable();").is_some());
    assert!(thaw_js_test_blocker("node:test", "t.mock.timers.tick(10);").is_some());
    assert!(thaw_js_test_blocker("node:test", "const { mock } = require('node:test'); mock.fn();").is_none());
    assert!(thaw_js_test_blocker("node:test", "const timers = require('node:timers');").is_none());
    assert!(thaw_js_test_blocker("node:path", "mock.timers.enable();").is_none());
}

#[cfg(test)]
#[test]
fn thaw_js_perf_hooks_blocker_keeps_marks_on_quickjs() {
    let marks = "import { performance } from 'node:perf_hooks'; performance.mark('a');";
    assert!(thaw_js_perf_hooks_blocker("node:perf_hooks", marks).is_some());
    assert!(thaw_js_perf_hooks_blocker("node:perf_hooks", "new PerformanceObserver(cb);").is_some());
    assert!(thaw_js_perf_hooks_blocker("node:perf_hooks", "monitorEventLoopDelay(); performance.now();").is_none());
    assert!(thaw_js_perf_hooks_blocker("node:perf_hooks", "performance.eventLoopUtilization();").is_none());
    assert!(thaw_js_perf_hooks_blocker("node:perf_hooks", "const marked = 1;").is_none());
    assert!(thaw_js_perf_hooks_blocker("node:path", marks).is_none());
}
