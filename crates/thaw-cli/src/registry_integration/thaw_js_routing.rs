// Experimental thaw-js routing (`THAW_JS_RUNTIME=1`): which packages the Thaw-native JavaScript
// runtime can serve for a given program. Everything else keeps the QuickJS path unchanged.

/// Only a bare `1` turns thaw-js routing on.
fn thaw_js_routing_enabled() -> bool {
    std::env::var("THAW_JS_RUNTIME").as_deref() == Ok("1")
}

/// Whether the program text names `name` (the last segment of a dotted path). Conservative: a
/// substring match, so it can only over-report, which keeps a package on QuickJS.
fn thaw_js_mentions(user_source: &str, name: &str) -> bool {
    user_source.contains(name.rsplit('.').next().unwrap_or(name))
}

/// A thaw-js package returns plain data, so a bare alias' opaque `JsValue` result becomes `Json`.
fn thaw_js_retarget_alias(alias: String, thaw_js_package: bool) -> String {
    if thaw_js_package {
        alias.replace("JsValue", "Json")
    } else {
        alias
    }
}

/// Whether a generated `declare function __thaw_typed_js_*` line only mentions types that cross the
/// thaw-js JSON boundary: `number`, `string`, `boolean`, `Json`, `void`, `null`, `undefined`, its own
/// type parameters, unions/array/object-literal shapes of those, and an opaque `JsValue` (a type the
/// `.d.ts` classifier could not resolve), which the retargeted declaration turns into `Json`.
/// Callbacks and every other type name (classes, `Buffer`, ...) are rejected.
fn thaw_js_declaration_plain(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("declare function ") else {
        return false;
    };
    if rest.contains("=>") {
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
                    && (chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '$')
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
    identifiers_plain(params) && identifiers_plain(result)
}

/// The function name encoded in a `__thaw_typed_js_<hex>[__suffix...]` symbol.
fn thaw_js_declared_function(line: &str, package: &str) -> Option<String> {
    let symbol = line
        .strip_prefix("declare function __thaw_typed_js_")?
        .split(['(', '<'])
        .next()?;
    let hex = symbol.split('_').next()?;
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let key = String::from_utf8(bytes).ok()?;
    key.strip_prefix(&format!("{package}::")).map(str::to_string)
}

/// Whether the thaw-js runtime can serve `pkg` for this program: its bundle passes the thaw-js
/// feature scan, the program touches no class of it (those are live JavaScript objects), and
/// every function the program mentions is a typed call whose declaration only carries plain data.
/// `segment` is the package's slice of the generated shim.
fn thaw_js_package_eligible(
    pkg: &ResolvedPackage,
    bundle_js: &str,
    user_source: &str,
    typed_targets: &std::collections::HashMap<(String, String), String>,
    segment: &str,
) -> bool {
    let mentions = |name: &str| thaw_js_mentions(user_source, name);
    if pkg.native_addon.is_some()
        || pkg
            .classes
            .iter()
            .any(|class| !pkg.type_only_value_names.contains(&class.name) && mentions(&class.name))
        || !thaw_js::scan::scan_bundle(bundle_js).is_ok_and(|report| report.is_clean())
    {
        return false;
    }
    pkg.functions
        .iter()
        .filter(|function| mentions(&function.name))
        .all(|function| {
            let Some(target) = typed_targets.get(&(pkg.name.clone(), function.name.clone())) else {
                return false;
            };
            // A class-typed result is also an opaque `JsValue`; it must stay a live object.
            if let thaw_bridge::DtsType::Unsupported(reason) = &function.ret {
                if pkg.classes.iter().any(|class| reason.contains(class.name.as_str())) {
                    return false;
                }
            }
            if target.starts_with("__thaw_typed_jit_") {
                return true;
            }
            let lines = segment
                .lines()
                .filter(|line| {
                    thaw_js_declared_function(line, &pkg.name).as_deref() == Some(function.name.as_str())
                })
                .collect::<Vec<_>>();
            !lines.is_empty() && lines.into_iter().all(thaw_js_declaration_plain)
        })
}
