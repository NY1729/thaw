#[test]
fn nullable_and_nullish_parameters_round_trip_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export declare function nullable(value: number | null): number | null;\nexport declare function nullish(value: number | null | undefined): number | null | undefined;\n",
    )
    .unwrap();
    let source = "module.exports.nullable = value => value; module.exports.nullish = value => value;";
    assert!(jit_numeric_export(source, "nullable", false, &declarations[0]).is_some());
    assert!(jit_numeric_export(source, "nullish", false, &declarations[1]).is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullish-parameters-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullish-parameters");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function nullable(value: number | null): number | null;\nexport declare function nullish(value: number | null | undefined): number | null | undefined;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.nullable = value => value; module.exports.nullish = value => value;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { nullable, nullish } from 'jit-nullish-parameters';\nfunction main(): void { const numberValue: number | null = nullable(7); const nullValue: number | null = nullable(null); const nullishNumber: number | null | undefined = nullish(8); const nullishNull: number | null | undefined = nullish(null); const undefinedValue: number | null | undefined = nullish(undefined); console.log(numberValue!); console.log(nullValue === null); console.log(nullishNumber!); console.log(nullishNull === null); console.log(undefinedValue === undefined); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "7\ntrue\n8\ntrue\ntrue\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nullable_and_nullish_collections_round_trip_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export declare function nullable(value: number[] | null): number[] | null;\nexport declare function nullish(value: Record<string, number> | null | undefined): Record<string, number> | null | undefined;\n",
    )
    .unwrap();
    let source = "module.exports.nullable = value => value; module.exports.nullish = value => value;";
    assert!(jit_numeric_export(source, "nullable", false, &declarations[0]).is_some());
    assert!(jit_numeric_export(source, "nullish", false, &declarations[1]).is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullish-collections-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullish-collections");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function nullable(value: number[] | null): number[] | null;\nexport declare function nullish(value: Record<string, number> | null | undefined): Record<string, number> | null | undefined;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.nullable = value => value; module.exports.nullish = value => value;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { nullable, nullish } from 'jit-nullish-collections';\nfunction main(): void { const arrayValue: number[] | null = nullable([1, 2]); const arrayNull: number[] | null = nullable(null); const dictionaryValue: Record<string, number> | null | undefined = nullish({ score: 7 }); const dictionaryNull: Record<string, number> | null | undefined = nullish(null); const dictionaryUndefined: Record<string, number> | null | undefined = nullish(undefined); console.log(arrayValue!.join(',')); console.log(arrayNull === null); console.log(dictionaryValue!.score); console.log(dictionaryNull === null); console.log(dictionaryUndefined === undefined); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "1,2\ntrue\n7\ntrue\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nullable_and_nullish_aggregates_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Item { label: string; score: number; }\nexport declare function object(value: Item | null): string;\nexport declare function tuple(value: [number, string] | null | undefined): string;\n",
    )
    .unwrap();
    let source = "module.exports.object = value => value?.label.toUpperCase() ?? 'missing'; module.exports.tuple = value => value?.[1] ?? 'missing';";
    assert!(jit_numeric_export(source, "object", false, &declarations[0]).is_some());
    assert!(jit_numeric_export(source, "tuple", false, &declarations[1]).is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullish-aggregates-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullish-aggregates");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { label: string; score: number; }\nexport declare function object(value: Item | null): string;\nexport declare function tuple(value: [number, string] | null | undefined): string;\n",
    )
    .unwrap();
    std::fs::write(package.join("bundle.js"), format!("{source}\n")).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { object, tuple } from 'jit-nullish-aggregates';\nfunction main(): void { console.log(object({ label: 'ready', score: 7 })); console.log(object(null)); console.log(tuple([8, 'pair'])); console.log(tuple(null)); console.log(tuple(undefined)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "READY\nmissing\npair\nmissing\nmissing\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nullable_and_nullish_aggregate_results_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Item { label: string; score: number; }\nexport declare function object(value: boolean): Item | null;\nexport declare function tuple(value: number): [number, string] | null | undefined;\n",
    )
    .unwrap();
    let source = "module.exports.object = value => value ? { label: 'made', score: 7 } : null; module.exports.tuple = value => value === 0 ? [8, 'pair'] : value === 1 ? null : undefined;";
    assert_eq!(declarations.len(), 2);
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullish-aggregate-results-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullish-aggregate-results");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { label: string; score: number; }\nexport declare function object(value: boolean): Item | null;\nexport declare function tuple(value: number): [number, string] | null | undefined;\n",
    )
    .unwrap();
    std::fs::write(package.join("bundle.js"), format!("{source}\n")).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { object, tuple } from 'jit-nullish-aggregate-results';\nfunction main(): void { const item = object(true); const noItem = object(false); const pair = tuple(0); const nullPair = tuple(1); const missingPair = tuple(2); console.log(item!.label); console.log(item!.score); console.log(noItem === null); console.log(pair![0]); console.log(pair![1]); console.log(nullPair === null); console.log(missingPair === undefined); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "made\n7\ntrue\n8\npair\ntrue\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_tuple_elements_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-tuple-element-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-tuple-element");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { values: [string | undefined, number | undefined, boolean | undefined]; }\nexport declare function name(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function enabled(value: Item | string): boolean;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.name = value => typeof value === 'object' ? value.values[0]?.toUpperCase() ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.values[1] ?? 0 : -1; module.exports.enabled = value => typeof value === 'object' ? value.values[2] ?? false : true; module.exports.make = full => full ? { values: ['made', 8, true] } : { values: [undefined, undefined, undefined] };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { enabled, make, name, score } from 'jit-optional-tuple-element';\ntype Item = { values: [string | undefined, number | undefined, boolean | undefined] };\nfunction main(): void { const present: Item = { values: ['thaw', 7, true] }; const absent: Item = { values: [undefined, undefined, undefined] }; console.log(name(present)); console.log(score(present)); console.log(enabled(present)); console.log(name(absent)); console.log(score(absent)); console.log(enabled(absent)); console.log(name('plain')); console.log(name(make(true))); console.log(score(make(true))); console.log(enabled(make(true))); console.log(name(make(false))); console.log(score(make(false))); console.log(enabled(make(false))); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "THAW\n7\ntrue\nmissing\n0\nfalse\nplain\nMADE\n8\ntrue\nmissing\n0\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nullable_aggregate_fields_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullable-aggregate-field-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullable-aggregate-field");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { name: string | null; values: [number | null, boolean | null]; }\nexport declare function name(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function enabled(value: Item | string): boolean;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.name = value => typeof value === 'object' ? value.name?.toUpperCase() ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.values[0] ?? 0 : -1; module.exports.enabled = value => typeof value === 'object' ? value.values[1] ?? false : true; module.exports.make = full => full ? { name: 'made', values: [8, true] } : { name: null, values: [null, null] };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { enabled, make, name, score } from 'jit-nullable-aggregate-field';\ntype Item = { name: string | null; values: [number | null, boolean | null] };\nfunction main(): void { const present: Item = { name: 'thaw', values: [7, true] }; const absent: Item = { name: null, values: [null, null] }; console.log(name(present)); console.log(score(present)); console.log(enabled(present)); console.log(name(absent)); console.log(score(absent)); console.log(enabled(absent)); console.log(name('plain')); console.log(name(make(true))); console.log(score(make(true))); console.log(enabled(make(true))); console.log(name(make(false))); console.log(score(make(false))); console.log(enabled(make(false))); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "THAW\n7\ntrue\nmissing\n0\nfalse\nplain\nMADE\n8\ntrue\nmissing\n0\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nullish_aggregate_fields_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nullish-aggregate-field-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nullish-aggregate-field");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { name: string | null | undefined; values: [number | null | undefined, boolean | null | undefined]; }\nexport declare function name(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function enabled(value: Item | string): boolean;\nexport declare function make(mode: number): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.name = value => typeof value === 'object' ? value.name?.toUpperCase() ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.values[0] ?? 0 : -1; module.exports.enabled = value => typeof value === 'object' ? value.values[1] ?? false : true; module.exports.make = mode => mode === 1 ? { name: 'made', values: [8, true] } : mode === 2 ? { name: null, values: [null, null] } : { name: undefined, values: [undefined, undefined] };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { enabled, make, name, score } from 'jit-nullish-aggregate-field';\ntype Item = { name: string | null | undefined; values: [number | null | undefined, boolean | null | undefined] };\nfunction print(value: Item | string): void { console.log(name(value)); console.log(score(value)); console.log(enabled(value)); }\nfunction main(): void { print({ name: 'thaw', values: [7, true] }); print({ name: null, values: [null, null] }); print({ name: undefined, values: [undefined, undefined] }); console.log(name('plain')); print(make(1)); print(make(2)); print(make(3)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "THAW\n7\ntrue\nmissing\n0\nfalse\nmissing\n0\nfalse\nplain\nMADE\n8\ntrue\nmissing\n0\nfalse\nmissing\n0\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nested_tagged_aggregate_fields_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Item { child?: { label: string }; nullable: { score: number } | null; pair: [number, string] | null | undefined; }\nexport declare function label(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function numberValue(value: Item | string): number;\nexport declare function stringValue(value: Item | string): string;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    let source = "module.exports.label = value => typeof value === 'object' ? value.child?.label ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.nullable?.score ?? 0 : -1; module.exports.numberValue = value => typeof value === 'object' ? value.pair?.[0] ?? 0 : -1; module.exports.stringValue = value => typeof value === 'object' ? value.pair?.[1] ?? 'missing' : value; module.exports.make = full => full ? { child: { label: 'made' }, nullable: { score: 9 }, pair: [8, 'pair'] } : { nullable: null, pair: null };";
    for (name, declaration) in ["label", "score", "numberValue", "stringValue", "make"]
        .into_iter()
        .zip(&declarations)
    {
        assert!(
            jit_numeric_export(source, name, false, declaration).is_some(),
            "{name} did not specialize"
        );
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nested-tagged-aggregate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nested-tagged-aggregate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { child?: { label: string }; nullable: { score: number } | null; pair: [number, string] | null | undefined; }\nexport declare function label(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function numberValue(value: Item | string): number;\nexport declare function stringValue(value: Item | string): string;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.label = value => typeof value === 'object' ? value.child?.label ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.nullable?.score ?? 0 : -1; module.exports.numberValue = value => typeof value === 'object' ? value.pair?.[0] ?? 0 : -1; module.exports.stringValue = value => typeof value === 'object' ? value.pair?.[1] ?? 'missing' : value; module.exports.make = full => full ? { child: { label: 'made' }, nullable: { score: 9 }, pair: [8, 'pair'] } : { nullable: null, pair: null };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { label, make, numberValue, score, stringValue } from 'jit-nested-tagged-aggregate';\ntype Item = { child?: { label: string }; nullable: { score: number } | null; pair: [number, string] | null | undefined };\nfunction main(): void { const present: Item = { child: { label: 'thaw' }, nullable: { score: 6 }, pair: [7, 'local'] }; const absent: Item = { nullable: null, pair: undefined }; const made = make(true); const empty = make(false); console.log(label(present)); console.log(score(present)); console.log(numberValue(present)); console.log(stringValue(present)); console.log(label(absent)); console.log(score(absent)); console.log(numberValue(absent)); console.log(stringValue(absent)); console.log(label('plain')); console.log(score('plain')); console.log(numberValue('plain')); console.log(stringValue('plain')); console.log(label(made)); console.log(score(made)); console.log(numberValue(made)); console.log(stringValue(made)); console.log(label(empty)); console.log(score(empty)); console.log(numberValue(empty)); console.log(stringValue(empty)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "thaw\n6\n7\nlocal\nmissing\n0\n0\nmissing\nplain\n-1\n-1\nplain\nmade\n9\n8\npair\nmissing\n0\n0\nmissing\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tagged_array_and_dictionary_fields_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tagged-collections-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tagged-collections");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { values?: number[]; lookup: Record<string, string> | null | undefined; pair: [boolean[] | null, Record<string, number> | undefined]; }\nexport declare function values(value: Item | string): string;\nexport declare function lookup(value: Item | string): string;\nexport declare function flags(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.values = value => typeof value === 'object' ? value.values?.join(',') ?? 'missing' : value; module.exports.lookup = value => typeof value === 'object' ? value.lookup?.name ?? 'missing' : value; module.exports.flags = value => typeof value === 'object' ? value.pair[0]?.join(',') ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.pair[1]?.score ?? 0 : -1; module.exports.make = full => full ? { values: [8, 9], lookup: { name: 'made' }, pair: [[true, false], { score: 7 }] } : { lookup: null, pair: [null, undefined] };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { flags, lookup, make, score, values } from 'jit-tagged-collections';\ntype Item = { values?: number[]; lookup: Record<string, string> | null | undefined; pair: [boolean[] | null, Record<string, number> | undefined] };\nfunction main(): void { const present: Item = { values: [1, 2], lookup: { name: 'local' }, pair: [[true, false], { score: 6 }] }; const absent: Item = { lookup: undefined, pair: [null, undefined] }; const made = make(true); const empty = make(false); console.log(values(present)); console.log(lookup(present)); console.log(flags(present)); console.log(score(present)); console.log(values(absent)); console.log(lookup(absent)); console.log(flags(absent)); console.log(score(absent)); console.log(values('plain')); console.log(values(made)); console.log(lookup(made)); console.log(flags(made)); console.log(score(made)); console.log(values(empty)); console.log(lookup(empty)); console.log(flags(empty)); console.log(score(empty)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "1,2\nlocal\ntrue,false\n6\nmissing\nmissing\nmissing\n0\nplain\n8,9\nmade\ntrue,false\n7\nmissing\nmissing\nmissing\n0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_dictionary_unions_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-dictionary-union-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-dictionary-union");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function valueOr(value?: Record<string, number> | string): string;\nexport declare function valueDefault(value?: Record<string, number> | string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.valueOr = value => String(value ?? 'missing'); module.exports.valueDefault = (value = { count: 3 }) => String(value);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { valueOr, valueDefault } from 'jit-optional-dictionary-union';\nfunction main(): void { const first: Record<string, number> = { count: 1 }; const second: Record<string, number> = { count: 2 }; console.log(valueOr(first)); console.log(valueOr('text')); console.log(valueOr()); console.log(valueDefault(second)); console.log(valueDefault('given')); console.log(valueDefault()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "[object Object]\ntext\nmissing\n[object Object]\ngiven\n[object Object]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

