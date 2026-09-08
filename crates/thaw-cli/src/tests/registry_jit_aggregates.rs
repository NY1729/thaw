#[test]
fn bare_imports_automatically_register_installed_packages() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-installed-bare-imports-{}",
        std::process::id()
    ));
    let project = dir.join("project");
    let package = project.join("node_modules/installed-kit");
    let registry = dir.join("registry");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{"name":"installed-kit","version":"1.0.0","types":"index.d.ts","main":"index.js"}"#,
    )
    .unwrap();
    std::fs::write(
        package.join("index.d.ts"),
        "export declare function twice(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(package.join("index.js"), "exports.twice = value => value * 2;\n").unwrap();
    let entry = project.join("src/main.ts");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(
        &entry,
        "import { twice } from 'installed-kit';\nfunction main(): void { console.log(twice(21)); }\n",
    )
    .unwrap();

    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    assert!(registry.join("installed-kit/package.d.ts").is_file());
    let result = Command::new(output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_bare_import_suggests_installing_the_project() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-missing-bare-import-{}",
        std::process::id()
    ));
    let project = dir.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("package.json"), r#"{"dependencies":{"missing-kit":"1.0.0"}}"#)
        .unwrap();
    let entry = project.join("src/main.ts");
    std::fs::write(
        &entry,
        "import { value } from 'missing-kit';\nfunction main(): void { console.log(value); }\n",
    )
    .unwrap();

    let error = build(
        &entry,
        &dir.join("app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap_err();
    assert!(error.contains(&format!("help: run `thaw install {}`", project.display())));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bare_imports_automatically_resolve_registry_packages() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-bare-imports-{}", std::process::id()));
    let registry = dir.join("modules");
    std::fs::create_dir_all(registry.join("math-kit")).unwrap();
    std::fs::write(
            registry.join("math-kit/package.d.ts"),
            "export interface Point { x: number; y: number; }\nexport declare function add(a: number, b: number): number;\nexport declare function sub(a: number, b: number): number;\nexport declare function greet(name: string): string;\nexport declare function negate(value: boolean): boolean;\nexport declare function echo(value: Json): Json;\nexport declare function sum(values: number[]): number;\nexport declare function reverse(values: number[]): number[];\nexport declare function shift(point: Point): Point;\nexport declare function fail(): number;\n",
        )
        .unwrap();
    std::fs::write(
            registry.join("math-kit/bundle.js"),
            "module.exports = { add: function(a,b){ return a+b; }, sub: function(a,b){ return a-b; }, greet: function(name){ return 'hello ' + name; }, negate: function(value){ return !value; }, echo: function(value){ return value; }, sum: function(values){ return values.reduce(function(a,b){ return a+b; }, 0); }, reverse: function(values){ return values.reverse(); }, shift: function(point){ return { x: point.x + 1, y: point.y + 2 }; }, fail: function(){ throw new Error('typed dynamic failed'); } };\n",
        )
        .unwrap();
    std::fs::create_dir_all(registry.join("math-kit/subpaths/advanced")).unwrap();
    std::fs::write(
        registry.join("math-kit/subpaths/advanced/package.d.ts"),
        "export declare function square(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        registry.join("math-kit/subpaths/advanced/bundle.js"),
        "module.exports = { square: function(value){ return value * value; } };\n",
    )
    .unwrap();
    std::fs::create_dir_all(registry.join("twice")).unwrap();
    std::fs::write(
        registry.join("twice/package.d.ts"),
        "export default function twice(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        registry.join("twice/bundle.js"),
        "module.exports = function(value){ return value * 2; };\n",
    )
    .unwrap();

    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import { add, greet, negate, echo, sum, reverse, shift, fail } from "math-kit";
                import * as math from "math-kit";
                import { square } from "math-kit/advanced";
                import twice from "twice";
                function main(): void {
                    const sum: number = add(10, 11);
                    const difference: number = math.sub(13, 2);
                    console.log(twice(21));
                    console.log(sum + difference);
                    console.log(square(7));
                    console.log(greet("thaw"));
                    console.log(negate(false));
                    console.log(String(echo(JSON.parse("{\"ok\":true}")).ok));
                    console.log(sum([10, 20, 12]));
                    const reversed = reverse([1, 2, 3]);
                    console.log(reversed[0]);
                    const point = shift({ x: 3, y: 4 });
                    console.log(point.x * 10 + point.y);
                    console.log(import.meta.resolve("math-kit"));
                    console.log(import.meta.resolve("math-kit/advanced?raw"));
                    try {
                        const ignored = fail();
                    } catch (error) {
                        console.log(error);
                    }
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!(
                "42\n32\n49\nhello thaw\ntrue\ntrue\n42\n3\n46\nfile://{}\nfile://{}?raw\n`math-kit::fail` threw: typed dynamic failed\n",
                registry.join("math-kit/bundle.js").display(),
                registry
                    .join("math-kit/subpaths/advanced/bundle.js")
                    .display()
            )
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nested_primitive_objects_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-fields-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-object-fields");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Meta { label: string; enabled: boolean; }\nexport interface Payload { value: number; meta: Meta; values: number[]; flags: boolean[]; names: string[]; }\nexport declare function summarize(record: Payload, offset: number): string;\nexport declare function score(record: Payload, multiplier: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.summarize = (record, offset) => record.meta.enabled ? record.meta.label + ':' + (record.value + record.values[0] + offset) : 'disabled'; module.exports.score = (record, multiplier) => record.meta.enabled ? record.value * multiplier + record.meta.label.length + record.values.length + (record.flags[0] ? 1 : 0) + record.names[0].length : 0;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { summarize, score } from 'jit-object-fields';\nfunction main(): void { const active = { value: 39, meta: { label: 'ok', enabled: true }, values: [1, 2], flags: [true], names: ['x'] }; const inactive = { value: 40, meta: { label: 'no', enabled: false }, values: [1], flags: [true], names: ['x'] }; console.log(summarize(active, 2)); console.log(score(active, 1)); console.log(summarize(inactive, 2)); console.log(score(inactive, 1)); }\n",
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
        "ok:42\n45\ndisabled\n0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn fixed_aggregate_results_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-results-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-object-results");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Meta { doubled: number; first: string; }\nexport interface Result { total: number; ok: boolean; label: string; numbers: number[]; meta: Meta; }\nexport interface Echo { value: number; name: string; values: number[]; }\nexport interface LocalResult { first: number; remaining: number; doubled: number; copy: number[]; }\nexport interface Choice { selected: string; value: number; remaining: number; }\nexport declare function build(value: number, name: string, values: number[]): Result;\nexport declare function echo(value: number, name: string, values: number[]): Echo;\nexport declare function tuple(value: number, name: string, values: number[]): [number, string, boolean, number[], { label: string; pair: [number, number[]] }];\nexport declare function summarizeTuple(input: [number, string, boolean, number[], { scale: number }, [boolean, string[]]]): string;\nexport declare function locals(values: number[]): LocalResult;\nexport declare function randomPair(): [number, number];\nexport declare function choose(values: number[]): Choice;\nexport declare function chooseStatement(values: number[]): Choice;\nexport declare function mutable(values: number[], start: number): Choice;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.build = (value, name, values) => ({ total: value + values[0], ok: name.length > 0, label: name.toUpperCase(), numbers: values.map(item => item + value), meta: { doubled: value * 2, first: name.charAt(0) } }); module.exports.echo = (value, name, values) => ({ values, name, value }); module.exports.tuple = (value, name, values) => [value + 2, name.toUpperCase(), name.length > 0, values.map(item => item + value), { label: name, pair: [value * 2, values] }]; module.exports.summarizeTuple = input => input[2] && input[5][0] ? input[1] + ':' + (input[0] + input[3][0] + input[4].scale) + ':' + input[5][1][0] : 'disabled'; module.exports.locals = function(values) { const first = values.pop(); const doubled = first * 2; const copy = values.slice(); return { first, remaining: values.length, doubled, copy }; }; module.exports.randomPair = function() { const value = Math.random(); return [value, value]; }; module.exports.choose = values => values.pop() > 0 ? { selected: 'yes', value: values.pop(), remaining: values.length } : { selected: 'no', value: values.pop(), remaining: values.length }; module.exports.chooseStatement = function(values) { if (values.pop() > 0) { return { selected: 'yes', value: values.pop(), remaining: values.length }; } return { selected: 'no', value: values.pop(), remaining: values.length }; }; module.exports.mutable = function(values, start) { let total = start; total += values.pop(); total *= 2; total--; values.pop(); return { selected: 'mutable', value: total, remaining: values.length }; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { build, echo, tuple, summarizeTuple, locals, randomPair, choose, chooseStatement, mutable } from 'jit-object-results';\nfunction main(): void { const result = build(40, 'thaw', [2, 3]); console.log(result.total); console.log(result.ok); console.log(result.label); console.log(result.numbers.join('|')); console.log(result.meta.doubled); console.log(result.meta.first); const copy = echo(7, 'ok', [8, 9]); console.log(copy.value + ':' + copy.name + ':' + copy.values.join('|')); const values = tuple(5, 'hi', [1, 2]); console.log(values[0] + ':' + values[1] + ':' + values[2] + ':' + values[3].join('|')); console.log(values[4].label + ':' + values[4].pair[0] + ':' + values[4].pair[1].join('|')); console.log(summarizeTuple([39, 'ok', true, [1], { scale: 2 }, [true, ['done']]])); const source = [1, 2, 3]; const local = locals(source); console.log(local.first + ':' + local.remaining + ':' + local.doubled + ':' + local.copy.join('|') + ':' + source.length); const random = randomPair(); console.log(random[0] === random[1]); const yesSource = [1, 2, 3]; const yes = choose(yesSource); console.log(yes.selected + ':' + yes.value + ':' + yes.remaining + ':' + yesSource.length); const noSource = [1, 2, -1]; const no = choose(noSource); console.log(no.selected + ':' + no.value + ':' + no.remaining + ':' + noSource.length); const statementSource = [4, 5, 6]; const statement = chooseStatement(statementSource); console.log(statement.selected + ':' + statement.value + ':' + statement.remaining + ':' + statementSource.length); const mutableSource = [1, 2, 3]; const changed = mutable(mutableSource, 1); console.log(changed.selected + ':' + changed.value + ':' + changed.remaining + ':' + mutableSource.length); }\n",
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
        "42\ntrue\nTHAW\n42|43\n80\nt\n7:ok:8|9\n7:HI:true:6|7\nhi:10:1|2\nok:42:done\n3:2:6:1|2:2\ntrue\nyes:2:1:1\nno:2:1:1\nyes:5:1:1\nmutable:7:1:1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn logical_expressions_short_circuit_in_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-logical-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-logical");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Result { value: number; remaining: number; }\nexport declare function orPop(values: number[]): number;\nexport declare function andPop(values: number[]): number;\nexport declare function nested(values: number[]): number;\nexport declare function embedded(values: number[]): number;\nexport declare function conditional(values: number[], chooseLast: boolean): number;\nexport declare function nestedConditional(values: number[], mutate: boolean, chooseLast: boolean): number;\nexport declare function coalesce(values: number[], value?: number): number;\nexport declare function coalesceNested(values: number[], first?: number, second?: number): number;\nexport declare function coalescePop(values: number[]): number;\nexport declare function optionalPop(values?: number[]): number;\nexport declare function coalesceAt(values: number[], index: number): number;\nexport declare function coalesceGet(values: number[], index: number): number;\nexport declare function coalesceFind(values: number[]): number;\nexport declare function coalesceStringAt(value: string, index: number): string;\nexport declare function coalesceString(values: string[], value?: string): string;\nexport declare function coalesceFlag(values: boolean[], value?: boolean): boolean;\nexport declare function optionalLength(value?: string): number | undefined;\nexport declare function optionalUpper(value?: string): string;\nexport declare function optionalPush(source: number[], values?: number[]): number;\nexport declare function object(values: number[]): Result;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orPop = values => values.pop() || values.pop(); module.exports.andPop = values => values.pop() && values.pop(); module.exports.nested = values => values.pop() || values.pop() || values.pop(); module.exports.embedded = values => (values.pop() || values.pop()) + values.length; module.exports.conditional = (values, chooseLast) => (chooseLast ? values.pop() : values.shift()) + values.length; module.exports.nestedConditional = (values, mutate, chooseLast) => (mutate ? (chooseLast ? values.pop() : values.shift()) : values[1]) + values.length; module.exports.coalesce = (values, value) => (value ?? values.pop()) + values.length; module.exports.coalesceNested = (values, first, second) => (first ?? second ?? values.pop()) + values.length; module.exports.coalescePop = values => values.pop() ?? values.pop() ?? 42; module.exports.optionalPop = values => values?.pop() ?? 42; module.exports.coalesceAt = (values, index) => values.at(index) ?? 42; module.exports.coalesceGet = (values, index) => values[index] ?? 42; module.exports.coalesceFind = values => values.find(value => value > 2) ?? 42; module.exports.coalesceStringAt = (value, index) => value.at(index) ?? 'missing'; module.exports.coalesceString = (values, value) => value ?? values.pop(); module.exports.coalesceFlag = (values, value) => value ?? values.pop(); module.exports.optionalLength = value => value?.length; module.exports.optionalUpper = value => value?.toUpperCase() ?? 'missing'; module.exports.optionalPush = (source, values) => values?.push(source.pop()) ?? 0; module.exports.object = values => ({ value: values.pop() || values.pop(), remaining: values.length });\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orPop, andPop, nested, embedded, conditional, nestedConditional, coalesce, coalesceNested, coalescePop, optionalPop, coalesceAt, coalesceGet, coalesceFind, coalesceStringAt, coalesceString, coalesceFlag, optionalLength, optionalUpper, optionalPush, object } from 'jit-logical';\nfunction main(): void { const orHit = [1, 2, 3]; console.log(orPop(orHit) + ':' + orHit.length); const orMiss = [1, 2, 0]; console.log(orPop(orMiss) + ':' + orMiss.length); const andMiss = [1, 2, 0]; console.log(andPop(andMiss) + ':' + andMiss.length); const andHit = [1, 2, 3]; console.log(andPop(andHit) + ':' + andHit.length); const nestedValues = [1, 4, 0, 0]; console.log(nested(nestedValues) + ':' + nestedValues.length); const nestedHit = [1, 2, 3]; console.log(nested(nestedHit) + ':' + nestedHit.length); const embeddedValues = [1, 2, 3]; console.log(embedded(embeddedValues) + ':' + embeddedValues.length); const trueValues = [1, 2, 3]; console.log(conditional(trueValues, true) + ':' + trueValues.join('|')); const falseValues = [1, 2, 3]; console.log(conditional(falseValues, false) + ':' + falseValues.join('|')); const nestedTrue = [1, 2, 3]; console.log(nestedConditional(nestedTrue, true, true) + ':' + nestedTrue.join('|')); const nestedFalse = [1, 2, 3]; console.log(nestedConditional(nestedFalse, false, true) + ':' + nestedFalse.join('|')); const presentZero = [1, 2, 3]; console.log(coalesce(presentZero, 0) + ':' + presentZero.length); const absent = [1, 2, 3]; console.log(coalesce(absent) + ':' + absent.join('|')); const firstZero = [1, 2, 3]; console.log(coalesceNested(firstZero, 0, 9) + ':' + firstZero.length); const secondPresent = [1, 2, 3]; console.log(coalesceNested(secondPresent, undefined, 5) + ':' + secondPresent.length); const bothAbsent = [1, 2, 3]; console.log(coalesceNested(bothAbsent) + ':' + bothAbsent.join('|')); const popped = [1, 2]; console.log(coalescePop(popped) + ':' + popped.join('|')); const poppedZero = [1, 0]; console.log(coalescePop(poppedZero) + ':' + poppedZero.join('|')); console.log(coalescePop([])); console.log(optionalPop([5])); console.log(optionalPop([])); console.log(optionalPop()); console.log(coalesceAt([3], 0)); console.log(coalesceAt([3], 1)); console.log(coalesceGet([4], 0)); console.log(coalesceGet([4], 1)); console.log(coalesceFind([1, 3])); console.log(coalesceFind([1, 2])); console.log(coalesceStringAt('x', 0)); console.log(coalesceStringAt('x', 1)); const presentEmpty = ['fallback']; console.log('<' + coalesceString(presentEmpty, '') + '>:' + presentEmpty.length); const absentString = ['fallback']; console.log(coalesceString(absentString) + ':' + absentString.length); const presentFalse = [true]; console.log(coalesceFlag(presentFalse, false) + ':' + presentFalse.length); const absentFlag = [true]; console.log(coalesceFlag(absentFlag) + ':' + absentFlag.length); console.log(optionalLength('😀')); console.log(optionalLength() === undefined); console.log(optionalUpper('thaw')); console.log(optionalUpper()); const pushSource = [2, 3]; const pushTarget = [1]; console.log(optionalPush(pushSource, pushTarget) + ':' + pushSource.join('|') + ':' + pushTarget.join('|')); const skippedSource = [2, 3]; console.log(optionalPush(skippedSource) + ':' + skippedSource.join('|')); const objectValues = [1, 5, 0]; const result = object(objectValues); console.log(result.value + ':' + result.remaining + ':' + objectValues.length); }\n",
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
        "3:2\n2:1\n0:2\n2:1\n4:1\n3:2\n5:2\n5:1|2\n3:2|3\n5:1|2\n5:1|2|3\n3:3\n5:1|2\n3:3\n8:3\n5:1|2\n2:1\n0:1\n42\n5\n42\n42\n3\n42\n4\n42\n3\n42\nx\nmissing\n<>:1\nfallback:0\nfalse:1\ntrue:0\n2\ntrue\nTHAW\nmissing\n2:2:1|3\n0:2|3\n5:1:1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_operation_receivers_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-result-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-result");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function formatPop(values: number[]): string;\nexport declare function formatPopWithDigits(values: number[], digits: number[]): string;\nexport declare function upperAt(values: string[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.formatPop = values => values.pop()?.toFixed(1) ?? 'missing'; module.exports.formatPopWithDigits = (values, digits) => values.pop()?.toFixed(digits.pop() ?? 0) ?? 'missing'; module.exports.upperAt = values => values.at(0)?.toUpperCase() ?? 'missing';\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { formatPop, formatPopWithDigits, upperAt } from 'jit-optional-result';\nfunction main(): void { console.log(formatPop([2])); console.log(formatPop([])); const skipped = [1]; console.log(formatPopWithDigits([], skipped) + ':' + skipped.length); const used = [1]; console.log(formatPopWithDigits([2], used) + ':' + used.length); console.log(upperAt(['thaw'])); console.log(upperAt([])); }\n",
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
        "2.0\nmissing\nmissing:1\n2.0:0\nTHAW\nmissing\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_recursion_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-recursion-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-recursion");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Counter { value: number; total: number; }\nexport interface Collection { values: number[]; bonus: number; }\nexport declare function factorial(value: number): number;\nexport declare function gcd(left: number, right: number): number;\nexport declare function isEven(value: number): boolean;\nexport declare function punctuate(count: number, value: string): string;\nexport declare function ping(value: number): number;\nexport declare function sum(values: number[], index: number): number;\nexport declare function drain(values: number[]): number;\nexport declare function joinDown(values: string[], index: number): string;\nexport declare function objectSum(state: Counter): number;\nexport declare function tupleSum(state: [number, number]): number;\nexport declare function collectionSum(state: Collection, index: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function factorial(value) { return value <= 1 ? 1 : value * factorial(value - 1); } function gcd(left, right) { return right === 0 ? left : gcd(right, left % right); } function isEven(value) { return value === 0 ? true : !isEven(value - 1); } function punctuate(count, value) { return count <= 0 ? value : punctuate(count - 1, value + '!'); } function ping(value) { return value <= 0 ? 0 : pong(value - 1) + 1; } function pong(value) { return value <= 0 ? 0 : ping(value - 1) + 1; } function sum(values, index) { return index >= values.length ? 0 : values[index] + sum(values, index + 1); } function drain(values) { return values.length === 0 ? 0 : values.pop() + drain(values); } function joinDown(values, index) { return index >= values.length ? '' : values[index] + joinDown(values, index + 1); } function objectSum(state) { return state.value <= 0 ? state.total : objectSum({ value: state.value - 1, total: state.total + state.value }); } function tupleSum(state) { return state[0] <= 0 ? state[1] : tupleSum([state[0] - 1, state[1] + state[0]]); } function collectionSum(state, index) { return index >= state.values.length ? state.bonus : state.values[index] + collectionSum(state, index + 1); } module.exports = { factorial, gcd, isEven, punctuate, ping, sum, drain, joinDown, objectSum, tupleSum, collectionSum };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { factorial, gcd, isEven, punctuate, ping, sum, drain, joinDown, objectSum, tupleSum, collectionSum } from 'jit-recursion';\nfunction main(): void { console.log(factorial(6)); console.log(gcd(1071, 462)); console.log(isEven(6)); console.log(isEven(5)); console.log(punctuate(3, 'thaw')); console.log(ping(5)); console.log(sum([1, 2, 3, 4], 0)); const values = [1, 2, 3]; console.log(drain(values) + ':' + values.length); console.log(joinDown(['a', 'b', 'c'], 0)); console.log(objectSum({ value: 4, total: 0 })); console.log(tupleSum([4, 0])); console.log(collectionSum({ values: [2, 3, 4], bonus: 1 }, 0)); }\n",
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
        "720\n21\ntrue\nfalse\nthaw!!!\n5\n10\n6:0\nabc\n10\n10\n10\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_dictionaries_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-dictionary-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-dictionary");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function readNumber(values: Record<string, number>, key: string): number;\nexport declare function readBool(values: Record<string, boolean>, key: string): boolean;\nexport declare function readString(values: Record<string, string>, key: string): string;\nexport declare function updateNumber(values: Record<string, number>, key: string, value: number): number;\nexport declare function updateBool(values: Record<string, boolean>, key: string, value: boolean): boolean;\nexport declare function updateString(values: Record<string, string>, key: string, value: string): string;\nexport declare function removeBool(values: Record<string, boolean>, key: string): boolean;\nexport declare function updateAndReturn(values: Record<string, number>, key: string, value: number): Record<string, number>;\nexport declare function makeNumbers(value: number): Record<string, number>;\nexport declare function makeBools(value: boolean): Record<string, boolean>;\nexport declare function makeStrings(value: string): Record<string, string>;\nexport declare function makeDynamic(key: string, value: number, base: Record<string, number>): Record<string, number>;\nexport declare function keys(values: Record<string, number>): string[];\nexport declare function keyCount(values: Record<string, number>): number;\nexport declare function has(values: Record<string, number>, key: string): boolean;\nexport declare function numberValues(values: Record<string, number>): number[];\nexport declare function boolValues(values: Record<string, boolean>): boolean[];\nexport declare function stringValues(values: Record<string, string>): string[];\nexport declare function valueCount(values: Record<string, number>): number;\nexport declare function numberEntries(values: Record<string, number>): [string, number][];\nexport declare function boolEntries(values: Record<string, boolean>): [string, boolean][];\nexport declare function stringEntries(values: Record<string, string>): [string, string][];\nexport declare function entryCount(values: Record<string, number>): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function readNumber(values, key) { return values[key] + values.fixed; } function readBool(values, key) { return values[key] && values.enabled; } function readString(values, key) { return values.prefix + values[key]; } function updateNumber(values, key, value) { values[key] = value; values.fixed++; return values[key] + values.fixed; } function updateBool(values, key, value) { values[key] = value; return values[key]; } function updateString(values, key, value) { values[key] += value; return values[key]; } function removeBool(values, key) { delete values[key]; return !values[key]; } function updateAndReturn(values, key, value) { values[key] = value; return values; } function makeNumbers(value) { return { answer: value, doubled: value * 2 }; } function makeBools(value) { return { chosen: value, inverse: !value }; } function makeStrings(value) { return { prefix: 'th', suffix: value }; } function makeDynamic(key, value, base) { return { first: 1, ...base, [key]: value, first: 2 }; } function keys(values) { return Object.keys(values); } function keyCount(values) { return Object.keys(values).length; } function has(values, key) { return Object.hasOwn(values, key); } function numberValues(values) { return Object.values(values); } function boolValues(values) { return Object.values(values); } function stringValues(values) { return Object.values(values); } function valueCount(values) { return Object.values(values).length; } function numberEntries(values) { return Object.entries(values); } function boolEntries(values) { return Object.entries(values); } function stringEntries(values) { return Object.entries(values); } function entryCount(values) { return Object.entries(values).length; } module.exports = { readNumber, readBool, readString, updateNumber, updateBool, updateString, removeBool, updateAndReturn, makeNumbers, makeBools, makeStrings, makeDynamic, keys, keyCount, has, numberValues, boolValues, stringValues, valueCount, numberEntries, boolEntries, stringEntries, entryCount };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    assert_eq!(functions[0].params.len(), 2);
    assert!(
        matches!(
            &functions[0].params[0].1,
            thaw_bridge::DtsType::Native(thaw_hir::HirType::Dictionary(_))
        ),
        "{:?}",
        functions[0]
    );
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    assert!(jit_numeric_export(
        &bundle,
        "readNumber",
        false,
        &functions[0]
    )
    .is_some());
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { readNumber, readBool, readString, updateNumber, updateBool, updateString, removeBool, updateAndReturn, makeNumbers, makeBools, makeStrings, makeDynamic, keys, keyCount, has, numberValues, boolValues, stringValues, valueCount, numberEntries, boolEntries, stringEntries, entryCount } from 'jit-dictionary';\nfunction main(): void { console.log(readNumber({ chosen: 40, fixed: 2 }, 'chosen')); console.log(readBool({ chosen: true, enabled: true }, 'chosen')); console.log(readBool({ chosen: false, enabled: true }, 'chosen')); console.log(readString({ prefix: 'th', suffix: 'aw' }, 'suffix')); console.log(updateNumber({ chosen: 1, fixed: 2 }, 'chosen', 40)); console.log(updateBool({ chosen: false }, 'chosen', true)); console.log(updateString({ chosen: 'th' }, 'chosen', 'aw')); console.log(removeBool({ chosen: true }, 'chosen')); console.log(updateAndReturn({ chosen: 1 }, 'chosen', 42).chosen); console.log(makeNumbers(21).doubled); console.log(makeBools(false).inverse); console.log(makeStrings('aw').prefix + makeStrings('aw').suffix); const dynamic = makeDynamic('chosen', 42, { first: 9, base: 3 }); console.log(dynamic.first); console.log(dynamic.base); console.log(dynamic.chosen); console.log(keys({ zebra: 1, alpha: 2 }).join(',')); console.log(keyCount({ zebra: 1, alpha: 2 })); console.log(has({ chosen: 1 }, 'chosen')); console.log(has({ chosen: 1 }, 'missing')); console.log(numberValues({ zebra: 1, alpha: 2 }).join(',')); console.log(boolValues({ first: true, second: false }).join(',')); console.log(stringValues({ first: 'th', second: 'aw' }).join(',')); console.log(valueCount({ zebra: 1, alpha: 2 })); const numbers = numberEntries({ zebra: 1, alpha: 2 }); console.log(numbers[0][0] + ':' + numbers[0][1] + ',' + numbers[1][0] + ':' + numbers[1][1]); const bools = boolEntries({ first: true, second: false }); console.log(bools[0][0] + ':' + bools[0][1] + ',' + bools[1][0] + ':' + bools[1][1]); const strings = stringEntries({ first: 'th', second: 'aw' }); console.log(strings[0][0] + ':' + strings[0][1] + ',' + strings[1][0] + ':' + strings[1][1]); console.log(entryCount({ zebra: 1, alpha: 2 })); }\n",
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
        "42\ntrue\nfalse\nthaw\n43\ntrue\nthaw\ntrue\n42\n42\ntrue\nthaw\n2\n3\n42\nzebra,alpha\n2\ntrue\nfalse\n1,2\ntrue,false\nth,aw\n2\nzebra:1,alpha:2\nfirst:true,second:false\nfirst:th,second:aw\n2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_from_entries_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-from-entries-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-from-entries");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fromNumbers(entries: [string, number][]): Record<string, number>;\nexport declare function fromBools(entries: [string, boolean][]): Record<string, boolean>;\nexport declare function fromStrings(entries: [string, string][]): Record<string, string>;\nexport declare function roundTrip(values: Record<string, number>): Record<string, number>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function fromNumbers(entries) { return Object.fromEntries(entries); } function fromBools(entries) { return Object.fromEntries(entries); } function fromStrings(entries) { return Object.fromEntries(entries); } function roundTrip(values) { return Object.fromEntries(Object.entries(values)); } module.exports = { fromNumbers, fromBools, fromStrings, roundTrip };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fromNumbers, fromBools, fromStrings, roundTrip } from 'jit-from-entries';\nfunction main(): void { const numbers = fromNumbers([['first', 1], ['first', 2], ['second', 3]]); console.log(numbers.first); console.log(numbers.second); const bools = fromBools([['ready', true]]); console.log(bools.ready); const strings = fromStrings([['left', 'th'], ['right', 'aw']]); console.log(strings.left + strings.right); const values = roundTrip({ zebra: 1, alpha: 2 }); console.log(values.zebra); console.log(values.alpha); }\n",
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
        "2\n3\ntrue\nthaw\n1\n2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_assign_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-assign-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-object-assign");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function assignNumbers(target: Record<string, number>, first: Record<string, number>, second: Record<string, number>): Record<string, number>;\nexport declare function assignBools(target: Record<string, boolean>, source: Record<string, boolean>): Record<string, boolean>;\nexport declare function assignStrings(target: Record<string, string>, source: Record<string, string>): Record<string, string>;\nexport declare function assignAndRead(target: Record<string, number>, first: Record<string, number>, second: Record<string, number>): number;\nexport declare function identity(target: Record<string, number>): Record<string, number>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function assignNumbers(target, first, second) { return Object.assign(target, first, second); } function assignBools(target, source) { return Object.assign(target, source); } function assignStrings(target, source) { return Object.assign(target, source); } function assignAndRead(target, first, second) { return Object.assign(target, first, second).chosen; } function identity(target) { return Object.assign(target); } module.exports = { assignNumbers, assignBools, assignStrings, assignAndRead, identity };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { assignNumbers, assignBools, assignStrings, assignAndRead, identity } from 'jit-object-assign';\nfunction main(): void { const numbers = assignNumbers({ base: 1, chosen: 0 }, { chosen: 2, first: 3 }, { chosen: 4, second: 5 }); console.log(numbers.base); console.log(numbers.chosen); console.log(numbers.first); console.log(numbers.second); console.log(assignBools({ ready: false }, { ready: true }).ready); console.log(assignStrings({ left: 'th' }, { right: 'aw' }).left + assignStrings({ left: 'th' }, { right: 'aw' }).right); console.log(assignAndRead({ chosen: 1 }, { chosen: 2 }, { chosen: 42 })); console.log(identity({ answer: 42 }).answer); }\n",
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
        "1\n4\n3\n5\ntrue\nthaw\n42\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dictionary_key_queries_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-key-queries-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-key-queries");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function ownNames(values: Record<string, number>): string[];\nexport declare function reflectNames(values: Record<string, number>): string[];\nexport declare function contains(values: Record<string, number>, key: string): boolean;\nexport declare function containsNumber(values: Record<string, number>, key: number): boolean;\nexport declare function numericRecord(value: number): Record<string, number>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function ownNames(values) { return Object.getOwnPropertyNames(values); } function reflectNames(values) { return Reflect.ownKeys(values); } function contains(values, key) { return key in values; } function containsNumber(values, key) { return key in values; } function numericRecord(value) { return { ['7']: value }; } module.exports = { ownNames, reflectNames, contains, containsNumber, numericRecord };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { ownNames, reflectNames, contains, containsNumber, numericRecord } from 'jit-key-queries';\nfunction main(): void { console.log(ownNames({ zebra: 1, alpha: 2 }).join(',')); console.log(reflectNames({ zebra: 1, alpha: 2 }).join(',')); console.log(contains({ zebra: 1, alpha: 2 }, 'alpha')); console.log(contains({ zebra: 1, alpha: 2 }, 'missing')); console.log(containsNumber(numericRecord(3), 7)); }\n",
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
        "zebra,alpha\nzebra,alpha\ntrue\nfalse\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn returning_switch_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-switch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-switch");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numberSwitch(value: number): number;\nexport declare function stringSwitch(value: string): string;\nexport declare function boolSwitch(value: boolean): number;\nexport declare function evaluateOnce(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function numberSwitch(value) { switch (value) { case 1: case 2: return 10; case 3: return 30; default: return 40; } } function stringSwitch(value) { switch (value) { case 'a': return 'A'; default: case 'fallback': return 'D'; case 'b': return 'B'; } } function boolSwitch(value) { switch (value) { case true: return 1; default: return 0; } } function evaluateOnce(values) { switch (values.pop()) { case 2: return values.length; default: return 99; } } module.exports = { numberSwitch, stringSwitch, boolSwitch, evaluateOnce };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numberSwitch, stringSwitch, boolSwitch, evaluateOnce } from 'jit-switch';\nfunction main(): void { console.log(numberSwitch(1)); console.log(numberSwitch(2)); console.log(numberSwitch(3)); console.log(numberSwitch(9)); console.log(stringSwitch('a')); console.log(stringSwitch('b')); console.log(stringSwitch('x')); console.log(boolSwitch(true)); console.log(boolSwitch(false)); console.log(evaluateOnce([1, 2])); }\n",
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
        "10\n10\n30\n40\nA\nB\nD\n1\n0\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_loops_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loops-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loops");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function sum(limit: number): number;\nexport declare function factorial(value: number): number;\nexport declare function flip(): boolean;\nexport declare function sumFor(limit: number): number;\nexport declare function oddSum(limit: number): number;\nexport declare function bodySum(limit: number): number;\nexport declare function doSum(limit: number): number;\nexport declare function sumValues(values: number[]): number;\nexport declare function assignedSum(values: number[]): number;\nexport declare function drainValues(values: number[]): number;\nexport declare function concatValues(values: string[]): string;\nexport declare function lastBool(values: boolean[]): boolean;\nexport declare function whileControl(limit: number): number;\nexport declare function forControl(limit: number): number;\nexport declare function doControl(limit: number): number;\nexport declare function forOfControl(values: number[]): number;\nexport declare function nestedLoops(rows: number, columns: number): number;\nexport declare function nestedDo(outerLimit: number, innerLimit: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function sum(limit) { let index = 0; let total = 0; while (index < limit) { total += index; index++; } return total; } function factorial(value) { let current = value; let result = 1; while (current > 1) { result *= current; current--; } return result; } function flip() { let active = true; while (active) { active = false; } return active; } function sumFor(limit) { let total = 0; for (let index = 0; index < limit; index++) { total += index; } return total; } function oddSum(limit) { let index = 0; let total = 0; for (index = 1; index <= limit; index += 2) { total += index; } return total; } function bodySum(limit) { let index = 0; let total = 0; for (; index < limit;) { total += index; index++; } return total; } function doSum(limit) { let index = 0; let total = 0; do { total += index; index++; } while (index < limit); return total; } function sumValues(values) { let total = 0; for (const value of values) { total += value; } return total; } function assignedSum(values) { let value = 0; let total = 0; for (value of values) { total += value; } return total; } function drainValues(values) { let total = 0; for (const value of values.splice(0)) { total += value; } return total + values.length * 100; } function concatValues(values) { let result = ''; for (const value of values) { result += value; } return result; } function lastBool(values) { let result = false; for (const value of values) { result = value; } return result; } function whileControl(limit) { let index = 0; let total = 0; while (index < limit) { index++; if (index % 2 === 0) continue; total += index; if (total > 10) break; } return total; } function forControl(limit) { let total = 0; for (let index = 0; index < limit; index++) { if (index > 1) { if (index === 2) continue; } else { total += 0; } if (index === 7) break; total += index; } return total; } function doControl(limit) { let index = 0; let total = 0; do { index++; if (index < 3) continue; total += index; } while (index < limit); return total; } function forOfControl(values) { let total = 0; for (const value of values) { if (value % 2 === 0) continue; if (value > 5) break; total += value; } return total; } function nestedLoops(rows, columns) { let row = 0; let column = 0; let total = 0; while (row < rows) { column = 0; for (column = 0; column < columns; column++) { if (column === 1) continue; if (row === 2) break; total += row * 10 + column; } row++; } return total; } function nestedDo(outerLimit, innerLimit) { let outer = 0; let inner = 0; let total = 0; while (outer < outerLimit) { inner = 0; do { inner++; if (inner === 2) continue; total += outer * 10 + inner; } while (inner < innerLimit); outer++; } return total; } module.exports = { sum, factorial, flip, sumFor, oddSum, bodySum, doSum, sumValues, assignedSum, drainValues, concatValues, lastBool, whileControl, forControl, doControl, forOfControl, nestedLoops, nestedDo };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { sum, factorial, flip, sumFor, oddSum, bodySum, doSum, sumValues, assignedSum, drainValues, concatValues, lastBool, whileControl, forControl, doControl, forOfControl, nestedLoops, nestedDo } from 'jit-loops';\nfunction main(): void { console.log(sum(0)); console.log(sum(6)); console.log(factorial(1)); console.log(factorial(6)); console.log(flip()); console.log(sumFor(6)); console.log(oddSum(7)); console.log(bodySum(5)); console.log(doSum(0)); console.log(doSum(4)); console.log(sumValues([2, 3, 5])); console.log(assignedSum([4, 6])); console.log(drainValues([1, 2, 3])); console.log(concatValues(['a', 'b', 'c'])); console.log(lastBool([])); console.log(lastBool([false, true])); console.log(whileControl(10)); console.log(forControl(10)); console.log(doControl(4)); console.log(forOfControl([1, 2, 3, 7, 5])); console.log(nestedLoops(4, 3)); console.log(nestedDo(2, 3)); console.log(nestedDo(1, 0)); }\n",
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
        "0\n15\n1\n720\nfalse\n15\n16\n10\n0\n6\n10\n10\n6\nabc\nfalse\ntrue\n16\n19\n7\n4\n86\n28\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_for_of_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-string-loop-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-string-loop");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function copy(value: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function copy(value) { let result = ''; for (const character of value) { result += character; } return result; } module.exports = { copy };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    assert!(jit_numeric_export(&bundle, "copy", false, &functions[0]).is_some());
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { copy } from 'jit-string-loop'; function main(): void { console.log(copy('a😀c')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a😀c\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_for_of_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-loop-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-loop");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function render(value: number[] | string[] | boolean[]): string;\nexport declare function renderSlice(value: number[] | string[] | boolean[]): string;\nexport declare function size(value: number[] | string[] | boolean[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function render(value) { let result = ''; for (const item of value) { result += String(item); } return result; } function renderSlice(value) { let result = ''; for (const item of value.slice()) { result += String(item); } return result; } function size(value) { return value.length; } module.exports = { render, renderSlice, size };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { render, renderSlice, size } from 'jit-mixed-array-loop'; function main(): void { console.log(render([1, 2, 3])); console.log(render(['a', 'b'])); console.log(render([true, false])); console.log(renderSlice([1, 2, 3])); console.log(renderSlice(['a', 'b'])); console.log(renderSlice([true, false])); console.log(size([1, 2, 3])); console.log(size(['a', 'b'])); console.log(size([true, false])); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "123\nab\ntruefalse\n123\nab\ntruefalse\n3\n2\n2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nested_declared_loop_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nested-loop-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nested-loop");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function grid(rows: number, columns: number): number;\nexport declare function repeatSum(values: number[], rounds: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function grid(rows, columns) { let row = 0; let total = 0; while (row < rows) { for (let column = 0; column < columns; column++) { if (column === 1) continue; if (row === 2) break; total += row * 10 + column; } row++; } return total; } function repeatSum(values, rounds) { let round = 0; let total = 0; while (round < rounds) { for (const value of values) { if (value > 5) break; total += value; } round++; } return total; } module.exports = { grid, repeatSum };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(jit_numeric_export(&bundle, &function.name, false, function).is_some());
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { grid, repeatSum } from 'jit-nested-loop'; function main(): void { console.log(grid(4, 3)); console.log(repeatSum([1, 4, 7, 9], 3)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "86\n15\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_early_returns_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-return-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-return");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function findNumber(values: number[], target: number): number;\nexport declare function findString(values: string[]): string;\nexport declare function anyTrue(values: boolean[]): boolean;\nexport declare function nested(rows: number, columns: number): number;\nexport declare function doFind(target: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function findNumber(values, target) { for (const value of values) { if (value === target) return value; } return -1; } function findString(values) { for (const value of values) { if (value.length > 2) return value; } return 'missing'; } function anyTrue(values) { for (const value of values) { if (value) return true; } return false; } function nested(rows, columns) { let row = 0; while (row < rows) { for (let column = 0; column < columns; column++) { if (row === 1 && column === 2) return row * 10 + column; } row++; } return -1; } function doFind(target) { let value = 0; do { if (value === target) return value; value++; } while (value < 3); return -1; } module.exports = { findNumber, findString, anyTrue, nested, doFind };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { findNumber, findString, anyTrue, nested, doFind } from 'jit-loop-return'; function main(): void { console.log(findNumber([1, 3, 5], 3)); console.log(findNumber([1], 2)); console.log(findString(['a', 'long', 'later'])); console.log(anyTrue([false, true])); console.log(anyTrue([])); console.log(nested(3, 4)); console.log(doFind(2)); console.log(doFind(9)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "3\n-1\nlong\ntrue\nfalse\n12\n2\n-1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_block_locals_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-locals-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-locals");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function mapped(values: number[]): number;\nexport declare function selected(values: string[]): string;\nexport declare function controlled(values: number[]): number;\nexport declare function shadowed(limit: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function mapped(values) { let total = 0; for (const value of values) { const doubled = value * 2; let adjusted = doubled + 1; total += adjusted; } return total; } function selected(values) { let result = ''; for (const value of values) { const upper = value.toUpperCase(); if (upper.length > 2) return upper; result += upper; } return result; } function controlled(values) { let total = 0; for (const value of values) { const squared = value * value; if (squared === 4) continue; if (squared > 20) break; total += squared; } return total; } function shadowed(limit) { let value = 10; let total = 0; let index = 0; while (index < limit) { let value = index; if (value > 0) { const increment = value; total += increment; } index++; } return total + value; } module.exports = { mapped, selected, controlled, shadowed };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { mapped, selected, controlled, shadowed } from 'jit-loop-locals'; function main(): void { console.log(mapped([1, 2, 3])); console.log(selected(['a', 'long', 'later'])); console.log(selected(['a', 'b'])); console.log(controlled([1, 2, 3, 5, 4])); console.log(shadowed(3)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "15\nLONG\nAB\n10\n13\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_aggregate_locals_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-aggregates-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-aggregates");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function arrays(values: number[]): number;\nexport declare function strings(values: string[]): string;\nexport declare function booleans(values: boolean[]): boolean;\nexport declare function records(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function arrays(values) { let total = 0; for (const value of values) { let copy = [value, value + 1]; copy.push(value + 2); copy[0] = value + 1; total += copy.length; total += copy[0]; total += copy.pop(); copy = [value + 3]; total += copy[0]; } return total; } function strings(values) { let result = ''; for (const value of values) { let copy = [value]; copy.push(value.toUpperCase()); result += copy.join(':'); copy = [value + '!']; result += copy[0]; } return result; } function booleans(values) { let result = false; for (const value of values) { let copy = [value]; copy.push(!value); result = copy[0]; copy = [!value]; result = copy[0]; } return result; } function records(values) { let total = 0; for (const value of values) { let record = { current: value, next: value + 1 }; record.current = value + 2; total += record.current; total += record.next; record = { current: value + 3, next: value + 4 }; total += record.current; } return total; } module.exports = { arrays, strings, booleans, records };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { arrays, strings, booleans, records } from 'jit-loop-aggregates'; function main(): void { console.log(arrays([1, 2])); console.log(strings(['a', 'b'])); console.log(booleans([false, true])); console.log(records([1, 2])); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "27\na:Aa!b:Bb!\nfalse\n21\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_aggregate_early_returns_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-aggregate-returns-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-aggregate-returns");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function selectArray(values: number[]): number[];\nexport declare function selectRecord(values: number[]): Record<string, number>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function selectArray(values) { for (const value of values) { const result = [value, value + 1]; if (value > 1) return result; } return [0]; } function selectRecord(values) { for (const value of values) { const result = { current: value, next: value + 1 }; if (value > 1) return result; } return { current: 0, next: 0 }; } module.exports = { selectArray, selectRecord };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { selectArray, selectRecord } from 'jit-loop-aggregate-returns'; function main(): void { console.log(selectArray([1, 2, 3]).join(':')); const record = selectRecord([1, 2, 3]); console.log(record.current); console.log(record.next); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2:3\n2\n3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dictionary_for_in_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-for-in-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-for-in");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function sum(values: Record<string, number>): number;\nexport declare function countOne(values: Record<string, number>): number;\nexport declare function firstValue(values: Record<string, number>): number;\nexport declare function nested(values: Record<string, number>): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function sum(values) { let total = 0; for (const key in values) { if (key === 'skip') continue; total += values[key]; } return total; } function countOne(values) { let count = 0; for (const key in values) { count++; break; } return count; } function firstValue(values) { for (const key in values) { return values[key]; } return 0; } function nested(values) { let count = 0; for (const outer in values) { if (outer === 'skip') continue; for (const inner in values) { if (inner === 'skip') continue; count++; break; } } return count; } module.exports = { sum, countOne, firstValue, nested };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { sum, countOne, firstValue, nested } from 'jit-for-in'; function main(): void { console.log(sum({ a: 2, skip: 100, b: 3 })); console.log(countOne({ a: 1, b: 2 })); console.log(firstValue({ a: 7, b: 7 })); console.log(nested({ a: 1, skip: 2, b: 3 })); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n1\n7\n2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_switch_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-switch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-switch");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function score(values: number[]): number;\nexport declare function skip(values: number[]): number;\nexport declare function select(values: string[]): string;\nexport declare function middleDefault(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function score(values) { let total = 0; for (const value of values) { switch (value) { case 1: total += 10; break; case 2: total += 20; case 3: total += 3; break; default: total += 1; } } return total; } function skip(values) { let total = 0; for (const value of values) { switch (value) { case 0: continue; case 1: if (total === 0) break; total += 100; break; default: total += value; } total += 1; } return total; } function select(values) { for (const value of values) { switch (value) { case 'stop': return value.toUpperCase(); default: break; } } return 'none'; } function middleDefault(values) { let total = 0; for (const value of values) { switch (value) { case 1: total += 10; break; default: total += 5; case 2: total += 2; break; case 3: total += 30; break; } } return total; } module.exports = { score, skip, select, middleDefault };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { score, skip, select, middleDefault } from 'jit-loop-switch'; function main(): void { console.log(score([1, 2, 4])); console.log(skip([0, 1, 5])); console.log(select(['go', 'stop', 'later'])); console.log(middleDefault([1, 2, 4])); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "34\n7\nSTOP\n19\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_try_finally_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-finally-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-finally");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function cleanup(values: number[]): number;\nexport declare function nested(values: number[]): number;\nexport declare function returning(values: number[]): number;\nexport declare function leaving(values: number[]): number;\nexport declare function nestedLoop(values: number[]): number;\nexport declare function overridden(value: number): number;\nexport declare function breakOverride(values: number[]): number;\nexport declare function continueOverride(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function cleanup(values) { let total = 0; for (const value of values) { try { let copy = value * 2; total += copy; } finally { total += 1; } } return total; } function nested(values) { let total = 0; for (const value of values) { try { try { total += value; } finally { total += 2; } } finally { total += 3; } } return total; } function returning(values) { let total = 0; for (const value of values) { try { if (value > 1) return total; total += value; } finally { total += 10; } } return total; } function leaving(values) { let total = 0; for (const value of values) { try { if (value === 0) continue; if (value === 3) break; total += value; } finally { total += 10; } } return total; } function nestedLoop(values) { let total = 0; for (const value of values) { try { for (let inner = 0; inner < 3; inner++) { if (inner === 0) continue; total += value; break; } } finally { total += 10; } } return total; } function overridden(value) { while (true) { try { return value; } finally { return value + 10; } } return 0; } function breakOverride(values) { let total = 0; for (const value of values) { try { return 99; } finally { total += value; break; } } return total; } function continueOverride(values) { let total = 0; for (const value of values) { try { return 99; } finally { total += value; continue; } } return total; } module.exports = { cleanup, nested, returning, leaving, nestedLoop, overridden, breakOverride, continueOverride };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { cleanup, nested, returning, leaving, nestedLoop, overridden, breakOverride, continueOverride } from 'jit-loop-finally'; function main(): void { console.log(cleanup([1, 2, 3])); console.log(nested([1, 2, 3])); console.log(returning([1, 2, 3])); console.log(leaving([0, 1, 3, 5])); console.log(nestedLoop([1, 2])); console.log(overridden(2)); console.log(breakOverride([4, 5])); console.log(continueOverride([4, 5])); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "15\n21\n11\n31\n23\n12\n4\n9\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn loop_try_catch_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-loop-catch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-loop-catch");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function recover(values: number[]): number;\nexport declare function rethrow(value: number): number;\nexport declare function catchNumber(value: number): number;\nexport declare function catchWithoutBinding(): number;\nexport declare function catchNativeError(): number;\nexport declare function catchArray(values: number[]): number;\nexport declare function catchDictionary(values: Record<string, number>): number;\nexport declare function catchMixed(value: number): number;\nexport declare function catchObject(value: number): number;\nexport declare function catchKinds(mode: number, numbers: number[], strings: string[], flags: Record<string, boolean>, labels: Record<string, string>): number;\nexport declare function catchMixedError(value: number): number;\nexport declare function catchAllocation(value: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function recover(values) { let total = 0; for (const value of values) { try { if (value < 0) throw 'negative'; total += value; } catch (error) { total += error.length; } finally { total += 1; } } return total; } function rethrow(value) { let total = value; while (true) { try { try { throw 'original'; } catch (error) { throw error; } finally { total += 1; } } catch (error) { return total + error.length; } } return 0; } function catchNumber(value) { while (true) { try { throw 7; } catch (error) { return error + value; } } return 0; } function catchWithoutBinding() { while (true) { try { throw 'ignored'; } catch { return 4; } } return 0; } function catchNativeError() { while (true) { try { 'x'.repeat(-1); return 1; } catch (error) { return error.length; } } return 0; } function catchArray(values) { while (true) { try { throw values; } catch (error) { return error.length + error[0]; } } return 0; } function catchDictionary(values) { while (true) { try { throw values; } catch (error) { return error['answer']; } } return 0; } function catchMixed(value) { while (true) { try { if (value > 0) throw value; throw 'bad'; } catch (error) { if (typeof error === 'number') return error + 1; else return error.length; } } return 0; } function catchObject(value) { while (true) { try { if (value > 0) throw [value, 2]; throw { answer: 42 }; } catch (error) { if (Array.isArray(error)) return error.length + error[0]; else return error['answer']; } } return 0; } module.exports = { recover, rethrow, catchNumber, catchWithoutBinding, catchNativeError, catchArray, catchDictionary, catchMixed, catchObject };\n",
    )
    .unwrap();
    let bundle_path = package.join("bundle.js");
    let bundle = std::fs::read_to_string(&bundle_path).unwrap();
    let bundle = bundle.replace(
        "'x'.repeat(-1); return 1;",
        "'x'.repeat(-1).length; return 1;",
    );
    let bundle = bundle.replace(
        " module.exports = {",
        " function catchKinds(mode, numbers, strings, flags, labels) { while (true) { try { if (mode === 0) throw numbers; if (mode === 1) throw strings; if (mode === 2) throw flags; throw labels; } catch (error) { if (typeof error[0] === 'number') return error[0]; if (typeof error[0] === 'string') return error[0].length; if (typeof error.answer === 'boolean') return error.answer ? 1 : 0; if (typeof error.answer === 'string') return error.answer.length; return 0; } } return 0; } function catchMixedError(value) { while (true) { try { if (value > 0) throw 7; 'x'.repeat(-1); return 0; } catch (error) { if (typeof error === 'number') return error + 1; else return error.length; } } return 0; } function catchAllocation(value) { while (true) { try { return value.padStart(4, '0').length; } catch (error) { return error.length; } } return 0; } module.exports = {",
    );
    let bundle = bundle.replace(
        "catchObject };",
        "catchObject, catchKinds, catchMixedError, catchAllocation };",
    );
    std::fs::write(&bundle_path, bundle).unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        let operation = jit_numeric_export(&bundle, &function.name, false, function);
        assert!(
            operation.is_some(),
            "{function:?} did not specialize"
        );
        if function.name == "catchAllocation" {
            let operation = operation.unwrap();
            assert!(operation.contains("padstart") && operation.contains("checkerror"));
        } else if function.name == "catchNativeError" {
            assert!(operation.unwrap().contains("repeat,checkerror,strlen"));
        }
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { recover, rethrow, catchNumber, catchWithoutBinding, catchNativeError, catchArray, catchDictionary, catchMixed, catchObject, catchKinds, catchMixedError, catchAllocation } from 'jit-loop-catch'; function main(): void { console.log(recover([2, -1, 3])); console.log(rethrow(2)); console.log(catchNumber(2)); console.log(catchWithoutBinding()); console.log(catchNativeError()); console.log(catchArray([2, 3])); console.log(catchDictionary({ answer: 42 })); console.log(catchMixed(2)); console.log(catchMixed(-1)); console.log(catchObject(2)); console.log(catchObject(-1)); console.log(catchKinds(0, [42], ['thaw'], { answer: true }, { answer: 'ready' })); console.log(catchKinds(1, [42], ['thaw'], { answer: true }, { answer: 'ready' })); console.log(catchKinds(2, [42], ['thaw'], { answer: true }, { answer: 'ready' })); console.log(catchKinds(3, [42], ['thaw'], { answer: true }, { answer: 'ready' })); console.log(catchKinds(0, [], ['thaw'], { answer: true }, { answer: 'ready' })); console.log(catchKinds(2, [42], ['thaw'], {}, { answer: 'ready' })); console.log(catchMixedError(1)); console.log(catchMixedError(-1)); console.log(catchAllocation('7')); }\n",
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
        "16\n11\n9\n4\n27\n4\n42\n3\n3\n4\n42\n42\n4\n1\n5\n0\n0\n8\n27\n4\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn uncaught_loop_throw_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-uncaught-throw-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-uncaught-throw");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fail(value: string): number;\nexport declare function failArray(value: number[]): number;\nexport declare function failObject(value: Record<string, number>): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function fail(value) { while (true) { try { throw value; } finally { value.toUpperCase(); } } return 0; } function failArray(value) { while (true) throw value; return 0; } function failObject(value) { while (true) throw value; return 0; } module.exports = { fail, failArray, failObject };\n",
    )
    .unwrap();
    let declarations = std::fs::read_to_string(package.join("package.d.ts")).unwrap();
    let functions = thaw_bridge::parse_dts(&declarations).unwrap();
    let bundle = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for function in &functions {
        assert!(
            jit_numeric_export(&bundle, &function.name, false, function).is_some(),
            "{function:?} did not specialize"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fail } from 'jit-uncaught-throw'; function main(): void { console.log('before'); console.log(fail('boom')); console.log('after'); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    let result = Command::new(&output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "before\n");

    std::fs::write(
        &entry,
        "import { failArray } from 'jit-uncaught-throw'; function main(): void { failArray([2, 3]); }\n",
    )
    .unwrap();
    let array_output = dir.join("array-app");
    build(&entry, &array_output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&array_output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    let result = Command::new(&array_output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));

    std::fs::write(
        &entry,
        "import { failObject } from 'jit-uncaught-throw'; function main(): void { failObject({ answer: 42 }); }\n",
    )
    .unwrap();
    let object_output = dir.join("object-app");
    build(&entry, &object_output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&object_output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&object_output).output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    let _ = std::fs::remove_dir_all(dir);
}

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

#[test]
fn nested_object_destructuring_uses_jit_without_quickjs() {
    let dts = concat!(
        "export declare function unpack(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }): string;\n",
        "export declare function reassign(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }): string;\n",
        "export declare function reassignControl(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }, flag: boolean): string;\n",
        "export declare function reassignInsideIf(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }, alternate: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }, flag: boolean): string;\n",
        "export declare function reassignInsideLoop(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }): string;\n",
        "export declare function reassignInsideTry(value: { count: number; meta: { label: string; enabled: boolean }; pair: [number, string]; values: number[] }): string;\n",
        "export declare function unpackTuple(value: [[number, string], { enabled: boolean; values: number[] }]): string;\n",
        "export declare function reassignTuple(value: [[number, string], { enabled: boolean; values: number[] }]): string;\n",
        "export declare function literal(value: number): string;\n",
        "export declare function callLiteral(value: number): string;\n",
        "export declare function callLocals(value: number): number;\n",
        "export declare function callSteps(value: number): number;\n",
        "export declare function callArrayParts(value: number): number;\n",
        "export declare function callArrayAssign(value: number): number;\n",
        "export declare function callObjectParts(value: number): string;\n",
        "export declare function callObjectDefault(): string;\n",
        "export declare function callControl(value: number, flag: boolean): string;\n",
        "export declare function callEarly(value: number, flag: boolean): string;\n",
        "export declare function callNestedEarly(value: number, high: boolean): string;\n",
        "export declare function callHelperLoop(value: number): number;\n",
        "export declare function callHelperFor(value: number): number;\n",
        "export declare function callHelperFinally(value: number): number;\n",
        "export declare function callHelperSwitch(value: number): string;\n",
        "export declare function callHelperSwitchReturn(values: number[]): string;\n",
        "export declare function callHelperFinallyReturn(value: number): string;\n",
        "export declare function callHelperCatch(value: number, flag: boolean, values: number[]): string;\n",
        "export declare function callHelperWhileReturn(values: number[]): string;\n",
        "export declare function callHelperForReturn(values: number[]): string;\n",
        "export declare function callHelperDoWhileReturn(values: number[]): string;\n",
        "export declare function callHelperForOfReturn(values: number[]): string;\n",
        "export declare function callHelperForOfString(values: string[]): string;\n",
        "export declare function callHelperForInNumber(values: Record<string, number>): string;\n",
        "export declare function callHelperForInString(values: Record<string, string>): string;\n",
        "export declare function callHelperNestedLoopReturn(values: number[]): string;\n",
        "export declare function callHelperNestedForReturn(values: number[]): string;\n",
        "export declare function callHelperNestedForOfReturn(values: number[]): string;\n",
        "export declare function callHelperNestedForInReturn(values: Record<string, number>): string;\n",
        "export declare function callHelperNestedSwitchReturn(values: number[]): string;\n",
        "export declare function callHelperNestedFinallyReturn(values: number[], effects: number[]): string;\n",
        "export declare function callHelperNestedCatchReturn(value: number, fail: boolean, effects: number[]): string;\n",
        "export declare function callHelperLabeledReturn(values: number[]): string;\n",
        "export declare function callHelperLabeledControl(values: number[]): string;\n",
        "export declare function callLabeledNumber(values: number[]): number;\n",
    );
    let source = concat!(
        "function makeLiteral(value) { return { unused: Math.random(), first: value, second: value, label: 'helper' }; } ",
        "function makeLocals(value) { const count = value + 1; const values = [value]; const unused = Math.random(); return { count, values }; } ",
        "function makeSteps(value) { let count = value; value += 1; count += value; count++; const unused = Math.random(); Math.random(); return { count }; } ",
        "function makeArrayParts(value) { const [first, , fallback = 9] = [value, 8]; return { result: first * 10 + fallback }; } ",
        "function makeArrayAssign(value) { let first = 0; let rest = [0]; [first, ...rest] = [value, value + 1]; return { result: first * 10 + rest.length }; } ",
        "function makeObjectParts(value) { let count = 0; let label = ''; ({ count, label } = { unused: Math.random(), count: value + 1, label: 'box' }); return { result: label + ':' + String(count) }; } ",
        "function makeObjectDefault() { const { missing = 'fallback' } = {}; return { result: missing }; } ",
        "function makeControl(value, flag) { let count = value; let label = 'small'; if (flag) { if (value > 0) { count += 2; label = 'yes'; } } else { count -= 1; } return { count, label }; } ",
        "function makeEarly(value, flag) { if (flag) return { unused: Math.random(), count: value + 1, label: 'yes' }; return { unused: Math.random(), count: value - 1, label: 'no' }; } ",
        "function makeNestedEarly(value, high) { if (value > 0) { if (high) return { count: value + 10, label: 'high' }; return { count: value + 1, label: 'low' }; } return { count: 0, label: 'zero' }; } ",
        "function makeHelperLoop(value) { let count = 0; while (count < value) count++; return { count }; } ",
        "function makeHelperFor(value) { let count = 0; for (let index = 0; index < value; index++) count += index; return { count }; } ",
        "function makeHelperFinally(value) { let count = value; try { count += 2; } finally { count += 3; } return { count }; } ",
        "function makeHelperSwitch(value) { let label = 'other'; switch (value) { case 1: label = 'one'; break; case 2: label = 'two'; break; } return { label }; } ",
        "function makeHelperSwitchReturn(values) { switch (values.pop()) { case 1: return { count: values.length, label: 'one' }; case 2: return { count: values.length, label: 'two' }; default: return { count: values.length, label: 'other' }; } } ",
        "function makeHelperFinallyReturn(value) { let count = value; const values = [value]; try { count += 1; return { count, values }; } finally { count += 10; values.push(9); } } ",
        "function makeHelperCatch(value, flag, values) { try { if (value < 0) throw 'low'; if (flag) throw 'bad'; return { count: value, label: 'ok' }; } catch (error) { return { count: value + 1, label: error }; } finally { values.push(9); } } ",
        "function makeHelperWhileReturn(values) { let index = 0; while (index < values.length) { if (values[index] > 2) return { index, label: 'found' }; index++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperForReturn(values) { for (let index = 0; index < values.length; index++) { if (values[index] > 2) return { index, label: 'found' }; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperDoWhileReturn(values) { let index = 0; do { if (values[index] > 2) return { index, label: 'found' }; index++; } while (index < values.length); return { index: -1, label: 'missing' }; } ",
        "function makeHelperForOfReturn(values) { for (const value of values) { if (value > 2) return { index: value, label: 'found' }; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperForOfString(values) { for (const value of values) { if (value.length > 3) return { count: value.length, label: value }; } return { count: 0, label: 'missing' }; } ",
        "function makeHelperForInNumber(values) { for (const key in values) { if (key === 'target') return { count: values[key], label: key }; } return { count: 0, label: 'missing' }; } ",
        "function makeHelperForInString(values) { for (const key in values) { if (key === 'target') return { count: values[key].length, label: values[key] }; } return { count: 0, label: 'missing' }; } ",
        "function makeHelperNestedLoopReturn(values) { let outer = 0; let inner = 0; while (outer < 2) { inner = 0; while (inner < values.length) { if (values[inner] > outer + 3) return { index: inner, label: 'nested' }; inner++; } outer++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedForReturn(values) { let outer = 0; while (outer < 2) { for (let index = 0; index < values.length; index++) { if (values[index] > outer + 3) return { index, label: 'nested-for' }; } outer++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedForOfReturn(values) { let outer = 0; while (outer < 2) { for (const value of values) { if (value > outer + 3) return { index: value, label: 'nested-for-of' }; } outer++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedForInReturn(values) { let outer = 0; while (outer < 2) { for (const key in values) { if (key === 'target' && outer === 1) return { index: values[key] + outer, label: key }; } outer++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedSwitchReturn(values) { let index = 0; while (index < values.length) { switch (values[index]) { case 5: return { index, label: 'switch' }; case 0: index++; break; default: break; } index++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedFinallyReturn(values, effects) { let index = 0; while (index < values.length) { try { if (values[index] > 2) return { index, label: 'finally' }; } finally { effects.push(index); } index++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperNestedCatchReturn(value, fail, effects) { let outer = 0; while (outer < 1) { try { if (fail) throw 'bad'; return { index: value, label: 'ok' }; } catch (error) { return { index: value + 1, label: error }; } finally { effects.push(value); } } return { index: -1, label: 'missing' }; } ",
        "function makeHelperLabeledReturn(values) { let index = 0; scan: while (index < values.length) { if (values[index] > 2) return { index, label: 'labeled' }; index++; } return { index: -1, label: 'missing' }; } ",
        "function makeHelperLabeledControl(values) { let index = 0; outer: while (index < values.length) { if (values[index] === 0) { index++; continue outer; } if (values[index] > 2) return { index, label: 'labeled' }; if (values[index] < 0) break outer; index++; } return { index, label: 'done' }; } ",
        "module.exports.unpack = value => { const { count: amount, meta: { label, enabled }, pair: [index, text], values } = value; values.push(index); return label + ':' + String(amount) + ':' + String(enabled) + ':' + String(index) + ':' + text + ':' + values.join(','); }; ",
        "module.exports.reassign = value => { let count = 0; let label = ''; let index = 0; let text = ''; ({ count, meta: { label }, pair: [index, text] } = value); return String(count) + ':' + label + ':' + String(index) + ':' + text; }; ",
        "module.exports.reassignControl = (value, flag) => { let count = 0; let label = ''; ({ count, meta: { label } } = value); if (flag) count += 1; return String(count) + ':' + label; }; ",
        "module.exports.reassignInsideIf = (value, alternate, flag) => { let count = 0; let label = ''; if (flag) { ({ count, meta: { label } } = value); } else { ({ count, meta: { label } } = alternate); } return String(count) + ':' + label; }; ",
        "module.exports.reassignInsideLoop = value => { let count = 0; let label = ''; let active = true; while (active) { ({ count, meta: { label } } = value); active = false; } return String(count) + ':' + label; }; ",
        "module.exports.reassignInsideTry = value => { let count = 0; let label = ''; try { ({ count, meta: { label } } = value); } finally { count += 1; } return String(count) + ':' + label; }; ",
        "module.exports.unpackTuple = value => { const [[count, label], { enabled, values }] = value; values.push(count); return label + ':' + String(count) + ':' + String(enabled) + ':' + values.join(','); }; ",
        "module.exports.reassignTuple = value => { let count = 0; let label = ''; let enabled = true; [[count, label], { enabled }] = value; return String(count) + ':' + label + ':' + String(enabled); }; ",
        "module.exports.literal = value => { const { count, nested: { label, values } } = { unused: Math.random(), count: value + 1, nested: { label: 'made', values: [value] } }; values.push(9); return label + ':' + String(count) + ':' + values.join(','); }; ",
        "module.exports.callLiteral = value => { const { label } = makeLiteral(Math.random()); return label; }; ",
        "module.exports.callLocals = value => { const { count, values } = makeLocals(value); values.push(9); return count * 10 + values.length; };",
        "module.exports.callSteps = value => { const { count } = makeSteps(value); return count; };",
        "module.exports.callArrayParts = value => { const { result } = makeArrayParts(value); return result; }; ",
        "module.exports.callArrayAssign = value => { const { result } = makeArrayAssign(value); return result; }; ",
        "module.exports.callObjectParts = value => { const { result } = makeObjectParts(value); return result; };",
        "module.exports.callObjectDefault = () => { const { result } = makeObjectDefault(); return result; };",
        "module.exports.callControl = (value, flag) => { const { count, label } = makeControl(value, flag); return label + ':' + String(count); };",
        "module.exports.callEarly = (value, flag) => { const { count, label } = makeEarly(value, flag); return label + ':' + String(count); }; ",
        "module.exports.callNestedEarly = (value, high) => { const { count, label } = makeNestedEarly(value, high); return label + ':' + String(count); };",
        "module.exports.callHelperLoop = value => { const { count } = makeHelperLoop(value); return count; }; ",
        "module.exports.callHelperFor = value => { const { count } = makeHelperFor(value); return count; }; ",
        "module.exports.callHelperFinally = value => { const { count } = makeHelperFinally(value); return count; }; ",
        "module.exports.callHelperSwitch = value => { const { label } = makeHelperSwitch(value); return label; };",
        "module.exports.callHelperSwitchReturn = values => { const { count, label } = makeHelperSwitchReturn(values); return label + ':' + String(count); };",
        "module.exports.callHelperFinallyReturn = value => { const { count, values } = makeHelperFinallyReturn(value); return String(count) + ':' + values.join(','); };",
        "module.exports.callHelperCatch = (value, flag, values) => { const { count, label } = makeHelperCatch(value, flag, values); return label + ':' + String(count); };",
        "module.exports.callHelperWhileReturn = values => { const { index, label } = makeHelperWhileReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperForReturn = values => { const { index, label } = makeHelperForReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperDoWhileReturn = values => { const { index, label } = makeHelperDoWhileReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperForOfReturn = values => { const { index, label } = makeHelperForOfReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperForOfString = values => { const { count, label } = makeHelperForOfString(values); return label + ':' + String(count); };",
        "module.exports.callHelperForInNumber = values => { const { count, label } = makeHelperForInNumber(values); return label + ':' + String(count); };",
        "module.exports.callHelperForInString = values => { const { count, label } = makeHelperForInString(values); return label + ':' + String(count); };",
        "module.exports.callHelperNestedLoopReturn = values => { const { index, label } = makeHelperNestedLoopReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperNestedForReturn = values => { const { index, label } = makeHelperNestedForReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperNestedForOfReturn = values => { const { index, label } = makeHelperNestedForOfReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperNestedForInReturn = values => { const { index, label } = makeHelperNestedForInReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperNestedSwitchReturn = values => { const { index, label } = makeHelperNestedSwitchReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperNestedFinallyReturn = (values, effects) => { const { index, label } = makeHelperNestedFinallyReturn(values, effects); return label + ':' + String(index) + ':' + String(effects.length); };",
        "module.exports.callHelperNestedCatchReturn = (value, fail, effects) => { const { index, label } = makeHelperNestedCatchReturn(value, fail, effects); return label + ':' + String(index) + ':' + String(effects.length); };",
        "module.exports.callHelperLabeledReturn = values => { const { index, label } = makeHelperLabeledReturn(values); return label + ':' + String(index); };",
        "module.exports.callHelperLabeledControl = values => { const { index, label } = makeHelperLabeledControl(values); return label + ':' + String(index); };",
        "module.exports.callLabeledNumber = values => { let index = values.length - 3; outer: while (index < 3) { if (index === 0) { index++; continue outer; } if (index === 2) break outer; index++; } return index; };",
    );
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in [
        "unpack",
        "reassign",
        "reassignControl",
        "reassignInsideIf",
        "reassignInsideLoop",
        "reassignInsideTry",
        "unpackTuple",
        "reassignTuple",
        "literal",
        "callLiteral",
        "callLocals",
        "callSteps",
        "callArrayParts",
        "callArrayAssign",
        "callObjectParts",
        "callObjectDefault",
        "callControl",
        "callEarly",
        "callNestedEarly",
        "callHelperLoop",
        "callHelperFor",
        "callHelperFinally",
        "callHelperSwitch",
        "callHelperSwitchReturn",
        "callHelperFinallyReturn",
        "callHelperCatch",
        "callHelperWhileReturn",
        "callHelperForReturn",
        "callHelperDoWhileReturn",
        "callHelperForOfReturn",
        "callHelperForOfString",
        "callHelperForInNumber",
        "callHelperForInString",
        "callHelperNestedLoopReturn",
        "callHelperNestedForReturn",
        "callHelperNestedForOfReturn",
        "callHelperNestedForInReturn",
        "callHelperNestedSwitchReturn",
        "callHelperNestedFinallyReturn",
        "callHelperNestedCatchReturn",
        "callHelperLabeledReturn",
        "callHelperLabeledControl",
        "callLabeledNumber",
    ]
        .into_iter()
        .enumerate()
    {
        let expression = jit_numeric_export(source, name, false, &declarations[index]);
        assert!(expression.is_some(), "{name}");
        if name == "literal" {
            assert!(expression.unwrap().contains("random"));
        } else if name == "callLiteral" {
            assert_eq!(expression.unwrap().matches("random").count(), 2);
        } else if name == "callLocals" {
            assert_eq!(expression.unwrap().matches("random").count(), 1);
        } else if name == "callSteps" {
            assert_eq!(expression.unwrap().matches("random").count(), 2);
        } else if name == "callObjectParts" {
            assert_eq!(expression.unwrap().matches("random").count(), 1);
        } else if name == "callEarly" {
            assert_eq!(expression.unwrap().matches("random").count(), 2);
        } else if name == "callHelperSwitchReturn" {
            assert_eq!(expression.unwrap().matches("rnpop").count(), 1);
        } else if name == "callHelperFinallyReturn" {
            assert_eq!(expression.unwrap().matches("push").count(), 1);
        } else if name == "callHelperCatch" {
            let expression = expression.unwrap();
            assert!(expression.contains("trystart"));
            assert!(expression.contains("catch"));
        } else if matches!(name, "callHelperLabeledControl" | "callLabeledNumber") {
            let expression = expression.unwrap();
            assert!(expression.contains("break0"));
            assert!(expression.contains("continue0"));
        } else if matches!(
            name,
            "callHelperWhileReturn"
                | "callHelperForReturn"
                | "callHelperDoWhileReturn"
                | "callHelperForOfReturn"
                | "callHelperForOfString"
                | "callHelperForInNumber"
                | "callHelperForInString"
                | "callHelperNestedLoopReturn"
                | "callHelperNestedForReturn"
                | "callHelperNestedForOfReturn"
                | "callHelperNestedForInReturn"
                | "callHelperNestedSwitchReturn"
                | "callHelperNestedFinallyReturn"
                | "callHelperNestedCatchReturn"
                | "callHelperLabeledReturn"
                | "callHelperLabeledControl"
        ) {
            let expression = expression.unwrap();
            assert!(expression.contains("resultstart"));
            assert!(expression.contains("resultreturn"));
        }
    }

    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nested-object-destructuring-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nested-object-destructuring");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { callArrayAssign, callArrayParts, callControl, callEarly, callHelperCatch, callHelperDoWhileReturn, callHelperFinally, callHelperFinallyReturn, callHelperFor, callHelperForInNumber, callHelperForInString, callHelperForOfReturn, callHelperForOfString, callHelperForReturn, callHelperLabeledControl, callHelperLabeledReturn, callLabeledNumber, callHelperLoop, callHelperNestedCatchReturn, callHelperNestedFinallyReturn, callHelperNestedForInReturn, callHelperNestedForOfReturn, callHelperNestedForReturn, callHelperNestedLoopReturn, callHelperNestedSwitchReturn, callHelperSwitch, callHelperSwitchReturn, callHelperWhileReturn, callLiteral, callLocals, callNestedEarly, callObjectDefault, callObjectParts, callSteps, literal, reassign, reassignControl, reassignInsideIf, reassignInsideLoop, reassignInsideTry, reassignTuple, unpack, unpackTuple } from 'jit-nested-object-destructuring';\nfunction main(): void { const value = { count: 3, meta: { label: 'box', enabled: true }, pair: [7, 'pair'] as [number, string], values: [1, 2] }; const alternate = { count: 8, meta: { label: 'alt', enabled: false }, pair: [9, 'other'] as [number, string], values: [4] }; const nested = [[4, 'deep'], { enabled: false, values: [1] }] as [[number, string], { enabled: boolean; values: number[] }]; console.log(unpack(value)); console.log(reassign(value)); console.log(reassignControl(value, true)); console.log(reassignInsideIf(value, alternate, false)); console.log(reassignInsideLoop(value)); console.log(reassignInsideTry(value)); console.log(unpackTuple(nested)); console.log(reassignTuple(nested)); console.log(literal(3)); console.log(callLiteral(3)); console.log(callLocals(3)); console.log(callSteps(3)); console.log(callArrayParts(3)); console.log(callArrayAssign(3)); console.log(callObjectParts(3)); console.log(callObjectDefault()); console.log(callControl(3, true)); console.log(callControl(3, false)); console.log(callEarly(3, true)); console.log(callEarly(3, false)); console.log(callNestedEarly(3, true)); console.log(callNestedEarly(3, false)); console.log(callNestedEarly(0, true)); console.log(callHelperLoop(4)); console.log(callHelperFor(4)); console.log(callHelperFinally(5)); console.log(callHelperSwitch(1)); console.log(callHelperSwitch(2)); console.log(callHelperSwitch(3)); console.log(callHelperSwitchReturn([7, 2])); console.log(callHelperSwitchReturn([7, 9])); console.log(callHelperFinallyReturn(3)); const catchOk = [3]; console.log(callHelperCatch(3, false, catchOk)); console.log(catchOk.join(',')); const catchBad = [3]; console.log(callHelperCatch(3, true, catchBad)); console.log(catchBad.join(',')); const catchLow = [-1]; console.log(callHelperCatch(-1, false, catchLow)); console.log(catchLow.join(',')); console.log(callHelperWhileReturn([1, 4, 2])); console.log(callHelperWhileReturn([1, 2])); console.log(callHelperForReturn([1, 4, 2])); console.log(callHelperForReturn([1, 2])); console.log(callHelperDoWhileReturn([1, 4, 2])); console.log(callHelperDoWhileReturn([1, 2])); console.log(callHelperForOfReturn([1, 4, 2])); console.log(callHelperForOfReturn([1, 2])); console.log(callHelperForOfString(['a', 'word'])); console.log(callHelperForOfString(['a'])); console.log(callHelperForInNumber({ low: 1, target: 4 })); console.log(callHelperForInNumber({ low: 1 })); console.log(callHelperForInString({ short: 'a', target: 'word' })); console.log(callHelperForInString({ short: 'a' })); console.log(callHelperNestedLoopReturn([1, 5])); console.log(callHelperNestedLoopReturn([1, 2])); console.log(callHelperNestedForReturn([1, 5])); console.log(callHelperNestedForReturn([1, 2])); console.log(callHelperNestedForOfReturn([1, 5])); console.log(callHelperNestedForOfReturn([1, 2])); console.log(callHelperNestedForInReturn({ low: 1, target: 4 })); console.log(callHelperNestedForInReturn({ low: 1 })); console.log(callHelperNestedSwitchReturn([1, 5])); const finallyFound = []; console.log(callHelperNestedFinallyReturn([1, 4], finallyFound)); const finallyMissing = []; console.log(callHelperNestedFinallyReturn([1, 2], finallyMissing)); const catchNestedOk = []; console.log(callHelperNestedCatchReturn(3, false, catchNestedOk)); const catchNestedBad = []; console.log(callHelperNestedCatchReturn(3, true, catchNestedBad)); console.log(callHelperNestedSwitchReturn([1, 2])); console.log(callHelperLabeledReturn([1, 4])); console.log(callHelperLabeledReturn([1, 2])); console.log(callHelperLabeledControl([1, 0, 4])); console.log(callHelperLabeledControl([1, 4])); console.log(callHelperLabeledControl([0, 1])); console.log(callHelperLabeledControl([0, 4])); console.log(callLabeledNumber([1, 0, -1])); console.log(callLabeledNumber([1, 0, 4])); }\n",
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
        "box:3:true:7:pair:1,2,7\n3:box:7:pair\n4:box\n8:alt\n3:box\n4:box\ndeep:4:false:1,4\n4:deep:false\nmade:4:3,9\nhelper\n42\n8\n39\n31\nbox:4\nfallback\nyes:5\nsmall:2\nyes:4\nno:2\nhigh:13\nlow:4\nzero:0\n4\n6\n10\none\ntwo\nother\ntwo:1\nother:1\n4:3,9\nok:3\n3,9\nbad:4\n3,9\nlow:0\n-1,9\nfound:1\nmissing:-1\nfound:1\nmissing:-1\nfound:1\nmissing:-1\nfound:4\nmissing:-1\nword:4\nmissing:0\ntarget:4\nmissing:0\nword:4\nmissing:0\nnested:1\nmissing:-1\nnested-for:1\nmissing:-1\nnested-for-of:5\nmissing:-1\ntarget:5\nmissing:-1\nswitch:1\nfinally:1:2\nmissing:-1:2\nok:3:1\nbad:4:1\nmissing:-1\nlabeled:1\nmissing:-1\nlabeled:2\nlabeled:1\ndone:2\nlabeled:1\n2\n2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn aggregate_catch_uses_collection_throw_values_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-aggregate-collection-catch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-aggregate-collection-catch");
    std::fs::create_dir_all(&package).unwrap();
    let dts = concat!(
        "export declare function arrayCatch(values: number[], fail: boolean): string;\n",
        "export declare function dictionaryCatch(values: Record<string, number>, fail: boolean): string;\n",
        "export declare function mixedCatch(mode: number, values: number[]): string;\n",
    );
    let source = concat!(
        "function arrayResult(values, fail) { try { if (fail) throw values; return { count: 0, label: 'ok' }; } catch (error) { return { count: error.length, label: 'array' }; } } ",
        "function dictionaryResult(values, fail) { try { if (fail) throw values; return { count: 0, label: 'ok' }; } catch (error) { return { count: error.value, label: 'dictionary' }; } } ",
        "function mixedResult(mode, values) { try { if (mode === 1) throw 'text'; if (mode === 2) throw values; return { count: 0, label: 'ok' }; } catch (error) { if (Array.isArray(error)) return { count: error.length, label: 'array' }; return { count: 0, label: error }; } } ",
        "module.exports.arrayCatch = (values, fail) => { const { count, label } = arrayResult(values, fail); return label + ':' + String(count); }; ",
        "module.exports.dictionaryCatch = (values, fail) => { const { count, label } = dictionaryResult(values, fail); return label + ':' + String(count); };",
        "module.exports.mixedCatch = (mode, values) => { const { count, label } = mixedResult(mode, values); return label + ':' + String(count); };",
    );
    std::fs::write(package.join("package.d.ts"), dts).unwrap();
    std::fs::write(package.join("bundle.js"), source).unwrap();
    let declarations = thaw_bridge::parse_dts(dts).unwrap();
    for (index, name) in ["arrayCatch", "dictionaryCatch", "mixedCatch"]
        .into_iter()
        .enumerate()
    {
        let expression = jit_numeric_export(source, name, false, &declarations[index])
            .unwrap_or_else(|| panic!("{name}"));
        assert!(expression.contains("trystart"));
        assert!(expression.contains("catch"));
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { arrayCatch, dictionaryCatch, mixedCatch } from 'jit-aggregate-collection-catch';\nfunction main(): void { console.log(arrayCatch([1, 2, 3], false)); console.log(arrayCatch([1, 2, 3], true)); const values: Record<string, number> = { value: 42 }; console.log(dictionaryCatch(values, false)); console.log(dictionaryCatch(values, true)); console.log(mixedCatch(0, [1, 2])); console.log(mixedCatch(1, [1, 2])); console.log(mixedCatch(2, [1, 2])); }\n",
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
        "ok:0\narray:3\nok:0\ndictionary:42\nok:0\ntext:0\narray:2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_fixed_object_unions_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-object-union-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-object-union");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Meta { label: string; enabled: boolean; }\nexport interface Item { count: number; meta: Meta; values: number[]; scores: Record<string, number>; pair: [number, string, boolean]; }\nexport declare function valueOr(value?: Item | string): string;\nexport declare function describe(value: Item | string): string;\nexport declare function identity(value: Item | string): Item | string;\nexport declare function make(flag: boolean): Item | string;\nexport declare function wrap(values: number[]): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.valueOr = value => String(value ?? 'missing'); module.exports.describe = value => typeof value === 'object' ? String(value['count']) + ':' + value['meta']['label'].toUpperCase() + ':' + String(value.meta.enabled) + ':' + value.values.join(',') + ':' + String(value.scores.primary) + ':' + String(value.pair[0]) + ':' + value.pair[1] + ':' + String(value.pair[2]) : value.toUpperCase(); module.exports.identity = value => value; module.exports.make = flag => flag ? { count: 2, meta: { label: 'made', enabled: false }, values: [3, 4], scores: { primary: 7 }, pair: [8, 'tuple', true] } : 'none'; module.exports.wrap = values => ({ count: values.length, meta: { label: 'wrapped', enabled: true }, values, scores: { primary: values.length }, pair: [values.length, 'shared', false] });\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { valueOr, describe, identity, make, wrap } from 'jit-optional-object-union';\ntype Item = { count: number; meta: { label: string; enabled: boolean }; values: number[]; scores: Record<string, number>; pair: [number, string, boolean] };\nfunction show(value: Item | string): void { if (typeof value === 'object') console.log(String(value.count) + ':' + value.meta.label + ':' + String(value.meta.enabled) + ':' + value.values.join(',') + ':' + String(value.scores.primary) + ':' + String(value.pair[0]) + ':' + value.pair[1] + ':' + String(value.pair[2])); else console.log(value); }\nfunction main(): void { const item: Item = { count: 1, meta: { label: 'one', enabled: true }, values: [1, 2], scores: { primary: 9 }, pair: [3, 'local', false] }; console.log(valueOr(item)); console.log(valueOr('text')); console.log(valueOr()); console.log(describe(item)); console.log(describe('text')); show(identity(item)); show(identity('result')); show(make(true)); show(make(false)); const values = [5]; const wrapped = wrap(values); if (typeof wrapped === 'object') { wrapped.values.push(6); show(wrapped); console.log(values.join(',')); } }\n",
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
        "[object Object]\ntext\nmissing\n1:ONE:true:1,2:9:3:local:false\nTEXT\n1:one:true:1,2:9:3:local:false\nresult\n2:made:false:3,4:7:8:tuple:true\nnone\n1:wrapped:true:5,6:1:1:shared:false\n5,6\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn finite_computed_object_keys_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Pair { left: number; right: number; }\nexport declare function pick(value: Pair | string, right: boolean): number;\nexport declare function pickRuntime(value: Pair | string, key: string): number;\nexport declare function set(value: Pair | string, right: boolean, next: number): number;\nexport declare function add(value: Pair | string, right: boolean, amount: number): number;\nexport declare function postIncrement(value: Pair | string, right: boolean): number;\nexport declare function preDecrement(value: Pair | string, right: boolean): number;\nexport declare function orAssign(value: Pair | string, right: boolean, next: number): number;\nexport declare function andAssign(value: Pair | string, right: boolean, next: number): number;\n",
    )
    .unwrap();
    let source = "module.exports.pick = (value, right) => typeof value === 'object' ? value[right ? 'right' : 'left'] : -1; module.exports.pickRuntime = (value, key) => typeof value === 'object' ? value[key] ?? 0 : -1; module.exports.set = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] = next : -1; module.exports.add = (value, right, amount) => typeof value === 'object' ? value[right ? 'right' : 'left'] += amount : -1; module.exports.postIncrement = (value, right) => typeof value === 'object' ? value[right ? 'right' : 'left']++ : -1; module.exports.preDecrement = (value, right) => typeof value === 'object' ? --value[right ? 'right' : 'left'] : -1; module.exports.orAssign = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] ||= next : -1; module.exports.andAssign = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] &&= next : -1;";
    assert!(jit_numeric_export(source, "pick", false, &declarations[0]).is_some());
    assert!(
        jit_numeric_export(source, "pickRuntime", false, &declarations[1]).is_some(),
        "runtime key did not specialize"
    );
    assert!(jit_numeric_export(source, "set", false, &declarations[2]).is_some());
    assert!(jit_numeric_export(source, "add", false, &declarations[3]).is_some());
    assert!(jit_numeric_export(source, "postIncrement", false, &declarations[4]).is_some());
    assert!(jit_numeric_export(source, "preDecrement", false, &declarations[5]).is_some());
    assert!(jit_numeric_export(source, "orAssign", false, &declarations[6]).is_some());
    assert!(jit_numeric_export(source, "andAssign", false, &declarations[7]).is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-finite-object-key-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-finite-object-key");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Pair { left: number; right: number; }\nexport declare function pick(value: Pair | string, right: boolean): number;\nexport declare function pickRuntime(value: Pair | string, key: string): number;\nexport declare function set(value: Pair | string, right: boolean, next: number): number;\nexport declare function add(value: Pair | string, right: boolean, amount: number): number;\nexport declare function postIncrement(value: Pair | string, right: boolean): number;\nexport declare function preDecrement(value: Pair | string, right: boolean): number;\nexport declare function orAssign(value: Pair | string, right: boolean, next: number): number;\nexport declare function andAssign(value: Pair | string, right: boolean, next: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.pick = (value, right) => typeof value === 'object' ? value[right ? 'right' : 'left'] : -1; module.exports.pickRuntime = (value, key) => typeof value === 'object' ? value[key] ?? 0 : -1; module.exports.set = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] = next : -1; module.exports.add = (value, right, amount) => typeof value === 'object' ? value[right ? 'right' : 'left'] += amount : -1; module.exports.postIncrement = (value, right) => typeof value === 'object' ? value[right ? 'right' : 'left']++ : -1; module.exports.preDecrement = (value, right) => typeof value === 'object' ? --value[right ? 'right' : 'left'] : -1; module.exports.orAssign = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] ||= next : -1; module.exports.andAssign = (value, right, next) => typeof value === 'object' ? value[right ? 'right' : 'left'] &&= next : -1;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { add, andAssign, orAssign, pick, pickRuntime, postIncrement, preDecrement, set } from 'jit-finite-object-key';\nfunction main(): void { const pair = { left: 3, right: 7 }; console.log(pick(pair, false)); console.log(pick(pair, true)); console.log(pick('none', true)); console.log(pickRuntime(pair, 'left')); console.log(pickRuntime(pair, 'right')); console.log(pickRuntime(pair, 'missing')); console.log(set(pair, false, 11)); console.log(set(pair, true, 13)); console.log(pick(pair, false)); console.log(pick(pair, true)); console.log(add(pair, false, 4)); console.log(postIncrement(pair, true)); console.log(preDecrement(pair, false)); console.log(pick(pair, false)); console.log(pick(pair, true)); console.log(orAssign(pair, false, 99)); console.log(andAssign(pair, true, 20)); console.log(set(pair, false, 0)); console.log(orAssign(pair, false, 8)); console.log(pick(pair, false)); console.log(pick(pair, true)); }\n",
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
        "3\n7\n-1\n3\n7\n0\n11\n13\n11\n13\n15\n13\n14\n14\n14\n14\n20\n0\n8\n8\n20\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_dictionary_unions_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-dictionary-union-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-object-dictionary-union");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { count: number; label: string; }\nexport type Mixed = Item | Record<string, number> | string;\nexport declare function identity(value: Mixed): Mixed;\nexport declare function text(value: Mixed): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.identity = value => value; module.exports.text = value => String(value);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { identity, text } from 'jit-object-dictionary-union';\nfunction main(): void { const item = { count: 2, label: 'item' }; const record: Record<string, number> = { count: 3 }; console.log(text(item)); console.log(text(record)); console.log(text('word')); console.log(text(identity(item))); console.log(text(identity(record))); console.log(text(identity('done'))); }\n",
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
        "[object Object]\n[object Object]\nword\n[object Object]\n[object Object]\ndone\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn nested_tuple_fields_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-nested-tuple-field-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-nested-tuple-field");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Meta { score: number; label: string; }\nexport interface Item { pair: [boolean, [number, string], Meta, number[], Record<string, number>]; }\nexport declare function numberValue(value: Item | string): number;\nexport declare function stringValue(value: Item | string): string;\nexport declare function objectNumber(value: Item | string): number;\nexport declare function objectString(value: Item | string): string;\nexport declare function arrayValue(value: Item | string): string;\nexport declare function dictionaryValue(value: Item | string): number;\nexport declare function make(flag: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.numberValue = value => typeof value === 'object' ? value.pair[1][0] : 0; module.exports.stringValue = value => typeof value === 'object' ? value.pair[1][1] : value; module.exports.objectNumber = value => typeof value === 'object' ? value.pair[2].score : 0; module.exports.objectString = value => typeof value === 'object' ? value.pair[2].label : value; module.exports.arrayValue = value => typeof value === 'object' ? value.pair[3].join(',') : value; module.exports.dictionaryValue = value => typeof value === 'object' ? value.pair[4].score : 0; module.exports.make = flag => flag ? { pair: [false, [7, 'made'], { score: 8, label: 'object' }, [10, 11], { score: 12 }] } : 'none';\n",
    )
    .unwrap();
    let declarations = thaw_bridge::parse_dts(
        &std::fs::read_to_string(package.join("package.d.ts")).unwrap(),
    )
    .unwrap();
    let source = std::fs::read_to_string(package.join("bundle.js")).unwrap();
    for (index, name) in [
        "numberValue",
        "stringValue",
        "objectNumber",
        "objectString",
        "arrayValue",
        "dictionaryValue",
        "make",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            jit_numeric_export(&source, name, false, &declarations[index]).is_some(),
            "{name}"
        );
    }
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { arrayValue, dictionaryValue, make, numberValue, objectNumber, objectString, stringValue } from 'jit-nested-tuple-field';\nfunction main(): void { const record: Record<string, number> = { score: 3 }; const value = { pair: [true, [42, 'nested'], { score: 9, label: 'local' }, [1, 2], record] as [boolean, [number, string], { score: number; label: string }, number[], Record<string, number>] }; console.log(numberValue(value)); console.log(stringValue(value)); console.log(objectNumber(value)); console.log(objectString(value)); console.log(arrayValue(value)); console.log(dictionaryValue(value)); console.log(numberValue('none')); console.log(stringValue('plain')); const made = make(true); console.log(numberValue(made)); console.log(stringValue(made)); console.log(objectNumber(made)); console.log(objectString(made)); console.log(arrayValue(made)); console.log(dictionaryValue(made)); console.log(stringValue(make(false))); }\n",
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
        "42\nnested\n9\nlocal\n1,2\n3\n0\nplain\n7\nmade\n8\nobject\n10,11\n12\nnone\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_object_fields_use_jit_without_quickjs() {
    let functions = thaw_bridge::parse_dts(
        "export interface Item { name?: string; score?: number; enabled?: boolean; }\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    assert!(jit_numeric_export(
        "module.exports.make = full => full ? { name: 'made' } : {};",
        "make",
        false,
        &functions[0],
    )
    .is_some());
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-object-field-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional-object-field");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { name?: string; score?: number; enabled?: boolean; }\nexport declare function name(value: Item | string): string;\nexport declare function upper(value: Item | string): string;\nexport declare function score(value: Item | string): number;\nexport declare function enabled(value: Item | string): boolean;\nexport declare function make(full: boolean): Item | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.name = value => typeof value === 'object' ? value.name ?? 'missing' : value; module.exports.upper = value => typeof value === 'object' ? value.name?.toUpperCase() ?? 'missing' : value; module.exports.score = value => typeof value === 'object' ? value.score ?? 0 : -1; module.exports.enabled = value => typeof value === 'object' ? value.enabled ?? false : true; module.exports.make = full => full ? { name: 'made' } : {};\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { enabled, make, name, score, upper } from 'jit-optional-object-field';\ntype Item = { name?: string; score?: number; enabled?: boolean };\nfunction main(): void { const present: Item = { name: 'thaw', score: 7, enabled: true }; const absent: Item = {}; console.log(name(present)); console.log(upper(present)); console.log(score(present)); console.log(enabled(present)); console.log(name(absent)); console.log(upper(absent)); console.log(score(absent)); console.log(enabled(absent)); console.log(name('plain')); console.log(name(make(true))); console.log(score(make(true))); console.log(enabled(make(true))); console.log(name(make(false))); console.log(score(make(false))); console.log(enabled(make(false))); }\n",
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
        "thaw\nTHAW\n7\ntrue\nmissing\nmissing\n0\nfalse\nplain\nmade\n0\nfalse\nmissing\n0\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn tagged_object_field_assignments_use_jit_without_quickjs() {
    let declarations = thaw_bridge::parse_dts(
        "export interface Item { optional?: number; nullable: number | null; nullish: number | null | undefined; }\nexport declare function setOptional(value: Item | string, next: number): number;\nexport declare function setNullable(value: Item | string, next: number): number;\nexport declare function setNullish(value: Item | string, next: number): number;\nexport declare function postOptional(value: Item | string): number;\nexport declare function preNullable(value: Item | string): number;\nexport declare function postNullish(value: Item | string): number;\nexport declare function preNullish(value: Item | string): number;\nexport declare function addOptional(value: Item | string, next: number): number;\nexport declare function addNullable(value: Item | string, next: number): number;\nexport declare function addNullish(value: Item | string, next: number): number;\nexport declare function defaultOptional(value: Item | string, next: number): number;\nexport declare function defaultNullable(value: Item | string, next: number): number;\nexport declare function defaultNullish(value: Item | string, next: number): number;\nexport declare function defaultFrom(value: Item | string, source: Item | string): number;\nexport declare function orOptional(value: Item | string, next: number): number;\nexport declare function orNullish(value: Item | string, next: number): number;\nexport declare function andNullable(value: Item | string, next: number): number | null;\nexport declare function andNullish(value: Item | string, next: number): number | null | undefined;\n",
    )
    .unwrap();
    let source = "module.exports.setOptional = (value, next) => typeof value === 'object' ? value.optional = next : -1; module.exports.setNullable = (value, next) => typeof value === 'object' ? value.nullable = next : -1; module.exports.setNullish = (value, next) => typeof value === 'object' ? value.nullish = next : -1; module.exports.postOptional = value => typeof value === 'object' ? value.optional++ : -1; module.exports.preNullable = value => typeof value === 'object' ? ++value.nullable : -1; module.exports.postNullish = value => typeof value === 'object' ? value.nullish++ : -1; module.exports.preNullish = value => typeof value === 'object' ? ++value.nullish : -1; module.exports.addOptional = (value, next) => typeof value === 'object' ? value.optional += next : -1; module.exports.addNullable = (value, next) => typeof value === 'object' ? value.nullable += next : -1; module.exports.addNullish = (value, next) => typeof value === 'object' ? value.nullish += next : -1; module.exports.defaultOptional = (value, next) => typeof value === 'object' ? value.optional ??= next : -1; module.exports.defaultNullable = (value, next) => typeof value === 'object' ? value.nullable ??= next : -1; module.exports.defaultNullish = (value, next) => typeof value === 'object' ? value.nullish ??= next : -1; module.exports.defaultFrom = (value, source) => typeof value === 'object' ? (typeof source === 'object' ? value.optional ??= ++source.nullable : -1) : -1; module.exports.orOptional = (value, next) => typeof value === 'object' ? value.optional ||= next : -1; module.exports.orNullish = (value, next) => typeof value === 'object' ? value.nullish ||= next : -1; module.exports.andNullable = (value, next) => typeof value === 'object' ? value.nullable &&= next : 0; module.exports.andNullish = (value, next) => typeof value === 'object' ? value.nullish &&= next : 0;";
    for (index, name) in [
        "setOptional",
        "setNullable",
        "setNullish",
        "postOptional",
        "preNullable",
        "postNullish",
        "preNullish",
        "addOptional",
        "addNullable",
        "addNullish",
        "defaultOptional",
        "defaultNullable",
        "defaultNullish",
        "defaultFrom",
        "orOptional",
        "orNullish",
        "andNullable",
        "andNullish",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            jit_numeric_export(source, name, false, &declarations[index]).is_some(),
            "{name} did not specialize"
        );
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tagged-object-field-assignment-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tagged-object-field-assignment");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Item { optional?: number; nullable: number | null; nullish: number | null | undefined; }\nexport declare function setOptional(value: Item | string, next: number): number;\nexport declare function setNullable(value: Item | string, next: number): number;\nexport declare function setNullish(value: Item | string, next: number): number;\nexport declare function postOptional(value: Item | string): number;\nexport declare function preNullable(value: Item | string): number;\nexport declare function postNullish(value: Item | string): number;\nexport declare function preNullish(value: Item | string): number;\nexport declare function addOptional(value: Item | string, next: number): number;\nexport declare function addNullable(value: Item | string, next: number): number;\nexport declare function addNullish(value: Item | string, next: number): number;\nexport declare function defaultOptional(value: Item | string, next: number): number;\nexport declare function defaultNullable(value: Item | string, next: number): number;\nexport declare function defaultNullish(value: Item | string, next: number): number;\nexport declare function defaultFrom(value: Item | string, source: Item | string): number;\nexport declare function orOptional(value: Item | string, next: number): number;\nexport declare function orNullish(value: Item | string, next: number): number;\nexport declare function andNullable(value: Item | string, next: number): number | null;\nexport declare function andNullish(value: Item | string, next: number): number | null | undefined;\nexport declare function optional(value: Item | string): number;\nexport declare function nullable(value: Item | string): number;\nexport declare function nullish(value: Item | string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.setOptional = (value, next) => typeof value === 'object' ? value.optional = next : -1; module.exports.setNullable = (value, next) => typeof value === 'object' ? value.nullable = next : -1; module.exports.setNullish = (value, next) => typeof value === 'object' ? value.nullish = next : -1; module.exports.postOptional = value => typeof value === 'object' ? value.optional++ : -1; module.exports.preNullable = value => typeof value === 'object' ? ++value.nullable : -1; module.exports.postNullish = value => typeof value === 'object' ? value.nullish++ : -1; module.exports.preNullish = value => typeof value === 'object' ? ++value.nullish : -1; module.exports.addOptional = (value, next) => typeof value === 'object' ? value.optional += next : -1; module.exports.addNullable = (value, next) => typeof value === 'object' ? value.nullable += next : -1; module.exports.addNullish = (value, next) => typeof value === 'object' ? value.nullish += next : -1; module.exports.defaultOptional = (value, next) => typeof value === 'object' ? value.optional ??= next : -1; module.exports.defaultNullable = (value, next) => typeof value === 'object' ? value.nullable ??= next : -1; module.exports.defaultNullish = (value, next) => typeof value === 'object' ? value.nullish ??= next : -1; module.exports.defaultFrom = (value, source) => typeof value === 'object' ? (typeof source === 'object' ? value.optional ??= ++source.nullable : -1) : -1; module.exports.orOptional = (value, next) => typeof value === 'object' ? value.optional ||= next : -1; module.exports.orNullish = (value, next) => typeof value === 'object' ? value.nullish ||= next : -1; module.exports.andNullable = (value, next) => typeof value === 'object' ? value.nullable &&= next : 0; module.exports.andNullish = (value, next) => typeof value === 'object' ? value.nullish &&= next : 0; module.exports.optional = value => typeof value === 'object' ? value.optional ?? 0 : -1; module.exports.nullable = value => typeof value === 'object' ? value.nullable ?? 0 : -1; module.exports.nullish = value => typeof value === 'object' ? value.nullish ?? 0 : -1;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { andNullable, andNullish, addNullable, addNullish, addOptional, defaultFrom, defaultNullable, defaultNullish, defaultOptional, nullable, nullish, optional, orNullish, orOptional, postNullish, postOptional, preNullable, preNullish, setNullable, setNullish, setOptional } from 'jit-tagged-object-field-assignment';\ntype Item = { optional?: number; nullable: number | null; nullish: number | null | undefined };
function main(): void { const value: Item = { nullable: null, nullish: undefined }; const nullValue: Item = { nullable: null, nullish: null }; const missing: Item = { nullable: null, nullish: undefined }; const defaults: Item = { nullable: null, nullish: undefined }; const lazy: Item = { nullable: null, nullish: undefined }; const source: Item = { nullable: 5, nullish: undefined }; const logical: Item = { nullable: null, nullish: undefined }; const truthy: Item = { optional: 2, nullable: 3, nullish: 4 }; const nullAnd: Item = { nullable: null, nullish: null }; const undefinedAnd: Item = { nullable: null, nullish: undefined }; const nullableResult: number | null = andNullable(logical, 20); const nullableTruthy: number | null = andNullable(truthy, 21); const nullResult: number | null | undefined = andNullish(nullAnd, 20); const undefinedResult: number | null | undefined = andNullish(undefinedAnd, 20); const nullishTruthy: number | null | undefined = andNullish(truthy, 22); console.log(defaultFrom(lazy, source)); console.log(defaultFrom(lazy, source)); console.log(nullable(source)); console.log(defaultOptional(defaults, 11)); console.log(defaultNullable(defaults, 13)); console.log(defaultNullish(defaults, 17)); console.log(defaultOptional(defaults, 99)); console.log(defaultNullable(defaults, 99)); console.log(defaultNullish(defaults, 99)); console.log(optional(value)); console.log(nullable(value)); console.log(nullish(value)); console.log(postOptional(value)); console.log(preNullable(value)); console.log(postNullish(value)); console.log(preNullish(nullValue)); console.log(addOptional(missing, 2)); console.log(addNullable(missing, 2)); console.log(addNullish(missing, 2)); console.log(setOptional(value, 3)); console.log(setNullable(value, 5)); console.log(setNullish(value, 7)); console.log(addOptional(value, 2)); console.log(addNullable(value, 2)); console.log(addNullish(value, 2)); console.log(optional(value)); console.log(nullable(value)); console.log(nullish(value)); console.log(orOptional(logical, 11)); console.log(orOptional(logical, 99)); console.log(orNullish(logical, 17)); console.log(orNullish(logical, 99)); console.log(nullableResult === null); console.log(nullable(logical)); console.log(nullableTruthy!); console.log(nullable(truthy)); console.log(nullResult === null); console.log(undefinedResult === undefined); console.log(nullishTruthy!); console.log(nullish(truthy)); }\n",
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
        "6\n6\n6\n11\n13\n17\n11\n13\n17\n0\n0\n0\nNaN\n1\nNaN\n1\nNaN\n2\nNaN\n3\n5\n7\n5\n7\n9\n5\n7\n9\n11\n11\n17\n17\ntrue\n0\n21\n21\ntrue\ntrue\n22\n22\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

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

