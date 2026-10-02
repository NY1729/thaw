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
    matches!(name, "undefined" | "NaN" | "Infinity")
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
/// becomes a thin wrapper wired to the QuickJS-NG path (`callDynamic`).
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
                    params.push(format!("...{name}: ({})[]", render_ts_type(variadic)));
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
                        "// Fallback (QuickJS-NG), package-qualified alias: {reason}\n"
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
                out.push_str(&format!("// Fallback (QuickJS-NG): {reason}\n"));
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
/// targets the synchronous N-API host instead of QuickJS.
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

/// One registry package's bundled JS to auto-load, for
/// `generate_module_init`.
pub struct ModuleBundle<'a> {
    pub package_name: &'a str,
    pub js_source: &'a str,
    /// This package's Fallback function names (from its `.d.ts`,
    /// `Classification::Fallback { function, .. }`) -- used by
    /// `wrap_as_commonjs_module` to bind a bare `module.exports = fn`
    /// default export (very common for small single-function utility
    /// packages, e.g. `left-pad`) to the name `callDynamic` will look it
    /// up by.
    pub fallback_names: &'a [String],
    /// `(bare_name, qualified_key)` pairs for names that collide with
    /// another `--use`d package (thaw-cli detects this across packages,
    /// which no single `.d.ts`/package knows about on its own). Right
    /// after this bundle's `loadScript`, `generate_module_init` also
    /// captures `globalThis[bare_name]` under `globalThis[qualified_key]`
    /// (e.g. `"qs::stringify"`) -- *before* a later colliding package's
    /// own `loadScript` overwrites the bare name -- so a package-
    /// qualified call (`qs.stringify(...)`, rewritten by thaw-cli to the
    /// matching `generate_shim` alias) reaches the right one regardless
    /// of `--use` order. See `generate_shim`'s `qualified_names` doc
    /// comment for the other half of this.
    pub qualified_aliases: &'a [(String, String)],
    /// `(namespace.member, qualified_key)` pairs for a nested-namespace
    /// re-export (`export * as NAME from "...";` -- see
    /// `thaw_bridge::nested_namespace_members`'s own doc comment; real
    /// example: zod's `coerce.number`). Captured *inside*
    /// `wrap_as_commonjs_module`'s own wrapped script, immediately after
    /// `module.exports` is fully set (the same place/timing the plain
    /// `for...in` copy loop already runs at) -- unlike `qualified_aliases`
    /// above, this can't be captured via a *separate*, later `loadScript`
    /// call: confirmed via a direct experiment that a function value
    /// reached through a two-level property chain (`module.exports.ns.
    /// member`) becomes silently uninvokable through the native
    /// `callDynamic` boundary specifically when captured from a different
    /// `loadScript`/`eval` call than the one that set `module.exports` in
    /// the first place -- while remaining perfectly callable from JS, and
    /// while a same-eval capture of the very same chain works fine. Root
    /// cause not fully understood (looks like a QuickJS-NG/rquickjs
    /// engine limitation around cross-`eval` native invocation of a
    /// property-chain-derived function value), but capturing within the
    /// same `eval` call that created the object sidesteps it.
    pub nested_namespace_aliases: &'a [(String, String)],
    /// `(export_name, runtime_getter, local, typed_getter_symbol)` for
    /// non-callable values captured once after this bundle is loaded.
    pub value_exports: &'a [(String, String, String, String)],
    /// This package's constructible class names (`pkg.classes`, not just
    /// the ones reached through a named export). A `new Class(...)`
    /// construction for the Fallback/QuickJS backend resolves its
    /// constructor by looking up a JS *global* named after the class
    /// (`thaw_js_get_global`, `dynamic_host/typed_calls.rs`'s `$new$`
    /// key) -- which the generic `module.exports` -> `globalThis`
    /// copy-loop below only ever populates for a *named* export
    /// (`module.exports = { PQueue }`, where `PQueue` is `module.
    /// exports`'s own enumerable key). A `export default class PQueue`
    /// package's `module.exports` instead *is* the class itself (`for
    /// (var k in module.exports)` then enumerates the class's own static
    /// properties -- `default`, `__esModule`, real statics -- never a
    /// property literally named `"PQueue"`), so `globalThis.PQueue` was
    /// never set and `new PQueue()` failed at run time with `JavaScript
    /// value handle N is not a constructor` (`N` being whatever
    /// `thaw_js_get_global` happened to resolve for a name nothing ever
    /// bound). Bound here the same way `fallback_names` binds a
    /// default-exported *function*.
    pub class_names: &'a [String],
}

/// Wraps a real npm package's CommonJS source so it can run inside
/// QuickJS-NG's bare global scope: defines `module`/`exports`/`require`
/// as *globals* before the source runs (real CommonJS/UMD source
/// references them unconditionally, and QuickJS-NG's global scope has
/// none of them), runs the source completely unwrapped/at top level
/// (deliberately -- nesting it inside a function scope would break the
/// existing simple case of a hand-authored bundle using bare top-level
/// `function` declarations, which rely on top-level scope becoming
/// global properties directly, the same way `loadScript` already worked
/// before this), then copies whatever the source assigned to
/// `module.exports` into still-unoccupied global names, so package
/// exports cannot replace platform globals such as `setTimeout`.
/// Package-qualified aliases are captured directly from `module.exports`.
///
/// Validated by running real, unmodified npm packages through the
/// Fallback path: `left-pad` (`module.exports = leftPad`) and `slugify`
/// (a UMD wrapper that takes the CommonJS branch once `module`/`exports`
/// exist) both load and run correctly, and a plain hand-authored bundle
/// with no `module.exports` at all keeps working exactly as before.
/// `is-odd` -- which calls `require('is-number')` at load time -- fails
/// with a clear error instead of the pre-existing opaque
/// `ReferenceError: module is not defined`, correctly surfacing a real,
/// documented limitation (see below) rather than silently misbehaving.
///
/// `require` is a stub that throws immediately: resolving a real
/// inter-package dependency graph inside QuickJS-NG remains out of scope
/// (docs/design/bridge.md section 7's "未解決の論点"/"unresolved
/// questions"). This only unblocks packages with no runtime dependencies
/// of their own, which covers plenty of real small utility packages.
///
/// A `fallback_names` binding also checks `module.exports.default`, not
/// just `module.exports` itself, being a function: thaw-registry's ESM
/// rewrite (a real ESM package's `export default function foo(){}`)
/// always sets `module.exports.default`, matching real `import x from
/// 'y'` interop semantics, rather than replacing `module.exports`
/// outright the way a plain CommonJS `module.exports = foo` does.
///
/// The `module.exports` -> `globalThis` copy loop skips (rather than
/// aborts on) a key it can't assign: `for...in` enumerates own AND
/// inherited enumerable keys in insertion order, so one key throwing
/// (in strict mode -- this bundle runs with `"use strict"`) used to
/// silently drop *every* key enumerated after it too, not just that one
/// -- real example: zod, which exports a schema-builder function
/// literally named `undefined` (`z.undefined()`), and `globalThis`'s own
/// `undefined` property is non-writable, so assigning it throws
/// `TypeError: 'undefined' is read-only`; every other export enumerated
/// after `undefined` in zod's own property order (`optional`, `object`,
/// ...) silently never reached `globalThis` at all before this fix,
/// even though none of them have anything to do with `undefined`
/// itself.
///
/// The same copy loop also `.bind()`s each copied *function* value to
/// `module.exports` (its own original owner) before handing it to
/// `globalThis`, rather than copying the bare reference. Most real
/// packages' exported "methods" don't actually read `this` at all (real
/// example: lodash's `declare const _: LoDashStatic` -- `_.chunk(...)`'s
/// own implementation is a plain, `this`-free function, unaffected
/// either way), so this changes nothing for them. But some genuinely
/// do -- real example: joi's `declare const Joi: Joi.Root`, whose
/// `Root.string()`/`.object()`/etc are real instance methods needing
/// `this` to equal the live `root` object joi's own internal
/// `internals.generate(this, ...)` asserts on (`Must be invoked on a Joi
/// instance`); same shape for handlebars' `Handlebars.registerHelper`,
/// which reads `this.helpers`. Every such method gets pulled out of its
/// owning object and rebound to `globalThis` under its own bare name
/// (`extract_interface_method_decls`'s whole point, matching
/// `thaw_bridge::classify_all`'s "package-level Fallback function"
/// model) -- without `.bind()`, calling it later as a bare global
/// function (`thaw_js_call("string", args)`) permanently loses its
/// receiver, exactly the way copying `const f = obj.method` and calling
/// bare `f()` always does in plain JS.
fn wrap_as_commonjs_module(
    js_source: &str,
    fallback_names: &[String],
    class_names: &[String],
    nested_namespace_aliases: &[(String, String)],
) -> String {
    // `name` is always a valid JS identifier here: it's a function name
    // SWC already parsed out of a `.d.ts` `declare function` statement,
    // not arbitrary text, so splicing it directly as a property-access
    // identifier (not a bracketed string) is safe -- same for a class
    // name, parsed out of a `declare class` statement.
    let bind_default_export = |name: &str| {
        // The `globalThis.<name>` slot here is a *staging* value the
        // immediately-following qualified-alias `loadScript` captures (see
        // this function's caller), not a durable binding -- so it must be
        // overwritten unconditionally. It used to be skipped when
        // `globalThis.<name>` was already a function, which silently broke
        // the *second* package exporting a whole-function under the same
        // name: express's own `export = e` binds `globalThis.e`, so cors's
        // identical `export = e` saw `typeof globalThis.e === 'function'`
        // and left it as express -- `cors()` then called express, so
        // `app.use(cors())` registered an express app and no CORS header
        // was ever set. Both packages' `pkg::e` keys had already been
        // captured before the next one ran, so overwriting is safe.
        format!(
            "if (typeof module.exports === 'function' && typeof module.exports.{name} === 'undefined') {{ globalThis.{name} = module.exports; }}\n\
             else if (typeof module.exports === 'object' && module.exports !== null && module.exports.__esModule && typeof module.exports.default === 'function') {{ globalThis.{name} = module.exports.default; }}\n\
             else if (typeof module.exports === 'object' && module.exports !== null && !module.exports.__esModule && Object.keys(module.exports).length === 1 && (function() {{ var descriptor = Object.getOwnPropertyDescriptor(module.exports, 'default'); return descriptor !== undefined && typeof descriptor.value === 'function'; }})()) {{ globalThis.{name} = module.exports.default; }}\n"
        )
    };
    let bind_default_exports: String = fallback_names
        .iter()
        .chain(class_names)
        .map(|name| bind_default_export(name))
        .collect();
    // Captured *here*, inside the same wrapped script that just set
    // `module.exports` (not via a separate, later `loadScript` call the
    // way `ModuleBundle::qualified_aliases` is) -- see
    // `ModuleBundle::nested_namespace_aliases`'s own doc comment for why
    // that distinction matters.
    let bind_nested_namespaces: String = nested_namespace_aliases
        .iter()
        .filter_map(|(bare_name, qualified_key)| {
            let (receiver, member) = bare_name.rsplit_once('.')?;
            let receiver_lookup = format!("module.exports?.{}", receiver.replace('.', "?."));
            Some(format!(
                "{{ var __thaw_nested_receiver = {receiver_lookup}; if (__thaw_nested_receiver != null) {{ var __thaw_nested_value = __thaw_nested_receiver.{member}; if (typeof __thaw_nested_value !== 'undefined') globalThis[\"{}\"] = typeof __thaw_nested_value === 'function' ? globalThis.__thaw_bind_preserving_statics(__thaw_nested_value, __thaw_nested_receiver) : __thaw_nested_value; }} }}\n",
                escape_ts_string_literal(qualified_key)
            ))
        })
        .collect();
    format!(
        "globalThis.module = {{ exports: {{}} }};\n\
         globalThis.exports = globalThis.module.exports;\n\
         // A package's own JS glue can load its native addon by\n\
         // computing a filesystem path at runtime and handing it\n\
         // straight to `require(...)`, rather than a literal\n\
         // `require('./addon.node')` a bundler's static require map\n\
         // could ever see ahead of time -- real trigger: better-\n\
         // sqlite3's own `lib/binding.js`, whose `getBinding()` builds\n\
         // `path.join(__dirname, '..', 'prebuilds', '<platform>-\n\
         // <arch>.node')` (or a `build/Release/...` fallback) and\n\
         // `require()`s that computed string directly. Every `require`\n\
         // in scope for bundled/wrapped source (this stub, and each\n\
         // per-module `require` `render_bundle` hands a factory\n\
         // function, which falls back to this same stub for any spec\n\
         // its own static map doesn't recognize) ultimately reaches\n\
         // this one base case, so checking here covers every call site\n\
         // at once rather than duplicating the check at each fallback.\n\
         globalThis.require = function(name) {{ if (String(name).endsWith('.node') && typeof globalThis.require.addon === 'function') return globalThis.require.addon(); var error = new Error(\"Cannot find module '\" + name + \"'\"); error.code = 'MODULE_NOT_FOUND'; throw error; }};\n\
         // Real packages commonly *guard* Node-only globals before using\n\
         // them (`Buffer && Buffer.isBuffer(x)`, `Buffer?.from(x)`) for\n\
         // exactly this situation -- a non-Node environment. But an\n\
         // undeclared bare identifier throws `ReferenceError` just from\n\
         // being *referenced*, guard or not (found via `@hapi/hoek`,\n\
         // which does this unconditionally at load time); explicitly\n\
         // assigning it `undefined` makes the identifier exist without\n\
         // pretending Buffer support exists, so those guards correctly\n\
         // take their \"not available\" branch instead of throwing.\n\
         if (typeof globalThis.Buffer === 'undefined') {{ globalThis.Buffer = undefined; }}\n\
         // Unlike `Buffer` above, real code sometimes reaches for `URL`\n\
         // *unconditionally* (e.g. `URL.prototype` as a lookup-table key,\n\
         // found via `@hapi/hoek`) rather than guarding it first --\n\
         // `undefined` doesn't survive a `.prototype` access, so this\n\
         // needs an actual (empty) constructor stand-in instead, which\n\
         // gets a `.prototype` object for free like any JS function.\n\
         if (typeof globalThis.URL === 'undefined') {{ globalThis.URL = function URL() {{}}; }}\n\
         // Node exposes `process` as an *ambient global*, not only as a\n\
         // requirable core module (thaw-registry's `builtin_module_source`\n\
         // covers the `require('process')` half) -- real packages\n\
         // (`node-gyp-build.js`, chasing a native addon's load path) read\n\
         // `process.config`/`process.env`/`process.versions`/\n\
         // `process.execPath` as a bare global with no guard at all, which\n\
         // would otherwise throw `ReferenceError: process is not defined`\n\
         // the same way an unguarded `Buffer`/`URL` reference would.\n\
         if (typeof globalThis.process === 'undefined') {{\n\
         \x20\x20globalThis.process = {{ argv: [], env: {{}}, platform: 'linux', version: '', execPath: 'node', config: {{ variables: {{}} }}, versions: {{ node: '', modules: '', uv: '' }}, nextTick: function(fn) {{ var args = Array.prototype.slice.call(arguments, 1); var run = function() {{ fn.apply(undefined, args); }}; if (typeof queueMicrotask === 'function') queueMicrotask(run); else Promise.resolve().then(run); }} }};\n\
         }}\n\
         // Likewise `__dirname`/`__filename`: real per-module Node\n\
         // locals, but every package here already runs unwrapped at\n\
         // global scope (see this function's doc comment), so a shared\n\
         // global stand-in is consistent with the rest of this wrapper.\n\
         // The exact path is inert -- `fs` above always reports \"nothing\n\
         // here\", so no real lookup ever depends on this value being\n\
         // accurate, only present as a string.\n\
         if (typeof globalThis.__dirname === 'undefined') {{ globalThis.__dirname = '/thaw_modules/package'; }}\n\
         if (typeof globalThis.__filename === 'undefined') {{ globalThis.__filename = '/thaw_modules/package/index.js'; }}\n\
         if (typeof globalThis.__thaw_napi_bridge_exports === 'function') {{\n\
         \x20\x20var __thaw_addon = {{}};\n\
         \x20\x20var __thaw_napi_reference_id = 0;\n\
         \x20\x20var __thaw_napi_reference_ids = new WeakMap();\n\
         \x20\x20var __thaw_napi_reference_values = new Map();\n\
         \x20\x20var __thaw_napi_handles = new WeakMap();\n\
         \x20\x20var __thaw_napi_proxies = new Map();\n\
         \x20\x20var __thaw_napi_symbol_values = new Map(), __thaw_napi_symbol_ids = new Map();\n\
         \x20\x20var __thaw_napi_binary_values = new Map();\n\
         \x20\x20var __thaw_napi_binary_finalizers = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(function(held) {{ if (__thaw_napi_binary_values.get(held.id) !== held.entry) return; __thaw_napi_binary_values.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }}) : null;\n\
         \x20\x20var __thaw_napi_finalizers = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(function(id) {{ __thaw_napi_reference_values.delete(id); __thaw_napi_handle('release', id, '', []); }}) : null;\n\
         \x20\x20var __thaw_napi_proxy_finalizers = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(function(held) {{ if (__thaw_napi_proxies.get(held.id) !== held.entry) return; __thaw_napi_proxies.delete(held.id); __thaw_napi_handle('release_handle', held.id, '', []); }}) : null;\n\
         \x20\x20var __thaw_napi_reference_value = function(id, active) {{ var stored = __thaw_napi_reference_values.get(id), value = stored && typeof stored.deref === 'function' ? stored.deref() : stored, properties = {{}}; if (!value) return properties; Object.keys(value).forEach(function(key) {{ properties[key] = __thaw_napi_argument(value[key], active); }}); return properties; }};\n\
         \x20\x20var __thaw_napi_argument = function(value, active) {{\n\
         \x20\x20\x20\x20if (value === null || (typeof value !== 'function' && typeof value !== 'object')) return value;\n\
         \x20\x20\x20\x20var handle = __thaw_napi_handles.get(value); if (handle) return {{ __thaw_napi_handle__: handle }};\n\
         \x20\x20\x20\x20active = active || new WeakSet();\n\
         \x20\x20\x20\x20var id = __thaw_napi_reference_ids.get(value);\n\
         \x20\x20\x20\x20if (id && active.has(value)) return {{ __thaw_napi_ref__: id }};\n\
         \x20\x20\x20\x20var fresh = !id; if (fresh) {{ id = ++__thaw_napi_reference_id; __thaw_napi_reference_ids.set(value, id); }}\n\
         \x20\x20\x20\x20if (typeof value === 'function') {{ if (fresh) {{ __thaw_napi_reference_values.set(id, typeof WeakRef === 'function' ? new WeakRef(value) : value); globalThis['__thaw_napi_reference_' + id] = function() {{ var args = Array.prototype.slice.call(arguments), meta = args.shift(), receiver; if (meta && meta.__thaw_napi_argument_handles__) meta.__thaw_napi_argument_handles__.forEach(function(entry) {{ var path = entry[0], parent = args; for (var i = 0; i + 1 < path.length; i++) {{ var own = Object.getOwnPropertyDescriptor(parent, path[i]); if (!own) throw new TypeError('invalid native callback path'); parent = own.value; }} Object.defineProperty(parent, path[path.length - 1], {{ value: __thaw_napi_proxy(entry[1]), writable: true, enumerable: true, configurable: true }}); }}); if (meta && meta.__thaw_napi_this_handle__) {{ var stored = __thaw_napi_proxies.get(meta.__thaw_napi_this_handle__); receiver = stored && typeof stored.deref === 'function' ? stored.deref() : stored; if (!receiver) receiver = __thaw_napi_proxy(meta.__thaw_napi_this_handle__); }} var returned = value.apply(receiver, args); return typeof returned === 'function' ? __thaw_napi_argument(returned) : returned; }}; }} return {{ __thaw_napi_function__: id }}; }}\n\
         \x20\x20\x20\x20__thaw_napi_reference_values.set(id, typeof WeakRef === 'function' ? new WeakRef(value) : value);\n\
         \x20\x20\x20\x20if (fresh && __thaw_napi_finalizers) __thaw_napi_finalizers.register(value, id);\n\
         \x20\x20\x20\x20active.add(value);\n\
         \x20\x20\x20\x20var kinds = {{ Int8Array: 0, Uint8Array: 1, Uint8ClampedArray: 2, Int16Array: 3, Uint16Array: 4, Int32Array: 5, Uint32Array: 6, Float32Array: 7, Float64Array: 8, BigInt64Array: 9, BigUint64Array: 10 }}, view = typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView(value), buffer = typeof ArrayBuffer !== 'undefined' && (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && value instanceof SharedArrayBuffer)), isBuffer = typeof Buffer !== 'undefined' && Buffer.isBuffer(value);\n\
         \x20\x20\x20\x20if (view && !isBuffer && !(value instanceof DataView) && kinds[value.constructor.name] === undefined) throw new TypeError('unsupported native typed array kind');\n\
         \x20\x20\x20\x20var encoded = isBuffer ? {{ __thaw_napi_buffer__: id, value: Array.prototype.slice.call(value) }} : buffer ? {{ __thaw_napi_arraybuffer__: id, shared: typeof SharedArrayBuffer !== 'undefined' && value instanceof SharedArrayBuffer, value: Array.prototype.slice.call(new Uint8Array(value)) }} : view ? {{ __thaw_napi_view__: id, kind: value instanceof DataView ? -1 : kinds[value.constructor.name], length: value instanceof DataView ? value.byteLength : value.length, byte_offset: value.byteOffset, buffer: __thaw_napi_argument(value.buffer, active) }} : Array.isArray(value) ? {{ __thaw_napi_array__: id, value: value.map(function(item) {{ return __thaw_napi_argument(item, active); }}) }} : {{ __thaw_napi_object__: id, value: __thaw_napi_reference_value(id, active) }};\n\
         \x20\x20\x20\x20active.delete(value); return encoded;\n\
         \x20\x20}};\n\
         \x20\x20var __thaw_napi_arguments = function(args) {{ var active = new WeakSet(); return Array.prototype.map.call(args, function(value) {{ return __thaw_napi_argument(value, active); }}); }};\n\
         \x20\x20// A plain object argument can carry a getter-only accessor\n\
         \x20\x20// property (real trigger: better-sqlite3's own `Database`\n\
         \x20\x20// instance, passed as `prepare(sql, this, ...)`'s second\n\
         \x20\x20// argument, whose `name`/`open`/`inTransaction`/`readonly`/\n\
         \x20\x20// `memory` are all getter-only via `Object.defineProperties`)\n\
         \x20\x20// -- syncing the native side's echoed-back value onto it\n\
         \x20\x20// with a plain `=` assignment threw a real engine\n\
         \x20\x20// `TypeError: no setter for property` the moment any such\n\
         \x20\x20// key came back, even though the native round trip never\n\
         \x20\x20// actually changed it. A getter-only property is always\n\
         \x20\x20// freshly computed on the next real read anyway, so there is\n\
         \x20\x20// nothing meaningful to \"sync\" for one -- skip a key the\n\
         \x20\x20// assignment itself rejects instead of crashing the whole\n\
         \x20\x20// call.\n\
         \x20\x20var __thaw_napi_decode = function(value, origins, path) {{ path = path || []; if (Array.isArray(origins)) origins = new Map(origins.map(function(entry) {{ return [JSON.stringify(entry[0]), entry]; }})); var origin = origins && origins.get(JSON.stringify(path)); if (origin && origin[1] === 'date') return new Date(origin[2] === null ? NaN : origin[2]); if (origin && origin[1] === 'nonfinite') return Number(origin[2]); var plain = origin && origin[1] === 'plain'; if (value && typeof value === 'object') {{ if (!plain && value['$__thaw_napi_undefined$'] === true) return undefined; if (!plain && Object.prototype.hasOwnProperty.call(value, '__thaw_napi_error__')) {{ var ctor = value.name === 'TypeError' ? TypeError : value.name === 'RangeError' ? RangeError : Error, error = new ctor(value.__thaw_napi_error__); if (value.name && error.name !== value.name) error.name = value.name; return error; }} if (!plain && value.__thaw_napi_symbol__) {{ var symbol = __thaw_napi_symbol_values.get(value.__thaw_napi_symbol__); if (!symbol) {{ symbol = value.global ? Symbol.for(value.description) : Symbol(value.description); __thaw_napi_symbol_values.set(value.__thaw_napi_symbol__, symbol); __thaw_napi_symbol_ids.set(symbol, value.__thaw_napi_symbol__); }} return symbol; }} if (!plain && value.__thaw_napi_ref__) {{ var stored = __thaw_napi_reference_values.get(value.__thaw_napi_ref__); return stored && typeof stored.deref === 'function' ? stored.deref() : stored; }} if (!plain && value.__thaw_napi_handle__) return __thaw_napi_proxy(value.__thaw_napi_handle__); if (!plain && value.__thaw_napi_promise__) return __thaw_napi_result_value(value); if (!plain && value.__thaw_napi_binary__) {{ var id = value.__thaw_napi_binary__, stored = __thaw_napi_binary_values.get(id), existing = stored && typeof stored.deref === 'function' ? stored.deref() : stored; if (existing) return existing; var binary; if (value.kind === 'ArrayBuffer' || value.kind === 'SharedArrayBuffer') {{ binary = value.kind === 'SharedArrayBuffer' ? new SharedArrayBuffer(value.data.length) : new ArrayBuffer(value.data.length); new Uint8Array(binary).set(value.data); }} else {{ var backing = __thaw_napi_decode(value.buffer, origins, path.concat('buffer')); if (value.kind === 'DataView') binary = new DataView(backing, value.byte_offset, value.length); else {{ var constructors = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array, globalThis.BigInt64Array, globalThis.BigUint64Array], viewCtor = constructors[value.array_type]; if (!viewCtor) throw new TypeError('unsupported native typed array kind'); binary = new viewCtor(backing, value.byte_offset, value.length); }} }} var entry = typeof WeakRef === 'function' ? new WeakRef(binary) : binary; __thaw_napi_binary_values.set(id, entry); __thaw_napi_handles.set(binary, id); if (__thaw_napi_binary_finalizers) __thaw_napi_binary_finalizers.register(binary, {{ id: id, entry: entry }}); return binary; }} if (!plain && value.type === 'Buffer' && Array.isArray(value.data) && typeof Buffer !== 'undefined') return Buffer.from(value.data); if (Array.isArray(value)) return value.map(function(child, index) {{ return __thaw_napi_decode(child, origins, path.concat(index)); }}); var decoded = {{}}; Object.keys(value).forEach(function(key) {{ Object.defineProperty(decoded, key, {{ value: __thaw_napi_decode(value[key], origins, path.concat(key)), enumerable: true, configurable: true, writable: true }}); }}); return decoded; }} return value; }};\n\
         \x20\x20var __thaw_napi_sync_arguments = function(args) {{ var seen = new WeakSet(); var sync = function(value) {{ if (value === null || typeof value !== 'object' || seen.has(value)) return; seen.add(value); if (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView(value) && !(typeof Buffer !== 'undefined' && Buffer.isBuffer(value))) {{ sync(value.buffer); return; }} var handle = __thaw_napi_handles.get(value), id = __thaw_napi_reference_ids.get(value); if (!handle && !id) return; var response = __thaw_napi_handle(handle ? 'sync_handle' : 'sync_reference', handle || id, '', []), updated = response.value, originMap = new Map((response.origins || []).map(function(entry) {{ return [JSON.stringify(entry[0]), entry]; }})); if (typeof ArrayBuffer !== 'undefined' && (value instanceof ArrayBuffer || (typeof SharedArrayBuffer !== 'undefined' && value instanceof SharedArrayBuffer))) {{ new Uint8Array(value).set(updated.slice(0, value.byteLength)); return; }} if (typeof Buffer !== 'undefined' && Buffer.isBuffer(value)) {{ for (var i = 0; i < Math.min(value.length, updated.length); i++) value[i] = updated[i]; return; }} if (handle) return; if (Array.isArray(value)) value.length = updated.length; else Object.keys(value).forEach(function(key) {{ if (!Object.prototype.hasOwnProperty.call(updated, key)) {{ try {{ delete value[key]; }} catch (e) {{}} }} }}); Object.keys(updated).forEach(function(key) {{ var next = handle ? updated[key] : __thaw_napi_decode(updated[key], originMap, [Array.isArray(updated) ? Number(key) : key]); if (key === '__proto__') Object.defineProperty(value, key, {{ value: next, writable: true, enumerable: true, configurable: true }}); else if (value[key] !== next) {{ try {{ value[key] = next; }} catch (e) {{}} }} if (next && typeof next === 'object') sync(next); }}); }}; Array.prototype.forEach.call(args, sync); }};\n\
         \x20\x20var __thaw_napi_promises = new Map();\n\
         \x20\x20var __thaw_napi_promise_finalizers = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(function(held) {{ if (__thaw_napi_promises.get(held.id) === held.entry) __thaw_napi_promises.delete(held.id); }}) : null;\n\
         \x20\x20var __thaw_napi_result_value = function(value, origins) {{\n\
         \x20\x20\x20\x20if (value && typeof value === 'object' && !(origins || []).some(function(entry) {{ return entry[0].length === 0 && entry[1] === 'plain'; }}) && Object.prototype.hasOwnProperty.call(value, '__thaw_napi_promise__')) {{ var id = value.__thaw_napi_promise__, stored = __thaw_napi_promises.get(id), existing = stored && typeof stored.deref === 'function' ? stored.deref() : stored; if (existing) return existing; var promise = new Promise(function(resolve, reject) {{ var check = function() {{ var state; try {{ state = __thaw_napi_handle('promise_state', id, '', []); }} catch (error) {{ reject(error); return; }} if (state.kind === 'pending') {{ setTimeout(check, 0); return; }} if (state.kind === 'rejected') {{ reject(state.value); return; }} resolve(state.value); }}; setTimeout(check, 0); }}); var entry = typeof WeakRef === 'function' ? new WeakRef(promise) : promise; __thaw_napi_promises.set(id, entry); if (__thaw_napi_promise_finalizers) __thaw_napi_promise_finalizers.register(promise, {{ id: id, entry: entry }}); return promise; }}\n\
         \x20\x20\x20\x20return __thaw_napi_decode(value, origins);\n\
         \x20\x20}};\n\
         \x20\x20var __thaw_napi_handle = function(operation, target, name, args) {{\n\
         \x20\x20\x20\x20var result = JSON.parse(globalThis.__thaw_napi_bridge_handle(operation, String(target), name || '', JSON.stringify(__thaw_napi_arguments(args || []))));\n\
         \x20\x20\x20\x20if (result && Object.prototype.hasOwnProperty.call(result, '__thaw_error__')) throw new Error(result.__thaw_error__);\n\
         \x20\x20\x20\x20if (result && result.value && !(result.origins || []).some(function(entry) {{ return entry[0].length === 0 && entry[1] === 'plain'; }}) && Object.prototype.hasOwnProperty.call(result.value, '$__thaw_napi_undefined$') && result.value['$__thaw_napi_undefined$'] === true) result.value = undefined;\n\
         \x20\x20\x20\x20else if (result && Object.prototype.hasOwnProperty.call(result, 'value') && result.kind !== 'method' && operation !== 'sync_reference') result.value = __thaw_napi_result_value(result.value, result.origins);\n\
         \x20\x20\x20\x20return result;\n\
         \x20\x20}};\n\
         \x20\x20var __thaw_napi_symbol_id = function(handle, symbol) {{ var id = __thaw_napi_symbol_ids.get(symbol), global = Symbol.keyFor(symbol), created = __thaw_napi_handle('symbol', handle, global === undefined ? symbol.description || '' : global, [global !== undefined, id || null]); id = created.value; __thaw_napi_symbol_ids.set(symbol, id); __thaw_napi_symbol_values.set(id, symbol); return id; }};\n\
         \x20\x20var __thaw_napi_key = function(handle, name, operation, args) {{ return typeof name === 'symbol' ? __thaw_napi_handle(operation + '_symbol', handle, String(__thaw_napi_symbol_id(handle, name)), args) : __thaw_napi_handle(operation, handle, String(name), args); }};\n\
         \x20\x20var __thaw_napi_proxy = function(handle, prototype) {{ var stored = __thaw_napi_proxies.get(String(handle)), existing = stored && typeof stored.deref === 'function' ? stored.deref() : stored; if (existing) return existing; var target = Object.create(prototype || Object.prototype), methods = new Map();\n\
         \x20\x20var read = function(name) {{ var result = __thaw_napi_key(handle, name, 'get', []); if (result.kind !== 'method') return result.value; var method = methods.get(result.value); if (!method) {{ var captured = result.value; method = function() {{ var args = Array.prototype.slice.call(arguments), value = __thaw_napi_handle('call_captured', captured, String(__thaw_napi_handles.get(proxy)), args).value; __thaw_napi_sync_arguments(args); return value; }}; methods.set(captured, method); }} return method; }};\n\
         \x20\x20var descriptor = function(name) {{ var meta = __thaw_napi_key(handle, name, 'descriptor', []).value; if (!meta) return undefined; var desc = meta.accessor ? {{ configurable: meta.configurable, enumerable: meta.enumerable, get: function() {{ return read(name); }}, set: meta.setter ? function(value) {{ __thaw_napi_key(handle, name, 'set', [value]); }} : undefined }} : {{ configurable: meta.configurable, enumerable: meta.enumerable, writable: meta.writable, value: read(name) }}; if (!meta.configurable) {{ var own = Reflect.getOwnPropertyDescriptor(target, name); if (!own) Reflect.defineProperty(target, name, desc); else if (!meta.accessor && own.writable) Reflect.defineProperty(target, name, {{ value: desc.value }}); return Reflect.getOwnPropertyDescriptor(target, name); }} return desc; }};\n\
         \x20\x20var proxy = new Proxy(target, {{\n\
         \x20\x20\x20\x20get: function(_, name, receiver) {{ if (!__thaw_napi_key(handle, name, 'has', []).value) return Reflect.get(_, name, receiver); var own = Reflect.getOwnPropertyDescriptor(_, name); if (own && !own.configurable && !own.writable && Object.prototype.hasOwnProperty.call(own, 'value')) return own.value; return read(name); }},\n\
         \x20\x20\x20\x20set: function(_, name, value) {{ __thaw_napi_key(handle, name, 'set', [value]); __thaw_napi_sync_arguments([value]); var own = Reflect.getOwnPropertyDescriptor(_, name); if (own && !own.configurable && own.writable) Reflect.defineProperty(_, name, {{ value: read(name) }}); return true; }},\n\
         \x20\x20\x20\x20has: function(_, name) {{ return Reflect.has(_, name) || __thaw_napi_key(handle, name, 'has', []).value; }},\n\
         \x20\x20\x20\x20ownKeys: function(_) {{ var keys = __thaw_napi_handle('own_keys', handle, '', []).value; Reflect.ownKeys(_).forEach(function(key) {{ if (keys.indexOf(key) < 0) keys.push(key); }}); keys.forEach(function(key) {{ descriptor(key); }}); return keys; }},\n\
         \x20\x20\x20\x20getOwnPropertyDescriptor: function(_, name) {{ return descriptor(name) || Reflect.getOwnPropertyDescriptor(_, name); }},\n\
         \x20\x20\x20\x20preventExtensions: function() {{ return false; }}\n\
         \x20\x20}}); __thaw_napi_handles.set(proxy, handle); var entry = typeof WeakRef === 'function' ? new WeakRef(proxy) : proxy, id = String(handle); __thaw_napi_proxies.set(id, entry); if (__thaw_napi_proxy_finalizers) __thaw_napi_proxy_finalizers.register(proxy, {{ id: id, entry: entry }}); return proxy; }};\n\
         \x20\x20JSON.parse(globalThis.__thaw_napi_bridge_exports()).forEach(function(name) {{\n\
         \x20\x20\x20\x20__thaw_addon[name] = function() {{\n\
         \x20\x20\x20\x20\x20\x20if (new.target) {{ var created = __thaw_napi_handle('construct', name, '', Array.prototype.slice.call(arguments)), prototype = new.target.prototype; Object.keys(prototype).forEach(function(key) {{ __thaw_napi_handle('set', created.value, key, [prototype[key]]); }}); return __thaw_napi_proxy(created.value, prototype); }}\n\
         \x20\x20\x20\x20\x20\x20var originalArguments = arguments, result = JSON.parse(globalThis.__thaw_napi_bridge_call(name, JSON.stringify(__thaw_napi_arguments(arguments))));\n\
         \x20\x20\x20\x20\x20\x20if (result && Object.prototype.hasOwnProperty.call(result, '__thaw_error__')) throw new Error(result.__thaw_error__);\n\
         \x20\x20\x20\x20\x20\x20__thaw_napi_sync_arguments(originalArguments); return __thaw_napi_result_value(result.value, result.origins);\n\
         \x20\x20\x20\x20}};\n\
         \x20\x20}});\n\
         \x20\x20globalThis.require.addon = function() {{ return __thaw_addon; }};\n\
         \x20\x20globalThis.process.dlopen = function(target) {{ target.exports = __thaw_addon; return target.exports; }};\n\
         \x20\x20if (__thaw_addon.QueryEngine && !globalThis.process.env.PRISMA_QUERY_ENGINE_LIBRARY) globalThis.process.env.PRISMA_QUERY_ENGINE_LIBRARY = '/proc/self/exe';\n\
         }}\n\
         {js_source}\n\
         var __thaw_bind_module_exports = function() {{\n\
         \x20\x20if (module.exports !== null && (typeof module.exports === 'object' || typeof module.exports === 'function')) {{ for (var k in module.exports) {{ if (k === 'default' || k === '__esModule') continue; try {{ if (typeof globalThis[k] === 'undefined') globalThis[k] = typeof module.exports[k] === 'function' ? globalThis.__thaw_bind_preserving_statics(module.exports[k], module.exports) : module.exports[k]; }} catch (e) {{}} }} }}\n\
         {bind_nested_namespaces}\
         {bind_default_exports}\
         }};\n\
         if (globalThis.__thaw_module_ready && typeof globalThis.__thaw_module_ready.then === 'function') {{ globalThis.__thaw_module_ready.then(__thaw_bind_module_exports); }}\n\
         else {{ __thaw_bind_module_exports(); }}"
    )
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
/// rely on earlier ones already being loaded into the shared QuickJS-NG
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
            "Intl.RelativeTimeFormat", "Intl.NumberFormat", "Intl.DateTimeFormat", "Intl.ListFormat"][..]),
    ].into_iter().filter_map(|(feature, markers)| {
        markers.iter().any(|marker| source.contains(marker)).then_some(feature)
    }).collect()
}

pub fn generate_module_init(bundles: &[ModuleBundle]) -> String {
    if bundles.is_empty() {
        return String::new();
    }
    let mut out = String::from("function __thaw_module_init(): void {\n");
    for bundle in bundles {
        out.push_str(&format!("    // {}\n", bundle.package_name));
        let wrapped = wrap_as_commonjs_module(
            bundle.js_source,
            bundle.fallback_names,
            bundle.class_names,
            bundle.nested_namespace_aliases,
        );
        let wrapped = if bundle.js_source.len() >= 1024 {
            encode_embedded_script(&wrapped)
        } else {
            wrapped
        };
        out.push_str("    if (!loadScript(\"globalThis.__thaw_intrinsic_Array = globalThis.Array;\")) throw new Error(\"failed to initialize module globals\");\n");
        out.push_str(&format!(
            "    if (!loadScript(\"{}\")) {{ loadScript(\"globalThis.Array = globalThis.__thaw_intrinsic_Array;\"); throw new Error(\"failed to load module script\"); }}\n",
            escape_ts_string_literal(&wrapped)
        ));
        out.push_str("    if (!loadScript(\"globalThis.Array = globalThis.__thaw_intrinsic_Array;\")) throw new Error(\"failed to restore module globals\");\n");
        for (export_name, runtime_getter, _, _) in bundle.value_exports {
            let path = export_name
                .split('.')
                .map(|part| format!("\"{}\"", escape_ts_string_literal(part)))
                .collect::<Vec<_>>()
                .join(",");
            let getter_source = format!(
                "globalThis[\"{}\"] = (function(value) {{ return function() {{ var current = value, path = [{path}], found = true; for (var i = 0; i < path.length; i++) {{ if (current == null || !Object.prototype.hasOwnProperty.call(current, path[i])) {{ found = false; break; }} current = current[path[i]]; }} return found ? current : value != null && Object.prototype.hasOwnProperty.call(value, \"default\") ? value.default : value; }}; }})(globalThis.module.exports);",
                escape_ts_string_literal(runtime_getter),
            );
            out.push_str(&format!(
                "    if (!loadScript(\"{}\")) throw new Error(\"failed to capture module value\");\n",
                escape_ts_string_literal(&getter_source)
            ));
            // Also capture the *bare* name directly (not just the
            // getter function above, keyed by the qualified runtime
            // key) -- needed for a non-constructible class's own
            // static method call (`compile_typed_napi_method`,
            // `crates/thaw-llvm/src/hir_codegen/dynamic_host/napi.rs`):
            // its receiver is looked up via `thaw_js_get_global`
            // against the class's plain, unqualified name (the same
            // way any other named global is looked up), since a
            // static method has no instance to carry its own live
            // handle. Real example: luxon's `DateTime.fromISO(...)`
            // (`DateTime` has a `private constructor`, so it's a
            // Fallback class with no constructor-based binding at
            // all) -- this used to fail at runtime ("Error converting
            // from js 'undefined' into type 'function'") since
            // nothing ever bound `globalThis.DateTime` for a
            // bundle.js/QuickJS-NG-backed package (unlike a real
            // N-API module, whose export object is looked up by name
            // through a different, already-working mechanism).
            let bare_capture_js = format!(
                "if (typeof globalThis.{export_name} === 'undefined') {{ globalThis[\"{}\"] = globalThis.module.exports != null && Object.prototype.hasOwnProperty.call(globalThis.module.exports, \"{}\") ? globalThis.module.exports[\"{}\"] : globalThis.module.exports != null && Object.prototype.hasOwnProperty.call(globalThis.module.exports, \"default\") ? globalThis.module.exports.default : globalThis.module.exports; }}",
                escape_ts_string_literal(export_name),
                escape_ts_string_literal(export_name),
                escape_ts_string_literal(export_name),
            );
            out.push_str(&format!(
                "    if (!loadScript(\"{}\")) throw new Error(\"failed to capture module export\");\n",
                escape_ts_string_literal(&bare_capture_js)
            ));
        }
        // Captured immediately, before any later package's own
        // `loadScript` can overwrite the bare name -- see
        // `ModuleBundle::qualified_aliases`'s doc comment.
        for (bare_name, qualified_key) in bundle.qualified_aliases {
            // This alias-capture logic must run as *JS* (QuickJS-NG),
            // inside its own `loadScript`, not as bare Thaw source --
            // `__thaw_module_init` is a real, compiled Thaw function, and
            // Thaw's compiler has no `typeof`/bracket-indexing/etc. at
            // all. Double-escaped: `qualified_key` for the inner JS
            // string literal, then the whole JS snippet again for the
            // outer TS string literal `loadScript` takes, same as
            // `wrap_as_commonjs_module`'s own output already is.
            //
            // Reads `globalThis.module.exports.{bare_name}` first, not
            // `globalThis.{bare_name}` directly -- `globalThis.module`
            // still holds *this* package's own fresh `{ exports: {} }`
            // at this exact point (nothing else has run in between; see
            // this loop's own comment above), so `module.exports` is a
            // perfectly ordinary object property lookup regardless of
            // what `bare_name` is. `globalThis.{bare_name}` itself can't
            // work at all for a real ECMAScript reserved word used as an
            // export name (`undefined` -- zod has one, `z.undefined()`
            // -- `null`, `NaN`; `globalThis.undefined` can never be
            // reassigned in any JS engine, so it would always read back
            // the literal, never the real function). Falls back to the
            // old `globalThis.{bare_name}` read only when the property
            // lookup finds nothing, for the one shape it doesn't cover:
            // a CommonJS package whose *whole* `module.exports` (not a
            // property of it) is the single exported function --
            // `wrap_as_commonjs_module`'s own `bind_default_exports`
            // already binds `globalThis.{bare_name} = module.exports`
            // directly for exactly that case, with no corresponding
            // property to read here instead.
            // `.bind()`s a function value to `globalThis.module.exports`
            // before capturing it, for the same reason the `module.
            // exports` -> `globalThis` copy loop above does -- this
            // reads the very same `module.exports.{bare_name}` property,
            // just under this package's own qualified key instead of the
            // bare name, and needs the receiver preserved for exactly
            // the same real packages (joi's `Root.string()`, handlebars'
            // `registerHelper`).
            // A real package can install a *throwing* getter on one of its
            // own exports (real trigger: winston's own deprecated
            // `format.padLevels`, whose getter `common.js` installs to
            // throw "{ padLevels } was removed in winston@3.0.0" -- real
            // Node never touches it, so `require('winston')` stays
            // silent). Reading it here just to bind the package-qualified
            // key would abort this whole snippet; wrapped so one
            // deprecated/throwing export can't take down module init,
            // matching the copy loop above's own per-property try/catch.
            let capture_js = format!(
                "try {{ if (typeof globalThis.module !== 'undefined' && globalThis.module && typeof globalThis.module.exports !== 'undefined' && globalThis.module.exports !== null && typeof globalThis.module.exports.{bare_name} !== 'undefined') {{ globalThis[\"{}\"] = typeof globalThis.module.exports.{bare_name} === 'function' ? globalThis.__thaw_bind_preserving_statics(globalThis.module.exports.{bare_name}, globalThis.module.exports) : globalThis.module.exports.{bare_name}; }} else if (typeof globalThis.{bare_name} !== 'undefined') {{ globalThis[\"{}\"] = globalThis.{bare_name}; }} }} catch (e) {{}}",
                escape_ts_string_literal(qualified_key),
                escape_ts_string_literal(qualified_key)
            );
            out.push_str(&format!(
                "    if (!loadScript(\"{}\")) throw new Error(\"failed to capture module alias\");\n",
                escape_ts_string_literal(&capture_js)
            ));
        }
    }
    out.push_str("}\n");
    out
}
use base64::Engine;
use flate2::{write::GzEncoder, Compression};
use std::io::Write;

fn encode_embedded_script(source: &str) -> String {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(source.as_bytes()).is_ok() {
        if let Ok(compressed) = encoder.finish() {
            return format!(
                "gz:{}",
                base64::engine::general_purpose::STANDARD.encode(compressed)
            );
        }
    }
    source.to_string()
}

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
