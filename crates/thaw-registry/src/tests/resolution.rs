#[test]
fn selects_package_exports_conditions_for_runtime_and_types() {
    let manifest: serde_json::Value = serde_json::from_str(
        r#"{
                "main": "legacy.js",
                "types": "legacy.d.ts",
                "exports": {
                    ".": {
                        "types": "./dist/index.d.ts",
                        "import": "./dist/index.mjs",
                        "require": "./dist/index.cjs",
                        "default": "./dist/index.js"
                    },
                    "./feature": {
                        "types": "./dist/feature.d.ts",
                        "require": "./dist/feature.cjs"
                    }
                }
            }"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&manifest, None, &["require", "import", "default"]),
        Some("./dist/index.cjs")
    );
    assert_eq!(
        package_export_target(&manifest, None, &["types"]),
        Some("./dist/index.d.ts")
    );
    assert_eq!(
        package_export_target(&manifest, Some("feature"), &["types"]),
        Some("./dist/feature.d.ts")
    );

    let array: serde_json::Value = serde_json::from_str(
        r#"{"exports":{".":[null,{"types":"./fallback.d.ts","require":"./fallback.cjs"}]}}"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&array, None, &["types"]),
        Some("./fallback.d.ts")
    );
    assert_eq!(
        package_export_target(&array, None, &["require", "default"]),
        Some("./fallback.cjs")
    );

    let nested: serde_json::Value = serde_json::from_str(
        r#"{"exports":{"./package.json":"./package.json",".":{"require":{"types":"./index.d.cts","default":"./index.cjs"},"import":{"types":"./index.d.ts","default":"./index.js"}}}}"#,
    )
    .unwrap();
    assert_eq!(
        package_export_target(&nested, None, &["types"]),
        Some("./index.d.cts")
    );
    assert_eq!(
        package_export_target(&nested, Some("package.json"), &["types"]),
        None
    );
}

#[test]
fn expands_wildcard_package_exports_from_type_files() {
    let package_dir = temp_registry("wildcard-exports");
    fs::create_dir_all(package_dir.join("dist/features")).unwrap();
    fs::write(package_dir.join("dist/features/alpha.d.ts"), "").unwrap();
    fs::write(package_dir.join("dist/features/alpha.cjs"), "").unwrap();
    fs::write(package_dir.join("dist/features/beta.d.ts"), "").unwrap();
    fs::write(package_dir.join("dist/features/beta.cjs"), "").unwrap();
    let manifest: serde_json::Value = serde_json::from_str(
            r#"{"exports":{"./features/*":{"types":"./dist/features/*.d.ts","require":"./dist/features/*.cjs"}}}"#,
        )
        .unwrap();
    assert_eq!(
        package_subpath_exports(&manifest, &package_dir).unwrap(),
        vec![
            PackageSubpathExport {
                subpath: "features/alpha".to_string(),
                runtime_entry: "./dist/features/alpha.cjs".to_string(),
                types_entry: "./dist/features/alpha.d.ts".to_string(),
            },
            PackageSubpathExport {
                subpath: "features/beta".to_string(),
                runtime_entry: "./dist/features/beta.cjs".to_string(),
                types_entry: "./dist/features/beta.d.ts".to_string(),
            },
        ]
    );
    let _ = fs::remove_dir_all(package_dir);
}

#[test]
fn installed_npm_layout_registers_wildcard_subpath_artifacts() {
    let scratch = temp_registry("installed-wildcard-scratch");
    let registry = temp_registry("installed-wildcard-registry");
    let package = scratch.join("node_modules/feature-kit");
    fs::create_dir_all(package.join("dist/features")).unwrap();
    fs::create_dir_all(package.join("dist/internal")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export declare function root(): number;",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { root: function() { return 1; } };",
    )
    .unwrap();
    fs::write(
        package.join("dist/features/double.d.ts"),
        "export { double } from '../internal/double-api';",
    )
    .unwrap();
    fs::write(
        package.join("dist/internal/double-api.d.ts"),
        "export declare function double(value: number): number;",
    )
    .unwrap();
    fs::write(
        package.join("dist/features/double.js"),
        "module.exports = { double: function(value) { return value * 2; } };",
    )
    .unwrap();

    let added = add_installed(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
    assert_eq!(added.resolved_version, "1.2.3");
    let subpath = resolve(&registry, "feature-kit/features/double").unwrap();
    assert!(subpath
        .dts_source
        .contains("declare function double(value: number): number"));
    assert!(subpath.bundle_js.unwrap().contains("value * 2"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_root_defers_subpaths_until_requested() {
    let scratch = temp_registry("lazy-subpath-scratch");
    let registry = temp_registry("lazy-subpath-registry");
    let package = scratch.join("node_modules/feature-kit");
    fs::create_dir_all(package.join("features")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"feature-kit","types":"./index.d.ts","main":"./index.js","exports":{".":{"types":"./index.d.ts","require":"./index.js"},"./double":{"types":"./features/double.d.ts","require":"./features/double.js"}}}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export declare const root: number;").unwrap();
    fs::write(package.join("index.js"), "exports.root = 1;").unwrap();
    fs::write(
        package.join("features/double.d.ts"),
        "export declare function double(value: number): number;",
    )
    .unwrap();
    fs::write(
        package.join("features/double.js"),
        "exports.double = function(value) { return value * 2; };",
    )
    .unwrap();

    add_installed_root(&registry, &scratch.join("node_modules"), "feature-kit").unwrap();
    assert!(resolve(&registry, "feature-kit").is_ok());
    assert!(resolve(&registry, "feature-kit/double").is_err());

    add_installed_subpath(
        &registry,
        &scratch.join("node_modules"),
        "feature-kit/double",
    )
    .unwrap();
    let subpath = resolve(&registry, "feature-kit/double").unwrap();
    assert!(subpath.bundle_js.unwrap().contains("value * 2"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_named_function_reexports() {
    let scratch = temp_registry("installed-dts-reexport-scratch");
    let registry = temp_registry("installed-dts-reexport-registry");
    let package = scratch.join("node_modules/parser-kit");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"parser-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export { parse, stringify as encode } from './public-api';\nexport { loop } from './cycle-a';\nexport * from './all';",
    )
    .unwrap();
    fs::write(
        package.join("dist/public-api.d.ts"),
        "export { parse } from './parse';\nexport { stringify } from './stringify';",
    )
    .unwrap();
    fs::write(
        package.join("dist/parse.d.ts"),
        "export declare function parse(value: string): any;",
    )
    .unwrap();
    fs::write(
        package.join("dist/stringify.d.ts"),
        "export declare function stringify(value: any): string;",
    )
    .unwrap();
    fs::write(
        package.join("dist/cycle-a.d.ts"),
        "export { loop } from './cycle-b';",
    )
    .unwrap();
    fs::write(
        package.join("dist/cycle-b.d.ts"),
        "export { loop } from './cycle-a';",
    )
    .unwrap();
    fs::write(
        package.join("dist/all.d.ts"),
        "export declare function decode(value: string): any;\nexport * from './all-cycle';",
    )
    .unwrap();
    fs::write(
        package.join("dist/all-cycle.d.ts"),
        "export * from './all';",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { parse: JSON.parse, encode: JSON.stringify };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "parser-kit").unwrap();
    let declarations = resolve(&registry, "parser-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function parse(value: string): any"));
    assert!(declarations.contains("declare function encode(value: any): string"));
    assert!(declarations.contains("declare function decode(value: string): any"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_default_reexports() {
    // `export { default as v4 } from './v4'` -- the common shape for a
    // package that splits one function per file and re-exports each one
    // under a name from a barrel (e.g. real-world `uuid`). The re-exported
    // name is never literally declared as `default` anywhere, so following
    // it means resolving `export default v4;` back to the real identifier
    // `v4` first and matching *that* against the plain (non-exported)
    // `declare function v4(...)` sitting next to it.
    let scratch = temp_registry("installed-dts-default-reexport-scratch");
    let registry = temp_registry("installed-dts-default-reexport-registry");
    let package = scratch.join("node_modules/id-kit");
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"id-kit","version":"1.0.0","types":"./dist/index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("dist/index.d.ts"),
        "export { default as v4 } from './v4';\nexport { default as validate } from './validate';",
    )
    .unwrap();
    fs::write(
        package.join("dist/v4.d.ts"),
        "declare function v4(options?: unknown): string;\nexport default v4;",
    )
    .unwrap();
    fs::write(
        package.join("dist/validate.d.ts"),
        // The less common inline shape (`export default function ...`),
        // covered alongside the `export default <ident>;` shape above.
        "export default function validate(value: unknown): boolean;",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { v4: function() { return 'id'; }, validate: function(x) { return true; } };",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "id-kit").unwrap();
    let declarations = resolve(&registry, "id-kit").unwrap().dts_source;
    assert!(declarations.contains("declare function v4(options?: unknown): string"));
    assert!(declarations.contains("function validate(value: unknown): boolean"));
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_named_import_reexports() {
    // `import { Name } from './path'; export { Name };` (ordinary ES
    // import + a *local* re-export, no `from` clause on the export
    // itself) -- real-world example: hono's `index.d.ts`, which does
    // exactly this for its `Hono` class (`import { Hono } from './hono';
    // export { Hono };`). Unlike `export { Name } from './path'` (a
    // single statement, already handled), the import and export are two
    // separate statements here, and `Name` can be a class/interface, not
    // just a function.
    let scratch = temp_registry("installed-dts-named-import-scratch");
    let registry = temp_registry("installed-dts-named-import-registry");
    let package = scratch.join("node_modules/web-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"web-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import { App } from './app';\nimport { route as routeFn } from './route';\nexport { App, routeFn as route };\nexport { Context } from './context';\n",
    )
    .unwrap();
    fs::write(
        package.join("app.d.ts"),
        "import { Base } from './base';\nexport declare class App<T = unknown> extends Base {\n    constructor(base?: string);\n    get(path: string): T;\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("base.d.ts"),
        "declare class InternalBase {\n    request(path: string): Promise<Response>;\n}\nexport { InternalBase as Base };\n",
    )
    .unwrap();
    fs::write(
        package.join("route.d.ts"),
        "export declare function route(path: string): string;\n",
    )
    .unwrap();
    fs::write(
        package.join("context.d.ts"),
        "export declare class Context {\n    text(value: string): Response;\n}\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { App: function() {}, route: function(p) { return p; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "web-kit").unwrap();
    let declarations = resolve(&registry, "web-kit").unwrap().dts_source;
    assert!(
        declarations.contains("class App"),
        "{declarations}"
    );
    assert!(
        declarations.contains("class Base")
            && declarations.contains("request(path: string): Promise<Response>"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function route(path: string): string"),
        "{declarations}"
    );
    assert!(declarations.contains("class Context"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_types_through_wildcard_barrels() {
    let scratch = temp_registry("installed-dts-wildcard-types-scratch");
    let registry = temp_registry("installed-dts-wildcard-types-registry");
    let package = scratch.join("node_modules/service-kit");
    fs::create_dir_all(package.join("types")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"service-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export * from './types';\n").unwrap();
    fs::write(
        package.join("types/index.d.ts"),
        "export type * from './server';\n",
    )
    .unwrap();
    fs::write(
        package.join("types/server.d.ts"),
        "export interface Result { status: number; }\n\
         export type Handler = (value: string) => Result;\n\
         export class Server { run(handler: Handler): Promise<Result>; }\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = {};\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "service-kit").unwrap();
    let declarations = resolve(&registry, "service-kit").unwrap().dts_source;
    assert!(declarations.contains("interface Result"), "{declarations}");
    assert!(declarations.contains("type Handler"), "{declarations}");
    assert!(declarations.contains("class Server"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_local_bare_declaration_reexported_under_a_reserved_word_alias() {
    // `declare function _enum(...)` (no `export` prefix at all) followed
    // by a separate, *same-file* `export { _enum as enum };` -- real-
    // world example: zod v4's own `schemas.d.cts`, which declares this
    // exact shape (two overloads) because `enum` is a reserved word and
    // can't be the function's own declared name. Neither the existing
    // `import_equals_targets` nor `named_import_targets` lookup covers
    // this: `_enum` isn't bound via any import at all, just declared
    // directly in the same file. Also covers the reserved-word alias
    // itself (`null`) that must be *skipped*, not emitted as invalid
    // syntax (`declare function null(...)` is a parse error, not just
    // an unusual name) -- confirmed it doesn't poison the rest of the
    // file's declarations.
    let scratch = temp_registry("installed-dts-local-bare-reexport-scratch");
    let registry = temp_registry("installed-dts-local-bare-reexport-registry");
    let package = scratch.join("node_modules/case-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export * from './schemas';\n",
    )
    .unwrap();
    fs::write(
        package.join("schemas.d.ts"),
        "declare function _enum(values: readonly string[]): string;\n\
         declare function _enum(entries: Record<string, string>): string;\n\
         export { _enum as enum };\n\
         declare function _null(): string;\n\
         export { _null as null };\n\
         export declare function string(): string;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { enum: function() { return 'enum'; }, string: function() { return 'string'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit").unwrap();
    let declarations = resolve(&registry, "case-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare function enum(values: readonly string[]): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare function enum(entries: Record<string, string>): string"),
        "{declarations}"
    );
    assert!(
        !declarations.contains("function null("),
        "reserved-word alias should be skipped, not emitted as invalid syntax: {declarations}"
    );
    assert!(
        declarations.contains("function string(): string"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_a_callable_const_reexported_from_another_file() {
    let scratch = temp_registry("installed-dts-reexported-callable-const-scratch");
    let registry = temp_registry("installed-dts-reexported-callable-const-registry");
    let package = scratch.join("node_modules/server-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"server-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export { serve } from './server.js';\n",
    )
    .unwrap();
    fs::write(
        package.join("server.d.ts"),
        "declare const serve: (options: { port?: number }) => unknown;\nexport { serve };\n",
    )
    .unwrap();
    fs::write(package.join("index.js"), "module.exports = { serve() {} };\n").unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "server-kit").unwrap();
    let declarations = resolve(&registry, "server-kit").unwrap().dts_source;
    assert!(declarations.contains("const serve:"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_flattens_a_nested_namespace_reexport() {
    // `export * as NAME from "SOURCE";` -- a namespace re-export, real-
    // world example: zod v4's own re-export barrel, `export * as coerce
    // from "./coerce.cjs";` (alongside `export * as core from
    // "../core/index.cjs";`, `export * as iso from "./iso.cjs";`, and
    // `export * as locales from "../locales/index.cjs";`), reached here
    // one file below the entry point's own (transitively, through a
    // plain `export * from "./external";`) -- the shape
    // `collect_namespace_reexports` recurses through. Each of `coerce`'s
    // own functions gets flattened under a synthesized, collision-free
    // top-level name (here `string`/`number` collide with the package's
    // own top-level `string`/`number` functions, exactly like zod's real
    // `coerce.number` vs. top-level `number`), plus a `declare namespace
    // coerce { export { ... }; }` block recording the mapping back --
    // see `thaw_bridge::nested_namespace_members`, which parses this
    // exact shape back out.
    let scratch = temp_registry("installed-dts-nested-namespace-scratch");
    let registry = temp_registry("installed-dts-nested-namespace-registry");
    let package = scratch.join("node_modules/case-kit5");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit5","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "export * from './external';\n",
    )
    .unwrap();
    fs::write(
        package.join("external.d.ts"),
        "export declare function string(): string;\n\
         export declare function number(): number;\n\
         export * as coerce from './coerce';\n",
    )
    .unwrap();
    fs::write(
        package.join("coerce.d.ts"),
        "export declare function string(): string;\n\
         export declare function number(): number;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { string: function() { return 'top'; }, number: function() { return 0; }, coerce: { string: function() { return 'coerced'; }, number: function() { return 1; } } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit5").unwrap();
    let declarations = resolve(&registry, "case-kit5").unwrap().dts_source;
    assert!(
        declarations.contains("function string(): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function number(): number"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare namespace coerce {"),
        "{declarations}"
    );
    // The namespace's members are synthesized, collision-free names --
    // not bare `string`/`number` (those are already taken by the
    // package's own top-level functions above) -- re-exported back to
    // their real member name via `export { synthetic as member }`.
    assert!(
        declarations.contains("function __thaw_ns_coerce_string(): string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function __thaw_ns_coerce_number(): number"),
        "{declarations}"
    );
    assert!(
        declarations.contains("__thaw_ns_coerce_string as string"),
        "{declarations}"
    );
    assert!(
        declarations.contains("__thaw_ns_coerce_number as number"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_carries_a_self_referential_namespace_alias_from_a_transitively_reexported_file() {
    // `import * as z from "SOURCE"; export { z }; export default z;` --
    // `thaw_bridge::self_referential_namespace_aliases`'s own shape, but
    // declared one file *below* the package's own entry point (reached
    // only transitively through a plain `export * from "./external";`),
    // not in the entry point itself. Real example: zod v3's own
    // `lib/index.d.ts` (`index.d.ts`, the package's entry point, is just
    // `export * from "./lib";`) -- unlike zod v4, whose entry point
    // declares this alias directly and so needed no special handling.
    // Without carrying this snippet into the flattened output, `import {
    // z } from "zod"` failed outright ("no export named `z`") even though
    // `z`'s own methods (`object`, `string`, ...) were all individually
    // reachable by their own bare names.
    let scratch = temp_registry("installed-dts-self-referential-alias-scratch");
    let registry = temp_registry("installed-dts-self-referential-alias-registry");
    let package = scratch.join("node_modules/case-kit6");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"case-kit6","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(package.join("index.d.ts"), "export * from './external';\n").unwrap();
    fs::write(
        package.join("external.d.ts"),
        "import * as z from './z';\n\
         export declare function greet(): string;\n\
         export { z };\n\
         export default z;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { greet: function() { return 'hi'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "case-kit6").unwrap();
    let declarations = resolve(&registry, "case-kit6").unwrap().dts_source;
    assert!(
        declarations.contains("import * as z from './z';") || declarations.contains("import * as z from \"./z\";"),
        "{declarations}"
    );
    assert!(declarations.contains("export { z };"), "{declarations}");
    assert!(declarations.contains("export default z;"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_import_equals_reexports() {
    // `import Name = require("./path")` (a TS import-equals declaration)
    // followed by a *local* `export { Name as exported };` (no `from`
    // clause -- `Name` is already a value bound earlier in the same
    // file) -- real-world example: semver's `index.d.ts`, which imports
    // one function per file this way and re-exports every one of them
    // together. `Name` resolves through its own file's `export = X;` to
    // the identifier actually declared there, one file per function, so
    // (unlike the `export { default as x } from './x'` shape) each
    // specifier in the same `export { ... }` can resolve to a different
    // file.
    let scratch = temp_registry("installed-dts-import-equals-scratch");
    let registry = temp_registry("installed-dts-import-equals-registry");
    let package = scratch.join("node_modules/ver-kit");
    fs::create_dir_all(package.join("functions")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"ver-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import verValid = require('./functions/valid');\n\
         import verMajor = require('./functions/major');\n\
         export { verValid as valid, verMajor as major };\n",
    )
    .unwrap();
    fs::write(
        package.join("functions/valid.d.ts"),
        "declare function valid(version: string): string | null;\nexport = valid;\n",
    )
    .unwrap();
    fs::write(
        package.join("functions/major.d.ts"),
        "declare function major(version: string): number;\nexport = major;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { valid: function(v) { return v; }, major: function(v) { return 1; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "ver-kit").unwrap();
    let declarations = resolve(&registry, "ver-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare function valid(version: string): string | null"),
        "{declarations}"
    );
    assert!(
        declarations.contains("function major(version: string): number"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_export_import_member_reexports() {
    // `export import NAME = BASE.MEMBER;` -- a TS import-equals
    // declaration whose module reference is a qualified *entity name* (a
    // property access into an already-imported value), not a
    // `require(...)` call (the shape the test above covers). Real
    // example: uuid@8.3.2's real `.d.ts` (via `@types/uuid`), `import
    // uuid from "./index.js"; export import v1 = uuid.v1; export import
    // validate = uuid.validate; ...`. `validate`'s own declared type
    // (`export const validate: validate;`) is itself a *local, unexported*
    // type alias (`type validate = (uuid: string) => boolean;`) in
    // `./index.js`'s own `.d.ts` -- not an interface, not a direct
    // function type on the const itself.
    let scratch = temp_registry("installed-dts-export-import-member-scratch");
    let registry = temp_registry("installed-dts-export-import-member-registry");
    let package = scratch.join("node_modules/uuid-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"uuid-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import uuid from './impl';\n\
         export import validate = uuid.validate;\n",
    )
    .unwrap();
    fs::write(
        package.join("impl.d.ts"),
        "export {};\n\
         type validate = (uuid: string) => boolean;\n\
         export const validate: validate;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { validate: function(id) { return id.length === 36; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "uuid-kit").unwrap();
    let declarations = resolve(&registry, "uuid-kit").unwrap().dts_source;
    assert!(
        declarations.contains("export const validate: validate;"),
        "{declarations}"
    );
    assert!(
        declarations.contains("type validate = (uuid: string) => boolean;"),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_the_type_an_import_equals_value_is_declared_with() {
    // `import Name = require("./path")` used as a *type* reference
    // (`declare const x: Name;`), not the value re-export
    // `installed_package_inlines_import_equals_reexports` already
    // covers -- real-world example: mime's `import Mime =
    // require("./Mime"); declare const mime: Mime; export = mime;`,
    // `Mime` itself an ambient class declared in a wholly different
    // file. Without following that reference, `Mime`'s class body
    // (needed to make `mime.getType(...)` classifiable at all) never
    // reaches the flattened `.d.ts`.
    let scratch = temp_registry("installed-dts-import-equals-type-scratch");
    let registry = temp_registry("installed-dts-import-equals-type-registry");
    let package = scratch.join("node_modules/type-kit");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"type-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "import Thing = require('./Thing');\n\
         declare const thing: Thing;\n\
         export = thing;\n",
    )
    .unwrap();
    fs::write(
        package.join("Thing.d.ts"),
        "declare class Thing {\n    label(): string;\n}\n\nexport = Thing;\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { label: function() { return 'thing'; } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "type-kit").unwrap();
    let declarations = resolve(&registry, "type-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare class Thing"),
        "{declarations}"
    );
    assert!(declarations.contains("label(): string"), "{declarations}");
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn installed_package_inlines_triple_slash_referenced_declarations() {
    // `/// <reference path="..." />` -- the classic DefinitelyTyped-style
    // split for a package whose real API is spread across many files
    // (real-world example: `@types/lodash`'s `index.d.ts` referencing a
    // dozen files under `common/`). A referenced file's own module
    // augmentation targeting the entry file itself (`declare module
    // "../index" { interface LoDashStatic { ... } }`) should land inside
    // the *entry file's own* exported namespace (`declare namespace
    // <name> { ... }`, `<name>` from `export as namespace <name>;`) once
    // inlined, since that's the namespace `export = <const>` actually
    // exposes; an augmentation aimed at some other module is kept as its
    // own (unresolved but harmless) `declare module "..." { ... }`.
    let scratch = temp_registry("installed-dts-triple-slash-scratch");
    let registry = temp_registry("installed-dts-triple-slash-registry");
    let package = scratch.join("node_modules/stat-kit");
    fs::create_dir_all(package.join("common")).unwrap();
    fs::write(
        package.join("package.json"),
        r#"{"name":"stat-kit","version":"1.0.0","types":"./index.d.ts","main":"./index.js"}"#,
    )
    .unwrap();
    fs::write(
        package.join("index.d.ts"),
        "/// <reference path=\"./common/array.d.ts\" />\n\
         export = _;\n\
         export as namespace _;\n\
         declare const _: _.StatStatic;\n\
         declare namespace _ {\n\
             interface StatStatic {}\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("common/array.d.ts"),
        "declare module \"../index\" {\n\
             interface StatStatic {\n\
                 sum(values: number[]): number;\n\
             }\n\
         }\n\
         declare module \"unrelated-package\" {\n\
             function untouched(): void;\n\
         }\n",
    )
    .unwrap();
    fs::write(
        package.join("index.js"),
        "module.exports = { sum: function(values) { return values.reduce(function(a, b) { return a + b; }, 0); } };\n",
    )
    .unwrap();

    add_installed(&registry, &scratch.join("node_modules"), "stat-kit").unwrap();
    let declarations = resolve(&registry, "stat-kit").unwrap().dts_source;
    assert!(
        declarations.contains("declare namespace _"),
        "{declarations}"
    );
    assert!(
        declarations.contains("sum(values: number[]): number"),
        "{declarations}"
    );
    assert!(
        declarations.contains("declare module \"unrelated-package\""),
        "{declarations}"
    );
    let _ = fs::remove_dir_all(scratch);
    let _ = fs::remove_dir_all(registry);
}

#[test]
fn resolves_an_installed_package_subpath() {
    let registry = temp_registry("subpath");
    let dir = registry.join("math-kit/subpaths/advanced");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("package.d.ts"),
        "export declare function square(value: number): number;",
    )
    .unwrap();
    fs::write(
        dir.join("bundle.js"),
        "module.exports = { square: function(value) { return value * value; } };",
    )
    .unwrap();

    let package = resolve(&registry, "math-kit/advanced").unwrap();
    assert_eq!(package.name, "math-kit/advanced");
    assert!(package.dts_source.contains("square"));
    assert!(package.bundle_js.unwrap().contains("value * value"));
    let _ = fs::remove_dir_all(registry);
}

fn temp_registry(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "thaw-registry-test-{test_name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn resolves_a_package_with_all_runtime_backends() {
    let registry = temp_registry("full");
    let pkg_dir = registry.join("left-pad");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function pad(s: string): string;",
    )
    .unwrap();
    fs::write(pkg_dir.join("native.a"), b"fake archive").unwrap();
    fs::write(pkg_dir.join("native.node"), b"fake addon").unwrap();
    fs::write(pkg_dir.join("bundle.js"), "function pad(s){return s;}").unwrap();
    fs::write(pkg_dir.join("version.txt"), "1.3.0").unwrap();

    let resolved = resolve(&registry, "left-pad").unwrap();
    assert_eq!(resolved.name, "left-pad");
    assert!(resolved.dts_source.contains("declare function pad"));
    assert_eq!(resolved.native_lib, Some(pkg_dir.join("native.a")));
    assert_eq!(resolved.native_addon, Some(pkg_dir.join("native.node")));
    assert_eq!(
        resolved.bundle_js.as_deref(),
        Some("function pad(s){return s;}")
    );
    assert_eq!(resolved.version.as_deref(), Some("1.3.0"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn selects_the_current_targets_bundled_node_prebuild() {
    let package = temp_registry("select_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    let filename = if libc == "musl" {
        "binding.musl.node"
    } else {
        "binding.node"
    };
    fs::write(target.join(filename), b"native bytes").unwrap();
    // The opposite Linux libc must not be selected accidentally.
    if platform == "linux" {
        let opposite = if libc == "musl" {
            "binding.node"
        } else {
            "binding.musl.node"
        };
        fs::write(target.join(opposite), b"wrong libc").unwrap();
    }

    let selected = select_prebuilt_addon(&package).unwrap().unwrap();
    assert_eq!(selected.path, target.join(filename));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(package);
}

#[test]
fn prefers_node_over_electron_prebuilds() {
    let package = temp_registry("prefer_node_prebuild");
    let (platform, arch, _) = target_prebuild_components();
    let target = package.join("prebuilds").join(format!("{platform}-{arch}"));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("electron.napi.node"), b"electron").unwrap();
    fs::write(target.join("node.napi.node"), b"node").unwrap();

    let selected = select_prebuilt_addon(&package).unwrap().unwrap();
    assert_eq!(selected.path, target.join("node.napi.node"));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn builds_prebuild_install_github_asset_for_the_current_target() {
    let manifest = serde_json::json!({
        "name": "sqlite3",
        "version": "5.1.7",
        "repository": {
            "type": "git",
            "url": "git+https://github.com/TryGhost/node-sqlite3.git"
        },
        "binary": { "napi_versions": [3, 6, 99] }
    });
    let (url, asset, platform, arch) = prebuild_install_asset(&manifest).unwrap();
    let (target_platform, target_arch, libc) = target_prebuild_components();
    let asset_platform = if target_platform == "linux" && libc == "musl" {
        "linuxmusl"
    } else {
        target_platform
    };
    assert_eq!(
        asset,
        format!("sqlite3-v5.1.7-napi-v6-{asset_platform}-{target_arch}.tar.gz")
    );
    assert_eq!(
        url,
        format!("https://github.com/TryGhost/node-sqlite3/releases/download/v5.1.7/{asset}")
    );
    assert_eq!(platform, asset_platform);
    assert_eq!(arch, target_arch);
}

#[test]
fn reports_available_targets_when_no_prebuild_matches() {
    let package = temp_registry("mismatched_native_prebuild");
    let target = package.join("prebuilds/imaginary-other");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("binding.node"), b"native bytes").unwrap();
    let diagnostic = select_prebuilt_addon(&package).unwrap_err();
    assert!(diagnostic.contains("no bundled native addon matches"));
    assert!(diagnostic.contains("imaginary-other"));
    let _ = fs::remove_dir_all(package);
}

#[test]
fn selects_a_platform_optional_dependency_node_addon() {
    let node_modules = temp_registry("optional_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let dependency = if platform == "linux" {
        format!("@example/addon-{platform}-{arch}-{libc}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let dependency_dir = node_modules.join(&dependency);
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        dependency_dir.join("package.json"),
        format!(r#"{{"name":"{dependency}","main":"binding.node"}}"#),
    )
    .unwrap();
    fs::write(dependency_dir.join("binding.node"), b"native bytes").unwrap();
    let manifest = serde_json::json!({
        "optionalDependencies": { dependency.clone(): "1.0.0" }
    });
    let selected = select_optional_dependency_addon(&node_modules, &manifest)
        .unwrap()
        .unwrap();
    assert_eq!(selected.path, dependency_dir.join("binding.node"));
    assert_eq!(selected.source, format!("{dependency}/binding.node"));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn selects_a_node_addon_behind_a_platform_dependency_js_entry() {
    let node_modules = temp_registry("optional_wrapped_native_prebuild");
    let (platform, arch, libc) = target_prebuild_components();
    let dependency = if platform == "linux" && libc == "musl" {
        format!("@example/addon-linuxmusl-{arch}")
    } else {
        format!("@example/addon-{platform}-{arch}")
    };
    let dependency_dir = node_modules.join(&dependency);
    fs::create_dir_all(dependency_dir.join("lib")).unwrap();
    fs::write(
        dependency_dir.join("package.json"),
        format!(r#"{{"name":"{dependency}","main":"index.js"}}"#),
    )
    .unwrap();
    fs::write(dependency_dir.join("index.js"), "module.exports = require('./lib/addon.node')").unwrap();
    fs::write(dependency_dir.join("lib/addon.node"), b"native bytes").unwrap();
    let manifest = serde_json::json!({
        "optionalDependencies": { dependency.clone(): "1.0.0" }
    });

    let selected = select_optional_dependency_addon(&node_modules, &manifest)
        .unwrap()
        .unwrap();
    assert_eq!(selected.path, dependency_dir.join("lib/addon.node"));
    assert_eq!(selected.platform, platform);
    assert_eq!(selected.arch, arch);
    assert_eq!(selected.libc, libc);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn resolves_a_packages_lock_json_when_present() {
    let registry = temp_registry("with_lock");
    let pkg_dir = registry.join("qs");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function stringify(x: string): string;",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("bundle.js"),
        "function stringify(x){return x;}",
    )
    .unwrap();
    fs::write(pkg_dir.join("version.txt"), "6.11.0").unwrap();
    fs::write(
        pkg_dir.join("lock.json"),
        r#"{"qs": "6.11.0", "side-channel": "1.0.4"}"#,
    )
    .unwrap();

    let resolved = resolve(&registry, "qs").unwrap();
    let deps = resolved.dependency_versions.expect("lock.json was written");
    assert_eq!(deps.get("qs").map(String::as_str), Some("6.11.0"));
    assert_eq!(deps.get("side-channel").map(String::as_str), Some("1.0.4"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn resolves_a_package_with_only_the_required_dts() {
    let registry = temp_registry("dts_only");
    let pkg_dir = registry.join("is-odd");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("package.d.ts"),
        "export declare function isOdd(n: number): boolean;",
    )
    .unwrap();

    let resolved = resolve(&registry, "is-odd").unwrap();
    assert!(resolved.native_lib.is_none());
    assert!(resolved.bundle_js.is_none());
    // A hand-curated package (or one `add`ed before `version.txt`
    // existed) has no version on record -- not an error, just unknown.
    assert!(resolved.version.is_none());

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn errors_when_package_dts_is_missing() {
    let registry = temp_registry("missing_dts");
    let pkg_dir = registry.join("ghost");
    fs::create_dir_all(&pkg_dir).unwrap();

    let err = resolve(&registry, "ghost").unwrap_err();
    assert!(err.contains("ghost"));
    assert!(err.contains("package.d.ts"));

    let _ = fs::remove_dir_all(&registry);
}

#[test]
fn errors_when_package_directory_does_not_exist() {
    let registry = temp_registry("no_such_pkg");
    let err = resolve(&registry, "nonexistent").unwrap_err();
    assert!(err.contains("nonexistent"));

    let _ = fs::remove_dir_all(&registry);
}

/// `add`'s actual `npm install` step needs network access and isn't
/// exercised by the automated suite (consistent with this project's
/// other network-touching work, which was validated manually rather
/// than in `cargo test` -- see docs/design/registry.md). `find_own_dts`/
/// `types_package_name` are the pieces of `add` with real decision
/// logic and no network dependency, so they get full offline coverage
/// here.
#[test]
fn finds_dts_from_types_field() {
    let manifest: serde_json::Value =
        serde_json::from_str(r#"{"types": "dist/index.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_types_field");
    let (rel, abs) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "dist/index.d.ts");
    assert_eq!(abs, dir.join("dist/index.d.ts"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_dts_from_typings_field_when_types_is_absent() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"typings": "index.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_typings_field");
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "index.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn prefers_types_field_over_typings_field() {
    let manifest: serde_json::Value =
        serde_json::from_str(r#"{"types": "a.d.ts", "typings": "b.d.ts"}"#).unwrap();
    let dir = temp_registry("dts_prefers_types");
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "a.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn falls_back_to_index_d_ts_when_no_field_is_present() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
    let dir = temp_registry("dts_index_fallback");
    fs::write(dir.join("index.d.ts"), "declare function f(): void;").unwrap();
    let (rel, _) = find_own_dts(&manifest, &dir).unwrap();
    assert_eq!(rel, "index.d.ts");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_no_dts_when_nothing_is_bundled() {
    let manifest: serde_json::Value = serde_json::from_str(r#"{"main": "index.js"}"#).unwrap();
    let dir = temp_registry("dts_none");
    assert!(find_own_dts(&manifest, &dir).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn types_package_name_for_an_unscoped_package() {
    assert_eq!(types_package_name("left-pad"), "@types/left-pad");
}

#[test]
fn types_package_name_for_a_scoped_package() {
    assert_eq!(types_package_name("@babel/core"), "@types/babel__core");
}

/// Found by running a real npm package (`ms`, `"main": "./index"`)
/// through `add`: reading the literal `main` string as a path fails
/// since Node resolves the missing `.js` extension at require-time,
/// which we don't get for free.
#[test]
fn resolves_main_field_missing_its_extension() {
    let dir = temp_registry("main_no_extension");
    fs::write(dir.join("index.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "./index").unwrap();
    assert_eq!(rel, "./index.js");
    assert_eq!(abs, dir.join("index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_main_field_pointing_at_a_directory() {
    let dir = temp_registry("main_directory");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib/index.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "./lib").unwrap();
    assert_eq!(rel, "./lib/index.js");
    assert_eq!(abs, dir.join("lib/index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_a_required_directory_through_its_own_package_manifest() {
    let dir = temp_registry("nested_directory_manifest");
    let feature = dir.join("feature");
    fs::create_dir_all(feature.join("dist")).unwrap();
    fs::write(
        feature.join("package.json"),
        r#"{"exports":{".":{"require":"./dist/index.cjs"}}}"#,
    )
    .unwrap();
    fs::write(feature.join("dist/index.cjs"), "module.exports = 42;").unwrap();
    let (relative, absolute) = resolve_module_path(&dir, "./feature").unwrap();
    assert_eq!(relative, "feature/dist/index.cjs");
    assert_eq!(absolute, feature.join("dist/index.cjs"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn resolves_main_field_written_exactly() {
    let dir = temp_registry("main_exact");
    fs::write(dir.join("main.js"), "module.exports = 1;").unwrap();
    let (rel, abs) = resolve_module_path(&dir, "main.js").unwrap();
    assert_eq!(rel, "main.js");
    assert_eq!(abs, dir.join("main.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn errors_with_all_tried_candidates_when_main_cannot_be_resolved() {
    let dir = temp_registry("main_missing");
    let err = resolve_module_path(&dir, "./index").unwrap_err();
    assert!(err.contains("./index"));
    assert!(err.contains("./index.js"));
    assert!(err.contains("./index/index.js"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn finds_single_and_double_quoted_relative_requires() {
    let specs: Vec<_> =
        find_module_specs(r#"var a = require('./a'); var b = require("../lib/b");"#)
            .into_iter()
            .filter(|spec| spec.starts_with('.'))
            .collect();
    assert_eq!(specs, vec!["./a".to_string(), "../lib/b".to_string()]);
}

#[test]
fn finds_literal_dependencies_called_through_create_require_aliases() {
    assert_eq!(
        find_module_specs(
            "import { createRequire as makeRequire } from 'node:module';\n\
                 import * as Module from 'module';\n\
                 const local = makeRequire('pkg/index.js');\n\
                 const other = Module.createRequire('pkg/index.js');\n\
                 local('./dependency'); other(`./other`);"
        ),
        vec!["./dependency", "./other", "node:module", "module"]
    );
}

#[test]
fn parser_collects_esm_reexports_and_dynamic_imports_without_false_positives() {
    let specs = find_module_specs(
        r#"
                import main from './main.js';
                export { value } from "./value.js";
                export * from './all.js';
                const package = import(`external-package`);
                const feature = import(("external-" + "feature"));
                const later = import('./later.js');
                const text = "require('./not-real.js')";
                // require('./also-not-real.js')
                object.require('./member.js');
                require(variable);
            "#,
    );
    assert_eq!(
        specs,
        vec![
            "external-package",
            "external-feature",
            "./later.js",
            "./main.js",
            "./value.js",
            "./all.js"
        ]
    );
}

#[test]
fn dynamic_import_candidate_expansion_is_bounded() {
    let analysis = analyze_module(
            "import((a ? 'a' : 'b') + (b ? 'a' : 'b') + (c ? 'a' : 'b') + (d ? 'a' : 'b') + (e ? 'a' : 'b') + (f ? 'a' : 'b') + (g ? 'a' : 'b'));",
        );
    assert!(analysis.has_nonliteral_dynamic_import);
    assert!(analysis.specs.is_empty());
}

#[test]
fn parser_identifies_commonjs_export_assignments() {
    let analysis = analyze_module(
        r#"
                exports.alpha = 1;
                module.exports.beta = 2;
                module.exports["gamma"] = 3;
                module.exports = function () {};
                object.exports.nope = 4;
            "#,
    );
    assert_eq!(
        analysis._commonjs_exports,
        vec!["alpha", "beta", "default", "gamma"]
    );
}

#[test]
fn rewrites_literal_dynamic_import_to_an_async_bundle_require() {
    let rewritten =
        rewrite_esm_to_commonjs("function load() { return import('./feature.js'); }").unwrap();
    assert!(rewritten.contains("requireAsync(String('./feature.js'))"));
    assert!(!rewritten.contains("import("));
}

#[test]
fn ignores_bare_specifier_requires() {
    let specs: Vec<_> = find_module_specs(r#"var x = require('is-number');"#)
        .into_iter()
        .filter(|spec| spec.starts_with('.'))
        .collect();
    assert!(specs.is_empty());
}

#[test]
fn finds_bare_specifiers_including_scoped_packages() {
    let specs: Vec<_> = find_module_specs(
            r#"var a = require('side-channel'); var b = require('@babel/core'); var c = require('./local');"#,
        )
        .into_iter()
        .filter(|spec| !spec.starts_with('.'))
        .collect();
    assert_eq!(
        specs,
        vec!["side-channel".to_string(), "@babel/core".to_string()]
    );
}

#[test]
fn splits_bare_specs_into_package_and_subpath() {
    assert_eq!(split_bare_spec("lodash"), ("lodash", None));
    assert_eq!(split_bare_spec("lodash/fp"), ("lodash", Some("fp")));
    assert_eq!(
        split_bare_spec("es-errors/type"),
        ("es-errors", Some("type"))
    );
    assert_eq!(split_bare_spec("@babel/core"), ("@babel/core", None));
    assert_eq!(
        split_bare_spec("@babel/core/lib/index"),
        ("@babel/core", Some("lib/index"))
    );
}

#[test]
fn splits_version_specs_from_add_arguments() {
    assert_eq!(split_package_spec("left-pad"), ("left-pad", None));
    assert_eq!(
        split_package_spec("left-pad@1.3.0"),
        ("left-pad", Some("1.3.0"))
    );
    assert_eq!(
        split_package_spec("left-pad@^1.2.0"),
        ("left-pad", Some("^1.2.0"))
    );
    assert_eq!(
        split_package_spec("left-pad@next"),
        ("left-pad", Some("next"))
    );
    // A scoped package's leading `@scope/` is never mistaken for a
    // version separator -- only an `@` after the scope's own `/`
    // starts one.
    assert_eq!(split_package_spec("@hapi/hoek"), ("@hapi/hoek", None));
    assert_eq!(
        split_package_spec("@hapi/hoek@9.0.0"),
        ("@hapi/hoek", Some("9.0.0"))
    );
}

/// The exact shape found in `qs`'s own real transitive dependency
/// chain: `require('es-errors/type')`, a "deep import" subpath into
/// another package, resolved directly against that package's root
/// (not through its `main` field).
#[test]
fn resolves_a_deep_import_subpath_into_a_dependency() {
    let node_modules = temp_registry("deep_import_node_modules");
    fs::create_dir_all(node_modules.join("es-errors")).unwrap();
    fs::write(
        node_modules.join("es-errors/package.json"),
        r#"{"main": "index.js"}"#,
    )
    .unwrap();
    fs::write(
        node_modules.join("es-errors/index.js"),
        "module.exports = {};",
    )
    .unwrap();
    fs::write(
        node_modules.join("es-errors/type.js"),
        "module.exports = TypeError;",
    )
    .unwrap();

    let (name, relative, abs, dir) = resolve_bare_require(&node_modules, "es-errors/type").unwrap();
    assert_eq!(name, "es-errors");
    assert_eq!(relative, "type.js");
    assert_eq!(abs, node_modules.join("es-errors/type.js"));
    assert_eq!(dir, node_modules.join("es-errors"));

    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn resolves_a_deep_import_into_a_scoped_package() {
    let node_modules = temp_registry("deep_import_scoped_node_modules");
    fs::create_dir_all(node_modules.join("@scope/pkg/lib")).unwrap();
    fs::write(
        node_modules.join("@scope/pkg/lib/util.js"),
        "module.exports = 1;",
    )
    .unwrap();

    let (name, relative, ..) = resolve_bare_require(&node_modules, "@scope/pkg/lib/util").unwrap();
    assert_eq!(name, "@scope/pkg");
    assert_eq!(relative, "lib/util.js");

    let _ = fs::remove_dir_all(&node_modules);
}

#[test]
fn resolves_generated_declarations_from_a_dot_named_package() {
    let root = temp_registry("generated_dot_package");
    let node_modules = root.join("node_modules");
    let entry = node_modules.join("@scope/client/default.d.ts");
    let generated = node_modules.join(".generated/client/default.d.ts");
    fs::create_dir_all(entry.parent().unwrap()).unwrap();
    fs::create_dir_all(generated.parent().unwrap()).unwrap();
    fs::write(&entry, "export * from '.generated/client/default';").unwrap();
    fs::write(&generated, "export declare class GeneratedClient {}").unwrap();

    assert_eq!(
        declaration_reexport_path(&entry, ".generated/client/default"),
        Some(generated)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn selects_a_single_addon_from_a_hidden_generated_package() {
    let node_modules = temp_registry("generated_native_addon");
    let addon = node_modules.join(".generated/client/engine.so.node");
    fs::create_dir_all(addon.parent().unwrap()).unwrap();
    fs::create_dir_all(node_modules.join(".bin")).unwrap();
    fs::write(&addon, b"addon").unwrap();

    assert_eq!(
        select_generated_addon(&node_modules, "require('.generated/client')")
            .unwrap()
            .unwrap()
            .path,
        addon
    );
    assert!(select_generated_addon(&node_modules, "module.exports = {}")
        .unwrap()
        .is_none());

    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn ignores_dynamic_and_malformed_require_calls() {
    // `require(name)` (a variable, not a literal) and a stray
    // "require" that isn't actually a call must not confuse the scan
    // -- and must not stop it from still finding a real one after.
    let specs: Vec<_> = find_module_specs(
        "var x = require(name); var note = 'requirements'; var y = require('./y');",
    )
    .into_iter()
    .filter(|spec| spec.starts_with('.'))
    .collect();
    assert_eq!(specs, vec!["./y".to_string()]);
}

#[test]
fn normalizes_dot_and_dot_dot_segments() {
    assert_eq!(normalize_path_string("lib/./stringify"), "lib/stringify");
    assert_eq!(normalize_path_string("lib/../parse"), "parse");
    assert_eq!(normalize_path_string("a/b/../../c"), "c");
}
