#[test]
fn optional_aggregates_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-aggregate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-aggregate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Nested { score: number; }\nexport interface Meta { count: number; label: string; enabled: boolean; nested: Nested; pair: [number, string]; flags: boolean[]; }\nexport interface Choice { value: number | string; }\nexport declare function count(value?: Meta): number | undefined;\nexport declare function labelOr(value?: Meta): string;\nexport declare function enabled(value?: Meta): boolean | undefined;\nexport declare function nestedScore(value?: Meta): number | undefined;\nexport declare function pairName(value?: Meta): string | undefined;\nexport declare function flags(value?: Meta): boolean[] | undefined;\nexport declare function optionalRecord(value?: Record<string, number>): Record<string, number> | undefined;\nexport declare function optionalChoice(value?: Choice): number | string | undefined;\nexport declare function choiceOr(value?: number | string): string;\nexport declare function choiceDefault(value?: number | string): string;\nexport declare function aggregateOr(value?: number[] | string): string;\nexport declare function aggregateDefault(value?: number[] | string): string;\nexport declare function flagCount(value?: Meta): number;\nexport declare function upper(value?: Meta): string;\nexport declare function tupleName(value?: [number, string]): string | undefined;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.count = value => value?.count; module.exports.labelOr = value => value?.label ?? 'missing'; module.exports.enabled = value => value?.enabled; module.exports.nestedScore = value => value?.nested.score; module.exports.pairName = value => value?.pair[1]; module.exports.flags = value => value?.flags; module.exports.optionalRecord = value => value?.valueOf(); module.exports.optionalChoice = value => value?.value; module.exports.choiceOr = value => String(value ?? 'missing'); module.exports.choiceDefault = (value = 12) => String(value); module.exports.aggregateOr = value => String(value ?? 'missing'); module.exports.aggregateDefault = (value = [3, 4]) => String(value); module.exports.flagCount = value => value?.flags.length ?? 42; module.exports.upper = value => value?.label.toUpperCase() ?? 'missing'; module.exports.tupleName = value => value?.[1];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { count, labelOr, enabled, nestedScore, pairName, flags, optionalRecord, optionalChoice, choiceOr, choiceDefault, aggregateOr, aggregateDefault, flagCount, upper, tupleName } from 'jit-optional-aggregate';\nfunction showChoice(value: (number | string) | undefined): void { if (value === undefined) console.log('undefined'); else if (typeof value === 'number') console.log(value); else console.log(value); }\nfunction main(): void { const value = { count: 7, label: 'ready', enabled: false, nested: { score: 9 }, pair: [1, 'nested'], flags: [true, false] }; const record: Record<string, number> = { count: 42 }; console.log(count(value)); console.log(count() === undefined); console.log(labelOr(value)); console.log(labelOr()); console.log(enabled(value)); console.log(enabled() === undefined); console.log(nestedScore(value)); console.log(nestedScore() === undefined); console.log(pairName(value)); console.log(pairName() === undefined); console.log(flags(value)?.length); console.log(flags() === undefined); console.log(optionalRecord(record)?.count); console.log(optionalRecord() === undefined); showChoice(optionalChoice({ value: 8 })); showChoice(optionalChoice({ value: 'choice' })); showChoice(optionalChoice()); console.log(choiceOr(9)); console.log(choiceOr('word')); console.log(choiceOr()); console.log(choiceDefault(10)); console.log(choiceDefault('set')); console.log(choiceDefault()); console.log(aggregateOr([1, 2])); console.log(aggregateOr('text')); console.log(aggregateOr()); console.log(aggregateDefault([5, 6])); console.log(aggregateDefault('given')); console.log(aggregateDefault()); console.log(flagCount(value)); console.log(flagCount()); console.log(upper(value)); console.log(upper()); console.log(tupleName([1, 'pair'])); console.log(tupleName() === undefined); }\n",
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
        "7\ntrue\nready\nmissing\nfalse\ntrue\n9\ntrue\nnested\ntrue\n2\ntrue\n42\ntrue\n8\nchoice\nundefined\n9\nword\nmissing\n10\nset\n12\n1,2\ntext\nmissing\n5,6\ngiven\n3,4\n2\n42\nREADY\nmissing\npair\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fixed_object_union_result_builds_native_storage() {
    let functions = thaw_bridge::parse_dts(
        "export interface Meta { label: string; enabled: boolean; }\nexport interface Item { count: number; meta: Meta; values: number[]; scores: Record<string, number>; pair: [number, string, boolean]; }\nexport declare function make(flag: boolean): Item | string;\nexport declare function describe(value: Item | string): string;\n",
    )
    .unwrap();
    let operation = jit_numeric_export(
        "module.exports.make = flag => flag ? { count: 2, meta: { label: 'made', enabled: false }, values: [3, 4], scores: { primary: 7 }, pair: [8, 'tuple', true] } : 'none';",
        "make",
        false,
        &functions[0],
    )
    .unwrap();
    assert!(operation.contains("objnew40"), "{operation}");
    assert!(operation.contains("objsetn0"), "{operation}");
    assert!(operation.contains("objnew16"), "{operation}");
    assert!(operation.contains("objsets0"), "{operation}");
    assert!(operation.contains("objsetb8"), "{operation}");
    assert!(operation.contains("objseto8"), "{operation}");
    assert!(operation.contains("objseta16"), "{operation}");
    assert!(operation.contains("objseto24"), "{operation}");
    assert!(operation.contains("tupnew3"), "{operation}");
    assert!(operation.contains("tupsetn0"), "{operation}");
    assert!(operation.contains("tupsets1"), "{operation}");
    assert!(operation.contains("tupsetb2"), "{operation}");
    assert!(operation.contains("objseta32"), "{operation}");
    assert!(operation.contains(",if,"), "{operation}");
    assert!(jit_numeric_export(
        "module.exports.describe = value => typeof value === 'object' ? value.meta.label : value.toUpperCase();",
        "describe",
        false,
        &functions[1],
    )
    .is_some(), "nested object field");
    assert!(jit_numeric_export(
        "module.exports.describe = value => typeof value === 'object' ? String(value.values.length) : value.toUpperCase();",
        "describe",
        false,
        &functions[1],
    )
    .is_some(), "array field length");
    assert!(jit_numeric_export(
        "module.exports.describe = value => typeof value === 'object' ? value.values.join(',') : value.toUpperCase();",
        "describe",
        false,
        &functions[1],
    )
    .is_some(), "array field join");
    assert!(jit_numeric_export(
        "module.exports.describe = value => typeof value === 'object' ? String(value['count']) + ':' + value['meta']['label'].toUpperCase() + ':' + String(value.meta.enabled) + ':' + value.values.join(',') + ':' + String(value.scores.primary) + ':' + String(value.pair[0]) + ':' + value.pair[1] + ':' + String(value.pair[2]) : value.toUpperCase();",
        "describe",
        false,
        &functions[1],
    )
    .is_some());
}

#[test]
fn fixed_tuple_unions_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Item { label: string; }\nexport declare function describe(value: [number, string] | string): string;\nexport declare function make(value: boolean): [number, string] | string;\nexport declare function mixed(value: Item | [number, string]): string;\nexport declare function makeMixed(value: boolean): Item | [number, string];\nexport declare function size(value: number[] | [number, string] | Item): number;\nexport declare function firstNumber(value: number[] | [number, number] | Item): number;\nexport declare function firstBoolean(value: boolean[] | [boolean, boolean] | Item): boolean;\nexport declare function firstString(value: string[] | [string, string] | Item): string;\nexport declare function joinNumbers(value: number[] | [number, number]): string;\nexport declare function hasBoolean(value: boolean[] | [boolean, boolean]): boolean;\nexport declare function atString(value: string[] | [string, string]): string;\nexport declare function sliceNumbers(value: number[] | [number, number]): number[];\nexport declare function indexNumber(value: number[] | [number, number], needle: number): number;\nexport declare function lastString(value: string[] | [string, string], needle: string): number;\nexport declare function concatNumbers(value: number[] | [number, number]): number[];\nexport declare function reversedNumbers(value: number[] | [number, number]): number[];\nexport declare function reverseFlags(value: boolean[] | [boolean, boolean]): boolean[];\nexport declare function sortedNumbers(value: number[] | [number, number]): number[];\nexport declare function sortStrings(value: string[] | [string, string]): string[];\n",
    )
    .unwrap();
    let source = "module.exports.describe = value => typeof value === 'object' ? value[1] : value; module.exports.make = value => value ? [7, 'pair'] : 'plain'; module.exports.mixed = value => Array.isArray(value) ? value[1] : value.label; module.exports.makeMixed = value => value ? { label: 'object' } : [9, 'tuple']; module.exports.size = value => Array.isArray(value) ? value.length : value.label.length; module.exports.firstNumber = value => Array.isArray(value) ? value[0] : value.label.length; module.exports.firstBoolean = value => Array.isArray(value) ? value[0] : value.label.length > 0; module.exports.firstString = value => Array.isArray(value) ? value[0] : value.label; module.exports.joinNumbers = value => Array.isArray(value) ? value.join('-') : ''; module.exports.hasBoolean = value => Array.isArray(value) ? value.includes(true) : false; module.exports.atString = value => Array.isArray(value) ? value.at(0) ?? 'missing' : 'missing'; module.exports.sliceNumbers = value => Array.isArray(value) ? value.slice(1) : [0]; module.exports.indexNumber = (value, needle) => Array.isArray(value) ? value.indexOf(needle) : -1; module.exports.lastString = (value, needle) => Array.isArray(value) ? value.lastIndexOf(needle) : -1; module.exports.concatNumbers = value => Array.isArray(value) ? value.concat(9) : [9]; module.exports.reversedNumbers = value => Array.isArray(value) ? value.toReversed() : [0]; module.exports.reverseFlags = value => Array.isArray(value) ? value.reverse() : [false]; module.exports.sortedNumbers = value => Array.isArray(value) ? value.toSorted() : [0]; module.exports.sortStrings = value => Array.isArray(value) ? value.sort() : [''];";
    assert!(jit_numeric_export(source, "describe", false, &declarations[0]).is_some());
    assert!(jit_numeric_export(source, "make", false, &declarations[1]).is_some());
    assert!(jit_numeric_export(source, "mixed", false, &declarations[2]).is_some());
    assert!(jit_numeric_export(source, "makeMixed", false, &declarations[3]).is_some());
    assert!(jit_numeric_export(source, "size", false, &declarations[4]).is_some());
    assert!(jit_numeric_export(source, "firstNumber", false, &declarations[5]).is_some());
    assert!(jit_numeric_export(source, "firstBoolean", false, &declarations[6]).is_some());
    assert!(jit_numeric_export(source, "firstString", false, &declarations[7]).is_some());
    assert!(jit_numeric_export(source, "joinNumbers", false, &declarations[8]).is_some());
    assert!(jit_numeric_export(source, "hasBoolean", false, &declarations[9]).is_some());
    assert!(jit_numeric_export(source, "atString", false, &declarations[10]).is_some());
    assert!(jit_numeric_export(source, "sliceNumbers", false, &declarations[11]).is_some());
    assert!(jit_numeric_export(source, "indexNumber", false, &declarations[12]).is_some());
    assert!(jit_numeric_export(source, "lastString", false, &declarations[13]).is_some());
    assert!(jit_numeric_export(source, "concatNumbers", false, &declarations[14]).is_some());
    assert!(jit_numeric_export(source, "reversedNumbers", false, &declarations[15]).is_some());
    assert!(jit_numeric_export(source, "reverseFlags", false, &declarations[16]).is_some());
    assert!(jit_numeric_export(source, "sortedNumbers", false, &declarations[17]).is_some());
    assert!(jit_numeric_export(source, "sortStrings", false, &declarations[18]).is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { label: string; }\nexport declare function describe(value: [number, string] | string): string;\nexport declare function make(value: boolean): [number, string] | string;\nexport declare function mixed(value: Item | [number, string]): string;\nexport declare function makeMixed(value: boolean): Item | [number, string];\nexport declare function size(value: number[] | [number, string] | Item): number;\nexport declare function firstNumber(value: number[] | [number, number] | Item): number;\nexport declare function firstBoolean(value: boolean[] | [boolean, boolean] | Item): boolean;\nexport declare function firstString(value: string[] | [string, string] | Item): string;\nexport declare function joinNumbers(value: number[] | [number, number]): string;\nexport declare function hasBoolean(value: boolean[] | [boolean, boolean]): boolean;\nexport declare function atString(value: string[] | [string, string]): string;\nexport declare function sliceNumbers(value: number[] | [number, number]): number[];\nexport declare function indexNumber(value: number[] | [number, number], needle: number): number;\nexport declare function lastString(value: string[] | [string, string], needle: string): number;\nexport declare function concatNumbers(value: number[] | [number, number]): number[];\nexport declare function reversedNumbers(value: number[] | [number, number]): number[];\nexport declare function reverseFlags(value: boolean[] | [boolean, boolean]): boolean[];\nexport declare function sortedNumbers(value: number[] | [number, number]): number[];\nexport declare function sortStrings(value: string[] | [string, string]): string[];\n",
    )
    .unwrap();
    std::fs::write(package.join("bundle.js"), format!("{source}\n")).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { atString, concatNumbers, describe, firstBoolean, firstNumber, firstString, hasBoolean, indexNumber, joinNumbers, lastString, make, makeMixed, mixed, reverseFlags, reversedNumbers, size, sliceNumbers, sortedNumbers, sortStrings } from 'jit-tuple-union';\nfunction main(): void { const numbers: number[] = [1, 2, 3]; const pair: [number, string] = [4, 'x']; const numberPair: [number, number] = [5, 6]; const booleans: boolean[] = [true]; const booleanPair: [boolean, boolean] = [false, true]; const strings: string[] = ['array']; const repeatedStrings: string[] = ['a', 'b', 'a']; const stringPair: [string, string] = ['tuple', 'other']; const unsortedNumbers: number[] = [10, 2, 1]; console.log(describe([3, 'local'])); console.log(describe('direct')); console.log(describe(make(true))); console.log(describe(make(false))); console.log(mixed({ label: 'local-object' })); console.log(mixed([4, 'local-tuple'])); console.log(mixed(makeMixed(true))); console.log(mixed(makeMixed(false))); console.log(size(numbers)); console.log(size(pair)); console.log(size({ label: 'four' })); console.log(firstNumber(numbers)); console.log(firstNumber(numberPair)); console.log(firstNumber({ label: 'four' })); console.log(firstBoolean(booleans)); console.log(firstBoolean(booleanPair)); console.log(firstBoolean({ label: 'x' })); console.log(firstString(strings)); console.log(firstString(stringPair)); console.log(firstString({ label: 'object' })); console.log(joinNumbers(numbers)); console.log(joinNumbers(numberPair)); console.log(hasBoolean(booleans)); console.log(hasBoolean(booleanPair)); console.log(atString(strings) ?? 'missing'); console.log(atString(stringPair) ?? 'missing'); console.log(sliceNumbers(numbers).join(',')); console.log(sliceNumbers(numberPair).join(',')); console.log(indexNumber(numbers, 2)); console.log(indexNumber(numberPair, 6)); console.log(lastString(repeatedStrings, 'a')); console.log(lastString(stringPair, 'tuple')); console.log(concatNumbers(numbers).join(',')); console.log(concatNumbers(numberPair).join(',')); console.log(reversedNumbers(numbers).join(',')); console.log(reversedNumbers(numberPair).join(',')); console.log(reverseFlags(booleans).join(',')); console.log(reverseFlags(booleanPair).join(',')); console.log(sortedNumbers(unsortedNumbers).join(',')); console.log(sortedNumbers(numberPair).join(',')); console.log(sortStrings(repeatedStrings).join(',')); console.log(sortStrings(stringPair).join(',')); }\n",
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
        "local\ndirect\npair\nplain\nlocal-object\nlocal-tuple\nobject\ntuple\n3\n2\n4\n1\n5\n4\ntrue\nfalse\ntrue\narray\ntuple\nobject\n1-2-3\n5-6\ntrue\ntrue\narray\ntuple\n2,3\n6\n1\n1\n2\n0\n1,2,3,9\n5,6,9\n3,2,1\n6,5\ntrue\ntrue,false\n1,10,2\n5,6\na,a,b\nother,tuple\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_array_updates_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function fillNumbers(value: number[] | [number, number], replacement: number): number[];\n",
        "export declare function copyNumbers(value: number[] | [number, number]): number[];\n",
        "export declare function withNumber(value: number[] | [number, number], replacement: number): number[];\n",
        "export declare function spliceNumbers(value: number[] | [number, number]): number[];\n",
        "export declare function toSplicedNumbers(value: number[] | [number, number]): number[];\n",
    );
    let source = concat!(
        "let catchFinalized = 0; ",
        "module.exports.fillNumbers = (value, replacement) => Array.isArray(value) ? value.fill(replacement, 1) : [replacement]; ",
        "module.exports.copyNumbers = value => Array.isArray(value) ? value.copyWithin(0, 1) : [0]; ",
        "module.exports.withNumber = (value, replacement) => Array.isArray(value) ? value.with(-1, replacement) : [replacement]; ",
        "module.exports.spliceNumbers = value => Array.isArray(value) ? value.splice(0, 1) : [0]; ",
        "module.exports.toSplicedNumbers = value => Array.isArray(value) ? value.toSpliced(1, 1, 9) : [0];",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "fillNumbers",
        "copyNumbers",
        "withNumber",
        "spliceNumbers",
        "toSplicedNumbers",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(jit_numeric_export(source, name, false, &declarations[index]).is_some());
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-updates-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-updates");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { copyNumbers, fillNumbers, spliceNumbers, toSplicedNumbers, withNumber } from 'jit-tuple-union-updates';\n",
            "function main(): void {\n",
            "  console.log(fillNumbers([1, 2, 3], 8).join(','));\n",
            "  console.log(fillNumbers([5, 6] as [number, number], 7).join(','));\n",
            "  console.log(copyNumbers([1, 2, 3]).join(','));\n",
            "  console.log(copyNumbers([5, 6] as [number, number]).join(','));\n",
            "  console.log(withNumber([1, 2, 3], 8).join(','));\n",
            "  console.log(withNumber([5, 6] as [number, number], 7).join(','));\n",
            "  console.log(spliceNumbers([1, 2, 3]).join(','));\n",
            "  console.log(spliceNumbers([5, 6] as [number, number]).join(','));\n",
            "  console.log(toSplicedNumbers([1, 2, 3]).join(','));\n",
            "  console.log(toSplicedNumbers([5, 6] as [number, number]).join(','));\n",
            "}\n",
        ),
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
        "1,8,8\n5,7\n2,3,3\n6,6\n1,2,8\n5,7\n1\n5\n1,9,3\n5,9\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_array_length_changes_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function append(value: number[] | [number, number], item: number): number;\n",
        "export declare function prepend(value: number[] | [number, number], item: number): number;\n",
        "export declare function dropLast(value: number[] | [number, number]): number;\n",
        "export declare function dropFirst(value: number[] | [number, number]): number;\n",
    );
    let source = concat!(
        "module.exports.append = (value, item) => Array.isArray(value) ? value.push(item) + value.at(-1) : 0; ",
        "module.exports.prepend = (value, item) => Array.isArray(value) ? value.unshift(item) + value[0] : 0; ",
        "module.exports.dropLast = value => Array.isArray(value) ? value.pop() ?? -1 : -1; ",
        "module.exports.dropFirst = value => Array.isArray(value) ? value.shift() ?? -1 : -1;",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in ["append", "prepend", "dropLast", "dropFirst"]
        .into_iter()
        .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-length-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-length");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { append, dropFirst, dropLast, prepend } from 'jit-tuple-union-length';\n",
            "function main(): void {\n",
            "  console.log(append([1, 2, 3], 4));\n",
            "  console.log(append([5, 6] as [number, number], 7));\n",
            "  console.log(prepend([1, 2, 3], 0));\n",
            "  console.log(prepend([5, 6] as [number, number], 4));\n",
            "  console.log(dropLast([1, 2, 3]));\n",
            "  console.log(dropLast([5, 6] as [number, number]));\n",
            "  console.log(dropFirst([1, 2, 3]));\n",
            "  console.log(dropFirst([5, 6] as [number, number]));\n",
            "}\n",
        ),
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
        "8\n10\n4\n7\n3\n6\n1\n5\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_array_predicates_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function someAbove(value: number[] | [number, number], threshold: number): boolean;\n",
        "export declare function everyPositive(value: number[] | [number, number]): boolean;\n",
        "export declare function findAbove(value: number[] | [number, number], threshold: number): number;\n",
        "export declare function findIndexAbove(value: number[] | [number, number], threshold: number): number;\n",
        "export declare function findLastBelow(value: number[] | [number, number], threshold: number): number;\n",
        "export declare function findLastIndexBelow(value: number[] | [number, number], threshold: number): number;\n",
        "export declare function filterAbove(value: number[] | [number, number], threshold: number): number[];\n",
    );
    let source = concat!(
        "module.exports.someAbove = (value, threshold) => Array.isArray(value) ? value.some(item => item > threshold) : false; ",
        "module.exports.everyPositive = value => Array.isArray(value) ? value.every(item => item > 0) : false; ",
        "module.exports.findAbove = (value, threshold) => Array.isArray(value) ? value.find(item => item > threshold) ?? -1 : -1; ",
        "module.exports.findIndexAbove = (value, threshold) => Array.isArray(value) ? value.findIndex(item => item > threshold) : -1; ",
        "module.exports.findLastBelow = (value, threshold) => Array.isArray(value) ? value.findLast(item => item < threshold) ?? -1 : -1; ",
        "module.exports.findLastIndexBelow = (value, threshold) => Array.isArray(value) ? value.findLastIndex(item => item < threshold) : -1; ",
        "module.exports.filterAbove = (value, threshold) => Array.isArray(value) ? value.filter(item => item > threshold) : [0];",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "someAbove",
        "everyPositive",
        "findAbove",
        "findIndexAbove",
        "findLastBelow",
        "findLastIndexBelow",
        "filterAbove",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-predicates-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-predicates");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { everyPositive, filterAbove, findAbove, findIndexAbove, findLastBelow, findLastIndexBelow, someAbove } from 'jit-tuple-union-predicates';\n",
            "function main(): void {\n",
            "  console.log(someAbove([1, 2, 3], 2));\n",
            "  console.log(someAbove([5, 6] as [number, number], 6));\n",
            "  console.log(everyPositive([-1, 2]));\n",
            "  console.log(everyPositive([5, 6] as [number, number]));\n",
            "  console.log(findAbove([1, 2, 3], 1));\n",
            "  console.log(findAbove([5, 6] as [number, number], 6));\n",
            "  console.log(findIndexAbove([1, 2, 3], 1));\n",
            "  console.log(findIndexAbove([5, 6] as [number, number], 5));\n",
            "  console.log(findLastBelow([1, 4, 2], 4));\n",
            "  console.log(findLastBelow([5, 6] as [number, number], 6));\n",
            "  console.log(findLastIndexBelow([1, 4, 2], 4));\n",
            "  console.log(findLastIndexBelow([5, 6] as [number, number], 6));\n",
            "  console.log(filterAbove([1, 2, 3], 1).join(','));\n",
            "  console.log(filterAbove([5, 6] as [number, number], 5).join(','));\n",
            "}\n",
        ),
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
        "true\nfalse\nfalse\ntrue\n2\n-1\n1\n1\n2\n5\n2\n0\n2,3\n6\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_array_transforms_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function scale(value: number[] | [number, number], factor: number): number[];\n",
        "export declare function sum(value: number[] | [number, number], initial: number): number;\n",
        "export declare function reverseDigits(value: number[] | [number, number]): number;\n",
    );
    let source = concat!(
        "module.exports.scale = (value, factor) => Array.isArray(value) ? value.map(item => item * factor) : [0]; ",
        "module.exports.sum = (value, initial) => Array.isArray(value) ? value.reduce((total, item) => total + item, initial) : initial; ",
        "module.exports.reverseDigits = value => Array.isArray(value) ? value.reduceRight((total, item) => total * 10 + item, 0) : 0;",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in ["scale", "sum", "reverseDigits"]
        .into_iter()
        .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-transforms-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-transforms");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { reverseDigits, scale, sum } from 'jit-tuple-union-transforms';\n",
            "function main(): void {\n",
            "  console.log(scale([1, 2, 3], 3).join(','));\n",
            "  console.log(scale([5, 6] as [number, number], 2).join(','));\n",
            "  console.log(sum([1, 2, 3], 10));\n",
            "  console.log(sum([5, 6] as [number, number], 10));\n",
            "  console.log(reverseDigits([1, 2, 3]));\n",
            "  console.log(reverseDigits([5, 6] as [number, number]));\n",
            "}\n",
        ),
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
        "3,6,9\n10,12\n16\n21\n321\n65\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_tuple_union_callbacks_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function anyFlag(value: boolean[] | [boolean, boolean]): boolean;\n",
        "export declare function allFlags(value: boolean[] | [boolean, boolean]): boolean;\n",
        "export declare function trueFlags(value: boolean[] | [boolean, boolean]): boolean[];\n",
        "export declare function invertFlags(value: boolean[] | [boolean, boolean]): boolean[];\n",
        "export declare function hasText(value: string[] | [string, string], needle: string): boolean;\n",
        "export declare function findText(value: string[] | [string, string], needle: string): string;\n",
        "export declare function matchingText(value: string[] | [string, string], needle: string): string[];\n",
        "export declare function upperText(value: string[] | [string, string]): string[];\n",
        "export declare function textLengths(value: string[] | [string, string]): number[];\n",
        "export declare function indexedLengths(value: string[] | [string, string], offset: number): number[];\n",
        "export declare function flagScores(value: boolean[] | [boolean, boolean], offset: number): number[];\n",
    );
    let source = concat!(
        "module.exports.anyFlag = value => Array.isArray(value) ? value.some(item => item) : false; ",
        "module.exports.allFlags = value => Array.isArray(value) ? value.every(item => item) : false; ",
        "module.exports.trueFlags = value => Array.isArray(value) ? value.filter(item => item) : [false]; ",
        "module.exports.invertFlags = value => Array.isArray(value) ? value.map(item => !item) : [false]; ",
        "module.exports.hasText = (value, needle) => Array.isArray(value) ? value.some(item => item === needle) : false; ",
        "module.exports.findText = (value, needle) => Array.isArray(value) ? value.find(item => item === needle) ?? 'missing' : 'missing'; ",
        "module.exports.matchingText = (value, needle) => Array.isArray(value) ? value.filter(item => item === needle) : ['missing']; ",
        "module.exports.upperText = value => Array.isArray(value) ? value.map(item => item.toUpperCase()) : ['missing']; ",
        "module.exports.textLengths = value => Array.isArray(value) ? value.map(item => item.length) : [0];",
        "module.exports.indexedLengths = (value, offset) => Array.isArray(value) ? value.map((item, index) => item.length + index + offset) : [0]; ",
        "module.exports.flagScores = (value, offset) => Array.isArray(value) ? value.map((item, index) => (item ? 10 : 0) + index + offset) : [0];",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "anyFlag",
        "allFlags",
        "trueFlags",
        "invertFlags",
        "hasText",
        "findText",
        "matchingText",
        "upperText",
        "textLengths",
        "indexedLengths",
        "flagScores",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-primitive-tuple-union-callbacks-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-primitive-tuple-union-callbacks");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { allFlags, anyFlag, findText, flagScores, hasText, indexedLengths, invertFlags, matchingText, textLengths, trueFlags, upperText } from 'jit-primitive-tuple-union-callbacks';\n",
            "function main(): void {\n",
            "  console.log(anyFlag([false, true, false]));\n",
            "  console.log(anyFlag([false, false] as [boolean, boolean]));\n",
            "  console.log(allFlags([true, true]));\n",
            "  console.log(allFlags([true, false] as [boolean, boolean]));\n",
            "  console.log(trueFlags([false, true, true]).join(','));\n",
            "  console.log(trueFlags([true, false] as [boolean, boolean]).join(','));\n",
            "  console.log(invertFlags([false, true]).join(','));\n",
            "  console.log(invertFlags([true, false] as [boolean, boolean]).join(','));\n",
            "  console.log(hasText(['a', 'b', 'a'], 'b'));\n",
            "  console.log(hasText(['x', 'y'] as [string, string], 'z'));\n",
            "  console.log(findText(['a', 'b', 'a'], 'b'));\n",
            "  console.log(findText(['x', 'y'] as [string, string], 'z'));\n",
            "  console.log(matchingText(['a', 'b', 'a'], 'a').join(','));\n",
            "  console.log(matchingText(['x', 'y'] as [string, string], 'y').join(','));\n",
            "  console.log(upperText(['a', 'bb']).join(','));\n",
            "  console.log(upperText(['x', 'yy'] as [string, string]).join(','));\n",
            "  console.log(textLengths(['a', 'bb']).join(','));\n",
            "  console.log(textLengths(['xxx', 'y'] as [string, string]).join(','));\n",
            "  console.log(indexedLengths(['a', 'bb'], 2).join(','));\n",
            "  console.log(indexedLengths(['xxx', 'y'] as [string, string], 1).join(','));\n",
            "  console.log(flagScores([true, false], 1).join(','));\n",
            "  console.log(flagScores([false, true] as [boolean, boolean], 2).join(','));\n",
            "}\n",
        ),
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
        "true\nfalse\ntrue\nfalse\ntrue,true\ntrue\ntrue,false\nfalse,true\ntrue\nfalse\nb\nmissing\na,a\ny\nA,BB\nX,YY\n1,2\n3,1\n3,5\n4,3\n11,2\n2,13\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_callback_context_uses_jit_without_quickjs() {
    let dts = concat!(
        "export declare function mapContext(value: number[] | [number, number], factor: number, offset: number): number[];\n",
        "export declare function filterContext(value: number[] | [number, number], minimum: number, bonus: number): number[];\n",
        "export declare function someContext(value: number[] | [number, number], minimum: number, bonus: number): boolean;\n",
        "export declare function reduceContext(value: number[] | [number, number], multiplier: number, offset: number): number;\n",
        "export declare function stringContext(value: string[] | [string, string], suffix: string, offset: number): string[];\n",
        "export declare function booleanContext(value: boolean[] | [boolean, boolean], expected: boolean, offset: number): boolean[];\n",
    );
    let source = concat!(
        "module.exports.mapContext = (value, factor, offset) => Array.isArray(value) ? value.map((item, index, source) => item * factor + index + source.length + offset) : [0]; ",
        "module.exports.filterContext = (value, minimum, bonus) => Array.isArray(value) ? value.filter((item, index, source) => item + index + bonus >= source.length + minimum) : [0]; ",
        "module.exports.someContext = (value, minimum, bonus) => Array.isArray(value) ? value.some((item, index, source) => item + index + bonus >= source.length + minimum) : false; ",
        "module.exports.reduceContext = (value, multiplier, offset) => Array.isArray(value) ? value.reduce((total, item, index, source) => total * multiplier + item + index + source.length + offset, 0) : 0;",
        "module.exports.stringContext = (value, suffix, offset) => Array.isArray(value) ? value.map((item, index, source) => item + suffix + String(index + source.length + offset)) : ['']; ",
        "module.exports.booleanContext = (value, expected, offset) => Array.isArray(value) ? value.map((item, index, source) => item === (expected && index + offset < source.length)) : [false];",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "mapContext",
        "filterContext",
        "someContext",
        "reduceContext",
        "stringContext",
        "booleanContext",
    ]
        .into_iter()
        .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-callback-context-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-callback-context");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { booleanContext, filterContext, mapContext, reduceContext, someContext, stringContext } from 'jit-tuple-union-callback-context';\n",
            "function main(): void {\n",
            "  console.log(mapContext([1, 2, 3], 2, 1).join(','));\n",
            "  console.log(mapContext([5, 6] as [number, number], 3, 2).join(','));\n",
            "  console.log(filterContext([1, 2, 3], 1, 0).join(','));\n",
            "  console.log(filterContext([5, 6] as [number, number], 5, 0).join(','));\n",
            "  console.log(someContext([1, 2, 3], 2, 0));\n",
            "  console.log(someContext([5, 6] as [number, number], 6, 0));\n",
            "  console.log(reduceContext([1, 2], 2, 1));\n",
            "  console.log(reduceContext([5, 6] as [number, number], 3, 2));\n",
            "  console.log(stringContext(['a', 'b'], '!', 1).join(','));\n",
            "  console.log(stringContext(['x', 'y'] as [string, string], '?', 0).join(','));\n",
            "  console.log(booleanContext([true, false], true, 0).join(','));\n",
            "  console.log(booleanContext([false, true] as [boolean, boolean], false, 1).join(','));\n",
            "}\n",
        ),
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
        "6,9,12\n19,23\n3\n6\ntrue\nfalse\n14\n38\na!3,b!4\nx?2,y?3\ntrue,false\ntrue,false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_reducers_without_initial_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function foldLeft(value: number[] | [number, number]): number;\n",
        "export declare function foldRight(value: number[] | [number, number]): number;\n",
        "export declare function foldContext(value: number[] | [number, number], factor: number, offset: number): number;\n",
    );
    let source = concat!(
        "module.exports.foldLeft = value => Array.isArray(value) ? value.reduce((total, item) => total * 10 + item) : 0; ",
        "module.exports.foldRight = value => Array.isArray(value) ? value.reduceRight((total, item) => total * 10 + item) : 0; ",
        "module.exports.foldContext = (value, factor, offset) => Array.isArray(value) ? value.reduce((total, item, index, source) => total * factor + item + index + source.length + offset) : 0;",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in ["foldLeft", "foldRight", "foldContext"]
        .into_iter()
        .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-reduce-no-initial-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-reduce-no-initial");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { foldContext, foldLeft, foldRight } from 'jit-tuple-union-reduce-no-initial';\n",
            "function main(): void {\n",
            "  console.log(foldLeft([1, 2, 3]));\n",
            "  console.log(foldLeft([5, 6] as [number, number]));\n",
            "  console.log(foldRight([1, 2, 3]));\n",
            "  console.log(foldRight([5, 6] as [number, number]));\n",
            "  console.log(foldContext([1, 2, 3], 2, 1));\n",
            "  console.log(foldContext([5, 6] as [number, number], 3, 2));\n",
            "  try { foldLeft([]); } catch { console.log('empty-left'); }\n",
            "  try { foldRight([]); } catch { console.log('empty-right'); }\n",
            "}\n",
        ),
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
        "123\n56\n321\n65\n27\n26\nempty-left\nempty-right\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_iteration_and_spread_use_jit_without_quickjs() {
    let dts = concat!(
        "export declare function spreadCopy(value: number[] | [number, number]): number[];\n",
        "export declare function sumForOf(value: number[] | [number, number]): number;\n",
        "export declare function weightedForOf(value: number[] | [number, number], factor: number): number;\n",
    );
    let source = concat!(
        "module.exports.spreadCopy = value => [0, ...value, 9]; ",
        "module.exports.sumForOf = value => { let total = 0; for (const item of value) total += item; return total; }; ",
        "module.exports.weightedForOf = (value, factor) => { let total = 0; for (const item of value) total += item * factor; return total; };",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in ["spreadCopy", "sumForOf", "weightedForOf"]
        .into_iter()
        .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-iteration-spread-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-iteration-spread");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { spreadCopy, sumForOf, weightedForOf } from 'jit-tuple-union-iteration-spread';\n",
            "function main(): void {\n",
            "  console.log(spreadCopy([1, 2, 3]).join(','));\n",
            "  console.log(spreadCopy([5, 6] as [number, number]).join(','));\n",
            "  console.log(sumForOf([1, 2, 3]));\n",
            "  console.log(sumForOf([5, 6] as [number, number]));\n",
            "  console.log(weightedForOf([1, 2, 3], 2));\n",
            "  console.log(weightedForOf([5, 6] as [number, number], 3));\n",
            "}\n",
        ),
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
        "0,1,2,3,9\n0,5,6,9\n6\n11\n12\n33\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tuple_union_destructuring_uses_jit_without_quickjs() {
    let dts = concat!(
        "export declare function numberPair(value: number[] | [number, number]): number;\n",
        "export declare function stringEnds(value: string[] | [string, string, string]): string;\n",
        "export declare function booleanPair(value: boolean[] | [boolean, boolean]): boolean;\n",
        "export declare function splicePair(value: number[] | [number, number]): number;\n",
        "export declare function reversedPair(value: number[] | [number, number]): number;\n",
        "export declare function joined(value: number[] | [number, number]): string;\n",
        "export declare function numberDefaults(value: number[] | [number, number]): number;\n",
        "export declare function stringDefaults(value: string[] | [string, string]): string;\n",
        "export declare function booleanDefaults(value: boolean[] | [boolean, boolean]): boolean;\n",
        "export declare function lazyDefault(value: number[] | [number, number]): number;\n",
        "export declare function numberRest(value: number[] | [number, number]): number;\n",
        "export declare function stringRest(value: string[] | [string, string]): string;\n",
        "export declare function restCopy(value: number[]): number;\n",
        "export declare function chainedCopy(value: number[] | [number, number]): string;\n",
        "export declare function chainedSplice(value: number[] | [number, number]): string;\n",
        "export declare function assignPair(value: number[] | [number, number]): number;\n",
        "export declare function assignDefaults(value: number[] | [number, number]): number;\n",
        "export declare function assignDuplicate(value: number[] | [number, number]): number;\n",
        "export declare function assignControl(value: number[] | [number, number], flag: boolean): number;\n",
        "export declare function assignRestControl(value: number[] | [number, number], flag: boolean): number;\n",
        "export declare function assignInsideIf(value: number[] | [number, number], reverse: boolean): number;\n",
        "export declare function assignInsideLoop(value: number[] | [number, number]): number;\n",
        "export declare function assignInsideTry(value: number[] | [number, number]): number;\n",
        "export declare function assignRestInsideIf(value: number[] | [number, number], active: boolean): number;\n",
    );
    let source = concat!(
        "module.exports.numberPair = value => { const [first, second] = value; return first * 10 + second; }; ",
        "module.exports.stringEnds = value => { const [first, , third] = value; return first + third; }; ",
        "module.exports.booleanPair = value => { let [first, second] = value; first = !first; return first && second; };",
        "module.exports.splicePair = value => { const [first, second] = value.splice(0, 2); return first * 100 + second * 10; }; ",
        "module.exports.reversedPair = value => { const [first, second] = value.toReversed(); return first * 10 + second; }; ",
        "module.exports.joined = value => value.join('-'); ",
        "module.exports.numberDefaults = value => { const [first = 7, second = first + 1] = value; return first * 10 + second; }; ",
        "module.exports.stringDefaults = value => { const [first = 'x', second = 'y'] = value; return first + second; }; ",
        "module.exports.booleanDefaults = value => { const [first = true, second = false] = value; return first && !second; }; ",
        "module.exports.lazyDefault = value => { const [first = value.pop()] = value; return first * 10 + value.join('-').length; }; ",
        "module.exports.numberRest = value => { const [first, ...rest] = value; rest.push(9); return first * 100 + rest.length * 10 + rest.at(-1); }; ",
        "module.exports.stringRest = value => { const [first, ...rest] = value; rest.push('z'); return first + rest.join(''); }; ",
        "module.exports.restCopy = value => { const [...copy] = value; copy.push(9); return copy.length * 10 + value.length; }; ",
        "module.exports.chainedCopy = value => value.toReversed().slice(0, 2).join('-'); ",
        "module.exports.chainedSplice = value => value.splice(0, 2).join('-') + ':' + value.join('-'); ",
        "module.exports.assignPair = value => { let first = 0; let second = 0; [first, second] = value.splice(0, 2); return first * 100 + second * 10; }; ",
        "module.exports.assignDefaults = value => { let first = 1; let second = 2; [first = 7, second = first + 1] = value; return first * 10 + second; }; ",
        "module.exports.assignDuplicate = value => { let selected = 0; [selected, selected] = value; return selected; }; ",
        "module.exports.assignControl = (value, flag) => { let first = 0; let second = 0; [first, second] = value; if (flag) first += 1; return first * 10 + second; }; ",
        "module.exports.assignRestControl = (value, flag) => { let first = 0; let rest = value.slice(0, 0); [first, ...rest] = value; if (flag) rest.push(9); return first * 100 + rest.length * 10 + rest.at(-1); }; ",
        "module.exports.assignInsideIf = (value, reverse) => { let first = 0; let second = 0; if (reverse) { [first, second] = value.toReversed(); } else { [first, second] = value; } return first * 10 + second; }; ",
        "module.exports.assignInsideLoop = value => { let first = 0; let second = 0; let index = 0; while (index < 1) { [first, second] = value; index++; } return first * 10 + second; }; ",
        "module.exports.assignInsideTry = value => { let first = 0; let second = 0; try { [first, second] = value; } finally { second += 1; } return first * 10 + second; }; ",
        "module.exports.assignRestInsideIf = (value, active) => { let first = 0; let rest = value.slice(0, 0); if (active) { [first, ...rest] = value; } return first * 100 + rest.length * 10 + rest.at(-1); };",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "numberPair",
        "stringEnds",
        "booleanPair",
        "splicePair",
        "reversedPair",
        "joined",
        "numberDefaults",
        "stringDefaults",
        "booleanDefaults",
        "lazyDefault",
        "numberRest",
        "stringRest",
        "restCopy",
        "chainedCopy",
        "chainedSplice",
        "assignPair",
        "assignDefaults",
        "assignDuplicate",
        "assignControl",
        "assignRestControl",
        "assignInsideIf",
        "assignInsideLoop",
        "assignInsideTry",
        "assignRestInsideIf",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tuple-union-destructuring-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tuple-union-destructuring");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        concat!(
            "import { assignControl, assignDefaults, assignDuplicate, assignInsideIf, assignInsideLoop, assignInsideTry, assignPair, assignRestControl, assignRestInsideIf, booleanDefaults, booleanPair, chainedCopy, chainedSplice, joined, lazyDefault, numberDefaults, numberPair, numberRest, restCopy, reversedPair, splicePair, stringDefaults, stringEnds, stringRest } from 'jit-tuple-union-destructuring';\n",
            "function main(): void {\n",
            "  console.log(numberPair([1, 2, 3]));\n",
            "  console.log(numberPair([5, 6] as [number, number]));\n",
            "  console.log(stringEnds(['a', 'b', 'c', 'd']));\n",
            "  console.log(stringEnds(['x', 'y', 'z'] as [string, string, string]));\n",
            "  console.log(booleanPair([false, true, false]));\n",
            "  console.log(booleanPair([true, true] as [boolean, boolean]));\n",
            "  console.log(splicePair([1, 2, 3]));\n",
            "  console.log(splicePair([5, 6] as [number, number]));\n",
            "  console.log(reversedPair([1, 2, 3]));\n",
            "  console.log(reversedPair([5, 6] as [number, number]));\n",
            "  console.log(joined([1, 2, 3]));\n",
            "  console.log(joined([5, 6] as [number, number]));\n",
            "  console.log(numberDefaults([]));\n",
            "  console.log(numberDefaults([3]));\n",
            "  console.log(stringDefaults(['a']));\n",
            "  console.log(booleanDefaults([true]));\n",
            "  console.log(booleanDefaults([false]));\n",
            "  console.log(lazyDefault([4, 5]));\n",
            "  console.log(numberRest([1, 2, 3]));\n",
            "  console.log(numberRest([5, 6] as [number, number]));\n",
            "  console.log(stringRest(['a', 'b', 'c']));\n",
            "  console.log(stringRest(['x', 'y'] as [string, string]));\n",
            "  console.log(restCopy([1, 2]));\n",
            "  console.log(chainedCopy([1, 2, 3]));\n",
            "  console.log(chainedCopy([5, 6] as [number, number]));\n",
            "  console.log(chainedSplice([1, 2, 3]));\n",
            "  console.log(chainedSplice([5, 6] as [number, number]));\n",
            "  console.log(assignPair([1, 2, 3]));\n",
            "  console.log(assignPair([5, 6] as [number, number]));\n",
            "  console.log(assignDefaults([]));\n",
            "  console.log(assignDefaults([3]));\n",
            "  console.log(assignDuplicate([4, 9]));\n",
            "  console.log(assignControl([1, 2], false));\n",
            "  console.log(assignControl([5, 6] as [number, number], true));\n",
            "  console.log(assignRestControl([1, 2, 3], false));\n",
            "  console.log(assignRestControl([5, 6] as [number, number], true));\n",
            "  console.log(assignInsideIf([1, 2], false));\n",
            "  console.log(assignInsideIf([5, 6] as [number, number], true));\n",
            "  console.log(assignInsideLoop([3, 4]));\n",
            "  console.log(assignInsideTry([7, 8]));\n",
            "  console.log(assignRestInsideIf([1, 2, 3], true));\n",
            "}\n",
        ),
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
        "12\n56\nac\nxz\ntrue\nfalse\n120\n560\n32\n65\n1-2-3\n5-6\n78\n34\nay\ntrue\nfalse\n43\n139\n529\nabcz\nxyz\n32\n3-2\n6-5\n1-2:3\n5-6:\n120\n560\n78\n34\n9\n12\n66\n123\n529\n12\n65\n34\n79\n123\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

