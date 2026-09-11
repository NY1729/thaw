/// `coerce_to_declared`'s missing symmetric case to its own `Json`
/// branch just above it: a dynamic method call with no `JsValue` hint
/// (`lower_dynamic_value_method_call`'s own default) always comes back
/// `Json`-typed, even when the caller's own declared slot is a concrete
/// scalar -- real trigger found while finishing the hono `app.get` arc:
/// `const body: string = await res.text()` used to fail outright
/// ("value has type Json, expected Str"), with no way to consume the
/// result except keeping it `JsValue`-typed and decoding it manually
/// later. Fixed by reusing the exact `JsonAsNumber`/`JsonAsString`/
/// `JsonAsBool` nodes `dictionary_value_from_json` (objects.rs) already
/// builds for the identical "decode a Json value into its declared
/// scalar type" problem elsewhere -- already fully supported by type
/// inference and both codegen backends, so no new HIR node or codegen
/// path was needed, just a new branch in `coerce_to_declared` itself.
///
/// Exercises all three scalar targets (`number`, `boolean`, `string`)
/// against one holder object whose methods all return plain data with
/// no annotation anywhere on the call site itself.
#[test]
fn a_dynamic_method_calls_json_result_decodes_into_a_declared_scalar_type() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-json-to-scalar-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("scalar-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         num: function() { return 42; }, \
         flag: function() { return true; }, \
         text: function() { return 'hi'; } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "scalar-kit";
function main(): void {
    const h: JsValue = makeHolder();
    const n: number = h.num();
    const b: boolean = h.flag();
    const s: string = h.text();
    console.log(n);
    console.log(b);
    console.log(s);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\ntrue\nhi\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue`-returning Fallback function's result passed straight into
/// *another* Fallback function's `Json`-declared parameter, both
/// unannotated at the call site (no intermediate `: JsValue`/`: Json`
/// binding) -- real-world example: real uuid's `stringify(parse(id))`,
/// where `parse`'s return type (`NonSharedArrayBuffer`) and `stringify`'s
/// parameter type (`Uint8Array`) are both unresolved reference types
/// thaw-bridge classifies the same "unclassified npm type" way `parse`'s
/// gets treated as `JsValue` (a return value) while `stringify`'s gets
/// treated as `Json` (a parameter) -- and `stringify` also has a second,
/// omittable optional parameter, so calling it with just one argument
/// (`stringify(bytes)`) routes through thaw-hir's omitted-parameter-mask
/// wrapper mechanism, an *ordinary* compiled function call like any
/// other, not a manually-built dynamic-call intrinsic.
///
/// Used to crash LLVM's own module verifier at build time ("Call
/// parameter type does not match function signature!") -- `coerce_to_
/// declared` (thaw-hir) already passes a `JsValue` through unchanged
/// into a `Json`-declared slot (relying on downstream codegen to
/// recognize the mismatch and thread the real handle through instead of
/// a raw, mistyped integer), but `build_call_with` (the codegen path an
/// *ordinary* function call like this one takes) never did any such
/// recognition at all -- only the JSON-args-array-construction call
/// sites did (guarded by `compiling_quickjs_dynamic_arguments`, which
/// this path never set). Fixed by having `build_call_with` itself detect
/// the same "expected a pointer (Json/Dictionary's own representation),
/// got a raw int (actually a `JsValue` handle)" mismatch per argument,
/// using `compile_dynamic_value_placeholder`'s own encoding via a new
/// gate-free variant (safe here since an N-API/native-addon target never
/// reaches this call path at all, unlike the gated call sites).
#[test]
fn a_jsvalue_returning_functions_result_passed_into_another_functions_json_parameter_works() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-into-json-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("buffer-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function parse(id: string): NonSharedArrayBuffer;\n\
         export declare function stringify(arr: Uint8Array, offset?: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         parse: function(id) { return { tag: id, length: 16 }; }, \
         stringify: function(arr, offset) { return 'stringified:' + arr.tag; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { parse, stringify } from "buffer-kit";
function main(): void {
    const bytes = parse("hello");
    console.log(bytes.length);
    const s: string = stringify(bytes);
    console.log(s);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "16\nstringified:hello\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A factory value bound directly to a name (`export declare const NAME:
/// SomeCallableInterface;`) instead of declared `function` -- the shape
/// drizzle-orm's `sqliteTable`/`pgTable` use (`SQLiteTableFn`/`PgTableFn`,
/// an interface with one call signature per overload). Confirms it's
/// reachable and callable end to end: `thaw_bridge::parse_dts` synthesizes
/// a `DtsFunction` from the interface's call signature(s), which flows
/// through the same `Classification::Fallback` shim-generation path as any
/// other npm function.
#[test]
fn a_declare_const_bound_to_a_callable_interface_is_reachable_and_callable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-const-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("table-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Factory {\n\
         \x20\x20\x20\x20(name: string, count: number): string;\n\
         }\n\
         export declare const factory: Factory;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         factory: function(name, count) { return name + ':' + count; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { factory } from "table-kit";
function main(): void {
    const result: string = factory("users", 3);
    console.log(result);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "users:3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn callable_object_properties_chain_through_live_javascript_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-object-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("style-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Style { (...text: unknown[]): string; readonly upper: Style; }\n\
         declare const style: Style;\nexport default style;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function style(value) { return String(value).toUpperCase(); }\n\
         style.upper = style;\nmodule.exports = { default: style };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import style from "style-kit";
function main(): void { console.log(style.upper("hello")); }
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "HELLO\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn non_callable_named_and_singleton_exports_are_reachable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-value-exports-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let constants = registry.join("constant-kit");
    let singleton = registry.join("mime-kit");
    std::fs::create_dir_all(&constants).unwrap();
    std::fs::create_dir_all(&singleton).unwrap();
    std::fs::write(constants.join("package.d.ts"), "export declare const NIL: string;\nexport declare const MAX: string;\n").unwrap();
    std::fs::write(constants.join("bundle.js"), "module.exports = { NIL: 'zero-id', MAX: 'max-id' };\n").unwrap();
    std::fs::write(singleton.join("package.d.ts"), "export declare class Mime { getType(path: string): string | null; }\ndeclare const mime: Mime;\nexport = mime;\n").unwrap();
    std::fs::write(singleton.join("bundle.js"), "module.exports = { getType: function(path) { return path.endsWith('.txt') ? 'text/plain' : null; } };\n").unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, r#"import { NIL, MAX } from "constant-kit";
import mime from "mime-kit";
function main(): void {
    console.log(NIL + ":" + MAX);
    console.log(JSON.stringify(mime.getType("note.txt")));
}"#).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["constant-kit".into(), "mime-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "zero-id:max-id\n\"text/plain\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn generic_fallback_callback_alias_is_contextually_typed() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-callback-alias-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "type Iterator<T, R> = (value: T, index: number, values: T[]) => R;\nexport declare function map<T, R>(values: T[], iterator: Iterator<T, R>): R[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { map: function(values, iterator) { return values.map(iterator); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { map } from 'callback-kit'; function main(): void { map([1, 2, 3], value => value * 2); console.log('ok'); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["callback-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "ok\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn discarded_dynamic_method_results_do_not_require_json_serialization() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-discarded-dynamic-result-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("listener-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function make(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "class Emitter { constructor() { this.events = {}; } on(name, callback) { (this.events[name] || (this.events[name] = [])).push(callback); return this; } emit(name, value) { (this.events[name] || []).forEach(function(callback) { callback(value); }); } } class Listener extends Emitter { constructor() { super(); this.self = this; } fire() { this.emit('value', this); return this; } } class Outer { constructor() { this.target = new Listener(); } fire() { this.target.fire(); return this; } } ['on'].forEach(function(name) { Outer.prototype[name] = function() { return this.target[name].apply(this.target, arguments); }; }); module.exports.make = function() { return new Outer(); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { make } from 'listener-kit'; function main(): void { const listener: JsValue = make(); listener.on('value', (value: JsValue): void => { console.log('called'); }); listener.fire(); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["listener-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "called\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn synchronous_void_native_callback_returns_undefined_to_javascript() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-sync-void-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function run(callback: () => void): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { run: function(callback) { return callback() === undefined; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { run } from 'callback-kit'; function main(): void { console.log(run((): void => {})); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["callback-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_method_signature_contextually_types_callbacks_and_type_only_imports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-contextual-method-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("web-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export type { Context } from './context';\n\
         export declare class Hono {\n\
         \x20 constructor();\n\
         \x20 get(path: string, handler: (context: JsValue) => Json): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Hono() {} Hono.prototype.get = function(path, handler) { return handler({ text: function(value) { return value; } }); }; module.exports = { Hono: Hono };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Hono, Context } from "web-kit";
function main(): void {
    const app = new Hono();
    app.get("/", (c) => { const value: string = c.text("Hello Thaw"); console.log(value); return JSON.parse("{}"); });
    app.get("/typed", (c: Context) => { const value: string = c.text("Typed"); console.log(value); return JSON.parse("{}"); });
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &["web-kit".into()]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Hello Thaw\nTyped\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fallback_object_callback_runs_as_native_code() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-contextual-object-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("server-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export type Handler = (request: { path: string }) => string;\n\
         export interface Route { method: string; path: string; handler?: Handler | object | undefined; }\n\
         export declare class Server { route(route: Route | Route[]): void; }\n\
         export declare function server(): Server;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Server() {} Server.prototype.route = function(route) { console.log(route.handler({ path: route.path })); }; function server() { return new Server(); } module.exports = { Server: Server, server: server };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as Kit from "server-kit";
function main(): void {
    const server = Kit.server();
    server.route({ method: "GET", path: "/jit", handler: (request: { path: string }) => request.path });
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["server-kit".into()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "/jit\n");
    let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn imports_node_builtin_object_values() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-node-object-values-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { Buffer } from "node:buffer";
           function main(): void { console.log(Number(Buffer.byteLength("thaw"))); }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "4\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An interface whose every field is itself a bare-call-signature
/// interface -- kleur's `interface Kleur { red: Color; ... }` where
/// `Color` is `{ (x): string }` (which thaw-bridge already classifies
/// to `JsValue`). A native `Object` of such fields can't be
/// materialised -- a JSON decode of the Fallback result drops the
/// functions, so `palette.red(...)` hit `invalid JavaScript value
/// handle 0` at run time. `resolve_interface` now collapses an
/// all-`JsValue`-field interface to one opaque `JsValue`, so the
/// Fallback returns a live handle and `palette.red("x")` routes through
/// the existing `JsValue`-receiver dynamic method call.
#[test]
fn an_all_opaque_field_interface_stays_an_opaque_handle() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callable-field-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("palette-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Painter { (text: string): string; }\n\
         export interface Palette { red: Painter; bold: Painter; }\n\
         export declare function makePalette(): Palette;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makePalette: function() { return { \
         red: function(t) { return '[' + t + ']'; }, \
         bold: function(t) { return '<' + t + '>'; } \
         }; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makePalette } from "palette-kit";
function main(): void {
    const p = makePalette();
    console.log(p.red("a"));
    console.log(p.bold("b"));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[a]\n<b>\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An interface that `extends` an already-opaque one (an interface
/// with a bare call signature, or one that itself collapsed via the
/// "every field is `JsValue`" rule directly above) used to crash
/// thaw-bridge outright: `resolve_interface`'s own `extends` handling
/// only expected the base to resolve to `Object`, `Dictionary`, or
/// `Unsupported` -- `unreachable!("resolve_interface always returns an
/// Object or Unsupported")` -- never accounting for the opaque
/// `JsValue` case its own sibling rule can produce. Found via axios:
/// `interface AxiosInstance extends Axios { <call signatures>; ... }`
/// (opaque, from its own call signatures) is itself the `extends`
/// target of `interface AxiosStatic extends AxiosInstance { ... }`.
/// Fixed generally: an interface extending an opaque base is opaque
/// too, regardless of what fields it adds of its own -- there's no
/// concrete `Object` shape to merge inherited fields into.
#[test]
fn an_interface_extending_an_opaque_base_stays_opaque_too() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-opaque-extends-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("palette-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Painter { (text: string): string; }\n\
         export interface Palette { red: Painter; bold: Painter; }\n\
         export interface RichPalette extends Palette { extra: string; }\n\
         export declare function makePalette(): RichPalette;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makePalette: function() { return { \
         red: function(t) { return '[' + t + ']'; }, \
         bold: function(t) { return '<' + t + '>'; }, \
         extra: 'more' \
         }; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makePalette } from "palette-kit";
function main(): void {
    const p: JsValue = makePalette();
    console.log(p.red("a"));
    console.log(p.bold("b"));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[a]\n<b>\n");
    let _ = std::fs::remove_dir_all(dir);
}


/// A Fallback function whose real name is a JS/TS reserved word --
/// real example: joi's `Root.in(ref: string, options?: ReferenceOptions):
/// Reference` (`Joi.in(...)`, a real, documented part of its API) --
/// used to crash thaw-cli's own shim generation outright: the generated
/// `function in(ref: string, options?: Json): JsValue { ... }` bare-alias
/// declaration failed to parse as thaw-hir source ("Expected ident"),
/// since `in` is reserved there for the same reason it is in plain
/// JS/TS. A property access (`pkg.in(...)`) is fine syntactically; only
/// the *bare*, unqualified identifier thaw-cli emits for the untyped/
/// typed bare-name binding isn't. Fixed generally via `is_reserved_js_
/// identifier` -- the package-qualified alias stays reachable
/// (`registry_in(...)`, exercised here through `rewrite_qualified_calls`
/// via the package's own qualifier).
#[test]
fn a_reserved_word_fallback_function_name_stays_reachable_through_its_qualified_alias() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-reserved-word-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("reflike-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "interface Reflike { in(ref: string, options?: string): string; }\n\
         declare const reflike: Reflike;\n\
         export = reflike;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { in: function(ref, options) { return '[' + ref + (options ? (':' + options) : '') + ']'; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import reflike from "reflike-kit";
function main(): void {
    console.log(reflike.in("value"));
    console.log(reflike.in("value", "opt"));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "[value]\n[value:opt]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A live `JsValue` nested inside an object-literal argument passed to
/// a Fallback function whose *matching* parameter is *optional* -- real
/// example: joi's `object(schema?: T)` (`schema`'s declared type renders
/// `Optional(Json)` once its unresolved generic `T` falls back), called
/// as `Joi.object({ name: Joi.string() })`, the exact pattern already
/// supported for zod's `z.object({ name: z.string() })` -- used to fail
/// outright with "a dynamic (JsValue) value can only be passed as an
/// argument to another QuickJS-backed dynamic call". `build_call_with`
/// (an ordinary compiled call to the generated typed wrapper) only
/// enabled that placeholder mechanism when a parameter's *LLVM* type was
/// a bare pointer -- but `Optional`/`Nullable`/`Nullish`/`Union` are this
/// codegen's own tagged *struct* representations, never bare pointers at
/// the top level even when the payload itself is pointer-shaped, so an
/// *optional* Json-ish parameter never triggered it the way a required
/// one already did. Fixed generally in `build_call_with` (also enable
/// the flag for a struct-typed parameter slot).
#[test]
fn a_dynamic_value_nests_inside_an_object_literal_passed_to_an_optional_parameter() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-optional-dynamic-arg-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("schema-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Callable { (value: string): string; }\n\
         export declare function str(): Callable;\n\
         export declare function schema(shape?: any): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = {\n\
         \x20\x20str: function() { return function(value) { return '<' + value + '>'; }; },\n\
         \x20\x20schema: function(shape) { return shape && shape.name ? shape.name('World') : 'empty'; }\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { str, schema } from "schema-kit";
function main(): void {
    console.log(schema({ name: str() }));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "<World>\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `this`-dependent method on a default-exported *stateful* namespace
/// object used to lose its receiver entirely -- real examples: joi's
/// `declare const Joi: Joi.Root`, whose `Root.string()`/`.object()`/etc
/// are real instance methods needing `this === root` (joi's own
/// `internals.generate(this, ...)` asserts "Must be invoked on a Joi
/// instance" otherwise); handlebars' `Handlebars.registerHelper(...)`,
/// which reads `this.helpers`. `extract_interface_method_decls` treats
/// every such interface method exactly like a real free function
/// (correct for zod's `z.string()`, which has no `this` dependency at
/// all) and extracts it into a detached, package-level Fallback
/// function, invoked later as a bare, receiverless `thaw_js_call(name,
/// args)` -- silently dropping the live object the method was declared
/// on. Fixed generally in the JS glue code two separate binding
/// mechanisms in `crates/thaw-bridge/src/bridge/generation.rs` both
/// go through: the `module.exports` -> `globalThis` copy loop in
/// `wrap_as_commonjs_module`, and the `qualified_aliases` capture
/// snippet in `generate_module_init` -- both now `.bind()` a function
/// value to `module.exports` (its own original owner) before handing it
/// off, so a genuinely stateless function (the vast majority of real
/// packages' methods, e.g. lodash's `_.chunk`) is unaffected, while a
/// `this`-dependent one keeps working. Requires *both* fixes together:
/// this package's `count()` reads `this.value`, and either binding
/// mechanism alone (found via inspecting the compiled binary's embedded
/// JS text with `strings`) left the other path still receiverless.
#[test]
fn a_stateful_namespace_objects_method_keeps_its_receiver_when_extracted() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-this-binding-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("statekit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "interface StateKit { count(): number; }\n\
         declare const kit: StateKit;\n\
         export = kit;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "var root = { value: 42 };\n\
         root.count = function() {\n\
         \x20\x20if (!this || this.value === undefined) throw new Error('this is broken');\n\
         \x20\x20return this.value;\n\
         };\n\
         module.exports = root;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import kit from "statekit";
function main(): void {
    console.log(kit.count());
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `value === undefined` (and `!==`, either operand order) for a live
/// `JsValue` unconditionally returned `false` -- even when the value
/// genuinely *is* `undefined` -- because `lower_optional_undefined_
/// equality` had no arm at all for `(JsValue, Undefined)`, only ones for
/// `Optional`/`Json`/`Nullable`/`Nullish`/`Union`; every call silently
/// fell through to the catch-all "an operand's type is `Undefined` and
/// nothing more specific matched -> `false`" arm at the bottom, without
/// ever asking the live value what it actually is. Real trigger: joi's
/// `schema.validate(...)` result -- `.error` is a real, absent property
/// on a valid input's result object, and `result.error === undefined`
/// (the documented way to check joi validated successfully) always
/// came back `false`.
///
/// Fixed by asking the engine directly, the same way `typeof` on a
/// `JsValue` already does (`__thaw_typeof_dynamic_value`): a new
/// bootstrap-registered `__thaw_is_undefined_dynamic_value = value =>
/// value === undefined` (`crates/thaw-quickjs/src/quickjs/
/// platform_globals/runtime.js`), invoked through the same
/// `callDynamicValueWithValue` mechanism.
///
/// Note: this originally only fixed a live `JsValue`, leaving the same
/// symptom open for a plain `Json` value (e.g. from `JSON.parse`) as a
/// separate, deeper gap -- since fixed too, see
/// `a_missing_json_key_or_index_is_distinguishable_from_an_explicit_null`
/// (`thaw-llvm`) and the sibling `JsValue === null`/loose-equality gaps
/// closed alongside it, covered by
/// `a_live_dynamic_values_property_compares_equal_to_null_or_via_loose_equality`
/// below.
#[test]
fn a_live_dynamic_values_property_compares_equal_to_undefined_when_actually_absent() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-dynamic-undefined-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("resultkit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        // Modeled on joi's own `validate(...): ValidationResult<TSchema>` --
        // a generic, unresolvable interface collapses to a single opaque
        // `JsValue` handle (round 6's kleur/joi fix broadened "every field
        // opaque" to "*any* field opaque"; `error`'s bare-call-signature
        // type is what makes it opaque here). A plain `any` return type
        // resolves as `Json` instead, a different code path this test
        // isn't after.
        "export interface Callable { (): void; }\n\
         export interface ValidationResult { error?: Callable; value: any; }\n\
         export declare function makeResult(withError: boolean): ValidationResult;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeResult: function(withError) {\n\
         \x20\x20var result = { value: 1 };\n\
         \x20\x20if (withError) { result.error = 'boom'; }\n\
         \x20\x20return result;\n\
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeResult } from "resultkit";
function main(): void {
    const good: JsValue = makeResult(false);
    const bad: JsValue = makeResult(true);
    console.log(good.error === undefined);
    console.log(undefined === good.error);
    console.log(good.error !== undefined);
    console.log(bad.error === undefined);
    console.log(bad.error !== undefined);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\ntrue\nfalse\nfalse\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Two sibling gaps found while fixing the `Json`-side missing-key-vs-
/// `null` distinction: (1) `value === null`/`null === value` for a live
/// `JsValue` was *also* still broken -- the fix above added a
/// `(JsValue, Undefined)` arm to `lower_optional_undefined_equality` but
/// no `(JsValue, Null)` one, so a live value that's genuinely `null`
/// still fell through to the blanket "false" catch-all; (2) `value ==
/// undefined`/`value == null` (loose equality, either operand order) for
/// a live `JsValue` crashed at *build time* -- `lower_loose_equality`
/// had no `JsValue`-aware arm at all, falling through to
/// `coerce_primitive_to_number`, which errors outright on a bare
/// `Undefined`/`Null` operand. Both fixed the same way as the strict-
/// undefined case: ask the live engine directly, via two more small
/// bootstrap globals (`__thaw_is_null_dynamic_value`,
/// `__thaw_is_nullish_dynamic_value`), reusing the exact
/// `callDynamicValueWithValue` mechanism.
#[test]
fn a_live_dynamic_values_property_compares_equal_to_null_or_via_loose_equality() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-dynamic-null-loose-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("resultkit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Callable { (): void; }\n\
         export interface ValidationResult { error?: Callable; nothing?: Callable; value: any; }\n\
         export declare function makeResult(withError: boolean): ValidationResult;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeResult: function(withError) {\n\
         \x20\x20var result = { value: 1 };\n\
         \x20\x20if (withError) { result.error = 'boom'; } else { result.nothing = null; }\n\
         \x20\x20return result;\n\
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeResult } from "resultkit2";
function main(): void {
    const good: JsValue = makeResult(false);
    const bad: JsValue = makeResult(true);
    console.log(good.error === null);
    console.log(good.error !== null);
    console.log(good.error == null);
    console.log(good.error == undefined);
    console.log(good.nothing === null);
    console.log(good.nothing === undefined);
    console.log(good.nothing == undefined);
    console.log(bad.error === null);
    console.log(bad.error == null);
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "false\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}
