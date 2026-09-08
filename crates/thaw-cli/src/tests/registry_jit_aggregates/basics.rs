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

