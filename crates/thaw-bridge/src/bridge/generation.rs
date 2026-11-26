/// `native_lib_available` is false: a real npm package fetched via
/// `thaw registry add` is pure JS with no native counterpart at all, so a
/// type shape that happens to be fully representable in a native ABI
/// (e.g. date-fns's `daysToWeeks(days: number): number`) still has
/// nothing to link against. Treating it as FastPath anyway produces a
/// confusing `undefined reference` linker error for a function that has
/// a perfectly good JS implementation sitting right next to it in
/// `bundle.js`.
///
/// This is the single source of truth for "is this name actually
/// callable via FFI" -- `generate_shim` and any caller that separately
/// needs to know which names require Fallback binding (e.g. thaw-cli's
/// `ModuleBundle::fallback_names`) both call this rather than
/// `classify_all` directly, so they can't disagree.
/// Whether `name` is one of the handful of identifiers thaw-hir gives a
/// special, *global* meaning to when it's read bare as a value
/// (`undefined`, `NaN`, `Infinity`) -- see `lower_expr`'s own
/// `Expr::Ident` arm, which treats a bare reference to one of these
/// names as the corresponding literal *unless* the compiled program
/// happens to declare a real top-level signature under that exact name
/// (a defensive check meant for a user shadowing one of these on
/// purpose). A Fallback package that genuinely exports something under
/// one of these names (real example: zod's own `z.undefined()`) is
/// still always reachable through its properly-typed, mangled symbol --
/// a plain import, or a namespace/package-qualified alias -- but a
/// *bare*, unqualified top-level declaration literally named
/// `undefined` would shadow thaw-hir's own special-casing globally, for
/// every `!= undefined`/`=== undefined` comparison anywhere in the
/// *entire* compiled program, not just calls to this one function --
/// including inside thaw's own generated arity-dispatch wrapper's own
/// `param != undefined` optional-parameter guard, which has nothing to
/// do with this function at all. So the bare (non-qualified) form is
/// skipped for exactly these names; the package-qualified alias
/// (`pkg_undefined`) is unaffected, since it can never collide with a
/// bare literal reference.
pub fn shadows_a_thaw_literal_identifier(name: &str) -> bool {
    // The timer names are compiler-intercepted globals: a bare `setImmediate` function from
    // `node:timers/promises` would capture every callback-style timer call in the program.
    matches!(
        name,
        "undefined"
            | "NaN"
            | "Infinity"
            | "setTimeout"
            | "setInterval"
            | "setImmediate"
            | "clearTimeout"
            | "clearInterval"
            | "clearImmediate"
    )
}

/// A real JS/TS reserved word: syntactically invalid as a bare
/// identifier no matter what, unlike `shadows_a_thaw_literal_identifier`'s
/// names above (those parse fine as an identifier; they're skipped for a
/// semantic reason instead). A real npm package can still export a
/// property *named* one of these (`obj.in`, `obj.default` are ordinary,
/// valid member accesses -- only a standalone identifier position is
/// restricted), so this only needs to gate a *bare* top-level
/// declaration/reference thaw-cli would otherwise emit under the literal
/// name -- the properly-typed, mangled symbol and the package-qualified
/// alias (`pkg_name`, e.g. `joi_in`) remain unaffected either way. Found
/// via joi's `Root.in(ref, options?): Reference` (`Joi.in(...)`, a real,
/// documented part of its API): thaw-cli's own generated `function
/// in(ref: string, options?: Json): JsValue { ... }` bare-alias
/// declaration failed to parse as thaw-hir source at all -- `in` is
/// reserved there for exactly the same reason it is in plain JS/TS
/// (`for...in`, the `in` operator).
pub fn is_reserved_js_identifier(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "implements"
            | "import"
            | "in"
            | "instanceof"
            | "interface"
            | "let"
            | "new"
            | "null"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

pub fn effective_classifications(
    functions: &[DtsFunction],
    native_lib_available: bool,
) -> Vec<(String, Classification)> {
    classify_all(functions)
        .into_iter()
        .map(|(name, classification)| match classification {
            Classification::FastPath(_) if name.contains('.') => (
                name.clone(),
                Classification::Fallback {
                    function: name,
                    reason: "namespace members use their qualified JavaScript runtime path".to_string(),
                },
            ),
            Classification::FastPath(_) if !native_lib_available => (
                name.clone(),
                Classification::Fallback {
                    function: name,
                    reason: "no linked native library backs this Fast path signature".to_string(),
                },
            ),
            other => (name, other),
        })
        .collect()
}

/// `native_lib_available` says whether there's actually a linkable
/// native library backing this `.d.ts`'s FastPath signatures -- see
/// `effective_classifications`. thaw-cli passes `true` only for the
/// manual `--bridge` path, where the user is already responsible for
/// supplying a matching `--link`ed library themselves (that contract
/// predates this flag and is unchanged); for a registry (`--use`)
/// package it passes whether `native.a` actually exists.
/// A Fallback name thaw-cli also wants reachable through a package-
/// qualified Thaw-callable identifier, so a user can write `qs.parse(x)`
/// (rewritten by thaw-cli to call `alias`, since Thaw has no real
/// object/member-call support backing this -- it's pure source-level
/// syntax sugar) regardless of whether `parse` happens to collide with
/// another `--use`d package's own name. `qualified_key` is the JS-global
/// lookup key `callDynamic` uses at runtime, which must match the same
/// key `ModuleBundle::qualified_aliases` captured right after this
/// package's `loadScript` (`generate_module_init`) -- these always
/// travel together as a pair, generated by the same thaw-cli pass.
///
/// `suppress_bare` is set when `name` collides with another `--use`d
/// package's own declared name (thaw-cli detects this -- no single
/// `.d.ts`/package knows about any other package on its own): the bare
/// name is then *not* emitted at all, forcing qualified syntax, since
/// leaving it in would silently resolve to whichever colliding package
/// loaded last. When `false` (the common, non-colliding case), both the
/// bare name and the qualified alias are emitted, so a name stays
/// callable either way.
pub struct QualifiedFallback {
    pub name: String,
    pub alias: String,
    pub qualified_key: String,
    pub suppress_bare: bool,
}

/// `qualified` names (see `QualifiedFallback`) additionally get a
/// package-qualified alias function (and, when `suppress_bare` is set,
/// have their bare name dropped entirely -- see that field's doc
/// comment). A name not in `qualified` is emitted exactly as before.
/// Generates a Thaw-compilable TypeScript "shim" for a `.d.ts`'s functions,
/// per their classification (docs/design/bridge.md section 6/7): a
/// `FastPath` function becomes a plain ambient `declare function` (so
/// calling it compiles to a direct FFI call, resolved at link time --
/// still requires `thaw build --link <path>` today, until thaw-registry
/// can fetch/build that library automatically); a `Fallback` function
/// becomes a thin wrapper wired to the thaw-js path (`callDynamic`).
///
/// This only generates the *callable surface* -- for `Fallback` functions,
/// something still has to `loadScript(...)` the package's actual JS source
/// before the wrapper is called (not generated here: there's no
/// module-level initialization mechanism in Thaw yet to hook that up
/// automatically, so it stays the caller's explicit responsibility, e.g.
/// as the first statement in `main`/`handler`).
/// `classify_all`, then downgrades every FastPath entry to Fallback when
pub fn generate_shim(
    functions: &[DtsFunction],
    native_lib_available: bool,
    qualified: &[QualifiedFallback],
    typed_bare_aliases: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::new();
    for (name, classification) in effective_classifications(functions, native_lib_available) {
        match classification {
            Classification::FastPath(sig) => {
                // `classify_all` only returns `FastPath` for a name with
                // exactly one signature, so this lookup is unambiguous.
                let func = functions
                    .iter()
                    .find(|f| f.name == name)
                    .expect("FastPath classification implies a matching DtsFunction exists");
                let mut params = func
                    .params
                    .iter()
                    .zip(&sig.params)
                    .map(|((name, _), ty)| format!("{name}: {}", render_ts_type(ty)))
                    .collect::<Vec<_>>();
                if let (Some((name, _)), Some(variadic)) = (&func.rest_param, &sig.variadic) {
                    params.push(format!("...{name}: {}", render_ts_array_of(variadic)));
                }
                let params = params.join(", ");
                out.push_str(&format!(
                    "declare function {}({params}): {};\n",
                    sig.symbol,
                    render_ts_type(&sig.ret)
                ));
            }
            Classification::Fallback { function, reason } => {
                // thaw-cli's own `typed_dynamic_bare_alias` already
                // generated a properly-typed forwarding wrapper under
                // both this name's package-qualified alias and its bare
                // name (whichever of those isn't itself suppressed by a
                // cross-package collision) -- generating the untyped
                // `(argsArray: Json): Json` shape here too would just be
                // a second, unreachable-in-practice-but-still-a-
                // duplicate-declaration-error declaration under the same
                // name(s).
                if typed_bare_aliases.contains(&function) {
                    continue;
                }
                let returns_callable = functions
                    .iter()
                    .filter(|candidate| candidate.name == function)
                    .all(|candidate| {
                        matches!(
                            candidate.ret,
                            DtsType::Native(
                                HirType::Function(_, _)
                                    | HirType::CallableFunction(..)
                                    | HirType::JsValue
                            )
                        )
                    });
                let qualified_entry = qualified.iter().find(|q| q.name == function);
                if let Some(q) = qualified_entry {
                    out.push_str(&format!(
                        "// Fallback (thaw-js), package-qualified alias: {reason}\n"
                    ));
                    if returns_callable {
                        out.push_str(&format!(
                            "function {}(argsArray: Json): JsValue {{\n",
                            q.alias
                        ));
                        out.push_str(&format!(
                            "    const callable: JsValue = getDynamicValue(\"{}\");\n    return callDynamicValueHandle(callable, argsArray);\n",
                            q.qualified_key
                        ));
                    } else {
                        out.push_str(&format!("function {}(argsArray: Json): Json {{\n", q.alias));
                        out.push_str(&format!(
                            "    return callDynamic(\"{}\", argsArray);\n",
                            q.qualified_key
                        ));
                    }
                    out.push_str("}\n");
                }
                if function.contains('.') || qualified_entry.is_some_and(|q| q.suppress_bare) {
                    continue;
                }
                if shadows_a_thaw_literal_identifier(&function) || is_reserved_js_identifier(&function) {
                    continue;
                }
                out.push_str(&format!("// Fallback (thaw-js): {reason}\n"));
                // `argsArray` (not `args`): `callDynamic` expects a JSON
                // *array* of positional arguments, e.g.
                // `identity(JSON.parse("[42]"))`, not `identity(JSON.parse("42"))` --
                // named to make that convention hard to miss at the call site.
                if returns_callable {
                    out.push_str(&format!(
                        "function {function}(argsArray: Json): JsValue {{\n"
                    ));
                    out.push_str(&format!(
                        "    const callable: JsValue = getDynamicValue(\"{function}\");\n    return callDynamicValueHandle(callable, argsArray);\n"
                    ));
                } else {
                    out.push_str(&format!("function {function}(argsArray: Json): Json {{\n"));
                    out.push_str(&format!(
                        "    return callDynamic(\"{function}\", argsArray);\n"
                    ));
                }
                out.push_str("}\n");
            }
        }
    }
    out
}

/// Generates the same JSON-shaped fallback wrappers as `generate_shim`, but
/// targets the synchronous N-API host instead of thaw-js.
pub fn generate_native_addon_shim(
    functions: &[DtsFunction],
    qualified: &[QualifiedFallback],
    typed_bare_aliases: &std::collections::HashSet<String>,
) -> String {
    let mut out = String::new();
    for (function, _) in effective_classifications(functions, false) {
        // See the matching check (and its own doc comment) in
        // `generate_shim` -- thaw-cli's own `typed_dynamic_bare_alias`
        // already generated a properly-typed wrapper under both this
        // name's forms for a native-addon Fallback function too (they
        // share the same declaration/symbol machinery, `napi: bool`
        // just picks the encoded symbol's prefix), so emitting the
        // untyped shape here as well would be a duplicate declaration
        // under the same name(s), not just a harmless dead one.
        if typed_bare_aliases.contains(&function) {
            continue;
        }
        if let Some(qualified) = qualified.iter().find(|entry| entry.name == function) {
            out.push_str("// Fallback (N-API), package-qualified alias\n");
            out.push_str(&format!(
                "function {}(argsArray: Json): Json {{\n",
                qualified.alias
            ));
            out.push_str(&format!(
                "    return callNativeAddon(\"{function}\", argsArray);\n"
            ));
            out.push_str("}\n");
            if function.contains('.') || qualified.suppress_bare {
                continue;
            }
        }
        if function.contains('.') || shadows_a_thaw_literal_identifier(&function) || is_reserved_js_identifier(&function) {
            continue;
        }
        out.push_str("// Fallback (N-API)\n");
        out.push_str(&format!("function {function}(argsArray: Json): Json {{\n"));
        out.push_str(&format!(
            "    return callNativeAddon(\"{function}\", argsArray);\n"
        ));
        out.push_str("}\n");
    }
    out
}

pub struct NativeAddon<'a> {
    pub package_name: &'a str,
    pub bytes: &'a [u8],
    pub dependencies: Vec<&'a [u8]>,
    pub root_export: Option<&'a str>,
}

pub fn generate_native_addon_init(addons: &[NativeAddon<'_>]) -> String {
    if addons.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_native_module_init(): void {\n");
    for addon in addons {
        out.push_str(&format!("    // {}\n", addon.package_name));
        for dependency in &addon.dependencies {
            let hex = encode_embedded_native(dependency);
            out.push_str(&format!(
                "    if (!loadNativeSharedLibraryEmbedded(\"{hex}\")) throw new Error(\"failed to load native dependency\");\n"
            ));
        }
        let hex = encode_embedded_native(addon.bytes);
        let root_export = escape_ts_string_literal(addon.root_export.unwrap_or(""));
        out.push_str(&format!(
            "    if (!loadNativeAddonEmbedded(\"{hex}\", \"{root_export}\", \"{}\")) throw new Error(\"failed to load native addon\");\n",
            escape_ts_string_literal(addon.package_name)
        ));
    }
    out.push_str("}\n");
    out
}

pub struct NativeAddonPath<'a> {
    pub package_name: &'a str,
    pub path: &'a str,
    pub dependencies: Vec<&'a str>,
    pub root_export: Option<&'a str>,
}

pub fn generate_native_addon_path_init(addons: &[NativeAddonPath<'_>]) -> String {
    if addons.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_native_module_init(): void {\n");
    for addon in addons {
        out.push_str(&format!("    // {}\n", addon.package_name));
        for dependency in &addon.dependencies {
            out.push_str(&format!(
                "    if (!loadNativeSharedLibrary(\"{}\")) throw new Error(\"failed to load native dependency\");\n",
                escape_ts_string_literal(dependency)
            ));
        }
        out.push_str(&format!(
            "    if (!loadNativeAddon(\"{}\", \"{}\", \"{}\")) throw new Error(\"failed to load native addon\");\n",
            escape_ts_string_literal(addon.path),
            escape_ts_string_literal(addon.root_export.unwrap_or("")),
            escape_ts_string_literal(addon.package_name)
        ));
    }
    out.push_str("}\n");
    out
}

/// Escapes JS source for embedding as a double-quoted TS string literal
/// (backslash, `"`, and newlines/carriage-returns -- the characters that
/// would otherwise terminate or corrupt the literal). `generate_module_init`
/// is the only caller; a package's real `bundle.js` can contain any of
/// these.
fn escape_ts_string_literal(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for ch in source.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

/// One registry package's bundled JS to auto-load, for `generate_module_init`.
pub struct ModuleBundle<'a> {
    pub package_name: &'a str,
    pub js_source: &'a str,
}

/// Generates the `__thaw_module_init` function that loads each registry
/// package's bundled JS source via `loadScript`, once, before user code
/// runs (thaw-llvm's `MODULE_INIT_SYMBOL`/`call_module_init_if_present`
/// call this automatically from both entry points). This is what makes a
/// registry package's Fallback functions (see `generate_shim`) callable
/// without the *user* having to call `loadScript` themselves -- they still
/// need to `--use` the package (thaw-cli), but not hand-write the load.
/// Each bundle's JS is passed through `wrap_as_commonjs_module` first, so
/// a real npm package's actual CommonJS/UMD source works unmodified.
///
/// `bundles` is in the order packages should load (later packages can
/// rely on earlier ones already being loaded into the shared thaw-js
/// global scope). Returns an empty string -- no function emitted at all
/// -- when `bundles` is empty, since there would be nothing for it to do.
/// Optional host features required by uncompressed source. Keep this metadata
/// separate from generated scripts so compression cannot hide a requirement.
pub fn required_runtime_features(source: &str) -> std::collections::BTreeSet<&'static str> {
    [
        ("tls", &["__thaw_tls_"][..]),
        ("wasm", &["WebAssembly", "node:wasi", "require('wasi')", "require(\"wasi\")"][..]),
        ("brotli", &["brotli", "Brotli"][..]),
        ("intl", &["Intl.Locale", "Intl.PluralRules", "Intl.Collator", "Intl.Segmenter",
            "Intl.RelativeTimeFormat", "Intl.NumberFormat", "Intl.DateTimeFormat", "Intl.ListFormat",
            "Intl.DisplayNames", "Intl"][..]),
        ("sqlite", &["node:sqlite", "__thaw_sqlite_"][..]),
        ("zstd", &["zstd", "Zstd"][..]),
    ].into_iter().filter_map(|(feature, markers)| {
        markers.iter().any(|marker| source.contains(marker)).then_some(feature)
    }).collect()
}

pub fn generate_module_init(bundles: &[ModuleBundle]) -> String {
    generate_module_init_with(bundles, false)
}

/// As [`generate_module_init`]; `lazy` (`--lazy-packages`) declares each bundle (`declareThawJs`) instead of
/// loading it: thaw-js evaluates it on the program's first use.
// ponytail: no "pure at load" scan yet, so lazy declares every plain bundle (opt-in flag); a package with
// load-time global side effects or load errors now acts at first use. Add the scan before any default flip.
pub fn generate_module_init_with(bundles: &[ModuleBundle], lazy: bool) -> String {
    if bundles.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_module_init(): void {\n");
    for bundle in bundles {
        out.push_str(&format!("    // {}\n", bundle.package_name));
        // realm/group views join another bundle's realm at load time: keep those eager
        let lazy = lazy
            && !bundle.js_source.contains("__thaw_bundle_realm")
            && !bundle.js_source.contains("__thaw_bundle_group");
        out.push_str(&format!(
            "    if (!{}ThawJs(\"{}\", \"{}\")) throw new Error(\"failed to load module script\");\n",
            if lazy { "declare" } else { "load" },
            escape_ts_string_literal(bundle.package_name),
            escape_ts_string_literal(bundle.js_source)
        ));
    }
    out.push_str("}\n");
    out
}
use base64::Engine;
use flate2::{write::GzEncoder, Compression};
use std::io::Write;

pub fn encode_embedded_native(bytes: &[u8]) -> String {
    // Tiny addons are cheaper as-is and retaining this form keeps old
    // generated shims readable. Real native addons cross this threshold by
    // orders of magnitude.
    if bytes.len() < 1024 {
        return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(bytes).is_ok() {
        if let Ok(compressed) = encoder.finish() {
            return format!(
                "gz:{}",
                base64::engine::general_purpose::STANDARD.encode(compressed)
            );
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
