// Experimental thaw-js routing (`THAW_JS_RUNTIME=1`): which packages the Thaw-native JavaScript
// runtime can serve for a given program. Everything else keeps the QuickJS path unchanged.

/// Only a bare `1` turns thaw-js routing on.
fn thaw_js_routing_enabled() -> bool {
    std::env::var("THAW_JS_RUNTIME").as_deref() == Ok("1")
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
        let called_function = member.is_some_and(|(member, tail)| {
            tail.starts_with('(') && pkg.functions.iter().any(|function| function.name == member)
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

/// Whether a generated `declare function __thaw_typed_js_*` line only mentions types that cross the
/// thaw-js JSON boundary: `number`, `string`, `boolean`, `Json`, `void`, `null`, `undefined`, its own
/// type parameters, unions/array/object-literal shapes of those, and an opaque `JsValue` (a type the
/// `.d.ts` classifier could not resolve), which the retargeted declaration turns into `Json`.
/// Callbacks and every other type name (classes, `Buffer`, ...) are rejected.
fn thaw_js_declaration_plain(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("declare function ") else {
        return false;
    };
    // A directly declared callback type (`f: (x: number) => number`) stays on QuickJS. (The LLVM side
    // can already pass such a parameter to thaw-js, see `thaw_js_callback_type`; accepting it here is
    // a one-line change that flips `thaw_js_routing_falls_back_to_quickjs_when_a_callback_...`.)
    // A callback coerced into a `Json` parameter (lodash's `iteratee: Json`) is routed.
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
    identifiers_plain(params) && identifiers_plain(result)
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

/// The member (`$method$pkg::Class$member$overload0$arity3`, `$staticmethod$...`) a class operation line
/// calls; `None` for a constructor or any other operation.
fn thaw_js_class_member(line: &str) -> Option<String> {
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
    let key = String::from_utf8(bytes).ok()?;
    let mut parts = key.strip_prefix('$')?.split('$');
    let kind = parts.next()?;
    if !kind.starts_with("method") && !kind.starts_with("staticmethod") {
        return None;
    }
    parts.nth(1).map(str::to_string)
}

/// Whether `name` (a function export) is used other than as `name.member` or in an `import` line: those
/// uses go through the package's value export, not through a call of `name` itself.
fn thaw_js_uses_function(user_source: &str, name: &str) -> bool {
    let name = name.rsplit('.').next().unwrap_or(name);
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    user_source.match_indices(name).any(|(start, _)| {
        let end = start + name.len();
        let line_start = user_source[..start].rfind('\n').map_or(0, |i| i + 1);
        !user_source[..start]
            .chars()
            .next_back()
            .is_some_and(is_word)
            && !user_source[end..].chars().next().is_some_and(is_word)
            && !user_source[line_start..].trim_start().starts_with("import")
            && !user_source[end..].starts_with('.')
    })
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
    let debug = std::env::var("THAW_JS_RUNTIME_DEBUG").as_deref() == Ok("1");
    let reject = |why: String| {
        if debug {
            eprintln!(
                "thaw-js routing: package `{}` stays on QuickJS: {why}",
                pkg.name
            );
        }
        false
    };
    if pkg.native_addon.is_some() {
        return reject("native addon".into());
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
                && !thaw_js_declaration_plain(line)
                // a method operation is only emitted where the program calls it, and the thaw-js backend
                // fails the build (which retries on QuickJS) for a signature it cannot serve
                && thaw_js_class_member(line).is_none()
            {
                return reject(format!("class `{}`: {line}", class.name));
            }
        }
    }
    match thaw_js::scan::scan_bundle(bundle_js) {
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
        .all(|function| {
            let Some(target) = typed_targets.get(&(pkg.name.clone(), function.name.clone())) else {
                return reject(format!("`{}` has no typed target", function.name));
            };
            // A class-typed result is also an opaque `JsValue`; it must stay a live object.
            if let thaw_bridge::DtsType::Unsupported(reason) = &function.ret {
                if pkg
                    .classes
                    .iter()
                    .any(|class| reason.contains(class.name.as_str()))
                {
                    return reject(format!("`{}` returns a class instance", function.name));
                }
            }
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
                thaw_js_declaration_plain(line) || reject(format!("`{}`: {line}", function.name))
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
