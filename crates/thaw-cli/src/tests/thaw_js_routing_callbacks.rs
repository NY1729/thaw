// thaw-js routing of native-to-JS callbacks (a TypeScript callback handed to a routed package
// function) and of `catch` bindings: neither may pull QuickJS into a routed build.

/// `tjs-cb`: its functions use `Map` (so the specialization JIT cannot take them) and take their
/// callback as `any` (a `Json` parameter once routed).
fn thaw_js_callback_fixture(name: &str, main: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-{name}-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("tjs-cb");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function mapAll(items: number[], f: any): any;\n\
         export declare function sumBy(items: string[], f: any): number;\n\
         export declare function each(items: number[], f: any): number;\n\
         export declare function nested(items: number[], f: any): any;\n\
         export declare function check(value: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-cb/index.js\": function(module, exports, require) {\n\
           var seen = new Map();\n\
           exports.mapAll = function (items, f) { seen.set('last', items.length); return items.map(function (x) { return f(x); }); };\n\
           exports.sumBy = function (items, f) { var t = 0; items.forEach(function (s) { t += f(s); }); return t; };\n\
           exports.each = function (items, f) { items.forEach(function (x, i) { f(x, i); }); return items.length; };\n\
           exports.nested = function (items, f) { return items.map(function (x) { return exports.mapAll([x, x + 1], f); }); };\n\
           exports.check = function (value) { if (value === 'bad') { var e = new TypeError('bad value'); e.code = 'E_BAD'; throw e; } return value.length; };\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-cb/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-cb/index.js\";\n\
         var __thaw_entry = { exports: {} };\n\
         __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
         return __thaw_entry.exports;\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, main).unwrap();
    (dir.clone(), registry, entry)
}

#[test]
fn thaw_js_routing_serves_a_package_that_calls_back_into_typescript() {
    let (dir, registry, entry) = thaw_js_callback_fixture(
        "callbacks",
        "import { mapAll, sumBy, each, nested } from 'tjs-cb';\n\
         function main(): void {\n\
           let calls = 0;\n\
           console.log(mapAll([1, 2, 3], (x: number): number => x * 2));\n\
           console.log(sumBy(['a', 'bb', 'ccc'], (s: string): number => s.length));\n\
           console.log(each([5, 6], (x: number, i: number): void => { calls += x + i; }), calls);\n\
           console.log(nested([1, 2], (x: number): number => x * x));\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-cb"]));
    std::fs::remove_dir_all(&registry).unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(
        thaw_js_run(&output),
        "[ 2, 4, 6 ]\n6\n2 12\n[ [ 1, 4 ], [ 4, 9 ] ]\n"
    );
    let bytes = std::fs::read(&output).unwrap();
    assert!(!bytes.windows(8).any(|window| window == b"rquickjs"), "QuickJS was linked");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_catches_a_package_error_without_linking_quickjs() {
    let (dir, registry, entry) = thaw_js_callback_fixture(
        "catch",
        "import { check } from 'tjs-cb';\n\
         function main(): void {\n\
           try { console.log(check('ok')); console.log(check('bad')); }\n\
           catch (e: any) { console.log(e.name, e.message, e.code, e instanceof TypeError, e instanceof RangeError, String(e)); }\n\
           finally { console.log('done'); }\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-cb"]));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(
        thaw_js_run(&output),
        "2\nTypeError bad value E_BAD true false TypeError: bad value\ndone\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_keeps_an_any_typed_result_of_a_callback_call_working() {
    // Whether this routes or the compiler rebuilds it on QuickJS, the program prints what Node does.
    let (dir, registry, entry) = thaw_js_callback_fixture(
        "fallback",
        "import { mapAll } from 'tjs-cb';\n\
         function main(): void {\n\
           const out: any = mapAll([1, 2], (x: number): number => x + 1);\n\
           console.log(out, typeof out);\n\
         }\n",
    );
    let output = dir.join("app");
    thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(thaw_js_run(&output), "[ 2, 3 ] object\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `tjs-ctr`: a package class (constructor, instance and static methods, a method returning
/// another instance), kept as handles into the thaw-js heap.
fn thaw_js_class_fixture(name: &str, main: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-{name}-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("tjs-ctr");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Counter {\n\
           constructor(start: number);\n\
           n: number;\n\
           add(n: number): number;\n\
           label(prefix: string): string;\n\
           clone(): Counter;\n\
           merge(other: Counter): number;\n\
           reset(): void;\n\
           static of(n: number): Counter;\n\
           static describe(): string;\n\
         }\n\
         export declare function ping(): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-ctr/index.js\": function(module, exports, require) {\n\
           class Counter {\n\
             constructor(start) { if (start < 0) throw new RangeError('negative start'); this.n = start; this.log = new Map(); }\n\
             add(n) { this.n += n; this.log.set(this.log.size, n); return this.n; }\n\
             label(prefix) { return prefix + this.n; }\n\
             clone() { return new Counter(this.n); }\n\
             merge(other) { this.n += other.n; return this.n; }\n\
             reset() { this.n = 0; }\n\
             static of(n) { return new Counter(n); }\n\
             static describe() { return 'Counter'; }\n\
           }\n\
           exports.Counter = Counter;\n\
           exports.ping = function () { return 'pong'; };\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-ctr/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-ctr/index.js\";\n\
         var __thaw_entry = { exports: {} };\n\
         __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
         return __thaw_entry.exports;\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, main).unwrap();
    (dir.clone(), registry, entry)
}

#[test]
fn thaw_js_routing_keeps_package_class_instances_in_the_thaw_js_heap() {
    let (dir, registry, entry) = thaw_js_class_fixture(
        "classes",
        "import { Counter } from 'tjs-ctr';\n\
         function main(): void {\n\
           const c = new Counter(5);\n\
           console.log(c.add(3));\n\
           console.log(c.label('n='));\n\
           const d = c.clone();\n\
           console.log(d.add(1), c.add(0));\n\
           console.log(c.merge(d));\n\
           console.log(Counter.describe());\n\
           c.reset();\n\
           console.log(c.label('after reset: '));\n\
           try { new Counter(-1); } catch (e: any) { console.log(e.name, e.message); }\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-ctr"]));
    std::fs::remove_dir_all(&registry).unwrap();
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(
        thaw_js_run(&output),
        "8\nn=8\n9 8\n17\nCounter\nafter reset: 0\nRangeError negative start\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_falls_back_when_a_class_instance_needs_live_property_access() {
    // A property write (and the read after it) is not one of the typed thaw-js class operations:
    // the program is rebuilt on QuickJS and still prints what Node prints.
    let (dir, registry, entry) = thaw_js_class_fixture(
        "class-property",
        "import { Counter } from 'tjs-ctr';\n\
         function main(): void {\n\
           const c = new Counter(5);\n\
           c.n = 40;\n\
           console.log(c.add(2), c.n);\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], true, "{manifest}");
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!([]));
    assert_eq!(thaw_js_run(&output), "42 42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn thaw_js_routing_passes_class_instances_back_into_the_package() {
    let (dir, registry, entry) = thaw_js_class_fixture(
        "class-argument",
        "import { Counter } from 'tjs-ctr';\n\
         function main(): void {\n\
           const a = new Counter(1);\n\
           const b = a.clone();\n\
           b.add(10);\n\
           console.log(a.merge(b), a.merge(a.clone()), b.label('b='));\n\
         }\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    assert_eq!(thaw_js_run(&output), "12 24 b=11\n");
    let _ = std::fs::remove_dir_all(dir);
}
