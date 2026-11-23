// A package function overloaded by arity whose optional parameter is a union with an object member
// (validator's `isIP`) must route to thaw-js: no QuickJS fallback, and the program prints what Node prints.

#[test]
fn thaw_js_routes_an_arity_overload_with_an_object_bearing_union_parameter() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-thaw-js-union-object-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("tjs-val");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function isIP(str: string, versionOrOptions?: string | number | { version: string | number | undefined }): boolean;\n\
         export declare function isIP(str: string): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = (function(require){\n\
         var __thaw_bundle_factories = {\n\
         \"tjs-val/index.js\": function(module, exports, require) {\n\
           function isIP(str, versionOrOptions) {\n\
             var version = typeof versionOrOptions === 'object' && versionOrOptions !== null ? versionOrOptions.version : versionOrOptions;\n\
             if (version === undefined) return true;\n\
             var isV6 = str.indexOf(':') >= 0;\n\
             return (version === 6 || version === '6') ? isV6 : !isV6;\n\
           }\n\
           exports.isIP = isIP;\n\
         },\n\
         };\n\
         var __thaw_bundle_require_maps = { \"tjs-val/index.js\": {}, };\n\
         var __thaw_bundle_entry_key = \"tjs-val/index.js\";\n\
         var __thaw_entry = { exports: {} };\n\
         __thaw_bundle_factories[__thaw_bundle_entry_key](__thaw_entry, __thaw_entry.exports, require);\n\
         return __thaw_entry.exports;\n\
         })(globalThis.require);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { isIP } from 'tjs-val';\n\
         const IPS = ['127.0.0.1', '::1', '256.1.1.1'];\n\
         function main(): void {\n\
           console.log(isIP('::1'));\n\
           console.log(isIP('1.2.3.4', 4));\n\
           console.log(isIP('::1', { version: 6 }));\n\
           console.log(isIP('1.2.3.4', 6));\n\
           console.log(isIP('1.2.3.4', { version: 6 }));\n\
           console.log(IPS.map((ip) => isIP(ip)).join(','));\n\
           let acc = 0;\n\
           for (let i = 0; i < 3; i++) { acc = (acc * 31 + (isIP('10.0.' + (i % 256) + '.1') ? 4 : 0)) | 0; }\n\
           console.log(acc);\n\
         }\n",
    )
    .unwrap();
    let output = dir.join("app");
    let built = build_with_thaw_js_routing(
        &entry, &output, &[], &[], &[], &registry, &[], false, None, true, false, false,
    );
    built.unwrap();
    assert_eq!(thaw_js_run(&output), "true\ntrue\ntrue\nfalse\nfalse\ntrue,true,true\n3972\n");
    let _ = std::fs::remove_dir_all(dir);
}
