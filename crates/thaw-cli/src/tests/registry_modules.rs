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

#[test]
fn pure_numeric_registry_export_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-math");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function add(left: number, right: number): number;\nexport declare function accumulate(left: number, right: number): number;\nexport declare function choose(left: number, right: number): number;\nexport declare function sub(left: number, right: number): number;\nexport declare function double(value: number): number;\nexport declare function negate(value: number): number;\nexport declare function mask(value: number): number;\nexport declare function fallbackOr(value: number, fallback: number): number;\nexport declare function guard(value: number, result: number): number;\nexport declare function magnitude(value: number): number;\nexport declare function roundedRoot(value: number): number;\nexport declare function logarithm(value: number): number;\nexport declare function integerMath(left: number, right: number): number;\nexport declare function numericPredicates(value: number): boolean;\nexport declare function stringPredicate(value: string): boolean;\nexport declare function stringLength(value: string): number;\nexport declare function stringLess(left: string, right: string): boolean;\nexport declare function greet(value: string): string;\nexport declare function matches(value: string): boolean;\nexport declare function find(value: string): number;\nexport declare function transform(value: string): string;\nexport declare function clean(value: string): string;\nexport declare function first(value: string, index: number): string;\nexport declare function code(value: string, index: number): number;\nexport declare function power(base: number, exponent: number): number;\nexport declare function less(left: number, right: number): boolean;\nexport declare function negateFlag(value: boolean): boolean;\nexport declare function remainder(left: number, right: number): number;\nexport declare function minimum(a: number, b: number, c: number): number;\nexport declare function maximum(a: number, b: number, c: number): number;\nexport declare function sum3(a: number, b: number, c: number): number;\nexport declare function answer(): number;\nexport declare function scaled(value: number): number;\nexport declare function localAlias(value: number): number;\nexport declare function chooseAlias(flag: boolean, value: number): number;\nexport declare function moduleAlias(value: number): number;\nexport declare function branchAlias(flag: boolean, value: number): number;\n",
    )
    .unwrap();
    let dts_path = package.join("package.d.ts");
    let mut declarations = std::fs::read_to_string(&dts_path).unwrap();
    declarations.push_str(
        "export declare function tableAlias(name: string, value: number): number;\nexport declare function pickedAlias(name: string, value: number): number;\nexport declare function namedAlias(value: number): number;\nexport declare function tableReassigned(name: string, value: number): number;\nexport declare function tableFlag(name: string, value: number): boolean;\nexport declare function tableText(name: string, value: number): string;\nexport declare function tableArray(name: string, value: number): number[];\nexport declare function tableRecord(name: string, value: number): Record<string, number>;\nexport declare function nextCounter(delta: number): number;\nexport declare function readCounter(): number;\nexport declare function useIncrement(): number;\nexport declare function useTwice(): number;\nexport declare function statefulTable(value: number): number;\n",
    );
    declarations.push_str(
        "export declare function useTwiceAt(flag: boolean): number;\nexport declare function statefulAt(name: string, value: number): number;\nexport declare function nestedSelect(flag: boolean, value: number): number;\nexport declare function trySelect(flag: boolean, value: number): number;\nexport declare function chooseHelper(flag: boolean): number;\nexport declare function chooseAt(keyFlag: boolean, helperFlag: boolean): number;\nexport declare function setDynamic(name: string, helperFlag: boolean): number;\nexport declare function stagedAlias(select: boolean, reset: boolean, value: number): number;\nexport declare function loopAlias(flag: boolean, value: number): number;\nexport declare function forAlias(flag: boolean, value: number): number;\nexport declare function doAlias(flag: boolean, value: number): number;\nexport declare function forOfAlias(flag: boolean, value: number): number;\nexport declare function forInAlias(flag: boolean, value: number): number;\nexport declare function controlledAlias(flag: boolean, value: number): number;\nexport declare function conditionalLoopAlias(flag: boolean, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function snapshotAlias(name: string, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function reassignAlias(first: string, second: string, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function branchSnapshot(flag: boolean, first: string, second: string, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function loopSnapshot(flag: boolean, first: string, second: string, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function controlledSnapshot(flag: boolean, first: string, second: string, value: number): number;\n",
    );
    declarations.push_str(
        "export declare function structuredSnapshot(mode: number, first: string, second: string, value: number): number;\nexport declare function catchSnapshot(flag: boolean, first: string, second: string, value: number): number;\nexport declare function switchSnapshot(mode: number, first: string, second: string, value: number): number;\nexport declare function finallySnapshot(flag: boolean, first: string, second: string, value: number): number;\nexport declare function fixedAlias(flag: boolean, value: number): number;\n",
    );
    std::fs::write(dts_path, declarations).unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "'use strict'; const SCALE = 2; const double = value => value * SCALE; module.exports = { add: function(left, right) { const sum = left + right; const doubled = sum * SCALE; const delta = left - right; if (left < right) return doubled; return delta; }, accumulate: function(left, right) { let total = left; total += right; total *= SCALE; total--; return total; }, choose: function(left, right) { if (left) { if (right < 0) return 1; return 2; } else if (right) return 3; else return 4; }, sub: (left, right) => left - right, double, negate: value => -value, mask: value => (value & 255) ^ 42, fallbackOr: (value, fallback) => value || fallback, guard: (value, result) => value && result, magnitude: value => Math.abs(value), roundedRoot: value => finish(Math.sqrt(value)), logarithm: value => value + Math.round(Math.PI), integerMath: (left, right) => Math.imul(left, right) + Math.clz32(1) + Math.fround(1), numericPredicates: value => Number.isSafeInteger(value), stringPredicate: value => Number.isNaN(value) || isNaN(value), stringLength: value => value.length, stringLess: (left, right) => left.localeCompare(right) < 0, greet: value => `hello, ${value}!`, matches: value => value.isWellFormed() && value.startsWith('pre', 0) && value.endsWith('fix', 6) && value.includes('ref', 1), find: value => value.indexOf('😀', 1) + value.lastIndexOf('😀', 3), transform: value => value.toWellFormed().toLowerCase().toUpperCase(), clean: value => value.trim().repeat(2).slice(0, 2).substring(1, 0).padStart(3, '0').padEnd(4, '1').replace('0', 'a').replaceAll('1', 'b'), first: (value, index) => value.charAt(index), code: (value, index) => value.charCodeAt(index), power: (base, exponent) => Math.pow(base, exponent), less: (left, right) => left < right, negateFlag: value => !value, remainder: (left, right) => left % right, minimum: (a, b, c) => Math.min(a, b, c), maximum: (a, b, c) => Math.max(a, b, c), sum3: (a, b, c) => a + b + c, answer: () => Math.round(Math.PI) + 39 }; function finish(value) { return Math.round(value); }\n",
    )
    .unwrap();
    let bundle_path = package.join("bundle.js");
    let bundle = std::fs::read_to_string(&bundle_path).unwrap().replace(
        "module.exports = {",
        "function scaled(value) { return value * factor; } let factor = 2; factor += 1; function increment(value) { return value + 1; } function twice(value) { return value * 2; } function localAlias(value) { const operation = twice; return operation(value); } function chooseAlias(flag, value) { return (flag ? increment : twice)(value); } let moduleOperation = increment; moduleOperation = twice; function moduleAlias(value) { return moduleOperation(value); } const operations = { increment, double: increment }; operations.double = twice; operations['plus'] = increment; const plusKey = 'plus'; operations[plusKey] = increment; function tableAlias(name, value) { return operations[name](value); } let reassigned = { increment: twice }; reassigned = { increment, double: twice }; function tableReassigned(name, value) { return reassigned[name](value); } function positive(value) { return value > 0; } function zero(value) { return value === 0; } const predicates = { positive, zero }; function tableFlag(name, value) { return predicates[name](value); } function prefix(value) { return 'x' + value; } function suffix(value) { return value + 'x'; } const texts = { prefix, suffix }; function tableText(name, value) { return texts[name](value); } function one(value) { return [value]; } function two(value) { return [value, value]; } const arrays = { one, two }; function tableArray(name, value) { return arrays[name](value); } function single(value) { return { value }; } function doubled(value) { return { value: value * 2 }; } const records = { single, doubled }; function tableRecord(name, value) { return records[name](value); } let counter = 5; function nextCounter(delta) { counter += delta; return counter; } function readCounter() { return counter; } const mutableOperations = { run: increment }; function useIncrement() { mutableOperations.run = increment; return 0; } function useTwice() { mutableOperations.run = twice; return 0; } function statefulTable(value) { return mutableOperations.run(value); } module.exports = { scaled, localAlias, chooseAlias, moduleAlias, tableAlias, tableReassigned, tableFlag, tableText, tableArray, tableRecord, nextCounter, readCounter, useIncrement, useTwice, statefulTable,",
    );
    let bundle = bundle.replace(
        "const operation = twice;",
        "let operation = increment; operation = twice;",
    );
    let bundle = bundle
        .replace(
            "function moduleAlias(value) { return moduleOperation(value); }",
            "function moduleAlias(value) { return moduleOperation(value); } function branchAlias(flag, value) { let operation = twice; switch (flag) { case true: operation = twice; break; default: operation = increment; } return operation(value); } function stagedAlias(select, reset, value) { let operation = increment; try { if (select) throw 'select'; } catch { operation = twice; } finally { if (reset) operation = increment; } return operation(value); } function loopAlias(flag, value) { let operation = increment; let index = 0; while (index < 2) { if (flag && index === 1) { operation = twice; } index++; } return operation(value); } function forAlias(flag, value) { let operation = increment; for (let index = 0; index < 2; index++) { if (flag && index === 1) operation = twice; } return operation(value); } function doAlias(flag, value) { let operation = increment; let index = 0; do { if (flag) operation = twice; index++; } while (index < 1); return operation(value); } function forOfAlias(flag, value) { let operation = increment; for (const item of [0, 1]) { if (flag && item === 1) operation = twice; } return operation(value); } function forInAlias(flag, value) { let operation = increment; for (const key in { left: 1, right: 2 }) { if (flag && key === 'right') operation = twice; } return operation(value); } function controlledAlias(flag, value) { let operation = increment; let index = 0; while (index < 2) { try { if (flag && index === 0) { operation = twice; continue; } if (index === 1) break; } finally { index++; } } return operation(value); } function conditionalLoopAlias(flag, value) { let operation = flag ? twice : increment; let index = 0; while (index < 1) { operation = flag ? increment : twice; index++; } return operation(value); }",
        )
        .replace(
            "scaled, localAlias, chooseAlias, moduleAlias,",
            "scaled, localAlias, chooseAlias, moduleAlias, branchAlias, stagedAlias, loopAlias, forAlias, doAlias, forOfAlias, forInAlias, controlledAlias, conditionalLoopAlias,",
        );
    let bundle = bundle
        .replace(
            "function tableAlias(name, value) { return operations[name](value); }",
            "function tableAlias(name, value) { return operations[name](value); } function pickedAlias(name, value) { const selected = operations[name]; return selected(value); } function namedAlias(value) { const selected = (operations.increment); return selected(value); }",
        )
        .replace("tableAlias, tableReassigned,", "tableAlias, pickedAlias, namedAlias, tableReassigned,");
    let bundle = bundle.replace(
        "const mutableOperations = { run: increment };",
        "const mutableOperations = { run: increment, left: increment, right: increment }; function useTwiceAt(flag) { mutableOperations[flag ? 'left' : 'right'] = twice; return 0; } function statefulAt(name, value) { return mutableOperations[name](value); } function nestedSelect(flag, value) { let index = 0; while (index < 1) { if (flag) { mutableOperations.run = twice; } index++; } return mutableOperations.run(value); } function trySelect(flag, value) { let index = 0; while (index < 1) { try { if (flag) { mutableOperations.run = increment; } } finally { index++; } } return mutableOperations.run(value); } function chooseHelper(flag) { mutableOperations.run = flag ? twice : increment; return 0; } function chooseAt(keyFlag, helperFlag) { mutableOperations[keyFlag ? 'left' : 'right'] = helperFlag ? twice : increment; return 0; } function setDynamic(name, helperFlag) { mutableOperations[name] = helperFlag ? twice : increment; return 0; } function snapshotAlias(name, value) { setDynamic(name, false); const selected = mutableOperations[name]; setDynamic(name, true); return selected(value); } function reassignAlias(first, second, value) { setDynamic(first, false); setDynamic(second, true); let selected = mutableOperations[first]; selected = mutableOperations[second]; setDynamic(second, false); return selected(value); } function branchSnapshot(flag, first, second, value) { setDynamic(first, false); setDynamic(second, true); let selected = mutableOperations[first]; if (flag) { const reached = 1; { selected = mutableOperations[second]; } } else if (first === second) { selected = mutableOperations[second]; } else { const reached = 0; { selected = mutableOperations[first]; } } return setDynamic(second, false) + selected(value); } function loopSnapshot(flag, first, second, value) { let selected = increment; let index = 0; index++; index -= 1; if (!flag) { selected = increment; } while (flag && index < 1) { selected = mutableOperations[second]; index++; } return setDynamic(second, false) + selected(value); } function controlledSnapshot(flag, first, second, value) { let selected = mutableOperations[first]; let index = 0; while (index < 2) { try { if (flag && index === 0) { selected = mutableOperations[second]; continue; } if (index === 1) break; } finally { index++; } } return setDynamic(second, false) + selected(value); }",
    ).replace(
        "useIncrement, useTwice, statefulTable,",
        "useIncrement, useTwice, statefulTable, snapshotAlias, reassignAlias, branchSnapshot, loopSnapshot, controlledSnapshot, structuredSnapshot, catchSnapshot, switchSnapshot, finallySnapshot, fixedAlias, useTwiceAt, statefulAt, nestedSelect, trySelect, chooseHelper, chooseAt, setDynamic,",
    );
    let bundle = bundle
        .replace(
            "function setDynamic(name, helperFlag)",
            "function fallbackIncrement(value) { return value + 1; } const fixedOperations = { run: increment }; function setFixed(flag) { fixedOperations.run = flag ? twice : increment; return 0; } function fixedAlias(flag, value) { let selected = fallbackIncrement; setFixed(true); if (flag) { selected = fixedOperations.run; } return setFixed(false) + selected(value); } function setDynamic(name, helperFlag)",
        )
        .replace(
            "else { const reached = 0; { selected = mutableOperations[first]; } } return setDynamic(second, false) + selected(value);",
            "else { const reached = 0; { selected = fallbackIncrement; } } return setDynamic(second, false) + selected(value);",
        )
        .replace(
            "function loopSnapshot(flag, first, second, value) { let selected = increment;",
            "function loopSnapshot(flag, first, second, value) { let selected = fallbackIncrement;",
        )
        .replace(
            "if (!flag) { selected = increment; } while (flag && index < 1)",
            "if (!flag) { selected = fallbackIncrement; } while (flag && index < 1)",
        );
    let bundle = bundle.replace(
        "function controlledSnapshot(flag, first, second, value) { let selected = mutableOperations[first]; let index = 0; while (index < 2) { try { if (flag && index === 0) { selected = mutableOperations[second]; continue; } if (index === 1) break; } finally { index++; } } return setDynamic(second, false) + selected(value); }",
        "function controlledSnapshot(flag, first, second, value) { let selected = mutableOperations[first]; let index = 0; while (index < 2) { try { if (flag && index === 0) { selected = mutableOperations[second]; continue; } if (index === 1) break; } finally { index++; } } return setDynamic(second, false) + selected(value); } function structuredSnapshot(mode, first, second, value) { let selected = mutableOperations[first]; let index = 0; while (index < 1) { switch (mode) { case 1: { selected = mutableOperations[second]; break; } default: { { selected = mutableOperations[first]; } } } index++; } return setDynamic(second, false) + selected(value); } function catchSnapshot(flag, first, second, value) { let selected = mutableOperations[first]; let index = 0; while (index < 1) { try { if (flag) throw 'pick'; } catch { selected = mutableOperations[second]; } index++; } return setDynamic(second, false) + selected(value); } function switchSnapshot(mode, first, second, value) { let selected = mutableOperations[first]; switch (mode) { case 1: { selected = mutableOperations[second]; break; } default: { selected = mutableOperations[first]; } } return setDynamic(second, false) + selected(value); } function finallySnapshot(flag, first, second, value) { let selected = mutableOperations[first]; try { if (flag) throw 'pick'; } catch { selected = mutableOperations[second]; } finally { setDynamic(second, false); } return selected(value); }",
    );
    std::fs::write(&bundle_path, bundle).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { add, accumulate, choose, sub, double, negate, mask, fallbackOr, guard, magnitude, roundedRoot, logarithm, integerMath, stringLength, stringLess, greet, matches, find, transform, clean, first, code, power, less, negateFlag, remainder, minimum, maximum, sum3, answer, scaled, localAlias, chooseAlias, moduleAlias } from 'jit-math';\nfunction main(): void { console.log(add(2, 19) + accumulate(1, 2) + choose(1, -1) + sub(1, 1) + double(0) + negate(0) + mask(42) + fallbackOr(0, 42) + guard(0, 42) + magnitude(-1) + roundedRoot(0) + logarithm(8) + integerMath(3, 4) + stringLength('😀') + (stringLess('a', 'b') ? 1 : 0) + stringLength(greet('x')) + (matches('prefix') ? 1 : 0) + find('😀a😀') + stringLength(transform('Ab')) + stringLength(clean(' x ')) + stringLength(first('abc', 0)) + code('A', 0) + power(2, 0) + (less(1, 2) && negateFlag(false) ? 1 : 0) + remainder(5, 2) + minimum(-1, 0, 1) + maximum(-1, 0, 1) + sum3(0, 0, 0) + answer() - 240); console.log(scaled(2)); console.log(localAlias(4)); console.log(chooseAlias(true, 4)); console.log(chooseAlias(false, 4)); console.log(moduleAlias(4)); }\n",
    )
    .unwrap();
    let source = std::fs::read_to_string(&entry)
        .unwrap()
        .replace(
            "moduleAlias }",
            "moduleAlias, branchAlias, stagedAlias, loopAlias, forAlias, doAlias, forOfAlias, forInAlias, controlledAlias, conditionalLoopAlias, tableAlias, pickedAlias, namedAlias, tableReassigned, tableFlag, tableText, tableArray, tableRecord, nextCounter, readCounter, useIncrement, useTwice, statefulTable, snapshotAlias, reassignAlias, branchSnapshot, loopSnapshot, controlledSnapshot, structuredSnapshot, catchSnapshot, switchSnapshot, finallySnapshot, fixedAlias, useTwiceAt, statefulAt, nestedSelect, trySelect, chooseHelper, chooseAt, setDynamic }",
        )
        .replace(
            "console.log(moduleAlias(4)); }",
            "console.log(moduleAlias(4)); console.log(tableAlias('increment', 4)); console.log(tableAlias('double', 4)); console.log(tableAlias('plus', 4)); console.log(tableReassigned('increment', 4)); console.log(tableReassigned('double', 4)); console.log(tableFlag('positive', 4)); console.log(tableText('prefix', 4)); console.log(tableArray('two', 4).length); console.log(tableRecord('doubled', 4).value); console.log(nextCounter(2)); console.log(readCounter()); console.log(nextCounter(3)); console.log(readCounter()); console.log(statefulTable(4)); useTwice(); console.log(statefulTable(4)); useIncrement(); console.log(statefulTable(4)); try { tableAlias('missing', 4); } catch { console.log('missing'); } }",
        )
        .replace(
            "console.log(moduleAlias(4)); console.log(tableAlias",
            "console.log(moduleAlias(4)); console.log(branchAlias(false, 4)); console.log(branchAlias(true, 4)); console.log(stagedAlias(false, false, 4)); console.log(stagedAlias(true, false, 4)); console.log(stagedAlias(true, true, 4)); console.log(loopAlias(false, 4)); console.log(loopAlias(true, 4)); console.log(forAlias(false, 4)); console.log(forAlias(true, 4)); console.log(doAlias(false, 4)); console.log(doAlias(true, 4)); console.log(forOfAlias(false, 4)); console.log(forOfAlias(true, 4)); console.log(forInAlias(false, 4)); console.log(forInAlias(true, 4)); console.log(controlledAlias(false, 4)); console.log(controlledAlias(true, 4)); console.log(conditionalLoopAlias(false, 4)); console.log(conditionalLoopAlias(true, 4)); console.log(pickedAlias('increment', 4)); console.log(pickedAlias('double', 4)); console.log(namedAlias(4)); console.log(tableAlias",
        )
        .replace(
            "try { tableAlias('missing', 4); }",
            "console.log(statefulAt('left', 4)); console.log(statefulAt('right', 4)); useTwiceAt(true); console.log(statefulAt('left', 4)); console.log(statefulAt('right', 4)); useTwiceAt(false); console.log(statefulAt('right', 4)); useIncrement(); console.log(nestedSelect(false, 4)); console.log(nestedSelect(true, 4)); useTwice(); console.log(trySelect(false, 4)); console.log(trySelect(true, 4)); chooseHelper(true); console.log(statefulTable(4)); chooseHelper(false); console.log(statefulTable(4)); chooseAt(true, false); chooseAt(false, true); console.log(statefulAt('left', 4)); console.log(statefulAt('right', 4)); setDynamic('created', true); console.log(statefulAt('created', 4)); setDynamic('created', false); console.log(statefulAt('created', 4)); setDynamic('run', true); console.log(statefulTable(4)); useIncrement(); console.log(statefulTable(4)); try { tableAlias('missing', 4); }",
        )
        .replace(
            "try { tableAlias('missing', 4); }",
            "console.log(snapshotAlias('snapshot', 4)); console.log(reassignAlias('first', 'second', 4)); console.log(branchSnapshot(true, 'branchFirst', 'branchSecond', 4)); console.log(branchSnapshot(false, 'branchFirst', 'branchSecond', 4)); setDynamic('loopFirst', false); setDynamic('loopSecond', true); console.log(loopSnapshot(true, 'loopFirst', 'loopSecond', 4)); setDynamic('loopSecond', true); console.log(loopSnapshot(false, 'loopFirst', 'loopSecond', 4)); setDynamic('controlledFirst', false); setDynamic('controlledSecond', true); console.log(controlledSnapshot(true, 'controlledFirst', 'controlledSecond', 4)); setDynamic('controlledSecond', true); console.log(controlledSnapshot(false, 'controlledFirst', 'controlledSecond', 4)); setDynamic('structuredFirst', false); setDynamic('structuredSecond', true); console.log(structuredSnapshot(1, 'structuredFirst', 'structuredSecond', 4)); setDynamic('structuredSecond', true); console.log(structuredSnapshot(0, 'structuredFirst', 'structuredSecond', 4)); setDynamic('catchFirst', false); setDynamic('catchSecond', true); console.log(catchSnapshot(true, 'catchFirst', 'catchSecond', 4)); setDynamic('catchSecond', true); console.log(catchSnapshot(false, 'catchFirst', 'catchSecond', 4)); setDynamic('switchFirst', false); setDynamic('switchSecond', true); console.log(switchSnapshot(1, 'switchFirst', 'switchSecond', 4)); setDynamic('switchSecond', true); console.log(switchSnapshot(0, 'switchFirst', 'switchSecond', 4)); setDynamic('finallyFirst', false); setDynamic('finallySecond', true); console.log(finallySnapshot(true, 'finallyFirst', 'finallySecond', 4)); setDynamic('finallySecond', true); console.log(finallySnapshot(false, 'finallyFirst', 'finallySecond', 4)); console.log(fixedAlias(false, 4)); console.log(fixedAlias(true, 4)); useIncrement(); try { tableAlias('missing', 4); }",
        );
    std::fs::write(&entry, source).unwrap();
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
        "42\n6\n8\n5\n8\n8\n5\n8\n5\n8\n5\n5\n8\n5\n8\n5\n8\n5\n8\n5\n8\n5\n8\n8\n5\n5\n8\n5\n5\n8\n5\n5\n8\ntrue\nx4\n2\n8\n7\n7\n10\n10\n5\n8\n5\n5\n5\n8\n5\n8\n5\n8\n8\n5\n8\n5\n5\n8\n8\n5\n8\n5\n5\n8\n8\n5\n8\n5\n8\n5\n8\n5\n8\n5\n8\n5\n8\n5\n5\n8\nmissing\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_number_boolean_callable_tables_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-callables-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-callables");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function mixed(name: string, value: number): number;\nexport declare function aliased(name: string, value: number): number;\nexport declare function fixedMixed(flag: boolean, value: number): number;\nexport declare function dynamicMixed(name: string, flag: boolean, value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function increment(value) { return value + 1; } function positive(value) { return value > 0; } const operations = { increment, positive }; const fixed = { run: increment }; const dynamic = { run: increment }; function mixed(name, value) { return operations[name](value); } function aliased(name, value) { const selected = operations[name]; return selected(value); } function setFixed(flag) { fixed.run = flag ? positive : increment; return 0; } function fixedMixed(flag, value) { setFixed(flag); const selected = fixed.run; setFixed(false); return selected(value); } function setDynamic(name, flag) { dynamic[name] = flag ? positive : increment; return 0; } function dynamicMixed(name, flag, value) { setDynamic(name, flag); const selected = dynamic[name]; setDynamic(name, false); return selected(value); } module.exports = { mixed, aliased, fixedMixed, dynamicMixed };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { mixed, aliased, fixedMixed, dynamicMixed } from 'jit-mixed-callables';\nfunction main(): void { console.log(mixed('increment', 4)); console.log(mixed('positive', -1)); console.log(mixed('positive', 4)); console.log(aliased('positive', 4)); console.log(fixedMixed(false, 4)); console.log(fixedMixed(true, 4)); console.log(dynamicMixed('created', false, 4)); console.log(dynamicMixed('created', true, 4)); }\n",
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
        "5\n0\n1\n1\n5\n1\n5\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_number_string_callable_tables_use_tagged_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tagged-callables-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tagged-callables");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function mixed(name: string, value: number): number | string;\nexport declare function aliased(name: string, value: number): number | string;\nexport declare function fixedMixed(useLabel: boolean, value: number): number | string;\nexport declare function dynamicMixed(name: string, useLabel: boolean, value: number): number | string;\nexport declare function choose(useLabel: boolean, value: number): number | string;\nexport declare function chooseLocal(useLabel: boolean, value: number): number | string;\nexport declare function localControl(useLabel: boolean, replace: boolean, value: number): number | string;\nexport declare function crossTypeLocal(replace: boolean, value: number): number | string;\nexport declare function crossTypeLoop(replace: boolean, value: number): number | string;\nexport declare function typeOfLocal(replace: boolean, value: number): string;\nexport declare function narrowLocal(replace: boolean, value: number): number | string;\nexport declare function chooseStatement(useLabel: boolean, value: number): number | string;\nexport declare function chooseSwitch(mode: number, value: number): number | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function increment(value) { return value + 1; } function label(value) { return 'value=' + value; } const operations = { increment, label }; const fixed = { run: increment }; const dynamic = { run: increment }; function mixed(name, value) { return operations[name](value); } function aliased(name, value) { const selected = operations[name]; return selected(value); } function setFixed(useLabel) { fixed.run = useLabel ? label : increment; return 0; } function fixedMixed(useLabel, value) { setFixed(useLabel); const selected = fixed.run; setFixed(false); return selected(value); } function setDynamic(name, useLabel) { dynamic[name] = useLabel ? label : increment; return 0; } function dynamicMixed(name, useLabel, value) { setDynamic(name, useLabel); const selected = dynamic[name]; setDynamic(name, false); return selected(value); } function choose(useLabel, value) { return useLabel ? 'value=' + value : value + 1; } function chooseLocal(useLabel, value) { const result = useLabel ? 'value=' + value : value + 1; return result; } function localControl(useLabel, replace, value) { let result = useLabel ? 'value=' + value : value + 1; if (replace) result = useLabel ? value + 2 : 'next'; return result; } function crossTypeLocal(replace, value) { let result = value + 1; if (replace) result = 'next'; return result; } function crossTypeLoop(replace, value) { let result = value + 1; let index = 0; while (index < 1) { if (replace) result = 'loop'; index++; } return result; } function typeOfLocal(replace, value) { let result = value + 1; if (replace) result = 'next'; return typeof result; } function narrowLocal(replace, value) { let result = value + 1; if (replace) result = 'next'; return typeof result === 'number' ? result + 1 : result + '!'; } function chooseStatement(useLabel, value) { if (useLabel) return 'value=' + value; return value + 1; } function chooseSwitch(mode, value) { switch (mode) { case 0: return value + 1; case 1: return 'value=' + value; default: return 'other'; } } module.exports = { mixed, aliased, fixedMixed, dynamicMixed, choose, chooseLocal, localControl, crossTypeLocal, crossTypeLoop, typeOfLocal, narrowLocal, chooseStatement, chooseSwitch };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { mixed, aliased, fixedMixed, dynamicMixed, choose, chooseLocal, localControl, crossTypeLocal, crossTypeLoop, typeOfLocal, narrowLocal, chooseStatement, chooseSwitch } from 'jit-tagged-callables'; function show(value: number | string): void { if (typeof value === 'number') console.log(value); else console.log(value.toUpperCase()); } function main(): void { show(mixed('increment', 4)); show(mixed('label', 4)); show(aliased('label', 4)); show(fixedMixed(false, 4)); show(fixedMixed(true, 4)); show(dynamicMixed('created', false, 4)); show(dynamicMixed('created', true, 4)); show(choose(false, 4)); show(choose(true, 4)); show(chooseLocal(false, 4)); show(chooseLocal(true, 4)); show(localControl(false, false, 4)); show(localControl(true, false, 4)); show(localControl(false, true, 4)); show(localControl(true, true, 4)); show(crossTypeLocal(false, 4)); show(crossTypeLocal(true, 4)); show(crossTypeLoop(false, 4)); show(crossTypeLoop(true, 4)); console.log(typeOfLocal(false, 4)); console.log(typeOfLocal(true, 4)); show(narrowLocal(false, 4)); show(narrowLocal(true, 4)); show(chooseStatement(false, 4)); show(chooseStatement(true, 4)); show(chooseSwitch(0, 4)); show(chooseSwitch(1, 4)); show(chooseSwitch(2, 4)); }\n",
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
        "5\nVALUE=4\nVALUE=4\n5\nVALUE=4\n5\nVALUE=4\n5\nVALUE=4\n5\nVALUE=4\n5\nVALUE=4\nNEXT\n6\n5\nNEXT\n5\nLOOP\nnumber\nstring\n6\nNEXT!\n5\nVALUE=4\n5\nVALUE=4\nOTHER\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn boolean_primitive_unions_use_tagged_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-tagged-booleans-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-tagged-booleans");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function chooseBool(flag: boolean): boolean | string;\nexport declare function chooseThree(mode: number, value: number): number | boolean | string;\nexport declare function describe(value: number | boolean | string): string;\nexport declare function describeBox(input: { value: number | boolean | string }): string;\nexport declare function describeTuple(input: [number | boolean | string, string]): string;\nexport declare function truthy(value: number | boolean | string): boolean;\nexport declare function stringify(value: number | boolean | string): string;\nexport declare function numeric(value: number | boolean | string): number;\nexport declare function logicalAnd(value: number | boolean | string): number | boolean | string;\nexport declare function logicalOr(value: number | boolean | string): number | boolean | string;\nexport declare function add(left: number | boolean | string, right: number | boolean | string): number | boolean | string;\nexport declare function less(left: number | boolean | string, right: number | boolean | string): boolean;\nexport declare function looseEqual(left: number | boolean | string, right: number | boolean | string): boolean;\nexport declare function strictEqual(left: number | boolean | string, right: number | boolean | string): boolean;\nexport declare function narrowBool(flag: boolean): boolean | string;\nexport declare function narrowNot(flag: boolean): boolean | string;\nexport declare function narrowThree(mode: number, value: number): number | boolean | string;\nexport declare function narrowStatements(mode: number, value: number): number | boolean | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function chooseBool(flag) { return flag ? true : 'no'; } function chooseThree(mode, value) { switch (mode) { case 0: return value + 1; case 1: return true; default: return 'three'; } } function describe(value) { if (typeof value === 'boolean') return value ? 'yes' : 'no'; if (typeof value === 'number') return 'n=' + value; return value.toUpperCase(); } function describeBox(input) { const value = input.value; if (typeof value === 'boolean') return value ? 'box-yes' : 'box-no'; if (typeof value === 'number') return 'box=' + value; return value.toUpperCase(); } function describeTuple(input) { const value = input[0]; if (typeof value === 'boolean') return value ? 'tuple-yes' : 'tuple-no'; if (typeof value === 'number') return input[1] + '=' + value; return value.toUpperCase(); } function truthy(value) { return !!value; } function stringify(value) { return String(value); } function numeric(value) { return Number(value); } function logicalAnd(value) { return value && 'selected'; } function logicalOr(value) { return value || 'fallback'; } function add(left, right) { return left + right; } function less(left, right) { return left < right; } function looseEqual(left, right) { return left == right; } function strictEqual(left, right) { return left === right; } function narrowBool(flag) { const result = flag ? true : 'no'; return typeof result === 'boolean' ? !result : result + '!'; } function narrowNot(flag) { const result = flag ? true : 'no'; return typeof result !== 'boolean' ? result + '!' : !result; } function narrowThree(mode, value) { const result = mode === 0 ? value + 1 : mode === 1 ? true : 'three'; return typeof result === 'boolean' ? !result : typeof result === 'number' ? result + 10 : result + '!'; } function narrowStatements(mode, value) { const result = mode === 0 ? value + 1 : mode === 1 ? true : 'three'; if (typeof result === 'boolean') return !result; if (typeof result === 'number') return result + 20; return result + '?'; } module.exports = { chooseBool, chooseThree, describe, describeBox, describeTuple, truthy, stringify, numeric, logicalAnd, logicalOr, add, less, looseEqual, strictEqual, narrowBool, narrowNot, narrowThree, narrowStatements };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { chooseBool, chooseThree, describe, describeBox, describeTuple, truthy, stringify, numeric, logicalAnd, logicalOr, add, less, looseEqual, strictEqual, narrowBool, narrowNot, narrowThree, narrowStatements } from 'jit-tagged-booleans'; function showBool(value: boolean | string): void { if (typeof value === 'boolean') console.log(value); else console.log(value.toUpperCase()); } function showThree(value: number | boolean | string): void { if (typeof value === 'number') console.log(value); else if (typeof value === 'boolean') console.log(value); else console.log(value.toUpperCase()); } function main(): void { showBool(chooseBool(true)); showBool(chooseBool(false)); showThree(chooseThree(0, 4)); showThree(chooseThree(1, 4)); showThree(chooseThree(2, 4)); console.log(describe(chooseThree(0, 4))); console.log(describe(chooseThree(1, 4))); console.log(describe(chooseThree(2, 4))); console.log(describeBox({ value: chooseThree(0, 4) })); console.log(describeBox({ value: chooseThree(1, 4) })); console.log(describeBox({ value: chooseThree(2, 4) })); console.log(describeTuple([chooseThree(0, 4), 'item'])); console.log(describeTuple([chooseThree(1, 4), 'item'])); console.log(describeTuple([chooseThree(2, 4), 'item'])); console.log(truthy(0)); console.log(truthy(NaN)); console.log(truthy(2)); console.log(truthy('')); console.log(truthy('x')); console.log(truthy(false)); console.log(truthy(true)); console.log(stringify(2)); console.log(stringify(false)); console.log(stringify('kept')); console.log(numeric(5)); console.log(numeric(true)); console.log(numeric(' 42 ')); showThree(logicalAnd(0)); showThree(logicalAnd('x')); showThree(logicalOr(false)); showThree(logicalOr('kept')); showThree(add(2, 3)); showThree(add('x', 2)); showThree(add(true, 2)); console.log(less('10', '2')); console.log(less('10', 2)); console.log(looseEqual('2', 2)); console.log(looseEqual(true, 1)); console.log(strictEqual('2', 2)); console.log(strictEqual(NaN, NaN)); showBool(narrowBool(true)); showBool(narrowBool(false)); showBool(narrowNot(true)); showBool(narrowNot(false)); showThree(narrowThree(1, 4)); showThree(narrowThree(0, 4)); showThree(narrowThree(2, 4)); showThree(narrowStatements(1, 4)); showThree(narrowStatements(0, 4)); showThree(narrowStatements(2, 4)); }\n",
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
        "true\nNO\n5\ntrue\nTHREE\nn=5\nyes\nTHREE\nbox=5\nbox-yes\nTHREE\nitem=5\ntuple-yes\nTHREE\nfalse\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\n2\nfalse\nkept\n5\n1\n42\n0\nSELECTED\nFALLBACK\nKEPT\n5\nX2\n3\ntrue\nfalse\ntrue\ntrue\nfalse\nfalse\nfalse\nNO!\nfalse\nNO!\nfalse\n15\nTHREE!\nfalse\n25\nTHREE?\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_primitive_loop_locals_narrow_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-dynamic-loop-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-dynamic-loop");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function loopNarrow(mode: number, value: number): number | boolean | string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function loopNarrow(mode, value) { let result = mode === 0 ? value + 1 : mode === 1 ? true : 'three'; let index = 0; while (index < 1) { if (typeof result === 'boolean') result = !result; else if (typeof result === 'number') result = result + 30; else result = result + '!'; index++; } return result; } module.exports = { loopNarrow };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { loopNarrow } from 'jit-dynamic-loop'; function show(value: number | boolean | string): void { if (typeof value === 'number') console.log(value); else if (typeof value === 'boolean') console.log(value); else console.log(value.toUpperCase()); } function main(): void { show(loopNarrow(0, 4)); show(loopNarrow(1, 4)); show(loopNarrow(2, 4)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "35\nfalse\nTHREE!\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_primitive_numeric_operators_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-dynamic-operators-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-dynamic-operators");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numericOps(value: number | boolean | string): number;\nexport declare function bitOps(value: number | boolean | string): number;\nexport declare function identity(value: number | boolean | string): number | boolean | string;\nexport declare function text(value: number | boolean | string): string;\nexport declare function upper(value: number | boolean | string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function numericOps(value) { const n = +value; return ((n * 3 - 2) / 2) % 7 + n ** 2; } function bitOps(value) { const n = +value; return ((n << 2) | 1) ^ ((n >> 1) & 3) ^ (n >>> 1) ^ ~n; } function identity(value) { return value.valueOf(); } function text(value) { return value.toString(); } function upper(value) { return value.toUpperCase(); } module.exports = { numericOps, bitOps, identity, text, upper };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numericOps, bitOps, identity, text, upper } from 'jit-dynamic-operators'; function show(value: number | boolean | string): void { if (typeof value === 'number') console.log(value); else if (typeof value === 'boolean') console.log(value); else console.log(value.toUpperCase()); } function main(): void { console.log(numericOps('4')); console.log(numericOps(true)); console.log(bitOps('4')); console.log(bitOps(true)); show(identity(5)); show(identity(false)); show(identity('same')); console.log(text(5)); console.log(text(false)); console.log(text('same')); console.log(upper('mixed')); try { upper(1); } catch { console.log('type-error'); } }\n",
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
        "21\n1.5\n-22\n-5\n5\nfalse\nSAME\n5\nfalse\nsame\nMIXED\ntype-error\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn aggregate_unions_round_trip_through_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-aggregate-unions-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-aggregate-unions");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function arrayIdentity(value: number[] | string): number[] | string;\nexport declare function arrayKind(value: number[] | string): string;\nexport declare function fixedArray(): number[] | string;\nexport declare function recordIdentity(value: Record<string, number> | string): Record<string, number> | string;\nexport declare function recordKind(value: Record<string, number> | string): string;\nexport declare function fixedRecord(): Record<string, number> | string;\nexport declare function text(value: number[] | string[] | Record<string, number> | string): string;\nexport declare function chooseArray(flag: boolean): number[] | string[];\nexport declare function arrayText(value: number[] | string[]): string;\nexport declare function chooseAggregate(mode: number): string;\nexport declare function chooseLocal(flag: boolean): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function arrayIdentity(value) { return value; } function arrayKind(value) { return typeof value; } function fixedArray() { return [4, 5]; } function recordIdentity(value) { return value.valueOf(); } function recordKind(value) { return typeof value; } function fixedRecord() { return { count: 6 }; } function text(value) { return String(value); } function chooseArray(flag) { return flag ? [1, 2] : ['a', 'b']; } function arrayText(value) { return String(value); } function chooseAggregate(mode) { return String(mode === 0 ? [1] : mode === 1 ? { count: 2 } : 'x'); } function chooseLocal(flag) { let result = flag ? [1, 2] : 'start'; if (!flag) result = { count: 3 }; return String(result); } module.exports = { arrayIdentity, arrayKind, fixedArray, recordIdentity, recordKind, fixedRecord, text, chooseArray, arrayText, chooseAggregate, chooseLocal };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { arrayIdentity, arrayKind, fixedArray, recordIdentity, recordKind, fixedRecord, text, chooseArray, arrayText, chooseAggregate, chooseLocal } from 'jit-aggregate-unions'; function showArray(value: number[] | string): void { if (typeof value === 'string') console.log(value.toUpperCase()); else console.log(value.length); } function showRecord(value: Record<string, number> | string): void { if (typeof value === 'string') console.log(value.toUpperCase()); else console.log(value.count); } function main(): void { const record: Record<string, number> = { count: 42 }; showArray(arrayIdentity([1, 2, 3])); showArray(arrayIdentity('array')); console.log(arrayKind([1])); console.log(arrayKind('x')); showArray(fixedArray()); showRecord(recordIdentity(record)); showRecord(recordIdentity('record')); console.log(recordKind(record)); console.log(recordKind('x')); showRecord(fixedRecord()); console.log(text([1, 2])); console.log(text(record)); console.log(text('kept')); console.log(arrayText(chooseArray(true))); console.log(arrayText(chooseArray(false))); console.log(chooseAggregate(0)); console.log(chooseAggregate(1)); console.log(chooseAggregate(2)); console.log(chooseLocal(true)); console.log(chooseLocal(false)); }\n",
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
        "3\nARRAY\nobject\nstring\n2\n42\nRECORD\nobject\nstring\n6\n1,2\n[object Object]\nkept\n1,2\na,b\n1\n[object Object]\nx\n1,2\n[object Object]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_predicate_narrows_dynamic_union_in_jit() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-narrowing-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-narrowing");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function size(value: number[] | string): number;\nexport declare function first(value: number[] | string): string;\nexport declare function inverted(value: number[] | string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function size(value) { if (Array.isArray(value)) return value.length; return value.length; } function first(value) { if (Array.isArray(value)) return String(value[0]); return value.toUpperCase(); } function inverted(value) { if (!Array.isArray(value)) return value.length; return value.length; } module.exports = { size, first, inverted };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { size, first, inverted } from 'jit-array-narrowing'; function main(): void { console.log(size([1, 2, 3])); console.log(size('word')); console.log(first([7, 8])); console.log(first('word')); console.log(inverted([1, 2])); console.log(inverted('word')); }\n",
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
        "3\n4\n7\nWORD\n2\n4\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn typeof_object_narrows_dynamic_dictionary_union_in_jit() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-dictionary-narrowing-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-dictionary-narrowing");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numberValue(value: Record<string, number> | string): number;\nexport declare function boolValue(value: Record<string, boolean> | string): boolean;\nexport declare function stringValue(value: Record<string, string> | string): string;\nexport declare function inverted(value: Record<string, number> | string): number;\nexport declare function keyCount(value: Record<string, number> | Record<string, boolean> | Record<string, string> | string): number;\nexport declare function hasTarget(value: Record<string, number> | Record<string, boolean> | Record<string, string> | string): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function numberValue(value) { if (typeof value === 'object') return value.count; return value.length; } function boolValue(value) { if (typeof value === 'object') return value.ready; return value.length > 0; } function stringValue(value) { if (typeof value === 'object') return value.name; return value.toUpperCase(); } function inverted(value) { if (typeof value !== 'object') return value.length; return value.count; } function keyCount(value) { if (typeof value === 'object') return Object.keys(value).length; return value.length; } function hasTarget(value) { if (typeof value === 'object') return Object.hasOwn(value, 'target'); return false; } module.exports = { numberValue, boolValue, stringValue, inverted, keyCount, hasTarget };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numberValue, boolValue, stringValue, inverted, keyCount, hasTarget } from 'jit-dictionary-narrowing'; function main(): void { const numbers: Record<string, number> = { count: 42, target: 1 }; const booleans: Record<string, boolean> = { ready: true }; const strings: Record<string, string> = { name: 'thaw', target: 'yes' }; console.log(numberValue(numbers)); console.log(numberValue('word')); console.log(boolValue(booleans)); console.log(boolValue('')); console.log(stringValue(strings)); console.log(stringValue('word')); console.log(inverted(numbers)); console.log(inverted('word')); console.log(keyCount(numbers)); console.log(keyCount(booleans)); console.log(keyCount(strings)); console.log(keyCount('word')); console.log(hasTarget(numbers)); console.log(hasTarget(booleans)); console.log(hasTarget(strings)); console.log(hasTarget('word')); }\n",
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
        "42\n4\ntrue\nfalse\nthaw\nWORD\n42\n4\n2\n1\n2\n4\ntrue\nfalse\ntrue\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn aggregate_union_narrows_array_then_dictionary_in_jit() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-staged-aggregate-narrowing-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-staged-aggregate-narrowing");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function describe(value: number[] | Record<string, number> | string): string;\nexport declare function multiSize(value: number[] | string[] | boolean[] | string): number;\nexport declare function multiJoin(value: number[] | string[] | boolean[] | string): string;\nexport declare function multiText(value: number[] | string[] | boolean[] | string): string;\nexport declare function multiSlice(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiConcat(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiReversed(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiReverse(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiSorted(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiSort(value: number[] | string[] | boolean[] | string): number[] | string[] | boolean[] | string;\nexport declare function multiLast(value: number[] | string[] | boolean[] | string): number | string | boolean;\nexport declare function multiIncludes(value: number[] | string[] | boolean[] | string, needle: number | string | boolean): boolean;\nexport declare function multiIndex(value: number[] | string[] | boolean[] | string, needle: number | string | boolean): number;\nexport declare function multiLastIndex(value: number[] | string[] | boolean[] | string, needle: number | string | boolean): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function describe(value) { if (Array.isArray(value)) return String(value.length); if (typeof value === 'object') return String(value.count); return value.toUpperCase(); } function multiSize(value) { if (Array.isArray(value)) return value.length; return value.length; } function multiJoin(value) { if (Array.isArray(value)) return value.join('|'); return value.toUpperCase(); } function multiText(value) { if (Array.isArray(value)) return value.toString(); return value.toUpperCase(); } function multiSlice(value) { if (Array.isArray(value)) return value.slice(1); return value.slice(1); } function multiConcat(value) { if (Array.isArray(value)) return value.concat(value); return value.concat(value); } function multiReversed(value) { if (Array.isArray(value)) return value.toReversed(); return value; } function multiReverse(value) { if (Array.isArray(value)) return value.reverse(); return value; } function multiSorted(value) { if (Array.isArray(value)) return value.toSorted(); return value; } function multiSort(value) { if (Array.isArray(value)) return value.sort(); return value; } function multiLast(value) { if (Array.isArray(value)) return value.at(-1); return value; } function multiIncludes(value, needle) { if (Array.isArray(value)) return value.includes(needle); return false; } function multiIndex(value, needle) { if (Array.isArray(value)) return value.indexOf(needle); return -1; } function multiLastIndex(value, needle) { if (Array.isArray(value)) return value.lastIndexOf(needle); return -1; } module.exports = { describe, multiSize, multiJoin, multiText, multiSlice, multiConcat, multiReversed, multiReverse, multiSorted, multiSort, multiLast, multiIncludes, multiIndex, multiLastIndex };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { describe, multiSize, multiJoin, multiText, multiSlice, multiConcat, multiReversed, multiReverse, multiLast, multiIncludes, multiIndex, multiLastIndex } from 'jit-staged-aggregate-narrowing'; function main(): void { const record: Record<string, number> = { count: 42 }; console.log(describe([1, 2, 3])); console.log(describe(record)); console.log(describe('word')); console.log(multiSize([1, 2])); console.log(multiSize(['a', 'b', 'c'])); console.log(multiSize([true])); console.log(multiSize('word')); console.log(multiJoin([1, 2])); console.log(multiJoin(['a', 'b'])); console.log(multiJoin([true, false])); console.log(multiJoin('word')); console.log(multiText([1, 2])); console.log(multiText(['a', 'b'])); console.log(multiText([true, false])); console.log(multiSize(multiSlice([1, 2, 3]))); console.log(multiSize(multiSlice(['a', 'b', 'c']))); console.log(multiSize(multiSlice([true, false]))); console.log(multiSize(multiSlice('word'))); console.log(multiSize(multiConcat([1, 2, 3]))); console.log(multiSize(multiConcat(['a', 'b', 'c']))); console.log(multiSize(multiConcat([true, false]))); console.log(multiSize(multiConcat('word'))); console.log(multiJoin(multiReversed([1, 2, 3]))); console.log(multiJoin(multiReversed(['a', 'b', 'c']))); console.log(multiJoin(multiReversed([true, false]))); console.log(multiJoin(multiReverse([1, 2, 3]))); console.log(multiJoin(multiReverse(['a', 'b', 'c']))); console.log(multiJoin(multiReverse([true, false]))); console.log(multiLast([1, 2, 3])); console.log(multiLast(['a', 'b', 'c'])); console.log(multiLast([true, false])); console.log(multiLast('word')); console.log(multiIncludes([1, 2, 3], 2)); console.log(multiIncludes(['a', 'b'], 'b')); console.log(multiIncludes([true, false], false)); console.log(multiIndex([1, 2, 3], 2)); console.log(multiIndex(['a', 'b'], 'b')); console.log(multiIndex([true, false], false)); console.log(multiLastIndex([1, 2, 1], 1)); console.log(multiLastIndex(['a', 'b', 'a'], 'a')); console.log(multiLastIndex([true, false, true], true)); }\n",
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
        "3\n42\nWORD\n2\n3\n1\n4\n1|2\na|b\ntrue|false\nWORD\n1,2\na,b\ntrue,false\n2\n2\n1\n3\n6\n6\n4\n8\n3|2|1\nc|b|a\nfalse|true\n3|2|1\nc|b|a\nfalse|true\n3\nc\nfalse\nword\ntrue\ntrue\ntrue\n1\n1\n1\n2\n2\n2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_default_sort_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-sort-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-sort");
    std::fs::create_dir_all(&package).unwrap();
    let union = "number[] | string[] | boolean[] | string";
    std::fs::write(
        package.join("package.d.ts"),
        format!(
            "export declare function sorted(value: {union}): {union};\nexport declare function sort(value: {union}): {union};\n"
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function sorted(value) { if (Array.isArray(value)) return value.toSorted(); return value; } function sort(value) { if (Array.isArray(value)) return value.sort(); return value; } module.exports = { sorted, sort };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { sorted, sort } from 'jit-mixed-array-sort'; function main(): void { console.log(sorted([10, 2, 1])); console.log(sorted(['z', 'a', 'b'])); console.log(sorted([true, false, true])); console.log(sort([10, 2, 1])); console.log(sort(['z', 'a', 'b'])); console.log(sort([true, false, true])); }\n",
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
        "[1,10,2]\n[\"a\",\"b\",\"z\"]\n[false,true,true]\n[1,10,2]\n[\"a\",\"b\",\"z\"]\n[false,true,true]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_range_updates_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-range-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-range");
    std::fs::create_dir_all(&package).unwrap();
    let union = "number[] | string[] | boolean[] | string";
    std::fs::write(
        package.join("package.d.ts"),
        format!(
            "export declare function fill(value: {union}, replacement: number | string | boolean): {union};\nexport declare function copy(value: {union}): {union};\nexport declare function withValue(value: {union}, replacement: number | string | boolean): {union};\nexport declare function insert(value: {union}, replacement: number | string | boolean): number;\nexport declare function prepend(value: {union}, replacement: number | string | boolean): number;\nexport declare function popValue(value: {union}): number | string | boolean;\nexport declare function shiftValue(value: {union}): number | string | boolean;\nexport declare function setValue(value: {union}, index: number, replacement: number | string | boolean): number | string | boolean;\nexport declare function splice(value: {union}, replacement: number | string | boolean): {union};\nexport declare function toSpliced(value: {union}, replacement: number | string | boolean): {union};\n"
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function fill(value, replacement) { if (Array.isArray(value)) return value.fill(replacement, 1); return value; } function copy(value) { if (Array.isArray(value)) return value.copyWithin(0, 1); return value; } function withValue(value, replacement) { if (Array.isArray(value)) return value.with(1, replacement); return value; } function insert(value, replacement) { if (Array.isArray(value)) return value.push(replacement); return 0; } function prepend(value, replacement) { if (Array.isArray(value)) return value.unshift(replacement); return 0; } function popValue(value) { if (Array.isArray(value)) return value.pop(); return value; } function shiftValue(value) { if (Array.isArray(value)) return value.shift(); return value; } function setValue(value, index, replacement) { if (Array.isArray(value)) return value[index] = replacement; return replacement; } function splice(value, replacement) { if (Array.isArray(value)) return value.splice(1, 1, replacement); return value; } function toSpliced(value, replacement) { if (Array.isArray(value)) return value.toSpliced(1, 1, replacement); return value; } module.exports = { fill, copy, withValue, insert, prepend, popValue, shiftValue, setValue, splice, toSpliced };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fill, copy, withValue, insert, prepend, popValue, shiftValue, setValue, splice, toSpliced } from 'jit-mixed-array-range'; function main(): void { console.log(fill([1, 2, 3], 9)); console.log(fill(['a', 'b', 'c'], 'x')); console.log(fill([true, false, false], true)); console.log(copy([1, 2, 3])); console.log(copy(['a', 'b', 'c'])); console.log(copy([true, false, false])); console.log(withValue([1, 2, 3], 9)); console.log(withValue(['a', 'b', 'c'], 'x')); console.log(withValue([true, false, false], true)); console.log(insert([1, 2], 9)); console.log(insert(['a', 'b'], 'x')); console.log(insert([true, false], true)); console.log(prepend([1, 2], 9)); console.log(prepend(['a', 'b'], 'x')); console.log(prepend([true, false], true)); console.log(popValue([1, 2])); console.log(popValue(['a', 'b'])); console.log(popValue([true, false])); console.log(shiftValue([1, 2])); console.log(shiftValue(['a', 'b'])); console.log(shiftValue([true, false])); console.log(setValue([1, 2], 1, 9)); console.log(setValue(['a', 'b'], 1, 'x')); console.log(setValue([true, false], 1, true)); console.log(splice([1, 2, 3], 9)); console.log(splice(['a', 'b', 'c'], 'x')); console.log(splice([true, false, false], true)); console.log(toSpliced([1, 2, 3], 9)); console.log(toSpliced(['a', 'b', 'c'], 'x')); console.log(toSpliced([true, false, false], true)); }\n",
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
        "[1,9,9]\n[\"a\",\"x\",\"x\"]\n[true,true,true]\n[2,3,3]\n[\"b\",\"c\",\"c\"]\n[false,false,false]\n[1,9,3]\n[\"a\",\"x\",\"c\"]\n[true,true,false]\n3\n3\n3\n3\n3\n3\n2\nb\nfalse\n1\na\ntrue\n9\nx\ntrue\n[2]\n[\"b\"]\n[false]\n[1,9,3]\n[\"a\",\"x\",\"c\"]\n[true,true,false]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_truthy_scans_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-truthy-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-truthy");
    std::fs::create_dir_all(&package).unwrap();
    let union = "number[] | string[] | boolean[] | string";
    std::fs::write(
        package.join("package.d.ts"),
        format!(
            "export declare function some(value: {union}): boolean;\nexport declare function every(value: {union}): boolean;\nexport declare function find(value: {union}): number | string | boolean;\nexport declare function findIndex(value: {union}): number;\nexport declare function findLast(value: {union}): number | string | boolean;\nexport declare function findLastIndex(value: {union}): number;\nexport declare function filter(value: {union}): {union};\nexport declare function computedSome(value: {union}): boolean;\nexport declare function computedEvery(value: {union}): boolean;\nexport declare function computedFind(value: {union}): number | string | boolean;\nexport declare function computedFindIndex(value: {union}): number;\nexport declare function computedFindLast(value: {union}): number | string | boolean;\nexport declare function computedFindLastIndex(value: {union}): number;\nexport declare function computedFilter(value: {union}, offset: number): {union};\n"
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function some(value) { if (Array.isArray(value)) return value.some(item => item); return false; } function every(value) { if (Array.isArray(value)) return value.every(item => item); return false; } function find(value) { if (Array.isArray(value)) return value.find(item => item); return value; } function findIndex(value) { if (Array.isArray(value)) return value.findIndex(item => item); return -1; } function findLast(value) { if (Array.isArray(value)) return value.findLast(item => item); return value; } function findLastIndex(value) { if (Array.isArray(value)) return value.findLastIndex(item => item); return -1; } function filter(value) { if (Array.isArray(value)) return value.filter(item => item); return value; } function computedSome(value) { if (Array.isArray(value)) return value.some((item, index, values) => Number(item) + index >= values.length); return false; } function computedEvery(value) { if (Array.isArray(value)) return value.every((item, index) => Number(item) + index > 0); return false; } function computedFind(value) { if (Array.isArray(value)) return value.find((item, index) => Number(item) + index >= 3); return value; } function computedFindIndex(value) { if (Array.isArray(value)) return value.findIndex((item, index) => Number(item) + index >= 3); return -1; } function computedFindLast(value) { if (Array.isArray(value)) return value.findLast((item, index) => Number(item) + index >= 3); return value; } function computedFindLastIndex(value) { if (Array.isArray(value)) return value.findLastIndex((item, index) => Number(item) + index >= 3); return -1; } function computedFilter(value, offset) { if (Array.isArray(value)) return value.filter((item, index, values) => Number(item) + index + offset >= values.length); return value; } module.exports = { some, every, find, findIndex, findLast, findLastIndex, filter, computedSome, computedEvery, computedFind, computedFindIndex, computedFindLast, computedFindLastIndex, computedFilter };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { some, every, find, findIndex, findLast, findLastIndex, filter, computedSome, computedEvery, computedFind, computedFindIndex, computedFindLast, computedFindLastIndex, computedFilter } from 'jit-mixed-array-truthy'; function main(): void { console.log(some([0, 0, 2])); console.log(some(['', 'x'])); console.log(some([false, false])); console.log(every([1, 2])); console.log(every(['x', ''])); console.log(every([true, true])); console.log(find([0, 2, 3])); console.log(find(['', 'x'])); console.log(find([false, true])); console.log(findIndex([0, 2, 3])); console.log(findLast([0, 2, 3])); console.log(findLastIndex(['', 'x', 'y'])); console.log(filter([0, 2, 3])); console.log(filter(['', 'x', 'y'])); console.log(filter([false, true])); console.log(computedSome([0, 1, 2])); console.log(computedSome(['0', '1'])); console.log(computedSome([false, true])); console.log(computedEvery([1, 1])); console.log(computedEvery([true, false])); console.log(computedFind(['1', '2', '0'])); console.log(computedFindIndex([1, 2, 0])); console.log(computedFindLast([1, 2, 3])); console.log(computedFindLastIndex([false, true, true])); console.log(computedFilter([0, 1, 2], 1)); console.log(computedFilter(['0', '1', '2'], 1)); console.log(computedFilter([false, true, false], 1)); }\n",
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
        "true\ntrue\nfalse\ntrue\nfalse\ntrue\n2\nx\ntrue\n1\n3\n2\n[2,3]\n[\"x\",\"y\"]\n[true]\ntrue\ntrue\ntrue\ntrue\ntrue\n2\n1\n3\n2\n[1,2]\n[\"1\",\"2\"]\n[true,false]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_comparison_scans_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-comparison-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-comparison");
    std::fs::create_dir_all(&package).unwrap();
    let arrays = "number[] | string[] | boolean[] | string";
    let primitive = "number | string | boolean";
    std::fs::write(
        package.join("package.d.ts"),
        format!(
            "export declare function some(value: {arrays}, needle: {primitive}): boolean;\nexport declare function looseSome(value: {arrays}, needle: {primitive}): boolean;\nexport declare function every(value: {arrays}, needle: {primitive}): boolean;\nexport declare function find(value: {arrays}, needle: {primitive}): {primitive};\nexport declare function findIndex(value: {arrays}, needle: {primitive}): number;\nexport declare function findLast(value: {arrays}, needle: {primitive}): {primitive};\nexport declare function findLastIndex(value: {arrays}, needle: {primitive}): number;\nexport declare function filter(value: {arrays}, needle: {primitive}): {arrays};\n"
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function some(value, needle) { if (Array.isArray(value)) return value.some(item => item === needle); return false; } function looseSome(value, needle) { if (Array.isArray(value)) return value.some(item => item == needle); return false; } function every(value, needle) { if (Array.isArray(value)) return value.every(item => item !== needle); return false; } function find(value, needle) { if (Array.isArray(value)) return value.find(item => item === needle); return value; } function findIndex(value, needle) { if (Array.isArray(value)) return value.findIndex(item => item >= needle); return -1; } function findLast(value, needle) { if (Array.isArray(value)) return value.findLast(item => item < needle); return value; } function findLastIndex(value, needle) { if (Array.isArray(value)) return value.findLastIndex(item => item === needle); return -1; } function filter(value, needle) { if (Array.isArray(value)) return value.filter(item => item !== needle); return value; } module.exports = { some, looseSome, every, find, findIndex, findLast, findLastIndex, filter };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { some, looseSome, every, find, findIndex, findLast, findLastIndex, filter } from 'jit-mixed-array-comparison'; function main(): void { console.log(some([1, 2], 2)); console.log(some(['a', 'b'], 'b')); console.log(some([false, true], true)); console.log(some([1, 2], '2')); console.log(looseSome([1, 2], '2')); console.log(every(['a', 'b'], 'x')); console.log(find(['a', 'b'], 'b')); console.log(findIndex([1, 3], 2)); console.log(findLast([1, 3, 2], 3)); console.log(findLastIndex([false, true, false], false)); console.log(filter(['a', 'b', 'a'], 'a')); }\n",
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
        "true\ntrue\ntrue\nfalse\ntrue\ntrue\nb\n1\n2\n2\n[\"b\"]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_array_conversion_maps_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-array-map-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-array-map");
    std::fs::create_dir_all(&package).unwrap();
    let union = "number[] | string[] | boolean[] | string";
    std::fs::write(
        package.join("package.d.ts"),
        format!(
            "export declare function numbers(value: {union}): number[];\nexport declare function booleans(value: {union}): boolean[];\nexport declare function strings(value: {union}): string[];\nexport declare function identity(value: {union}): {union};\nexport declare function adjust(value: {union}, offset: number): number[];\nexport declare function flags(value: {union}): boolean[];\nexport declare function labels(value: {union}): string[];\nexport declare function positions(value: {union}): number[];\nexport declare function fold(value: {union}, initial: number): number;\nexport declare function foldRight(value: {union}, initial: number): number;\nexport declare function foldCaptured(value: {union}, initial: number, factor: number): number;\nexport declare function foldFirst(value: {union}, enabled: boolean): number | string | boolean;\nexport declare function foldRightFirst(value: {union}, enabled: boolean): number | string | boolean;\n"
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function numbers(value) { if (Array.isArray(value)) return value.map(Number); return []; } function booleans(value) { if (Array.isArray(value)) return value.map(Boolean); return []; } function strings(value) { if (Array.isArray(value)) return value.map(String); return []; } function identity(value) { if (Array.isArray(value)) return value.map(item => item); return value; } function adjust(value, offset) { if (Array.isArray(value)) return value.map(item => Number(item) + offset); return []; } function flags(value) { if (Array.isArray(value)) return value.map(item => !!item); return []; } function labels(value) { if (Array.isArray(value)) return value.map((item, index) => String(item) + ':' + String(index)); return []; } function positions(value) { if (Array.isArray(value)) return value.map((item, index, values) => Number(item) + index + values.length); return []; } function fold(value, initial) { if (Array.isArray(value)) return value.reduce((accumulator, item, index, values) => accumulator * 2 + Number(item) + index + values.length, initial); return initial; } function foldRight(value, initial) { if (Array.isArray(value)) return value.reduceRight((accumulator, item, index, values) => accumulator * 2 + Number(item) + index + values.length, initial); return initial; } function foldCaptured(value, initial, factor) { if (Array.isArray(value)) return value.reduce((accumulator, item, index, values) => accumulator * factor + Number(item) + index + values.length, initial); return initial; } function foldFirst(value, enabled) { if (Array.isArray(value)) return value.reduce((accumulator, item) => enabled ? accumulator + item : String(accumulator) + String(item)); return value; } function foldRightFirst(value, enabled) { if (Array.isArray(value)) return value.reduceRight((accumulator, item) => enabled ? accumulator + item : String(accumulator) + String(item)); return value; } module.exports = { numbers, booleans, strings, identity, adjust, flags, labels, positions, fold, foldRight, foldCaptured, foldFirst, foldRightFirst };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numbers, booleans, strings, identity, adjust, flags, labels, positions, fold, foldRight, foldCaptured, foldFirst, foldRightFirst } from 'jit-mixed-array-map'; function main(): void { console.log(numbers([2, -1])); console.log(numbers(['2', ''])); console.log(numbers([true, false])); console.log(booleans([0, 2])); console.log(booleans(['', 'x'])); console.log(booleans([false, true])); console.log(strings([2, -1])); console.log(strings(['a', ''])); console.log(strings([true, false])); console.log(identity([2, -1])); console.log(identity(['a', ''])); console.log(identity([true, false])); console.log(adjust([1, 2], 10)); console.log(adjust(['2', '3'], 1)); console.log(adjust([true, false], 1)); console.log(flags([0, 2])); console.log(flags(['', 'x'])); console.log(flags([false, true])); console.log(labels([2, 3])); console.log(positions([1, 2])); console.log(positions(['2', '3'])); console.log(fold([1, 2], 0)); console.log(fold(['1', '2'], 0)); console.log(fold([true, false], 0)); console.log(foldRight([1, 2], 0)); console.log(foldRight(['1', '2'], 0)); console.log(foldRight([true, false], 0)); console.log(foldCaptured([1, 2], 1, 3)); console.log(fold([], 7)); console.log(foldFirst([1, 2, 3], true)); console.log(foldFirst(['a', 'b', 'c'], true)); console.log(foldFirst([true, false, true], true)); console.log(foldRightFirst(['a', 'b', 'c'], true)); console.log(foldFirst([1, 2], false)); try { foldFirst([], true); } catch { console.log('empty'); } }\n",
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
        "[2,-1]\n[2,0]\n[1,0]\n[false,true]\n[false,true]\n[false,true]\n[\"2\",\"-1\"]\n[\"a\",\"\"]\n[\"true\",\"false\"]\n[2,-1]\n[\"a\",\"\"]\n[true,false]\n[11,12]\n[3,4]\n[2,1]\n[false,true]\n[false,true]\n[false,true]\n[\"2:0\",\"3:1\"]\n[3,5]\n[4,6]\n11\n11\n9\n13\n13\n9\n23\n7\n6\nabc\n2\ncba\n12\nempty\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_number_sources_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-random-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-random");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function random(): number;\nexport declare function dateNow(): number;\nexport declare function performanceNow(): number;\nexport declare function processUptime(): number;\nexport declare function processPid(): number;\nexport declare function processPpid(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.random = () => 1 + Math.random(); module.exports.dateNow = () => Date.now(); module.exports.performanceNow = () => performance.now(); module.exports.processUptime = () => process.uptime(); module.exports.processPid = () => process.pid; module.exports.processPpid = () => process.ppid;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { random, dateNow, performanceNow, processUptime, processPid, processPpid } from 'jit-random';\nfunction main(): void { const value = random(); const first = performanceNow(); const second = performanceNow(); console.log(value >= 1 && value < 2); console.log(dateNow() > 0); console.log(first >= 0 && second >= first); console.log(processUptime() >= 0); console.log(processPid() > 0); console.log(processPpid() > 0); }\n",
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
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn numeric_predicates_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-predicates-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-predicates");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function safe(value: number): boolean;\nexport declare function minimum(values: number[]): number;\nexport declare function maximum(values: number[]): number;\nexport declare function minimumMixed(left: number, values: number[], tail: number[], right: number): number;\nexport declare function maximumMixed(left: number, values: number[], tail: number[], right: number): number;\nexport declare function distance(values: number[]): number;\nexport declare function constants(): boolean;\nexport declare function globals(): boolean;\nexport declare function stringNan(value: string): boolean;\nexport declare function float(value: string): number;\nexport declare function integer(value: string, radix: number): number;\nexport declare function fixed(value: number, digits: number): string;\nexport declare function precision(value: number, digits: number): string;\nexport declare function radix(value: number, base: number): string;\nexport declare function exponential(value: number, digits: number): string;\nexport declare function exponentialShortest(value: number): string;\nexport declare function isList(values: number[]): boolean;\nexport declare function isValue(value: number): boolean;\nexport declare function sameNumber(left: number, right: number): boolean;\nexport declare function sameText(left: string, right: string): boolean;\nexport declare function count(values: number[]): number;\nexport declare function pick(values: number[], index: number): number | undefined;\nexport declare function pickText(values: string[], index: number): string | undefined;\nexport declare function pickBool(values: boolean[], index: number): boolean | undefined;\nexport declare function getNumber(values: number[], index: number): number | undefined;\nexport declare function getText(values: string[], index: number): string | undefined;\nexport declare function getBool(values: boolean[], index: number): boolean | undefined;\nexport declare function hasNumber(values: number[], needle: number, from: number): boolean;\nexport declare function hasText(values: string[], needle: string, from: number): boolean;\nexport declare function boolIndex(values: boolean[], needle: boolean, from: number): number;\nexport declare function lastNumber(values: number[], needle: number): number;\nexport declare function lastText(values: string[], needle: string, from: number): number;\nexport declare function lastBool(values: boolean[], needle: boolean, from: number): number;\nexport declare function formatNumbers(values: number[], separator: string): string;\nexport declare function formatText(values: string[]): string;\nexport declare function formatBools(values: boolean[], separator: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { safe: value => Number.isSafeInteger(value), minimum: values => Math.min(...values), maximum: values => Math.max(...values), minimumMixed: (left, values, tail, right) => Math.min(left, ...values, ...tail, right), maximumMixed: (left, values, tail, right) => Math.max(left, ...values, ...tail, right), distance: values => Math.hypot(...values), constants: () => Number.MIN_VALUE > 0, globals: () => Number.isNaN(NaN) && !isFinite(Infinity), stringNan: value => Number.isNaN(value) || isNaN(value), float: value => Number.parseFloat(value), integer: (value, radix) => parseInt(value, radix), fixed: (value, digits) => value.toFixed(digits), precision: (value, digits) => value.toPrecision(digits), radix: (value, base) => value.toString(base), exponential: (value, digits) => value.toExponential(digits), exponentialShortest: value => value.toExponential(), isList: values => Array.isArray(values), isValue: value => Array.isArray(value), sameNumber: (left, right) => Object.is(left, right), sameText: (left, right) => Object.is(left, right), count: values => values.length, pick: (values, index) => values.at(index), pickText: (values, index) => values.at(index), pickBool: (values, index) => values.at(index), getNumber: (values, index) => values[index], getText: (values, index) => values[index], getBool: (values, index) => values[index], hasNumber: (values, needle, from) => values.includes(needle, from), hasText: (values, needle, from) => values.includes(needle, from), boolIndex: (values, needle, from) => values.indexOf(needle, from), lastNumber: (values, needle) => values.lastIndexOf(needle), lastText: (values, needle, from) => values.lastIndexOf(needle, from), lastBool: (values, needle, from) => values.lastIndexOf(needle, from), formatNumbers: (values, separator) => values.join(separator), formatText: values => values.toString(), formatBools: (values, separator) => values.join(separator) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { safe, minimum, maximum, minimumMixed, maximumMixed, distance, constants, globals, stringNan, float, integer, fixed, precision, radix, exponential, exponentialShortest, isList, isValue, sameNumber, sameText, count, pick, pickText, pickBool, getNumber, getText, getBool, hasNumber, hasText, boolIndex, lastNumber, lastText, lastBool, formatNumbers, formatText, formatBools } from 'jit-predicates';\nfunction main(): void { console.log(safe(42)); console.log(minimum([3, 1, 2])); console.log(maximum([3, 1, 2])); console.log(minimum([]) === Infinity); console.log(maximum([]) === -Infinity); console.log(Number.isNaN(minimum([1, NaN]))); console.log(Object.is(minimum([-0, 0]), -0)); console.log(minimumMixed(5, [3, 1], [], 2)); console.log(maximumMixed(5, [3, 1], [9], 2)); console.log(Number.isNaN(minimumMixed(0, [NaN], [], 1))); console.log(Object.is(minimumMixed(0, [-0], [], 0), -0)); console.log(distance([3, 4])); console.log(distance([])); const large = distance([3e200, 4e200]); console.log(large > 4.9e200 && large < 5.1e200); console.log(distance([NaN, Infinity]) === Infinity); console.log(constants()); console.log(globals()); console.log(stringNan('x')); console.log(float('  -12.5px')); console.log(integer('11', 2)); console.log(fixed(12.5, 2)); console.log(precision(12.5, 3)); console.log(radix(255, 16)); console.log(exponential(12.6, 1)); console.log(exponentialShortest(12.5)); console.log(isList([1])); console.log(isValue(1)); console.log(sameNumber(NaN, NaN)); console.log(sameNumber(0, -0)); console.log(sameText('same', 'same')); console.log(count([1, 2, 3])); console.log(pick([1, 2, 3], -1) ?? 0); console.log(pickText(['a', 'b'], -1) ?? 'none'); console.log(pickBool([false, true], -1) ?? false); console.log(getNumber([4, 5], 1) ?? 0); console.log(getNumber([4, 5], -1) ?? 9); console.log(getText(['x', 'y'], 0) ?? 'none'); console.log(getBool([false, true], 1) ?? false); console.log(hasNumber([NaN], NaN, 0)); console.log(hasText(['a', 'b'], 'b', 0)); console.log(boolIndex([false, true], true, 0)); console.log(lastNumber([1, 2, 1], 1)); console.log(lastText(['a', 'b', 'a'], 'a', 1)); console.log(lastBool([true, false, true], true, 1)); console.log(formatNumbers([1, 2], '|')); console.log(formatText(['a', 'b'])); console.log(formatBools([true, false], '|')); try { fixed(1, 101); } catch { console.log('range'); } try { exponential(1, 101); } catch { console.log('exp-range'); } }\n",
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
        "true\n1\n3\ntrue\ntrue\ntrue\ntrue\n1\n9\ntrue\ntrue\n5\n0\ntrue\ntrue\ntrue\ntrue\ntrue\n-12.5\n3\n12.50\n12.5\nff\n1.3e+1\n1.25e+1\ntrue\nfalse\ntrue\nfalse\ntrue\n3\n3\nb\ntrue\n5\n9\nx\ntrue\ntrue\ntrue\n1\n2\n0\n0\n1|2\na,b\ntrue|false\nrange\nexp-range\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_hypot_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-mixed-hypot-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-mixed-hypot");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function distance(left: number, values: number[], tail: number[], right: number): number;\nexport declare function magnitude(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { distance: (left, values, tail, right) => Math.hypot(left, ...values, ...tail, right), magnitude: value => Math.hypot(value) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { distance, magnitude } from 'jit-mixed-hypot';\nfunction main(): void { console.log(distance(2, [3], [6], 0)); console.log(distance(0, [], [], 0)); console.log(distance(NaN, [], [Infinity], 0)); console.log(magnitude(-3)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "7\n0\nInfinity\n3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn arithmetic_array_callbacks_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-reduce-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-reduce");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function sum(values: number[], initial: number): number;\nexport declare function namedSum(values: number[]): number;\nexport declare function sumFirst(values: number[]): number;\nexport declare function subtractFirst(values: number[]): number;\nexport declare function subtractRightFirst(values: number[]): number;\nexport declare function powerRightFirst(values: number[]): number;\nexport declare function minimum(values: number[]): number;\nexport declare function maximumRight(values: number[]): number;\nexport declare function subtract(values: number[], initial: number): number;\nexport declare function multiply(values: number[], initial: number): number;\nexport declare function divide(values: number[], initial: number): number;\nexport declare function remainder(values: number[], initial: number): number;\nexport declare function power(values: number[], initial: number): number;\nexport declare function someAbove(values: number[], threshold: number): boolean;\nexport declare function allAtLeast(values: number[], threshold: number): boolean;\nexport declare function hasPositive(values: number[]): boolean;\nexport declare function anyDifferent(values: number[], expected: number): boolean;\nexport declare function firstAbove(values: number[], threshold: number): number | undefined;\nexport declare function lastAbove(values: number[], threshold: number): number | undefined;\nexport declare function firstIndex(values: number[], expected: number): number;\nexport declare function lastIndex(values: number[], expected: number): number;\nexport declare function selectAtLeast(values: number[], threshold: number): number[];\nexport declare function scale(values: number[], factor: number): number[];\nexport declare function subtractFrom(values: number[], upper: number): number[];\nexport declare function negate(values: number[]): number[];\nexport declare function magnitudes(values: number[]): number[];\nexport declare function roots(values: number[]): number[];\nexport declare function compact(values: number[]): number[];\nexport declare function firstTruthy(values: number[]): number | undefined;\nexport declare function compactStrings(values: string[]): string[];\nexport declare function compactFlags(values: boolean[]): boolean[];\nexport declare function addIndexes(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "const addValues = (accumulator, value) => { const total = accumulator + value; return total; }; const positive = value => value > 0; const absolute = value => Math.abs(value); module.exports = { sum: (values, initial) => values.reduce(function(accumulator, value) { return accumulator + value; }, initial), namedSum: values => values.reduce(addValues, 0), sumFirst: values => values.reduce((accumulator, value) => accumulator + value), subtractFirst: values => values.reduce((accumulator, value) => accumulator - value), subtractRightFirst: values => values.reduceRight((accumulator, value) => accumulator - value), powerRightFirst: values => values.reduceRight((accumulator, value) => accumulator ** value), minimum: values => values.reduce((accumulator, value) => Math.min(accumulator, value)), maximumRight: values => values.reduceRight((accumulator, value) => Math.max(accumulator, value)), subtract: (values, initial) => values.reduce((accumulator, value) => accumulator - value, initial), multiply: (values, initial) => values.reduce((accumulator, value) => accumulator * value, initial), divide: (values, initial) => values.reduce((accumulator, value) => accumulator / value, initial), remainder: (values, initial) => values.reduce((accumulator, value) => accumulator % value, initial), power: (values, initial) => values.reduce((accumulator, value) => accumulator ** value, initial), someAbove: (values, threshold) => values.some(value => value > threshold), allAtLeast: (values, threshold) => values.every(value => threshold <= value), hasPositive: values => values.some(positive), anyDifferent: (values, expected) => values.some(value => value !== expected), firstAbove: (values, threshold) => values.find(value => value > threshold), lastAbove: (values, threshold) => values.findLast(value => value > threshold), firstIndex: (values, expected) => values.findIndex(value => value === expected), lastIndex: (values, expected) => values.findLastIndex(value => value === expected), selectAtLeast: (values, threshold) => values.filter(value => value >= threshold), scale: (values, factor) => values.map(value => value * factor), subtractFrom: (values, upper) => values.map(value => upper - value), negate: values => values.map(value => -value), magnitudes: values => values.map(absolute), roots: values => values.map(value => Math.sqrt(value)), compact: values => values.filter(Boolean), firstTruthy: values => values.find(value => value), compactStrings: values => values.filter(Boolean), compactFlags: values => values.filter(Boolean), addIndexes: values => values.map((value, index) => value + index) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { sum, namedSum, sumFirst, subtractFirst, subtractRightFirst, powerRightFirst, minimum, maximumRight, subtract, multiply, divide, remainder, power, someAbove, allAtLeast, hasPositive, anyDifferent, firstAbove, lastAbove, firstIndex, lastIndex, selectAtLeast, scale, subtractFrom, negate, magnitudes, roots, compact, firstTruthy, compactStrings, compactFlags, addIndexes } from 'jit-array-reduce';\nfunction main(): void { console.log(sum([1e16, -1e16, 1], 1)); console.log(Object.is(sum([], -0), -0)); console.log(sum([10, 20, 12], 0)); console.log(namedSum([10, 20, 12])); console.log(subtract([1, 2, 3], 10)); console.log(multiply([2, 3, 4], 1)); console.log(divide([2, 5], 100)); console.log(remainder([6, 4], 20)); console.log(power([3, 2], 2)); console.log(Object.is(power([3], -0), -0)); console.log(sumFirst([10, 20, 12])); console.log(subtractFirst([10, 2, 3])); console.log(subtractRightFirst([10, 2, 3])); console.log(powerRightFirst([2, 3])); console.log(Object.is(minimum([-0, 0]), -0)); console.log(Object.is(maximumRight([-0, 0]), 0)); console.log(Number.isNaN(minimum([1, NaN]))); console.log(someAbove([1, 4], 3)); console.log(allAtLeast([3, 4], 3)); console.log(hasPositive([-2, 0, 3])); console.log(someAbove([], 0)); console.log(allAtLeast([], 0)); console.log(anyDifferent([NaN], NaN)); console.log(firstAbove([1, 4, 5], 3) ?? -1); console.log(lastAbove([1, 4, 5], 3) ?? -1); console.log(firstIndex([1, 4, 5], 4)); console.log(lastIndex([4, 1, 4], 4)); console.log(firstAbove([], 0) === undefined); console.log(selectAtLeast([1, 4, 5], 4).join(',')); console.log(selectAtLeast([], 0).length); console.log(scale([1, 2, 3], 4).join(',')); console.log(subtractFrom([1, 2, 3], 10).join(',')); console.log(negate([1, -2, 3]).join(',')); console.log(magnitudes([-1, -2, 3]).join(',')); console.log(roots([1, 4, 9]).join(',')); console.log(compact([0, NaN, -2, 3]).join(',')); console.log(firstTruthy([0, NaN, -2]) ?? 99); console.log(compactStrings(['', 'x', 'y']).join(',')); console.log(compactFlags([false, true, false]).join(',')); console.log(addIndexes([10, 20, 30]).join(',')); try { sumFirst([]); } catch { console.log('empty'); } try { subtractRightFirst([]); } catch { console.log('right-empty'); } }\n",
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
        "1\ntrue\n42\n42\n4\n24\n10\n2\n64\ntrue\n42\n5\n-9\n9\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\n4\n5\n1\n2\ntrue\n4,5\n0\n4,8,12\n9,8,7\n-1,2,-3\n1,2,3\n1,2,3\n-2,3\n-2\nx,y\ntrue\n10,21,32\nempty\nright-empty\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn local_map_callbacks_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-local-map-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-local-map");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function scale(values: number[], factor: number): number[];\nexport declare function clamp(values: number[], minimum: number): number[];\nexport declare function distance(values: number[], pivot: number): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.scale = (values, factor) => values.map(value => { const scaled = value * factor; return scaled; });\nmodule.exports.clamp = (values, minimum) => values.map(value => value >= minimum ? value : minimum);\nmodule.exports.distance = (values, pivot) => values.map(value => value >= pivot ? value - pivot : pivot - value);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { scale, clamp, distance } from 'jit-local-map';\nfunction main(): void { console.log(scale([1, 2, 3], 4).join(',')); console.log(clamp([1, 4, 2], 3).join(',')); console.log(distance([1, 4, 3], 3).join(',')); }\n",
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
        "4,8,12\n3,4,3\n2,1,0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn composed_numeric_maps_use_jit_callbacks_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-composed-map-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-composed-map");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function polynomial(values: number[]): number[];\nexport declare function indexed(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function transform(value, index) { const sum = value + index; return sum * (value - index); } module.exports.polynomial = values => values.map(value => value * value + 1); module.exports.indexed = values => values.map(transform);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { polynomial, indexed } from 'jit-composed-map';\nfunction main(): void { console.log(polynomial([2, 3]).join(',')); console.log(indexed([2, 3, 4]).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5,10\n4,8,12\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn composed_numeric_predicates_use_jit_callbacks_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-composed-predicate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-composed-predicate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function some(values: number[]): boolean;\nexport declare function every(values: number[]): boolean;\nexport declare function find(values: number[]): number | undefined;\nexport declare function findIndex(values: number[]): number;\nexport declare function findLast(values: number[]): number | undefined;\nexport declare function findLastIndex(values: number[]): number;\nexport declare function filter(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "const predicate = (value, index) => value * value > index + 3; module.exports.some = values => values.some(predicate); module.exports.every = values => values.every(predicate); module.exports.find = values => values.find(predicate); module.exports.findIndex = values => values.findIndex(predicate); module.exports.findLast = values => values.findLast(predicate); module.exports.findLastIndex = values => values.findLastIndex(predicate); module.exports.filter = values => values.filter(predicate);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { some, every, find, findIndex, findLast, findLastIndex, filter } from 'jit-composed-predicate';\nfunction main(): void { const values = [1, 2, 3, 4]; console.log(some(values)); console.log(every(values)); console.log(find(values) ?? -1); console.log(findIndex(values)); console.log(findLast(values) ?? -1); console.log(findLastIndex(values)); console.log(filter(values).join(',')); }\n",
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
        "true\nfalse\n3\n2\n4\n3\n3,4\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn composed_primitive_predicates_use_jit_callbacks_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-composed-primitive-predicate-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-composed-primitive-predicate");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function stringSome(values: string[], minimum: number): boolean;\nexport declare function stringEvery(values: string[], minimum: number): boolean;\nexport declare function stringFind(values: string[], minimum: number): string | undefined;\nexport declare function stringFindIndex(values: string[], minimum: number): number;\nexport declare function stringFindLast(values: string[], minimum: number): string | undefined;\nexport declare function stringFindLastIndex(values: string[], minimum: number): number;\nexport declare function stringFilter(values: string[], minimum: number): string[];\nexport declare function boolSome(values: boolean[], start: number): boolean;\nexport declare function boolEvery(values: boolean[], start: number): boolean;\nexport declare function boolFind(values: boolean[], start: number): boolean | undefined;\nexport declare function boolFindIndex(values: boolean[], start: number): number;\nexport declare function boolFindLast(values: boolean[], start: number): boolean | undefined;\nexport declare function boolFindLastIndex(values: boolean[], start: number): number;\nexport declare function boolFilter(values: boolean[], start: number): boolean[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.stringSome = (values, minimum) => values.some((value, index, source) => value.length > index + minimum && source.length === 4); module.exports.stringEvery = (values, minimum) => values.every((value, index) => value.length > index + minimum); module.exports.stringFind = (values, minimum) => values.find((value, index) => value.length > index + minimum); module.exports.stringFindIndex = (values, minimum) => values.findIndex((value, index) => value.length > index + minimum); module.exports.stringFindLast = (values, minimum) => values.findLast((value, index) => value.length > index + minimum); module.exports.stringFindLastIndex = (values, minimum) => values.findLastIndex((value, index) => value.length > index + minimum); module.exports.stringFilter = (values, minimum) => values.filter((value, index) => value.length > index + minimum); module.exports.boolSome = (values, start) => values.some((value, index, source) => value && index >= start && source.length === 4); module.exports.boolEvery = (values, start) => values.every((value, index) => value && index >= start); module.exports.boolFind = (values, start) => values.find((value, index) => value && index >= start); module.exports.boolFindIndex = (values, start) => values.findIndex((value, index) => value && index >= start); module.exports.boolFindLast = (values, start) => values.findLast((value, index) => value && index >= start); module.exports.boolFindLastIndex = (values, start) => values.findLastIndex((value, index) => value && index >= start); module.exports.boolFilter = (values, start) => values.filter((value, index) => value && index >= start);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import * as p from 'jit-composed-primitive-predicate';\nfunction main(): void { const words = ['a', 'bbb', 'cc', 'dddd']; console.log(p.stringSome(words, 0)); console.log(p.stringEvery(words, 0)); console.log(p.stringFind(words, 0) ?? '-'); console.log(p.stringFindIndex(words, 0)); console.log(p.stringFindLast(words, 0) ?? '-'); console.log(p.stringFindLastIndex(words, 0)); console.log(p.stringFilter(words, 0).join(',')); const flags = [true, false, true, true]; console.log(p.boolSome(flags, 1)); console.log(p.boolEvery(flags, 1)); console.log(p.boolFind(flags, 1) ?? false); console.log(p.boolFindIndex(flags, 1)); console.log(p.boolFindLast(flags, 1) ?? false); console.log(p.boolFindLastIndex(flags, 1)); console.log(p.boolFilter(flags, 1).join(',')); }\n",
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
        "true\nfalse\na\n0\ndddd\n3\na,bbb,dddd\ntrue\nfalse\ntrue\n2\ntrue\n3\ntrue,true\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn composed_primitive_maps_use_jit_callbacks_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-composed-primitive-map-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-composed-primitive-map");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function stringNumbers(values: string[], offset: number): number[];\nexport declare function stringBooleans(values: string[], minimum: number): boolean[];\nexport declare function stringStrings(values: string[], suffix: string): string[];\nexport declare function boolNumbers(values: boolean[], offset: number): number[];\nexport declare function boolBooleans(values: boolean[], start: number): boolean[];\nexport declare function boolStrings(values: boolean[], prefix: string): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.stringNumbers = (values, offset) => values.map((value, index, source) => value.length + index + offset + source.length); module.exports.stringBooleans = (values, minimum) => values.map((value, index) => value.length > index + minimum); module.exports.stringStrings = (values, suffix) => values.map((value, index) => value + suffix + index); module.exports.boolNumbers = (values, offset) => values.map((value, index, source) => (value ? 10 : 0) + index + offset + source.length); module.exports.boolBooleans = (values, start) => values.map((value, index) => value === (index >= start)); module.exports.boolStrings = (values, prefix) => values.map((value, index) => prefix + value + index);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import * as m from 'jit-composed-primitive-map';\nfunction main(): void { const words = ['a', 'bbb', 'cc']; const flags = [true, false, true]; console.log(m.stringNumbers(words, 2).join(',')); console.log(m.stringBooleans(words, 0).join(',')); console.log(m.stringStrings(words, '-').join(',')); console.log(m.boolNumbers(flags, 2).join(',')); console.log(m.boolBooleans(flags, 1).join(',')); console.log(m.boolStrings(flags, 'v=').join(',')); }\n",
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
        "6,9,9\ntrue,true,false\na-0,bbb-1,cc-2\n15,6,17\nfalse,false,true\nv=true0,v=false1,v=true2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn composed_numeric_reducers_use_jit_callbacks_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-composed-reducer-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-composed-reducer");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function left(values: number[], initial: number): number;\nexport declare function leftFirst(values: number[]): number;\nexport declare function right(values: number[], initial: number): number;\nexport declare function rightLast(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function combine(accumulator, value, index, values) { return accumulator * 2 + value + index + values.length; } module.exports.left = (values, initial) => values.reduce(combine, initial); module.exports.leftFirst = values => values.reduce(combine); module.exports.right = (values, initial) => values.reduceRight(combine, initial); module.exports.rightLast = values => values.reduceRight(combine);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { left, leftFirst, right, rightLast } from 'jit-composed-reducer';\nfunction main(): void { const values = [1, 2, 3]; console.log(left(values, 1)); console.log(leftFirst(values)); console.log(right(values, 1)); console.log(rightLast(values)); try { leftFirst([]); } catch { console.log('left-empty'); } try { rightLast([]); } catch { console.log('right-empty'); } }\n",
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
        "44\n24\n56\n28\nleft-empty\nright-empty\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_jit_callbacks_capture_typed_outer_values_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-captures-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-captures");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function map(values: number[], factor: number, offset: number): number[];\nexport declare function filter(values: number[], minimum: number, maximum: number): number[];\nexport declare function reduce(values: number[], multiplier: number, offset: number): number;\nexport declare function above(values: number[], threshold: string): boolean;\nexport declare function addLength(values: number[], extras: number[]): number[];\nexport declare function signed(values: number[], enabled: boolean): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.map = (values, factor, offset) => values.map((value, index) => value * factor + offset + index); module.exports.filter = (values, minimum, maximum) => values.filter(value => value >= minimum && value <= maximum); module.exports.reduce = (values, multiplier, offset) => values.reduce((accumulator, value, index) => accumulator * multiplier + value + offset + index, 0); module.exports.above = (values, threshold) => values.some(value => value > Number(threshold)); module.exports.addLength = (values, extras) => values.map(value => value + extras.length); module.exports.signed = (values, enabled) => values.map(value => enabled ? value : -value);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { map, filter, reduce, above, addLength, signed } from 'jit-captures';\nfunction main(): void { console.log(map([1, 2], 3, 4).join(',')); console.log(filter([1, 3, 5, 7], 3, 5).join(',')); console.log(reduce([1, 2], 2, 3)); console.log(above([1, 4], '3')); console.log(addLength([1, 2], [9, 8, 7]).join(',')); console.log(signed([1, -2], false).join(',')); }\n",
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
        "7,11\n3,5\n14\ntrue\n4,5\n-1,2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_comparisons_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-primitive-comparisons-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-primitive-comparisons");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function selectStrings(values: string[], minimum: string): string[];\nexport declare function hasFlag(values: boolean[], expected: boolean): boolean;\nexport declare function copyStrings(values: string[]): string[];\nexport declare function trimStrings(values: string[]): string[];\nexport declare function uppercaseStrings(values: string[]): string[];\nexport declare function stringLengths(values: string[]): number[];\nexport declare function parseStrings(values: string[]): number[];\nexport declare function stringifyNumbers(values: number[]): string[];\nexport declare function stringFlags(values: string[]): boolean[];\nexport declare function invertFlags(values: boolean[]): boolean[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { selectStrings: (values, minimum) => values.filter(value => value >= minimum), hasFlag: (values, expected) => values.some(value => value === expected), copyStrings: values => values.map(value => value), trimStrings: values => values.map(value => value.trim()), uppercaseStrings: values => values.map(value => value.toUpperCase()), stringLengths: values => values.map(value => value.length), parseStrings: values => values.map(Number), stringifyNumbers: values => values.map(String), stringFlags: values => values.map(Boolean), invertFlags: values => values.map(value => !value) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { selectStrings, hasFlag, copyStrings, trimStrings, uppercaseStrings, stringLengths, parseStrings, stringifyNumbers, stringFlags, invertFlags } from 'jit-primitive-comparisons';\nfunction main(): void { console.log(selectStrings(['a', 'c', 'b'], 'b').join(',')); console.log(hasFlag([false, true], true)); console.log(copyStrings(['x', 'y']).join(',')); console.log(trimStrings([' a ', ' b']).join(',')); console.log(uppercaseStrings(['a', 'Straße']).join(',')); console.log(stringLengths(['', '😀', 'ab']).join(',')); console.log(parseStrings(['2', '3']).join(',')); console.log(stringifyNumbers([1, 2]).join(',')); console.log(stringFlags(['', 'x']).join(',')); console.log(invertFlags([false, true]).join(',')); }\n",
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
        "c,b\ntrue\nx,y\na,b\nA,STRASSE\n0,2,2\n2,3\n1,2\nfalse,true\ntrue,false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn typed_typeof_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-typeof-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-typeof");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numberKind(value: number): string;\nexport declare function booleanKind(value: boolean): string;\nexport declare function stringKind(value: string): string;\nexport declare function arrayKind(value: number[]): string;\nexport declare function emptyNumber(): number;\nexport declare function emptyString(): string;\nexport declare function emptyBoolean(): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { numberKind: value => typeof value, booleanKind: value => typeof value, stringKind: value => typeof value, arrayKind: value => typeof value, emptyNumber: () => Number(), emptyString: () => String(), emptyBoolean: () => Boolean() };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numberKind, booleanKind, stringKind, arrayKind, emptyNumber, emptyString, emptyBoolean } from 'jit-typeof';\nfunction main(): void { console.log(numberKind(1)); console.log(booleanKind(true)); console.log(stringKind('x')); console.log(arrayKind([1])); console.log(emptyNumber()); console.log('[' + emptyString() + ']'); console.log(emptyBoolean()); }\n",
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
        "number\nboolean\nstring\nobject\n0\n[]\nfalse\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_parameters_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-defaults-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-defaults");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function scale(value?: number, factor?: number): number;\nexport declare function defaultText(value?: string): string;\nexport declare function defaultFlag(value?: boolean): boolean;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.scale = (value = 2, factor = value + 1) => value * factor;\nmodule.exports.defaultText = (value = 'x') => value;\nmodule.exports.defaultFlag = (value = true) => value;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { scale, defaultText, defaultFlag } from 'jit-defaults';\nfunction main(): void { console.log(scale()); console.log(scale(4)); console.log(scale(4, 5)); console.log(scale(undefined, 5)); console.log(defaultText()); console.log(defaultText('y')); console.log(defaultText(undefined)); console.log(defaultFlag()); console.log(defaultFlag(false)); console.log(defaultFlag(undefined)); }\n",
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
        "6\n20\n20\n10\nx\ny\nx\ntrue\nfalse\ntrue\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_string_coercion_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-coercion-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-coercion");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function describe(value: string, flag: boolean): string;\nexport declare function codes(left: number, right: number): string;\nexport declare function points(left: number, right: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.describe = function(value, flag) { const parsed = Number(value.valueOf()); const rounded = Math.round(parsed).valueOf(); const matched = '421'.includes(rounded); const padded = 'x'.padEnd('2', flag); const replaced = '42'.replace(rounded, flag); const picked = 'abc'.charAt('1'); const repeated = 'x'.repeat('2'); let text = `value=${rounded}`; text += ':'; text += flag.toString(); return text.concat(':', rounded, ':', matched, ':', padded, ':', replaced, ':', picked, ':', repeated, ':', String(rounded > 0)); }; module.exports.codes = (left, right) => String.fromCharCode(left, right); module.exports.points = (left, right) => String.fromCodePoint(left, right);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { describe, codes, points } from 'jit-coercion';\nfunction main(): void { console.log(describe('42.4', true)); console.log(codes(65, 66)); console.log(points(0x1f600, 0x1f680)); try { points(0x110000, 65); } catch { console.log('range'); } }\n",
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
        "value=42:true:42:true:xt:true:b:xx:true\nAB\n😀🚀\nrange\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn unicode_normalization_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-normalize-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-normalize");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function nfc(value: string): string;\nexport declare function normalize(value: string, form: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.nfc = value => value.normalize(); module.exports.normalize = (value, form) => value.normalize(form);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { nfc, normalize } from 'jit-normalize';\nfunction main(): void { console.log(nfc('é')); console.log(normalize('é', 'NFD') === 'é'); try { normalize('x', 'invalid'); } catch { console.log('range'); } }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "é\ntrue\nrange\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_split_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-split-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-split");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function split(value: string, separator: string): string[];\nexport declare function limited(value: string, separator: string, limit: number): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.split = (value, separator) => value.split(separator); module.exports.limited = (value, separator, limit) => value.split(separator, limit);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { split, limited } from 'jit-split';\nfunction main(): void { console.log(split('a::b::c', '::').join('|')); console.log(limited('a,b,c', ',', 2).join('|')); console.log(limited('abc', '', 2).join('|')); console.log(limited('a,b', ',', 0).length); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "a|b|c\na|b\na|b\n0\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_copies_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-slice-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-slice");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numbers(values: number[], start: number, end: number): number[];\nexport declare function strings(values: string[], start: number, end: number): string[];\nexport declare function flags(values: boolean[], start: number, end: number): boolean[];\nexport declare function reverseNumbers(values: number[]): number[];\nexport declare function reverseStrings(values: string[]): string[];\nexport declare function reverseFlags(values: boolean[]): boolean[];\nexport declare function sortNumbers(values: number[]): number[];\nexport declare function sortStrings(values: string[]): string[];\nexport declare function sortFlags(values: boolean[]): boolean[];\nexport declare function withNumber(values: number[], index: number, value: number): number[];\nexport declare function withString(values: string[], index: number, value: string): string[];\nexport declare function withFlag(values: boolean[], index: number, value: boolean): boolean[];\nexport declare function concatNumbers(left: number[], first: number, middle: number[], last: number): number[];\nexport declare function concatStrings(left: string[], first: string, middle: string[], last: string): string[];\nexport declare function concatFlags(left: boolean[], first: boolean, middle: boolean[], last: boolean): boolean[];\nexport declare function concatCopy(values: number[]): number[];\nexport declare function literalNumbers(value: number, tail: number[]): number[];\nexport declare function literalStrings(value: string, tail: string[]): string[];\nexport declare function literalFlags(value: boolean, tail: boolean[]): boolean[];\nexport declare function emptyNumbers(): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.numbers = (values, start, end) => values.slice(start, end); module.exports.strings = (values, start, end) => values.slice(start, end); module.exports.flags = (values, start, end) => values.slice(start, end); module.exports.reverseNumbers = values => values.toReversed(); module.exports.reverseStrings = values => values.toReversed(); module.exports.reverseFlags = values => values.toReversed(); module.exports.sortNumbers = values => values.toSorted(); module.exports.sortStrings = values => values.toSorted(); module.exports.sortFlags = values => values.toSorted(); module.exports.withNumber = (values, index, value) => values.with(index, value); module.exports.withString = (values, index, value) => values.with(index, value); module.exports.withFlag = (values, index, value) => values.with(index, value); module.exports.concatNumbers = (left, first, middle, last) => left.concat(first, middle, last); module.exports.concatStrings = (left, first, middle, last) => left.concat(first, middle, last); module.exports.concatFlags = (left, first, middle, last) => left.concat(first, middle, last); module.exports.concatCopy = values => values.concat(); module.exports.literalNumbers = (value, tail) => [1, value, ...tail]; module.exports.literalStrings = (value, tail) => ['a', value, ...tail]; module.exports.literalFlags = (value, tail) => [true, value, ...tail]; module.exports.emptyNumbers = () => [];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numbers, strings, flags, reverseNumbers, reverseStrings, reverseFlags, sortNumbers, sortStrings, sortFlags, withNumber, withString, withFlag, concatNumbers, concatStrings, concatFlags, concatCopy, literalNumbers, literalStrings, literalFlags, emptyNumbers } from 'jit-array-slice';\nfunction main(): void { console.log(numbers([1, 2, 3, 4], 1, -1).join('|')); console.log(strings(['a', 'b', 'c'], 0, 2).join('|')); console.log(flags([true, false, true], 1, 3).join('|')); console.log(reverseNumbers([1, 2, 3]).join('|')); console.log(reverseStrings(['a', 'b', 'c']).join('|')); console.log(reverseFlags([true, false]).join('|')); console.log(sortNumbers([10, 2, 1]).join('|')); console.log(sortStrings(['z', 'a', 'b']).join('|')); console.log(sortFlags([true, false, true]).join('|')); const source = [1, 2, 3]; console.log(withNumber(source, -1, 9).join('|')); console.log(source.join('|')); console.log(withString(['a', 'b'], 0, 'z').join('|')); console.log(withFlag([true, false], 1, true).join('|')); try { withNumber(source, 3, 0); } catch { console.log('range'); } console.log(concatNumbers([1, 2], 3, [4, 5], 6).join('|')); console.log(concatStrings(['a'], 'b', ['c', 'd'], 'e').join('|')); console.log(concatFlags([true], false, [true, false], true).join('|')); console.log(concatCopy(source).join('|')); console.log(literalNumbers(2, [3, 4]).join('|')); console.log(literalStrings('b', ['c', 'd']).join('|')); console.log(literalFlags(false, [true, false]).join('|')); console.log(emptyNumbers().length); }\n",
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
        "2|3\na|b\nfalse|true\n3|2|1\nc|b|a\nfalse|true\n1|10|2\na|b|z\nfalse|true|true\n1|2|9\n1|2|3\nz|b\ntrue|true\nrange\n1|2|3|4|5|6\na|b|c|d|e\ntrue|false|true|false|true\n1|2|3\n1|2|3|4\na|b|c|d\ntrue|false|true|false\n0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn numeric_comparator_sorts_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-numeric-sort-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-numeric-sort");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function ascending(values: number[]): number[];\nexport declare function descending(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "const descending = (left, right) => right - left; module.exports.ascending = values => values.toSorted((left, right) => left - right); module.exports.descending = values => values.sort(descending);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { ascending, descending } from 'jit-numeric-sort';\nfunction main(): void { const source = [2, 10, 1]; console.log(ascending(source).join(',')); console.log(source.join(',')); console.log(descending(source).join(',')); console.log(source.join(',')); }\n",
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
        "1,2,10\n2,10,1\n10,2,1\n10,2,1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_comparator_sorts_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-string-sort-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-string-sort");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function ascending(values: string[]): string[];\nexport declare function descending(values: string[]): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function descending(left, right) { return right.localeCompare(left); } module.exports.ascending = values => values.toSorted((left, right) => left.localeCompare(right)); module.exports.descending = values => values.sort(descending);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { ascending, descending } from 'jit-string-sort';\nfunction main(): void { const source = ['b', 'aa', 'a']; console.log(ascending(source).join(',')); console.log(source.join(',')); console.log(descending(source).join(',')); console.log(source.join(',')); }\n",
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
        "a,aa,b\nb,aa,a\nb,aa,a\nb,aa,a\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_constructors_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-constructors-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-constructors");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numbers(value: number, tail: number[]): number[];\nexport declare function strings(value: string, tail: string[]): string[];\nexport declare function flags(value: boolean, tail: boolean[]): boolean[];\nexport declare function copyNumbers(values: number[]): number[];\nexport declare function copyStrings(values: string[]): string[];\nexport declare function copyFlags(values: boolean[]): boolean[];\nexport declare function characters(value: string): string[];\nexport declare function spreadCharacters(value: string): string[];\nexport declare function ofCharacters(value: string): string[];\nexport declare function empty(): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.numbers = (value, tail) => Array.of(1, value, ...tail); module.exports.strings = (value, tail) => Array.of('a', value, ...tail); module.exports.flags = (value, tail) => Array.of(true, value, ...tail); module.exports.copyNumbers = values => Array.from(values); module.exports.copyStrings = values => Array.from(values); module.exports.copyFlags = values => Array.from(values); module.exports.characters = value => Array.from(value); module.exports.spreadCharacters = value => ['<', ...value, '>']; module.exports.ofCharacters = value => Array.of(...value); module.exports.empty = () => Array.of();\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numbers, strings, flags, copyNumbers, copyStrings, copyFlags, characters, spreadCharacters, ofCharacters, empty } from 'jit-array-constructors';\nfunction main(): void { console.log(numbers(2, [3, 4]).join('|')); console.log(strings('b', ['c']).join('|')); console.log(flags(false, [true]).join('|')); console.log(copyNumbers([1, 2]).join('|')); console.log(copyStrings(['a', 'b']).join('|')); console.log(copyFlags([true, false]).join('|')); console.log(characters('A😀B').join('|')); console.log(spreadCharacters('A😀B').join('|')); console.log(ofCharacters('A😀B').join('|')); console.log(empty().length); }\n",
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
        "1|2|3|4\na|b|c\ntrue|false|true\n1|2\na|b\ntrue|false\nA|😀|B\n<|A|😀|B|>\nA|😀|B\n0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_mutation_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-mutation-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-mutation");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function reverse(values: number[]): number[];\nexport declare function sortNumbers(values: number[]): number[];\nexport declare function sortStrings(values: string[]): string[];\nexport declare function sortFlags(values: boolean[]): boolean[];\nexport declare function fillNumbers(values: number[], value: number): number[];\nexport declare function fillStrings(values: string[], value: string, start: number, end: number): string[];\nexport declare function fillFlags(values: boolean[], value: boolean, start: number): boolean[];\nexport declare function copyNumbers(values: number[], target: number, start: number, end: number): number[];\nexport declare function copyStrings(values: string[], target: number, start: number): string[];\nexport declare function copyFlags(values: boolean[], target: number, start: number, end: number): boolean[];\nexport declare function pushNumbers(values: number[], first: number, second: number): number;\nexport declare function pushStrings(values: string[], first: string, second: string): number;\nexport declare function pushFlags(values: boolean[], first: boolean, second: boolean): number;\nexport declare function pushNone(values: number[]): number;\nexport declare function unshiftNumbers(values: number[], first: number, second: number): number;\nexport declare function unshiftStrings(values: string[], first: string, second: string): number;\nexport declare function unshiftFlags(values: boolean[], first: boolean, second: boolean): number;\nexport declare function unshiftNone(values: number[]): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.reverse = values => values.reverse(); module.exports.sortNumbers = values => values.sort(); module.exports.sortStrings = values => values.sort(); module.exports.sortFlags = values => values.sort(); module.exports.fillNumbers = (values, value) => values.fill(value); module.exports.fillStrings = (values, value, start, end) => values.fill(value, start, end); module.exports.fillFlags = (values, value, start) => values.fill(value, start); module.exports.copyNumbers = (values, target, start, end) => values.copyWithin(target, start, end); module.exports.copyStrings = (values, target, start) => values.copyWithin(target, start); module.exports.copyFlags = (values, target, start, end) => values.copyWithin(target, start, end); module.exports.pushNumbers = (values, first, second) => values.push(first, second); module.exports.pushStrings = (values, first, second) => values.push(first, second); module.exports.pushFlags = (values, first, second) => values.push(first, second); module.exports.pushNone = values => values.push(); module.exports.unshiftNumbers = (values, first, second) => values.unshift(first, second); module.exports.unshiftStrings = (values, first, second) => values.unshift(first, second); module.exports.unshiftFlags = (values, first, second) => values.unshift(first, second); module.exports.unshiftNone = values => values.unshift();\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { reverse, sortNumbers, sortStrings, sortFlags, fillNumbers, fillStrings, fillFlags, copyNumbers, copyStrings, copyFlags, pushNumbers, pushStrings, pushFlags, pushNone, unshiftNumbers, unshiftStrings, unshiftFlags, unshiftNone } from 'jit-array-mutation';\nfunction main(): void { const reversed = [1, 2, 3]; console.log(reverse(reversed).join('|')); console.log(reversed.join('|')); const numbers = [10, 2, 1]; console.log(sortNumbers(numbers).join('|')); console.log(numbers.join('|')); const strings = ['z', 'a', 'b']; console.log(sortStrings(strings).join('|')); const flags = [true, false, true]; console.log(sortFlags(flags).join('|')); const filled = [1, 2, 3]; console.log(fillNumbers(filled, 9).join('|')); console.log(filled.join('|')); console.log(fillStrings(['a', 'b', 'c', 'd'], 'x', -3, -1).join('|')); console.log(fillFlags([true, true, true], false, 1).join('|')); const copied = [1, 2, 3, 4, 5]; console.log(copyNumbers(copied, 1, 0, 4).join('|')); console.log(copied.join('|')); console.log(copyStrings(['a', 'b', 'c', 'd'], -2, 0).join('|')); console.log(copyFlags([true, false, false, true], 0, 2, 4).join('|')); const pushedNumbers = [1]; console.log(pushNumbers(pushedNumbers, 2, 3)); console.log(pushedNumbers.join('|')); const pushedStrings = ['a']; console.log(pushStrings(pushedStrings, 'b', 'c')); console.log(pushedStrings.join('|')); const pushedFlags = [true]; console.log(pushFlags(pushedFlags, false, true)); console.log(pushedFlags.join('|')); console.log(pushNone(pushedNumbers)); const unshiftedNumbers = [3]; console.log(unshiftNumbers(unshiftedNumbers, 1, 2)); console.log(unshiftedNumbers.join('|')); const unshiftedStrings = ['c']; console.log(unshiftStrings(unshiftedStrings, 'a', 'b')); console.log(unshiftedStrings.join('|')); const unshiftedFlags = [true]; console.log(unshiftFlags(unshiftedFlags, false, true)); console.log(unshiftedFlags.join('|')); console.log(unshiftNone(unshiftedNumbers)); }\n",
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
        "3|2|1\n3|2|1\n1|10|2\n1|10|2\na|b|z\nfalse|true|true\n9|9|9\n9|9|9\na|x|x|d\ntrue|false|false\n1|1|2|3|4\n1|1|2|3|4\na|b|a|b\nfalse|true|false|true\n3\n1|2|3\n3\na|b|c\n3\ntrue|false|true\n3\n3\n1|2|3\n3\na|b|c\n3\nfalse|true|true\n3\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_element_assignment_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-set-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-set");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function setNumber(values: number[], index: number, value: number): number;\nexport declare function setNumbers(values: number[], index: number, value: number): number[];\nexport declare function setString(values: string[], index: number, value: string): string[];\nexport declare function setFlag(values: boolean[], index: number, value: boolean): boolean[];\nexport declare function addNumber(values: number[], index: number, value: number): number;\nexport declare function multiplyNumbers(values: number[], index: number, value: number): number[];\nexport declare function appendString(values: string[], index: number, value: string): string;\nexport declare function incrementNumber(values: number[], index: number): number;\nexport declare function decrementNumber(values: number[], index: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.setNumber = (values, index, value) => values[index] = value; module.exports.setNumbers = (values, index, value) => { values[index] = value; return values; }; module.exports.setString = (values, index, value) => { values[index] = value; return values; }; module.exports.setFlag = (values, index, value) => { values[index] = value; return values; }; module.exports.addNumber = (values, index, value) => values[index] += value; module.exports.multiplyNumbers = (values, index, value) => { values[index] *= value; return values; }; module.exports.appendString = (values, index, value) => values[index] += value; module.exports.incrementNumber = (values, index) => ++values[index]; module.exports.decrementNumber = (values, index) => values[index]--;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { setNumber, setNumbers, setString, setFlag, addNumber, multiplyNumbers, appendString, incrementNumber, decrementNumber } from 'jit-array-set';\nfunction main(): void { const numbers = [1]; const alias = numbers; console.log(setNumber(numbers, 3, 9)); console.log(alias.join('|')); console.log(setNumbers(numbers, 1, 2).join('|')); console.log(numbers.join('|')); console.log(addNumber(numbers, 1, 3)); console.log(numbers.join('|')); console.log(multiplyNumbers(numbers, 3, 2).join('|')); console.log(incrementNumber(numbers, 1)); console.log(numbers.join('|')); console.log(decrementNumber(numbers, 3)); console.log(numbers.join('|')); const strings = ['a']; console.log(setString(strings, 2, 'c').join('|')); console.log(strings.join('|')); console.log(appendString(strings, 0, 'b')); console.log(strings.join('|')); const flags = [true]; console.log(setFlag(flags, 2, true).join('|')); console.log(flags.join('|')); }\n",
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
        "9\n1|0|0|9\n1|2|0|9\n1|2|0|9\n5\n1|5|0|9\n1|5|0|18\n6\n1|6|0|18\n18\n1|6|0|17\na||c\na||c\nab\nab||c\ntrue|false|true\ntrue|false|true\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_removal_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-removal-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-removal");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function popNumber(values: number[]): number | undefined;\nexport declare function popString(values: string[]): string | undefined;\nexport declare function popFlag(values: boolean[]): boolean | undefined;\nexport declare function shiftNumber(values: number[]): number | undefined;\nexport declare function shiftString(values: string[]): string | undefined;\nexport declare function shiftFlag(values: boolean[]): boolean | undefined;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.popNumber = values => values.pop(); module.exports.popString = values => values.pop(); module.exports.popFlag = values => values.pop(); module.exports.shiftNumber = values => values.shift(); module.exports.shiftString = values => values.shift(); module.exports.shiftFlag = values => values.shift();\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { popNumber, popString, popFlag, shiftNumber, shiftString, shiftFlag } from 'jit-array-removal';\nfunction main(): void { const pn = [1, 2, 3]; const pnv = popNumber(pn); console.log(pnv === undefined ? 0 : pnv); console.log(pn.join('|')); const ps = ['a', 'b']; const psv = popString(ps); console.log(psv === undefined ? '' : psv); console.log(ps.join('|')); const pb = [true, false]; const pbv = popFlag(pb); console.log(pbv === undefined ? true : pbv); console.log(pb.join('|')); const sn = [1, 2, 3]; const snv = shiftNumber(sn); console.log(snv === undefined ? 0 : snv); console.log(sn.join('|')); const ss = ['a', 'b']; const ssv = shiftString(ss); console.log(ssv === undefined ? '' : ssv); console.log(ss.join('|')); const sb = [true, false]; const sbv = shiftFlag(sb); console.log(sbv === undefined ? false : sbv); console.log(sb.join('|')); const empty: number[] = []; console.log(popNumber(empty) === undefined ? 1 : 0); console.log(empty.length); }\n",
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
        "3\n1|2\nb\na\nfalse\ntrue\n1\n2|3\na\nb\ntrue\nfalse\n1\n0\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn primitive_array_splice_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-splice-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-array-splice");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function numbers(values: number[], start: number, count: number, first: number, second: number): number[];\nexport declare function strings(values: string[], start: number, count: number, value: string): string[];\nexport declare function flags(values: boolean[], start: number): boolean[];\nexport declare function none(values: number[]): number[];\nexport declare function copyNumbers(values: number[], start: number, count: number, first: number, second: number): number[];\nexport declare function copyStrings(values: string[], start: number): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.numbers = (values, start, count, first, second) => values.splice(start, count, first, second); module.exports.strings = (values, start, count, value) => values.splice(start, count, value); module.exports.flags = (values, start) => values.splice(start); module.exports.none = values => values.splice(); module.exports.copyNumbers = (values, start, count, first, second) => values.toSpliced(start, count, first, second); module.exports.copyStrings = (values, start) => values.toSpliced(start);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { numbers, strings, flags, none, copyNumbers, copyStrings } from 'jit-array-splice';\nfunction main(): void { const ns = [1, 2, 3, 4]; console.log(numbers(ns, 1, 2, 8, 9).join('|')); console.log(ns.join('|')); const ss = ['a', 'b', 'c']; console.log(strings(ss, -2, 1, 'x').join('|')); console.log(ss.join('|')); const bs = [true, false, true]; console.log(flags(bs, 1).join('|')); console.log(bs.join('|')); const emptyRemoval = [1, 2]; console.log(none(emptyRemoval).length); console.log(emptyRemoval.join('|')); const copied = [1, 2, 3, 4]; console.log(copyNumbers(copied, -3, 2, 8, 9).join('|')); console.log(copied.join('|')); const copiedStrings = ['a', 'b', 'c']; console.log(copyStrings(copiedStrings, 1).join('|')); console.log(copiedStrings.join('|')); }\n",
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
        "2|3\n1|8|9|4\nb\na|x|c\nfalse|true\ntrue\n0\n1|2\n1|8|9|4\n1|2|3|4\na\na|b|c\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_truthiness_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-truthiness-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-truthiness");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function choose(value: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.choose = value => String(Boolean(value)) + ':' + String(!value) + ':' + (value && 'yes') + ':' + (value || 'fallback') + ':' + (value ? 'set' : 'unset');\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { choose } from 'jit-truthiness';\nfunction main(): void { console.log(choose('x')); console.log(choose('')); }\n",
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
        "true:false:yes:x:set\nfalse:true::fallback:unset\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn mixed_primitive_comparisons_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-comparison-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-comparison");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function compare(value: string, expected: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.compare = (value, expected) => `${value < expected}:${value == expected}:${value === expected}:${value !== expected}:${value.trim() == expected}`;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { compare } from 'jit-comparison';\nfunction main(): void { console.log(compare(' 42 ', 42)); console.log(compare('nope', 42)); }\n",
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
        "false:true:false:true:true\nfalse:false:false:true:false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn optional_string_results_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-optional-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("jit-optional");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function last(value: string, index: number): string | undefined;\nexport declare function point(value: string, index: number): number | undefined;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { last: (value, index) => value.at(index), point: (value, index) => value.codePointAt(index) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { last, point } from 'jit-optional';\nfunction main(): void { console.log((last('abc', -1) !== undefined ? 1 : 0) + (last('abc', 3) === undefined ? 1 : 0) + (point('😀', 0) !== undefined ? 1 : 0) + (point('', 0) === undefined ? 1 : 0)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "4\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn import_meta_resolution_prefers_the_selected_registry_backend() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-import-meta-backend-{}",
        std::process::id()
    ));
    let package = dir.join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("bundle.js"), "module.exports = {};").unwrap();
    std::fs::write(package.join("native.node"), []).unwrap();
    let resolutions =
        registry_import_meta_resolutions(&dir, &["pkg".to_string(), "node:fs".to_string()]);
    assert_eq!(
        resolutions.get("pkg"),
        Some(&module_graph::file_url(&package.join("native.node")))
    );
    assert_eq!(
        resolutions.get("node:fs").map(String::as_str),
        Some("node:fs")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_bundle_uses_timers_microtasks_and_text_encoding() {
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-platform-globals-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("platform-work");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
            package.join("package.d.ts"),
            "export interface PendingValue { (): void; }\nexport declare function exercise(seed: number): PendingValue;\n",
        )
        .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { exercise: function() {
                return new Promise(resolve => {
                    const events = [];
                    const cancelled = setTimeout(() => events.push('cancelled'), 0);
                    clearTimeout(cancelled);
                    const cancelledImmediate = setImmediate(() => events.push('cancelled-immediate'));
                    clearImmediate(cancelledImmediate);
                    process.nextTick(value => events.push(value), 'nextTick');
                    queueMicrotask(() => events.push('microtask'));
                    setImmediate(value => events.push(value), 'immediate');
                    let ticks = 0;
                    const interval = setInterval(() => {
                        ticks++;
                        if (ticks === 2) {
                            clearInterval(interval);
                            const bytes = new TextEncoder().encode('雪');
                            const text = new TextDecoder().decode(bytes);
                            const original = { nested: { value: 1 } };
                            const copied = structuredClone(original);
                            copied.nested.value = 2;
                            const controller = new AbortController();
                            controller.abort('stopped');
                            const combined = AbortSignal.any([controller.signal]);
                            setTimeout(() => resolve(events.join(',') + ':' + ticks + ':' + text + ':' + btoa('hi') + ':' + atob('aGk=') + ':' + (performance.now() >= 0) + ':' + original.nested.value + ':' + copied.nested.value + ':' + combined.aborted + ':' + combined.reason), 0);
                        }
                    }, 1);
                });
            } };"#,
        )
        .unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { exercise } from "platform-work";
                function main(): void {
                    const pending: JsValue = exercise(0);
                    console.log(String(readDynamicValue(pending)));
                    releaseDynamicValue(pending);
                }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "nextTick,microtask,immediate:2:雪:aGk=:hi:true:1:2:true:stopped\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_supported_node_builtin_modules() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-imports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import * as path from "node:path";
                import { inspect, format } from "node:util";
                import { cwd } from "node:process";
                import { byteLength } from "node:buffer";
                import * as os from "node:os";
                import * as querystring from "node:querystring";
                import { EventEmitter } from "node:events";
                import { strictEqual } from "node:assert/strict";
                import { StringDecoder } from "node:string_decoder";
                import { isatty } from "node:tty";
                import { pathToFileURL, fileURLToPath, urlToHttpOptions } from "node:url";
                function main(): void {
                    console.log(String(path.join(JSON.parse("[\"a\",\"b\"]"))));
                    console.log(String(path.extname(JSON.parse("[\"archive.tar.gz\"]"))));
                    console.log(String(path.relative(JSON.parse("[\"/a/b\",\"/a/c/d\"]"))));
                    console.log(String(inspect(JSON.parse("[42]"))));
                    console.log(String(format(JSON.parse("[\"%s:%d\",\"value\",4]"))));
                    console.log(String(cwd(JSON.parse("[]"))));
                    console.log(Number(byteLength(JSON.parse("[\"thaw\"]"))));
                    console.log(String(os.arch(JSON.parse("[]"))) + ":" + String(os.platform(JSON.parse("[]"))) + ":" + String(os.type(JSON.parse("[]"))) + ":" + String(os.tmpdir(JSON.parse("[]"))));
                    console.log(Boolean(isatty(JSON.parse("[1]"))));
                    console.log(String(querystring.stringify(JSON.parse("[{\"a\":[1,2],\"space\":\"two words\"}]"))));
                    console.log(String(querystring.parse(JSON.parse("[\"a=1&a=2&space=two+words\"]"))));
                    console.log(String(EventEmitter(JSON.parse("[]"))));
                    console.log(String(pathToFileURL(JSON.parse("[\"/tmp/a b\"]"))));
                    console.log(String(fileURLToPath(JSON.parse("[\"file:///tmp/a%20b\"]"))));
                    console.log(String(urlToHttpOptions(JSON.parse("[\"https://user:pass@example.test:8443/a?b=1#c\"]"))));
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected_tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!("a/b\n.gz\n../c/d\n42\nvalue:4\n/\n4\nx64:linux:Linux:{expected_tmpdir}\nfalse\na=1&a=2&space=two%20words\n{{\"a\":[\"1\",\"2\"],\"space\":\"two words\"}}\n{{\"_events\":{{}}}}\nfile:///tmp/a%20b\n/tmp/a b\n{{\"protocol\":\"https:\",\"hostname\":\"example.test\",\"hash\":\"#c\",\"search\":\"?b=1\",\"pathname\":\"/a\",\"path\":\"/a?b=1\",\"href\":\"https://user:pass@example.test:8443/a?b=1#c\",\"port\":8443,\"auth\":\"user:pass\"}}\n")
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_fs_reads_and_writes_real_files_in_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-fs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data_dir = dir.join("data");
    let data_file = data_dir.join("message.txt");
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        format!(
            r#"
                    import {{ existsSync, readFileSync, writeFileSync, mkdirSync }} from "node:fs";
                    function main(): void {{
                        console.log(mkdirSync("{}"));
                        console.log(writeFileSync("{}", "hello from thaw"));
                        console.log(existsSync("{}"));
                        console.log(readFileSync("{}", "utf8"));
                    }}
                "#,
            data_dir.display(),
            data_file.display(),
            data_file.display(),
            data_file.display()
        ),
    )
    .unwrap();
    let output = dir.join("app");
    build_with_link_mode(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\ntrue\ntrue\nhello from thaw\n"
    );
    assert_eq!(
        std::fs::read_to_string(data_file).unwrap(),
        "hello from thaw"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_http_serves_a_real_request_from_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let occupied_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let occupied_port = occupied_listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-node-http-{}-{}",
        std::process::id(),
        port
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            format!(
                r#"
                    import {{ createServer }} from "node:http";
                    function main(): void {{
                        const prefix: string = "hello";
                        let requests: number = 0;
                        const server = createServer(
                            (
                                request: {{ method: string; url: string }},
                                response: {{
                                    statusCode: number;
                                    setHeader: (name: string, value: string) => boolean;
                                    end: (chunk: string) => boolean;
                                    write: (chunk: string) => boolean;
                                    endEncoded: (content: string, encoding: string) => boolean;
                                }}
                            ): boolean => {{
                                requests = requests + 1;
                                response.statusCode = 201;
                                response.setHeader("X-Thaw", request.method);
                                response.write(prefix);
                                return response.end(request.url);
                            }}
                        );
                        const target: string = server.listenMany({}, 2);
                        console.log(target);
                        console.log(requests);
                        console.log(server.close());
                        console.log(server.close());
                        server.on("listening", (): void => {{
                            console.log("event:listening");
                        }});
                        server.on("close", (): void => {{
                            console.log("event:close");
                        }});
                        server.on("error", (error: {{ message: string; code: string; syscall: string; address: string; port: number }}): void => {{
                            console.log(error.code);
                        }});
                        server.listen({}, (): void => {{
                            console.log("listening");
                        }});
                        console.log(server.close((): void => {{
                            console.log("closed");
                        }}));
                        server.listen(70000);
                        server.listen({});
                    }}
                "#,
                port, port, occupied_port
            ),
        )
        .unwrap();
    let executable = dir.join("app");
    build_with_link_mode(
        &entry,
        &executable,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    assert!(!elf_has_program_interpreter(&executable).unwrap());

    let child = Command::new(&executable)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fn request(port: u16, target: &str) -> String {
        let mut stream = (0..200)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    None
                }
            })
            .expect("compiled HTTP server did not start listening");
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    let first_request = std::thread::spawn(move || request(port, "/health"));
    let second_response = request(port, "/ready");
    let response = first_request.join().unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(response.contains("X-Thaw: GET\r\n"));
    assert!(response.ends_with("hello/health"));
    assert!(second_response.ends_with("hello/ready"));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let (last_target, events) = stdout.split_once('\n').unwrap();
    assert!(matches!(last_target, "/health" | "/ready"));
    assert_eq!(
        events,
        "2\ntrue\nfalse\ntrue\nevent:listening\nlistening\nERR_SOCKET_BAD_PORT\nEADDRINUSE\nevent:close\nclosed\n"
    );

    std::fs::write(
            &entry,
            r#"import { createServer } from "node:http";
            function main(): void {
                const server = createServer((
                    request: { method: string; url: string },
                    response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
                ): boolean => true);
                server.listen(70000);
            }"#,
        )
        .unwrap();
    let unhandled = dir.join("unhandled-error");
    build_with_link_mode(
        &entry,
        &unhandled,
        &[],
        &[],
        &[],
        &dir.join("registry-unhandled"),
        &[],
        true,
    )
    .unwrap();
    let failed = Command::new(&unhandled).output().unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Unhandled 'error' event"));
    drop(occupied_listener);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bare_import_resolution_errors_include_source_location() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-missing-bare-import-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { missing } from \"not-installed\";\nfunction main(): void {}\n",
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
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("not-installed"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_embedded_wasi_preview1_module() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-wasi-preview1-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("wasi-fixture");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function run(): number;\n",
    )
    .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { run: function () {
                 var options = { args: ['embedded'], env: { MODE: 'standalone' }, preopens: {}, returnOnExit: true, version: 'preview1' };
                 var imports = { wasi_snapshot_preview1: Object.freeze({ __thawWasiOptions: JSON.stringify(options) }) };
                 var source = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
                   (memory (export "memory") 1)
                   (func (export "_start") i32.const 6 call $exit))`);
                 var instance = new WebAssembly.Instance(new WebAssembly.Module(source), imports);
                 try { instance.exports._start(); } catch (error) { if (error && error.__thawWasiExit !== undefined) return Number(error.__thawWasiExit); throw error; }
                 return 0;
               } };
"#,
        )
        .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { run } from \"wasi-fixture\"; function main(): void { console.log(run()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_a_scoped_package_subpath_with_default_and_namespace_forms() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-scoped-subpath-import-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("@scope/tools/subpaths/feature");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export default function double(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = function(value) { return value * 2; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import double from "@scope/tools/feature";
                import * as feature from "@scope/tools/feature";
                function main(): void {
                    console.log(double(20));
                    console.log(feature.double(21));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_callable_import_exposes_export_assignment_namespace_methods() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-export-assignment-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("callable-tools");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function tools(value: number): number;\ndeclare namespace tools {\n    function answer(value: number): number;\n}\nexport = tools;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function tools(value) { return value * 2; } tools.answer = value => value; module.exports = tools;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import tools from \"callable-tools\"; function main(): void { console.log(tools(21)); console.log(tools.answer(42)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The `export = X;` shape the test above covers, but where `X` is bound
/// to a whole *interface-typed const* with several methods (real
/// example: lodash's `declare const _: LoDashStatic;`, ~300 methods) --
/// unlike `tools` above, `_` is never itself one of its own flattened
/// method names, so `package_exports.get("_")` always misses. `import {
/// chunk } from "..."` (a named import of one flattened method) already
/// worked before this fix; `import _ from "..."` (the default-import
/// form, as common in real lodash code as the named form) failed
/// outright ("no export named `default`") since nothing ever inserted a
/// `"default"` key for this shape.
#[test]
fn default_import_exposes_an_export_assignment_interfaces_methods() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-export-assignment-interface-default-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("lodash-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export = _;\n\
         export as namespace _;\n\
         declare const _: LoDashStatic;\n\
         interface LoDashStatic {\n\
         \x20\x20\x20\x20chunk(value: number): number;\n\
         \x20\x20\x20\x20capitalize(value: number): number;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         chunk: function(value) { return value * 2; }, \
         capitalize: function(value) { return value + 1; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import _ from \"lodash-kit\"; function main(): void { console.log(_.chunk(20)); console.log(_.capitalize(41)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commonjs_default_import_exposes_named_only_exports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-commonjs-named-class-default-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("store-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function answer(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { answer: function() { return 42; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import store from \"store-kit\"; function main(): void { console.log(store.answer()); }\n",
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

#[test]
fn dynamic_collection_results_coerce_to_declared_native_types() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-dynamic-collection-results-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("collection-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export = toolkit;\n\
         declare const toolkit: Toolkit;\n\
         interface Toolkit {\n\
         \x20\x20transform(values: number[], callback: (value: number) => number): number[];\n\
         \x20\x20select(values: string, callback: (value: string) => boolean): string[];\n\
         \x20\x20select(values: number[], callback: (value: number) => boolean): number[];\n\
         \x20\x20fold(values: number[], callback: (total: number, value: number) => number, initial: number): number;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         transform: function(values, callback) { return values.map(callback); }, \
         select: function(values, callback) { return values.filter(callback); }, \
         fold: function(values, callback, initial) { return values.reduce(callback, initial); } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import toolkit from \"collection-kit\"; function main(): void { const doubled: number[] = toolkit.transform([1, 2, 3, 4], (value: number): number => value * 2); const selected: number[] = toolkit.select(doubled, (value: number): boolean => value >= 6); const total: number = toolkit.fold(selected, (sum: number, value: number): number => sum + value, 0); console.log(doubled.join(',')); console.log(selected.join(',')); console.log(total); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4,6,8\n6,8\n14\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registers_builds_and_runs_an_installed_npm_wildcard_subpath() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-installed-wildcard-{}",
        std::process::id()
    ));
    let node_modules = dir.join("node_modules");
    let package = node_modules.join("feature-kit");
    std::fs::create_dir_all(package.join("dist/features")).unwrap();
    std::fs::write(
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
    std::fs::write(
        package.join("index.d.ts"),
        "export declare function root(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("index.js"),
        "module.exports = { root: function() { return 1; } };\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.d.ts"),
        "export default function triple(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.js"),
        "module.exports = function(value) { return value * 3; };\n",
    )
    .unwrap();
    let registry = dir.join("registry");
    thaw_registry::add_installed(&registry, &node_modules, "feature-kit").unwrap();

    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import triple from "feature-kit/features/triple";
                function main(): void { console.log(triple(14)); }
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_package_subpath_reports_the_import_location() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-missing-subpath-{}", std::process::id()));
    let registry = dir.join("registry");
    std::fs::create_dir_all(registry.join("math-kit")).unwrap();
    std::fs::write(
        registry.join("math-kit/package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { square } from \"math-kit/missing\";\nfunction main(): void {}\n",
    )
    .unwrap();
    let error = build(&entry, &dir.join("app"), &[], &[], &[], &registry, &[]).unwrap_err();
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("math-kit/missing"), "{error}");
    assert!(error.contains("package.d.ts"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runs_a_multifile_lambda_with_two_bare_import_packages() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-bare-import-lambda-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    for (package, value) in [("left-mark", 20), ("right-mark", 22)] {
        std::fs::create_dir_all(registry.join(package)).unwrap();
        std::fs::write(
            registry.join(package).join("package.d.ts"),
            "export declare function mark(): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join(package).join("bundle.js"),
            format!("module.exports = {{ mark: function() {{ return {value}; }} }};\n"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("work.ts"),
        r#"
                import { mark as leftMark } from "left-mark";
                import { mark as rightMark } from "right-mark";
                export async function work(): Promise<void> {
                    await sleep(1);
                    console.log(leftMark() + rightMark());
                }
            "#,
    )
    .unwrap();
    let entry = dir.join("handler.ts");
    std::fs::write(
        &entry,
        r#"
                import { work } from "./work";
                async function handler(event: Json): Promise<Json> {
                    await work();
                    return event;
                }
            "#,
    )
    .unwrap();
    let output = dir.join("bootstrap");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        let event = "{\"packages\":2}";
        conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: packages-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    event.len(), event
                )
                .as_bytes(),
            )
            .unwrap();
        drop(conn);
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let mut child = Command::new(&output)
        .env("AWS_LAMBDA_RUNTIME_API", addr)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("package Lambda handler did not post a response");
    server.join().unwrap();
    let _ = child.kill();
    let result = child.wait_with_output().unwrap();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/packages-request/response"));
    assert!(request.ends_with("{\"packages\":2}"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn numeric_comparison_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-compare-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("compare-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function compare(a: number, b: number): boolean[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.compare = (a, b) => [a < b, a <= b, a > b, a >= b, a === b, a !== b];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { compare } from 'compare-kit';\nfunction main(): void { const nan = 0 / 0; const zero = 0; const negZero = -zero; console.log(compare(nan, 5).join(',')); console.log(compare(5, nan).join(',')); console.log(compare(nan, nan).join(',')); console.log(compare(zero, negZero).join(',')); }\n",
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
        "false,false,false,false,false,true\nfalse,false,false,false,false,true\nfalse,false,false,false,false,true\nfalse,true,false,true,true,false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bitwise_int32_boundary_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-bitwise-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("bits-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function bits(a: number): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.bits = (a) => [a | 0, a >>> 0, ~a, a << 1, a >>> 1];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { bits } from 'bits-kit';\nfunction main(): void { const nan = 0 / 0; console.log(bits(-1).join(',')); console.log(bits(nan).join(',')); console.log(bits(4294967295).join(',')); console.log(bits(2147483648).join(',')); console.log(bits(5.9).join(',')); console.log(bits(-5.9).join(',')); }\n",
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
        "-1,4294967295,0,-2,2147483647\n\
         0,0,-1,0,0\n\
         -1,4294967295,0,-2,2147483647\n\
         -2147483648,2147483648,2147483647,0,1073741824\n\
         5,5,-6,10,2\n\
         -5,4294967291,4,-10,2147483645\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_surrogate_pair_and_empty_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-surrogate-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("strings-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function codePoints(s: string): number[];\nexport declare function charCodes(s: string): number[];\nexport declare function fromStringInfo(s: string): number[];\nexport declare function emptyProbe(s: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.codePoints = (s) => [s.codePointAt(0), s.codePointAt(1), s.codePointAt(2)]; module.exports.charCodes = (s) => [s.charCodeAt(0), s.charCodeAt(1), s.charCodeAt(2), s.charCodeAt(99)]; module.exports.fromStringInfo = (s) => [Array.from(s).length, s.length]; module.exports.emptyProbe = (s) => s.charCodeAt(0);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { codePoints, charCodes, fromStringInfo, emptyProbe } from 'strings-kit';\nfunction main(): void { const emoji = \"\\uD83D\\uDE00x\"; console.log(codePoints(emoji).join(',')); console.log(charCodes(emoji).join(',')); console.log(fromStringInfo(emoji).join(',')); console.log(emptyProbe(\"\")); }\n",
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
        "128512,56832,120\n55357,56832,120,NaN\n2,3\nNaN\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recursion_and_dictionary_aliasing_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-recursion-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("rec-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fib(n: number): number;\nexport declare function isEven(n: number): boolean;\nexport declare function isOdd(n: number): boolean;\nexport declare function dictAlias(seed: Record<string, number>): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }\nfunction isEven(n) { return n === 0 ? true : isOdd(n - 1); }\nfunction isOdd(n) { return n === 0 ? false : isEven(n - 1); }\nfunction dictAlias(seed) { const alias = seed; seed[\"a\"] = 1; alias[\"b\"] = 2; seed[\"a\"] = (seed[\"a\"] ?? 0) + 10; return alias[\"a\"] + alias[\"b\"]; }\nmodule.exports = { fib, isEven, isOdd, dictAlias };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fib, isEven, isOdd, dictAlias } from 'rec-kit';\nfunction main(): void { console.log(fib(20)); console.log(isEven(11)); console.log(isOdd(11)); console.log(dictAlias({})); }\n",
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
        "6765\nfalse\ntrue\n13\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_directly_on_array_from_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-from-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("from-chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function codeUnitLengths(s: string): number[];\nexport declare function evens(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.codeUnitLengths = (s) => Array.from(s).map((c) => c.length); module.exports.evens = (values) => Array.from(values).filter((v) => v % 2 === 0);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { codeUnitLengths, evens } from 'from-chain-kit';\nfunction main(): void { const emoji = \"\\uD83D\\uDE00x\"; console.log(codeUnitLengths(emoji).join(',')); console.log(evens([1, 2, 3, 4, 5, 6]).join(',')); }\n",
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
        "2,1\n2,4,6\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_from_and_of_chains_generalize_across_methods_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-from-of-chains-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function reduceFrom(values: number[]): number;\nexport declare function doubleChain(values: number[]): number[];\nexport declare function ofChain(): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.reduceFrom = (values) => Array.from(values).reduce((a, b) => a + b, 0); module.exports.doubleChain = (values) => Array.from(values).map((v) => v * 2).filter((v) => v > 4); module.exports.ofChain = () => Array.of(1, 2, 3).map((v) => v * 10);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { reduceFrom, doubleChain, ofChain } from 'chain-kit';\nfunction main(): void { console.log(reduceFrom([1, 2, 3, 4])); console.log(doubleChain([1, 2, 3, 4]).join(',')); console.log(ofChain().join(',')); }\n",
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
        "10\n6,8\n10,20,30\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_directly_on_object_keys_values_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-keys-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("obj-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function keyLengths(o: Record<string, number>): number[];\nexport declare function doubledValues(o: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.keyLengths = (o) => Object.keys(o).map((k) => k.length); module.exports.doubledValues = (o) => Object.values(o).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { keyLengths, doubledValues } from 'obj-kit';\nfunction main(): void { console.log(keyLengths({ ab: 1, cde: 2 }).join(',')); console.log(doubledValues({ ab: 1, cde: 2 }).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,3\n2,4\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_methods_chained_directly_on_from_char_code_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromcharcode-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("str-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function upper(code: number): string;\nexport declare function fromPoint(point: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.upper = (code) => String.fromCharCode(code).toUpperCase(); module.exports.fromPoint = (point) => String.fromCodePoint(point).trim();\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { upper, fromPoint } from 'str-kit';\nfunction main(): void { console.log(upper(97)); console.log(fromPoint(98)); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "A\nb\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_split_and_spread_literals_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-split-spread-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("mix-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function wordLens(s: string): number[];\nexport declare function spreadDouble(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.wordLens = (s) => s.split(' ').map((w) => w.length); module.exports.spreadDouble = (values) => [...values].map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { wordLens, spreadDouble } from 'mix-kit';\nfunction main(): void { console.log(wordLens(\"ab cde f\").join(',')); console.log(spreadDouble([1, 2, 3]).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,3,1\n2,4,6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_a_parenthesized_ternary_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-ternary-receiver-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("ternary-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function ternaryChain(flag: boolean, values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.ternaryChain = (flag, values) => (flag ? Array.from(values) : values).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { ternaryChain } from 'ternary-kit';\nfunction main(): void { console.log(ternaryChain(true, [1, 2, 3]).join(',')); console.log(ternaryChain(false, [1, 2, 3]).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4,6\n2,4,6\n");
    let _ = std::fs::remove_dir_all(dir);
}

// Object.fromEntries only accepts the output of Object.entries(...) (a round trip),
// not an arbitrary array-of-pairs literal; Object.assign requires every argument to
// already be dictionary-typed (a plain `{}` object literal defaults to a fixed-shape
// object type instead). Both are intentional scope limits, not receiver-gate bugs -
// this locks in the shapes that already work correctly through the JIT today.
#[test]
fn object_from_entries_and_assign_round_trips_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromentries-assign-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("obj-roundtrip-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function roundtripKeyCount(o: Record<string, number>): number;\nexport declare function assignedValues(a: Record<string, number>, b: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.roundtripKeyCount = (o) => Object.keys(Object.fromEntries(Object.entries(o))).length; module.exports.assignedValues = (a, b) => Object.values(Object.assign(a, b));\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { roundtripKeyCount, assignedValues } from 'obj-roundtrip-kit';\nfunction main(): void { console.log(roundtripKeyCount({ a: 1, b: 2 })); console.log(assignedValues({ x: 1 }, { y: 2 }).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2\n1,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_valued_short_circuit_operators_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-short-circuit-array-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("short-circuit-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orValue(a: number[], b: number[]): number[];\nexport declare function andValue(a: number[], b: number[]): number[];\nexport declare function nullishValue(a: number[], b: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orValue = (a, b) => a || b; module.exports.andValue = (a, b) => a && b; module.exports.nullishValue = (a, b) => a ?? b;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orValue, andValue, nullishValue } from 'short-circuit-kit';\nfunction main(): void { console.log(orValue([], [3, 4]).join(',')); console.log(orValue([1, 2], [3, 4]).join(',')); console.log(andValue([1, 2], [3, 4]).join(',')); console.log(nullishValue([1, 2], [3, 4]).join(',')); }\n",
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
        "\n1,2\n3,4\n1,2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_short_circuit_operators_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-short-circuit-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("short-circuit-chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orChain(a: number[], b: number[]): number[];\nexport declare function andChain(a: number[], b: number[]): number[];\nexport declare function nullishChain(a: number[], b: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orChain = (a, b) => (a || b).map((v) => v * 2); module.exports.andChain = (a, b) => (a && b).map((v) => v * 2); module.exports.nullishChain = (a, b) => (a ?? b).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orChain, andChain, nullishChain } from 'short-circuit-chain-kit';\nfunction main(): void { console.log(orChain([1, 2], [3, 4]).join(',')); console.log(andChain([1, 2], [3, 4]).join(',')); console.log(nullishChain([1, 2], [3, 4]).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4\n6,8\n2,4\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_assign_with_an_empty_literal_target_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-assign-empty-literal-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("assign-empty-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function assignChain(a: Record<string, number>, b: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.assignChain = (a, b) => Object.values(Object.assign({}, a, b));\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { assignChain } from 'assign-empty-kit';\nfunction main(): void { console.log(assignChain({ x: 1 }, { y: 2 }).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "1,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_from_entries_with_a_literal_array_of_pairs_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromentries-literal-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("fromentries-literal-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fromLiteralPairs(pairs: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.fromLiteralPairs = (pairs) => { const built = Object.fromEntries([[\"a\", pairs[0]], [\"b\", pairs[1]]]); return [built.a, built.b, Object.keys(built).length]; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fromLiteralPairs } from 'fromentries-literal-kit';\nfunction main(): void { console.log(fromLiteralPairs([10, 20]).join(',')); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "10,20,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function (forced by an unclassifiable `unknown`-typed
/// parameter, real-world example: uuid's `v4`/`validate`) with several
/// *trailing optional* parameters. Regression coverage for two bugs found
/// together while getting uuid itself to build: `typed_dynamic_declaration`
/// used to give up on a typed wrapper entirely the moment any one parameter
/// was unclassifiable, and even once that was fixed, its generated wrapper
/// only guarded the *last* optional slot with `!== undefined`, so the
/// type-narrowing pass left every earlier optional parameter still
/// `Optional(...)` where the underlying call needed it unwrapped.
#[test]
fn fallback_function_with_multiple_optional_unknown_params_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-multi-optional-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function make(prefix?: unknown, suffix?: unknown, length?: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.make = function(prefix, suffix, length) {\n\
             var p = prefix === undefined ? '' : String(prefix);\n\
             var s = suffix === undefined ? '' : String(suffix);\n\
             var n = length === undefined ? 0 : length;\n\
             return p + 'id' + s + ':' + n;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { make } from "id-kit";
function main(): void {
    console.log(make());
    console.log(make("a"));
    console.log(make("a", "b"));
    console.log(make("a", "b", 5));
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
        "id:0\naid:0\naidb:0\naidb:5\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Two `.d.ts` overloads for the same Fallback name (real-world example:
/// `ms`'s `(value: number, options?)` / `(value: string)`) can each
/// independently produce a valid typed wrapper. Regression coverage for a
/// bug introduced alongside the fix above: broadening which parameter types
/// count as classifiable meant a second, narrower overload could newly
/// succeed too and silently clobber the first (and better -- it covers both
/// arities) overload's registration, since call-site rewriting just kept
/// whichever overload was processed last.
#[test]
fn first_matching_overload_wins_for_a_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-order-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("format-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function format(value: number, upper?: boolean): string;\n\
         export declare function format(value: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.format = function(value, upper) {\n\
             if (typeof value === 'string') { return '[' + value + ']'; }\n\
             var text = String(value) + 'px';\n\
             return upper ? text.toUpperCase() : text;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { format } from "format-kit";
function main(): void {
    console.log(format(12));
    console.log(format(12, true));
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
    // If the second (narrower, single required-param) overload had won
    // instead, `format(12, true)` -- 2 arguments -- would be a compile
    // error, since that overload's own wrapper only ever accepts 1.
    assert_eq!(String::from_utf8_lossy(&result.stdout), "12px\n12PX\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose overloads are discriminated by *arity
/// range*, not `typeof` -- real-world example: uuid's `v4(options?):
/// string` (0-1 args) alongside its generic buffer-output
/// `v4<TBuf extends Uint8Array = Uint8Array>(options, buf, offset?):
/// TBuf` (2-3 args), previously unreachable no matter what a real call
/// passed, since "first successful overload wins" only ever exposed the
/// first. Exercises the full pipeline end to end (`generate_registry_
/// shims`'s new per-overload declarations plus `class_methods.rs`'s new
/// bare-call rewrite), not just the rewrite function in isolation --
/// this is the test that would have caught the original bug.
#[test]
fn fallback_function_overload_is_picked_by_call_site_arity() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-arity-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeId(seed?: number): string;\n\
         export declare function makeId(seed: number, salt: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeId = function(seed, salt) {\n\
             if (salt !== undefined) { return seed * 1000 + salt; }\n\
             return 'id-' + (seed === undefined ? 0 : seed);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeId } from "id-kit";
function main(): void {
    console.log(makeId());
    console.log(makeId(5));
    console.log(makeId(5, 7));
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
    // If the 2-argument call had stayed pinned to the first (0-1 arg)
    // overload's declaration instead, this would be a compile error
    // (too many arguments), not a wrong runtime value.
    assert_eq!(String::from_utf8_lossy(&result.stdout), "id-0\nid-5\n5007\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose `.d.ts` signature ends in a rest parameter
/// (real-world example: `clsx(...inputs: ClassValue[]): string`).
/// `typed_dynamic_declaration` used to ignore `rest_param` entirely and
/// declare a fixed 0-argument extern signature, so any real (non-empty)
/// call became an arity-mismatch compile error. Regression coverage for
/// `typed_dynamic_rest_declaration`, which instead declares one extern
/// signature per call-site argument count actually observed in the user's
/// own source and dispatches on the wrapper's own `...rest` array length.
#[test]
fn fallback_function_with_a_rest_parameter_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-rest-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("join-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function joinValues(...inputs: unknown[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.joinValues = function() {\n\
             var parts = [];\n\
             for (var i = 0; i < arguments.length; i++) {\n\
                 if (arguments[i]) { parts.push(String(arguments[i])); }\n\
             }\n\
             return parts.join(' ');\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { joinValues } from "join-kit";
function main(): void {
    console.log(joinValues("a", "b"));
    console.log(joinValues("a", false, "b", null, "c"));
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
        "a b\na b c\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn generic_rest_callbacks_are_inferred_and_passed_without_array_marshalling() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-rest-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("order-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orderBy<T>(values: T[], ...iteratees: Array<(value: T) => number>): T[];\n\
         export declare function orderBy<T extends object>(values: T, ...iteratees: Array<(value: T[keyof T]) => number>): Array<T[keyof T]>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orderBy = function(values) { \
         var iteratees = Array.prototype.slice.call(arguments, 1); \
         var result = Array.isArray(values) ? values : Object.values(values); \
         iteratees.forEach(function(iteratee) { result.forEach(iteratee); }); \
         return result; \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { orderBy } from "order-kit";
function main(): void {
    let total: number = 0;
    const result = orderBy([3, 1, 2], value => { total = total + value; return value; });
    console.log(result[0]);
    console.log(result.join(','));
    const objectResult = orderBy({ a: 4, b: 2 }, value => { total = total + value; return value; });
    console.log(objectResult[0]);
    console.log(objectResult.join(','));
    console.log(total);
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
        "3\n3,1,2\n4\n4,2\n12\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function (any single param/return referencing its
/// own unconstrained type parameter classifies Fallback, since that's an
/// unresolved reference as far as `classify` is concerned) declared as
/// returning `void`. The generic branch of `typed_dynamic_declaration`
/// used to render `return {base_symbol}__arity_N(...);` unconditionally,
/// but thaw-hir rejects `return <a void call>;` for a `void`-declared
/// function (only a bare `return;`), so any real call was a compile
/// error. Regression coverage for splitting the call and the return into
/// separate statements when the declared return is `void`.
#[test]
fn generic_fallback_function_with_void_return_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-void-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("seen-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function markSeen<T>(value: T): void;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "var seen = [];\nmodule.exports.markSeen = function(value) { seen.push(value); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { markSeen } from "seen-kit";
function main(): void {
    markSeen("a");
    markSeen(1);
    console.log("done");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "done\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function with a parameter type this classifier
/// can't resolve at all (a reference to a type alias never declared
/// anywhere in the file, e.g. because it lives in some other file a
/// simpler package never pulled in -- real-world example: lodash's
/// `uniq<T>(array: List<T> | null | undefined): T[]`, `List` being one
/// of lodash's own internal aliases). The generic branch used to reject
/// the whole declaration outright whenever any parameter didn't already
/// render as one of a few known-safe forms, falling all the way back to
/// the bare `(argsArray: Json): Json` passthrough shim -- silently wrong
/// for an ordinary single-argument call like `firstOf(someArray)`, whose
/// real argument would be passed as the whole *args array* instead.
/// Regression coverage for substituting `Json` for just that one
/// parameter instead of giving up on the whole function.
///
/// `firstOf`'s return is bare `T`, preserved by the same generic branch
/// as a real type parameter (see
/// `generic_fallback_function_with_return_only_type_parameter_infers_it_from_a_let_annotation`),
/// so this specific call needs a `let`/`const` annotation to give
/// thaw-hir something to infer `T` from -- an unannotated, directly
/// nested call has no argument that mentions `T` either, once `items`
/// itself was substituted to `Json`.
#[test]
fn generic_fallback_function_with_unresolvable_param_type_passes_it_through_as_json() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-json-passthrough-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("first-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function firstOf<T>(items: List<T> | null): T;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.firstOf = function(items) { return items && items.length ? items[0] : null; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { firstOf } from "first-kit";
function main(): void {
    const items: Json = JSON.parse("[10, 20, 30]");
    const first: number = firstOf(items);
    console.log(first);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "10\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose type parameter appears *only* in
/// the return position -- real-world example: nanoid's own `nanoid<Type
/// extends string>(size?: number): Type`. thaw-hir has no argument to
/// infer `Type` from at all here, but can still infer it from a
/// `let`/`const` declaration's own type annotation as a fallback (see
/// `infer_generic_type_tuple`/`lower_expr_with_expected_type` in
/// thaw-hir), so the generic branch keeps this one type parameter
/// declared (unlike a type parameter with no remaining occurrence
/// anywhere, dropped as dead syntax) and renders the real return type
/// instead of the usual hardcoded `JsValue`.
#[test]
fn generic_fallback_function_with_return_only_type_parameter_infers_it_from_a_let_annotation() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-return-only-generic-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("id-gen-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeId<Type extends string>(size?: number): Type;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeId = function(size) {\n\
             var n = size === undefined ? 3 : size;\n\
             var out = '';\n\
             for (var i = 0; i < n; i++) { out += 'x'; }\n\
             return out;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeId } from "id-gen-kit";
function main(): void {
    const id: string = makeId();
    console.log(id.length);
    const short: string = makeId(5);
    console.log(short.length);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n5\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose *return* type can't be classified at all --
/// real-world example: mime's `getType(path: string): string | null`
/// alongside a hypothetical sibling overload whose return type this
/// crude a classifier can't yet describe, modeled here with a `Promise`
/// return (still explicitly out of scope -- see `classify_ts_type`'s
/// `TsTypeRef` doc comment). `typed_dynamic_declaration` used to give up
/// on the whole declaration the moment the return type alone didn't
/// classify (even though every parameter already passes through as
/// `Json` for exactly this reason), falling all the way back to the
/// bare untyped `(argsArray: Json): Json` passthrough shim -- silently
/// wrong for any call that isn't already packing its own arguments into
/// one array itself, the same failure mode as an unclassifiable
/// parameter. Regression coverage for treating an unclassifiable return
/// the same way a callback-typed return already was: a `JsValue`
/// handle, usable even without dedicated support for whatever real
/// shape it holds.
///
/// (Not namespace-qualified, unlike this test's original form: a
/// namespace-qualified reference to an interface/type-alias, even an
/// empty one, now resolves -- see `extract_interface_decls`/
/// `extract_type_alias_decls` and `classify_ts_type`'s own doc comment
/// on `TsQualifiedName` -- so it no longer demonstrates "unresolvable"
/// at all.)
#[test]
fn fallback_function_with_an_unresolvable_return_type_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-unresolvable-return-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function clock(seed?: unknown): Promise<unknown>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.clock = function(seed) {\n\
             return { toString: function() { return 'tick'; } };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { clock } from "clock-kit";
function main(): void {
    console.log(clock());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "tick\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `export default function name(...): T;` alongside a second, separate
/// named export in the same `.d.ts` -- real-world example: leven's own
/// `export default function leven(...): number;` plus its named
/// `export function closestMatch(...): string | undefined;`.
/// `commonjs_export_name` used to recognize only CommonJS's `export = x;`,
/// so a package's "default" binding fell back to working only when the
/// package had *exactly one* function total -- true for most
/// single-purpose packages, but not one exporting more than one function
/// this way, where `import leven from "leven"` had nothing telling it
/// which of the two `leven` itself actually names.
#[test]
fn esm_default_export_alongside_a_second_named_export_resolves_the_default_import() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-esm-default-plus-named-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("distance-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export default function distance(a: string, b: string): number;\n\
         export function closestMatch(target: string, candidates: string[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.distance = function(a, b) { return a === b ? 0 : 1; };\n\
         module.exports.closestMatch = function(target, candidates) { return candidates[0]; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import distance from "distance-kit";
function main(): void {
    console.log(distance("cat", "cat"));
    console.log(distance("cat", "cow"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose parameter (or an object argument's own
/// field) is a union type -- real-world example: camelcase's own
/// `camelCase(input: string | readonly string[], options?: {
/// pascalCase?: boolean, ... }): string`. Marshaling a dynamic call's
/// argument to JSON had no case for `HirType::Union` at either the
/// top-level-argument or the nested-object-field layer, so passing a
/// plain string (matching the union's *first* member, but still a
/// union statically) failed outright rather than only breaking for
/// values that actually needed the second member.
#[test]
fn fallback_function_with_a_union_typed_argument_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-union-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        // `separator` is itself a union field *inside* the options
        // object, exercising the object-field JSON-marshaling path
        // separately from `input`'s own top-level union.
        "export interface CaseOptions { separator?: string | boolean; }\n\
         export declare function toCase(input: string | readonly string[], options?: CaseOptions): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.toCase = function(input, options) {\n\
             var s = Array.isArray(input) ? input.join('-') : input;\n\
             var sep = options && options.separator;\n\
             return typeof sep === 'string' ? s.split('-').join(sep) : s;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { toCase } from "case-kit";
function main(): void {
    console.log(toCase("foo"));
    console.log(toCase(["a", "b"], { separator: "_" }));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "foo\na_b\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose parameter/return type is `Date`, generic
/// over it (`<DateType extends Date>`) -- real-world example: date-fns's
/// `format<DateType extends Date>(date: DateType | number | string,
/// formatStr: string): string` and `addDays<DateType extends Date>(date:
/// ..., amount: number): DateType`. Exercises the whole round trip: a
/// Thaw-native `Date` (an object with a `timestamp` field) argument must
/// arrive on the QuickJS-NG side as a real `instanceof Date` (this fake
/// package's `format`/`addDays` both throw/misbehave otherwise), and a
/// real `Date` returned from QuickJS-NG must come back as that same
/// native shape, usable as a plain `Date` argument to a later call. Only
/// UTC-based formatting is asserted (never a local-time token like `HH`)
/// so this doesn't depend on the host's time zone.
#[test]
fn fallback_function_with_a_generic_date_argument_and_return_builds_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-date-generic-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("temporal-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function format<DateType extends Date>(date: DateType | number | string, formatStr: string): string;\n\
         export declare function addDays<DateType extends Date>(date: DateType | number | string, amount: number): DateType;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function pad(n) { return n < 10 ? '0' + n : '' + n; }\n\
         module.exports.format = function(date, formatStr) {\n\
             if (!(date instanceof Date)) throw new Error('Invalid time value');\n\
             return date.getUTCFullYear() + '-' + pad(date.getUTCMonth() + 1) + '-' + pad(date.getUTCDate());\n\
         };\n\
         module.exports.addDays = function(date, amount) {\n\
             if (!(date instanceof Date)) throw new Error('Invalid time value');\n\
             return new Date(date.getTime() + amount * 86400000);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { format, addDays } from "temporal-kit";
function main(): void {
    const d = new Date(2024, 0, 15);
    console.log(format(d, "yyyy-MM-dd"));
    const later: Date = addDays(d, 10);
    console.log(format(later, "yyyy-MM-dd"));
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
        "2024-01-15\n2024-01-25\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback (QuickJS-NG, no native addon) package whose factory
/// function returns an instance of one of its own classes -- real-world
/// example: dayjs's `declare function dayjs(...): dayjs.Dayjs`, `Dayjs`
/// declared inside `declare namespace dayjs { class Dayjs {...} }`
/// rather than at the top level. Item 4 of
/// docs/design/npm-interop-gaps-2026-09.md: `pkg.classes` used to be
/// bridged only for a native-addon package (`generate_native_addon_shim`
/// gated on `pkg.native_addon.is_some()`); a Fallback class's instance
/// stayed a permanently opaque, method-less `JsValue`, and a factory
/// function's return value in particular had no linkage back to which
/// class it even was (a namespace-qualified return type like
/// `dayjs.Dayjs` classifies `Unsupported` and loses the name).
/// Exercises the whole chain: `thaw_bridge::function_return_named_types`
/// linking the factory to its class, `parse_dts_classes` now descending
/// into `declare namespace` blocks to find the class at all, thaw-cli's
/// shims.rs generating a QuickJS-NG-backed (`__thaw_typed_js_`) instance
/// method declaration via the same generator a native addon's class
/// already used, `compile_typed_napi_method` (thaw-llvm) now dispatching
/// that declaration's backend to `thaw_js_call_method_result` instead of
/// only ever `thaw_napi_call_method_typed_result`, and class_methods.rs
/// tracking the factory call's result as a class instance the same way
/// `new ClassName(...)` already is.
#[test]
fn fallback_factory_function_result_supports_instance_method_calls() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-factory-class-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeClock(hour: number): clock.Clock;\n\
         declare namespace clock {\n\
             class Clock {\n\
                 format(): string;\n\
                 hourAt(offset: number): number;\n\
             }\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Clock(hour) { this.hour = hour; }\n\
         Clock.prototype.format = function() { return 'H:' + this.hour; };\n\
         Clock.prototype.hourAt = function(offset) { return this.hour + offset; };\n\
         module.exports.makeClock = function(hour) { return new Clock(hour); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeClock } from "clock-kit";
function main(): void {
    const c = makeClock(3);
    console.log(c.format());
    console.log(c.hourAt(5));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "H:3\n8\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Same shape as `fallback_factory_function_result_supports_instance_
/// method_calls`, but the method is called *chained directly* off the
/// factory call's own return value (`makeClock(3).format()`), with no
/// `const` binding in between -- real example: dayjs's own `dayjs(
/// "2024-01-15").format("YYYY-MM-DD")`. Used to fail to build
/// ("unsupported member call target"): the receiver-type lookup in
/// thaw-hir's member-call lowering only handled a plain `Ident` (bound
/// via `const`, recovering its type from `self.scope`), `new`, `this`,
/// and a few other forms -- a bare `Expr::Call` receiver had no arm at
/// all and fell through to `None`, so the call's *known* factory return
/// type (`clock.Clock`) was simply never consulted. Fixed by looking up
/// the callee's own declared (non-generic) return type directly from
/// `self.signatures` for exactly this shape; the receiver expression
/// itself is still only lowered and evaluated once, when it's spliced
/// into the class-method call the same way a bound receiver's `Ident`
/// already was.
#[test]
fn a_method_can_be_chained_directly_onto_a_fallback_factory_calls_return_value() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-factory-chained-call-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clock-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeClock(hour: number): clock.Clock;\n\
         declare namespace clock {\n\
             class Clock {\n\
                 format(): string;\n\
             }\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Clock(hour) { this.hour = hour; }\n\
         Clock.prototype.format = function() { return 'H:' + this.hour; };\n\
         module.exports.makeClock = function(hour) { return new Clock(hour); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeClock } from "clock-kit2";
function main(): void {
    console.log(makeClock(3).format());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"H:3\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function with two overloads whose *type-disjoint* first
/// parameter picks between genuinely different calling conventions --
/// real-world example: `ms`'s own `ms(value: number, options?: { long:
/// boolean }): string` and `ms(value: ms.StringValue): number` (the
/// same function either formats a millisecond count as a string or
/// parses a duration string into a millisecond count). `typed_dynamic_
/// declaration`'s "first successful overload wins" rule (needed to
/// avoid emitting two conflicting ambient declarations under the same
/// name) used to pick only the first-declared overload, silently
/// breaking every call shaped like the other -- `ms("2 days")` failed
/// with a type error demanding a `number`. Also exercises two
/// prerequisites this needed: a namespace-qualified type reference
/// (`ms.StringValue`) resolving through `declare namespace ms { ... }`
/// at all (`extract_type_alias_decls`), and a template literal type
/// resolving to `Str` (ms's own `StringValue` is a union of them).
#[test]
fn fallback_function_with_type_disjoint_overloads_dispatches_by_typeof() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-dispatch-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("duration-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function duration(value: number, options?: { long: boolean }): string;\n\
         declare function duration(value: duration.StringValue): number;\n\
         declare namespace duration {\n\
             type StringValue = `${number}` | `${number} days`;\n\
         }\n\
         export = duration;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = function(value, options) {\n\
             if (typeof value === 'number') {\n\
                 return options && options.long ? value + ' milliseconds' : value + 'ms';\n\
             }\n\
             var days = parseInt(value, 10);\n\
             return days * 86400000;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import duration from "duration-kit";
function main(): void {
    console.log(duration("2 days"));
    console.log(duration(60000));
    console.log(duration(60000, { long: true }));
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
        "172800000\n60000ms\n60000 milliseconds\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose return type is declared literally
/// `any` -- real-world example: lodash's `cloneDeepWith<TValue>(...):
/// any`. The generic branch's "a return that doesn't mention any type
/// parameter is just a concrete type" rule (see
/// `fallback_function_with_a_generic_date_argument_and_return_builds_
/// and_runs`) used to splice that `any` text straight into the
/// generated ambient declaration's own return position -- reparseable
/// for a type parameter's own `extends` *constraint* (`is_reparseable_
/// ts_type`, since thaw-hir keeps a constraint as raw, unlowered
/// syntax), but not for an ordinary *value* type position, which does
/// get lowered through thaw-hir's `lower_ts_type` and only supports
/// `number`/`string`/`boolean`/`void` there -- so the whole generated
/// shim failed to compile ("unsupported type keyword TsAnyKeyword")
/// the moment any Fallback package had a generic function shaped this
/// way, even one the user's own program never calls.
#[test]
fn generic_fallback_function_with_a_literal_any_return_type_builds() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-generic-any-return-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("clone-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function cloneDeepWith<T>(value: T, customizer: unknown): any;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.cloneDeepWith = function(value) { return value; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { cloneDeepWith } from "clone-kit";
function main(): void {
    console.log("built");
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "built\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Str`/`F64`-type-disjoint overload set whose two overloads name
/// their (structurally corresponding) first parameter *differently* --
/// unlike this session's own `ms` regression test, whose two overloads
/// happen to both call theirs `value`, masking a real bug:
/// `union_overload_dispatch_declaration`'s generated dispatcher
/// referenced each branch's *own* parameter name when forwarding the
/// call (`secondary_overload(name)`) instead of the dispatcher's own
/// shared variable for that slot (`primary`'s name, since the
/// dispatcher's outer signature is named after whichever overload
/// carries extra parameters) -- an undefined-reference bug that would
/// otherwise make the "secondary" (non-primary) branch always forward
/// `undefined` instead of the real argument.
#[test]
fn fallback_function_with_differently_named_overload_parameters_forwards_the_shared_variable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-overload-param-names-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("pick-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function pick(word: string): number;\n\
         export declare function pick(count: number, label?: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.pick = function(a, b) {\n\
             return typeof a === 'string' ? a.length : a + (b ? b.length : 0);\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { pick } from "pick-kit";
function main(): void {
    console.log(pick("hello"));
    console.log(pick(10, "ab"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n12\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `Bool`/`F64`-discriminated overload set (real example: lodash's
/// `random(floating?: boolean): number` / `random(max: number,
/// floating?: boolean): number`) whose generated dispatcher's shared
/// first-parameter variable is named after the primary overload's own
/// parameter (`max`) -- which, in a real package like lodash, collides
/// with an unrelated top-level function *also* named `max` (`_.max`)
/// declared elsewhere in the same `.d.ts`. `typeof`-lowering
/// (`UnaryOp::TypeOf` in thaw-hir) resolved a bare identifier's operand
/// type by checking `self.signatures` (the top-level function table)
/// *before* checking whether the name was actually a local variable in
/// `self.scope` -- so inside the dispatcher, `typeof max === "boolean"`
/// resolved `max`'s type from the unrelated top-level `max` function's
/// *signature* instead of the dispatcher's own `boolean | number`
/// parameter, folding the whole comparison to the constant string
/// `"function"` and permanently skipping the boolean branch. Every call
/// then fell through to the number branch, reinterpreting the packed
/// union payload's raw bits for a `boolean` argument as an `f64` --
/// observed as `5e-324` (`Number.MIN_VALUE`, the bit pattern of integer
/// `1`) and a SIGSEGV on repeated calls. Fixed by having the operand
/// type resolution check `self.scope` first, since a local
/// variable/parameter must shadow a same-named top-level function. This
/// only reproduced with a real, large `.d.ts` (lodash's) because that's
/// what supplied the colliding same-named top-level function --
/// isolated `Bool`-discriminator tests without one never hit it.
#[test]
fn bool_discriminated_overload_dispatch_is_not_corrupted_by_a_same_named_top_level_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-bool-overload-name-collision-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("stat-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function random(floating?: boolean): number;\n\
         export declare function random(max: number, floating?: boolean): number;\n\
         export declare function max(a: number, b: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.random = function(a, b) {\n\
             if (typeof a === 'boolean') { return a ? 1 : 0; }\n\
             return a + (b ? 0.5 : 0);\n\
         };\n\
         module.exports.max = function(a, b) { return a > b ? a : b; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { random, max } from "stat-kit";
function main(): void {
    console.log(random(true));
    console.log(random(false));
    console.log(random(5));
    console.log(random(5, true));
    console.log(max(3, 7));
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
        "1\n0\n5\n5.5\n7\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function whose declared name matches a C standard library
/// math function (`floor`, here -- the real-world trigger: lodash exports
/// `_.floor`/`_.ceil`/`_.round`, and any package declaring one of these
/// pulls in the same shape) used to corrupt *every* `Math.floor`/`Math.
/// ceil`/`Math.round` call made from *any* Fallback JS running in the same
/// program, not just calls to the colliding function itself.
///
/// Root cause: `declare_function` (thaw-llvm) gave every HIR-compiled
/// function -- including this one, generated verbatim from a `.d.ts`
/// declaration by `registry_integration/shims.rs` -- ordinary `external`
/// LLVM linkage, keyed on its bare source name with no mangling
/// (`llvm_symbol_for`). In the final binary that makes it a *global,
/// default-visibility* ELF symbol -- confirmed via `readelf --dyn-syms`
/// showing a `floor` entry in the executable's own dynamic symbol table.
/// Since libm is linked dynamically, Linux's default symbol interposition
/// rule then made *every* reference to `floor` process-wide -- including
/// the one QuickJS-NG's own `Math.floor` builtin makes internally via the
/// PLT -- resolve to *this* native function instead of libm's real
/// `floor(double): double`. Called with a `double` argument (in an XMM
/// register, per the C calling convention) as if it were thaw's own
/// `floor(argsArray: Json): Json` (expecting a pointer in a general-
/// purpose register), it dereferenced garbage and crashed inside
/// `thaw_std::json::ordered_object_fields` -- reproducing as `SIGSEGV` on
/// `Math.floor`/`Math.ceil`/`Math.round` calls anywhere in the program,
/// including ones with nothing to do with the colliding function (this is
/// also the real root cause of the previously-unresolved `_.chunk`
/// SIGSEGV: `_.chunk`'s own implementation calls `Math.floor` internally,
/// and lodash's full `.d.ts` always declares `floor`/`ceil`/`round` too).
///
/// Fixed by giving every HIR-compiled function `internal` LLVM linkage
/// instead: they never need to be called from outside the single LLVM
/// module thaw-llvm compiles the whole program into (intra-module calls
/// resolve directly regardless of linkage), so this closes off the
/// interposition risk entirely without changing how anything actually
/// runs -- confirmed by the existing frame-split IR-text assertions
/// (`define internal ptr @...` instead of `define ptr @...`) and by the
/// full test suite staying green.
#[test]
fn a_fallback_function_named_like_a_libm_function_does_not_corrupt_math_builtins() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-libm-name-collision-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("math-clash-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function floor(n: number): number;\n\
         export declare function useMathFloor(n: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.floor = function(x) { return x - 1000; };\n\
         module.exports.useMathFloor = function(x) { return Math.floor(x); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { floor, useMathFloor } from "math-clash-kit";
function main(): void {
    console.log(floor(5));
    console.log(useMathFloor(4.7));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "-995\n4\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A real `import * as pkg from "pkg"` namespace import, calling a
/// two-parameter (one optional) Fallback function through it
/// (`pkg.major(version, precise?)`), used to be silently hijacked by the
/// unrelated "bare qualifier, no import needed" convenience syntax
/// (`rewrite_qualified_calls`) whenever the chosen namespace alias
/// happened to equal the package's own qualifier -- the natural,
/// idiomatic choice, exactly what real code (`import * as semver from
/// "semver"`) does. That rewrite sends the call to the untyped,
/// single-`argsArray`-parameter Fallback alias instead of the properly
/// typed, arity-dispatching wrapper a plain `import { major }` gets,
/// crashing with "args_json is not a valid JSON array" the moment any
/// argument was passed un-packed. Fixed by having the rewrite skip any
/// qualifier name that's also bound by a real import in the file.
#[test]
fn namespace_import_member_call_is_not_hijacked_by_the_bare_qualifier_rewrite() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-namespace-import-not-hijacked-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("verkit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function major(version: string, precise?: boolean): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.major = function(version, precise) {\n\
             var n = parseInt(String(version).split('.')[0], 10);\n\
             return precise ? n + 0.5 : n;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as verkit from "verkit";
function main(): void {
    console.log(verkit.major("3.2.1"));
    console.log(verkit.major("3.2.1", true));
}
"#,
    )
    .unwrap();
    let output = dir.join("app");
    // The bare-qualifier rewrite this test guards against only fires for
    // an explicitly `--use`d package (see `qualified_call_rewrites`'s
    // `use_qualifiers` filter in `build_with_link_mode`) -- an
    // auto-resolved-from-the-import package alone doesn't reproduce it.
    build(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &registry,
        &["verkit".to_string()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3\n3.5\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real zod v4's own `package.d.ts` re-exports its whole API two ways
/// at once: every function flattened to a top-level named export (via
/// `export * from "./v4/classic/external.cjs"`, already resolved by
/// `thaw registry add`'s own install-time flattening into plain
/// `export declare function ...` lines in the same file), *and* the
/// same module star-imported under a name and re-exported as a
/// namespace object (`import * as z from "./v4/classic/external.cjs";
/// export { z, z as default };`) -- so `import { object, string } from
/// "zod"` and `import * as z from "zod"; z.object(...)` are meant to
/// reach the exact same declarations. This does *not* exercise a
/// `declare namespace` block (a genuinely different, still-unsupported
/// shape -- a namespace whose members are declared *inside* it, not a
/// star-import of an already-flattened sibling module) -- confirmed
/// via a synthetic reproduction of zod's literal shape that this one
/// already works end-to-end: thaw-bridge's parser simply has no
/// declaration named `z` to classify at all (it's only ever bound by
/// an `import *`, never a real top-level `function`/`class`/
/// `interface`), so it's silently skipped, and the *user's own*
/// `import * as z from "case-kit3"` is an entirely ordinary namespace
/// import of the package's already-flattened export table -- unrelated
/// machinery already handles it (see `namespace_import_member_call_is_
/// not_hijacked_by_the_bare_qualifier_rewrite` above).
#[test]
fn a_namespace_reexported_from_the_packages_own_flattened_module_works_like_a_plain_namespace_import()
 {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-self-reexported-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        // Mirrors real zod's exact `index.d.ts` shape post-flattening:
        // an unresolved self-referencing `import *`/`export *`/
        // `export { z, z as default }` trio (the relative path is
        // never actually followed -- there is no such file here --
        // exactly like thaw-bridge's single-file parser never follows
        // it for real zod either), followed by the already-flattened
        // top-level declarations that path's `export *` used to stand
        // for.
        "import * as z from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { z, z as default };\n\
         \n\
         export declare function double(value: number): number;\n\
         export declare function greet(name: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.double = function(value) { return value * 2; };\n\
         module.exports.greet = function(name) { return 'hi ' + name; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as z from "case-kit3";
function main(): void {
    console.log(z.double(21));
    console.log(z.greet("Alice"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\nhi Alice\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The same self-referential namespace shape as the test above
/// (`import * as z`), but imported by *name* instead --
/// `import { z } from "case-kit4";`, real-world shape: zod's own docs
/// show both `import * as z from "zod"` and `import { z } from "zod"`
/// as equivalent, since both reach the identical namespace object. Used
/// to fail outright at the module-graph level (`` `zod` has no export
/// named `z` ``): a named import resolves a specific key from the
/// package's flat export table, and `z` was never in it at all --
/// there's no function/class/interface actually named `z` anywhere in
/// the flattened `.d.ts` (it's only ever bound by the package's own
/// internal `import * as z`), unlike a namespace import, which just
/// hands over the *whole* export table under whatever local name the
/// user chose, with no per-name lookup needed at all.
///
/// Fixed with `thaw_bridge::self_referential_namespace_aliases`
/// (recognizing `import * as z from "./local"; export { z, z as
/// default };` in a package's own `.d.ts`) threaded through as a new
/// `ExternalNamespaceAliases` map (thaw-cli's `shims.rs` / `build.rs` /
/// `module_graph.rs`): a named import (or a same-file `export { z }
/// from "pkg";` re-export) of exactly one of these recognized names
/// now resolves the same way a namespace import already did, instead
/// of failing the ordinary single-symbol lookup.
#[test]
fn a_named_import_of_a_self_referential_namespace_alias_works_like_a_namespace_import() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-named-self-reexported-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit4");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "import * as z from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { z, z as default };\n\
         \n\
         export declare function double(value: number): number;\n\
         export declare function greet(name: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { double: function(value) { return value * 2; }, greet: function(name) { return 'hi ' + name; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { z } from "case-kit4";
function main(): void {
    console.log(z.double(21));
    console.log(z.greet("Alice"));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\nhi Alice\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Constructing a Fallback (pure-JS, QuickJS-NG-dispatched) class
/// instance via `new Class(...)` -- real-world example: hono's `new
/// Hono()`. Two bugs, found and fixed together:
///
/// 1. `generate_napi_class_constructors` (despite the name, also used
///    for a Fallback class's constructor -- see its own `napi`
///    parameter) always emitted `__thaw_typed_napi_...` symbols, which
///    `dynamic_symbol` always decodes as `DynamicBackend::Napi`. For a
///    Fallback class that's the wrong backend outright, and
///    `compile_typed_dynamic_call`'s own construct-a-value branch (LLVM
///    codegen) only existed for `DynamicBackend::Napi` at all -- there
///    was no QuickJS-NG equivalent using `thaw_js_get_global` +
///    `thaw_js_construct_handle_result` (both already existed, wired to
///    nothing).
/// 2. Separately: a package whose *only* export is a class (no
///    top-level Fallback functions at all -- exactly hono's shape) never
///    got its `bundle.js` loaded via `__thaw_module_init` in the first
///    place, since the "does this package need `loadScript`-ing"
///    condition checked only `fallback_names` (top-level functions),
///    never `pkg.classes` -- so even the class's own name was never
///    bound onto `globalThis` at all, regardless of the constructor
///    codegen gap above.
///
/// Also exercises the constructor's own optional-parameter arity
/// dispatch (`new Widget()` vs. `new Widget("hi")`), and that instance
/// methods keep working on a Fallback-constructed instance the same way
/// they already did on one obtained a different way (dayjs's factory
/// function, mime's ready-made export).
#[test]
fn fallback_class_is_constructible_via_new() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-fallback-class-new-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("widget-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Widget {\n\
             constructor(name?: string);\n\
             describe(): string;\n\
             $disconnect(): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget(name) { this.name = name || 'default'; }\n\
         Widget.prototype.describe = function() { return 'Widget:' + this.name; };\n\
         Widget.prototype.$disconnect = function() { console.log('disconnected'); };\n\
         module.exports.Widget = Widget;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "widget-kit";
function main(): void {
    const named = new Widget("hi");
    console.log(named.describe());
    const defaulted = new Widget();
    console.log(defaulted.describe());
    defaulted.$disconnect();
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
        "Widget:hi\nWidget:default\ndisconnected\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A package whose *only* export is a class (no top-level function at
/// all) still gets its `bundle.js` loaded and its exports bound to
/// `globalThis` -- see `fallback_class_is_constructible_via_new`'s doc
/// comment, point 2. This is really the same root cause, but written
/// as its own minimal test (a class exported via a getter-defined
/// property, `Object.defineProperty(module.exports, ..., { get, ...
/// })`, the shape a real esbuild/tsc-bundled package like hono actually
/// uses -- not a plain `module.exports.X = X` assignment) so a future
/// regression here is diagnosable without needing to reason through the
/// constructor-codegen half at all.
#[test]
fn a_class_only_package_still_loads_its_bundle() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-class-only-package-loads-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("getter-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare class Widget {\n\
             constructor(name?: string);\n\
             describe(): string;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Widget(name) { this.name = name || 'default'; }\n\
         Widget.prototype.describe = function() { return 'Widget:' + this.name; };\n\
         Object.defineProperty(module.exports, 'Widget', { get: function() { return Widget; }, enumerable: true });\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { Widget } from "getter-kit";
function main(): void {
    const widget = new Widget("hi");
    console.log(widget.describe());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "Widget:hi\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue` (the opaque handle a Fallback function returns when its
/// TS type can't be classified as anything JSON-representable, e.g.
/// zod's `z.string()` returning a live `ZodString` schema instance) used
/// to have nowhere to go once it existed: passing one as an argument to
/// *another* dynamic call -- even a bare, top-level one, not nested in
/// anything -- failed at HIR lowering ("value has type JsValue, expected
/// Json"), because there's no JSON encoding of "a live JS object".
/// `compile_dynamic_value_placeholder` (thaw-llvm's `json_bridge.rs`)
/// fixes this by encoding the handle's own permanent id as
/// `{"__thaw_js_handle_id__": N}` instead of trying to serialize it, and
/// the QuickJS-side JSON reviver (see `dates.js`'s doc comment) splices
/// the real value back in the moment that JSON gets parsed on the other
/// end -- the same mechanism already used to round-trip a `Date`.
/// Exercises the value being reused twice (proving the handle stays
/// live/valid across more than one such call, not just one-shot).
#[test]
fn a_js_value_can_be_passed_as_a_bare_argument_to_another_dynamic_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-bare-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("handle-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function wrapThing(inner: JsValue): JsValue;\n\
         export declare function describe(thing: JsValue): string;\n\
         export declare function makeBox(inner: JsValue): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function wrapThing(inner) {\n\
             return { toString: function () { return 'wrapped(' + inner.toString() + ')'; } };\n\
         }\n\
         function describe(thing) { return thing.toString(); }\n\
         function makeBox(inner) {\n\
             return { toString: function () { return 'box(' + inner.toString() + ')'; } };\n\
         }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.wrapThing = wrapThing;\n\
         module.exports.describe = describe;\n\
         module.exports.makeBox = makeBox;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, wrapThing, describe, makeBox } from "handle-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped = wrapThing(thing);
    console.log(describe(wrapped));
    const boxed = makeBox(thing);
    console.log(describe(boxed));
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
        "wrapped(thing:gadget)\nbox(thing:gadget)\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A bare `undefined` literal passed directly as a dynamic-call argument
/// (real example: zod's own `schema.safeParse(undefined)`, e.g. against
/// a `z.undefined()` schema) used to be a hard compile error ("value has
/// type Undefined, expected Json") -- JSON has no `undefined` at all,
/// only the case a nested *field* can omit itself entirely (already
/// handled elsewhere), which doesn't apply to a standalone value with no
/// field to omit. Fixed in `coerce_to_declared` (thaw-hir) by encoding it
/// as the same `{"$__thaw_napi_undefined$": true}` sentinel a NAPI
/// return value already uses for the identical problem, reviving it back
/// to the real literal on the QuickJS side (`__thaw_json_date_reviver`,
/// the same reviver `Date`/`JsValue` already go through). Distinguishes
/// a real `undefined` from `null` and from any other JSON value to rule
/// out an accidental "map to null" shortcut, which would be wrong: real
/// zod's `ZodUndefined` schema rejects `null`.
#[test]
fn a_bare_undefined_literal_can_be_passed_as_a_dynamic_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-arg-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             return {\n\
                 check: function(value) {\n\
                     if (value === undefined) return 'real-undefined';\n\
                     if (value === null) return 'null';\n\
                     return 'other:' + JSON.stringify(value);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "undef-arg-kit";
function main(): void {
    const thing = makeThing();
    console.log(thing.check(undefined));
    console.log(thing.check(null));
    console.log(thing.check(5));
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
        "\"real-undefined\"\n\"null\"\n\"other:5\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An `Optional`-typed variable (`string | undefined`) that actually
/// holds `undefined` at runtime, passed as a dynamic-call argument
/// (real example: `schema.safeParse(value)` against real zod's own
/// `z.undefined()`, where `value: string | undefined` happens to be
/// `undefined`) needs the exact same real-`undefined` round-trip the
/// bare-literal case above gets. Exercised through `wrap_native_value_
/// as_json`'s own temporary-object round trip (`JsonSet` then `JsonGet`)
/// -- unlike the bare-literal case, this one goes through the tagged
/// `Optional`/`Nullable`/`Nullish` encoding (`compile_json_object_set_
/// tagged`), which has its own long-standing `preserve_undefined` flag
/// for exactly "should an absent value be written as a real `undefined`
/// sentinel or simply omitted" -- omission is correct for a genuine
/// object-literal field (matching `JSON.stringify`'s own behavior), but
/// wrong here: there's no real field to omit, just a temporary one used
/// to round-trip a *standalone* value back out, and omitting it made
/// `thaw_json_get`'s own missing-key fallback report plain JSON `null`
/// instead -- silently turning a real `undefined` argument into `null`,
/// which real zod's `ZodUndefined` schema correctly rejects (`false`
/// where `true` was expected). Fixed by threading a `bool` through
/// `HirExpr::JsonSet` so `wrap_native_value_as_json`'s own use of it can
/// ask for `true` (preserve) while an ordinary object literal's own
/// field-by-field construction keeps the old `false` (omit) behavior.
#[test]
fn an_optional_variable_holding_undefined_can_be_passed_as_a_dynamic_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-optional-undefined-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("opt-undef-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             return {\n\
                 check: function(value) {\n\
                     if (value === undefined) return 'real-undefined';\n\
                     if (value === null) return 'null';\n\
                     return 'other:' + JSON.stringify(value);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "opt-undef-kit";
function main(): void {
    const thing = makeThing();
    const absent: string | undefined = undefined;
    console.log(thing.check(absent));
    const present: string | undefined = "hi";
    console.log(thing.check(present));
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
        "\"real-undefined\"\n\"other:\\\"hi\\\"\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Same underlying gap as
/// `a_js_value_can_be_passed_as_a_bare_argument_to_another_dynamic_call`,
/// but for a `JsValue` nested inside an object-literal field and inside
/// an array-literal element -- both go through the *same*
/// `compile_dynamic_value_placeholder` call, just reached via
/// `compile_json_object_set_native_with_undefined`'s and
/// `compile_json_array_push_native_with_undefined`'s own `HirType::JsValue`
/// arms rather than the bare-argument catch-all, and the QuickJS-side
/// reviver splices each one back in regardless of depth (it runs
/// bottom-up over the whole parsed value, exactly like it already does
/// for a nested `Date`). Real-world shape: zod's own `z.object({ name:
/// z.string() })`, a fixed-shape object literal with one field itself a
/// live schema value.
#[test]
fn a_js_value_can_be_nested_inside_an_object_or_array_literal_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-nested-argument-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("handle-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function group(shape: { name: JsValue; label: string }): string;\n\
         export declare function listGroup(items: JsValue[]): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function group(shape) {\n\
             return shape.label + '=' + shape.name.toString();\n\
         }\n\
         function listGroup(items) {\n\
             return items.map(function (item) { return item.toString(); }).join(',');\n\
         }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.group = group;\n\
         module.exports.listGroup = listGroup;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, group, listGroup } from "handle-kit2";
function main(): void {
    const a = makeThing("alpha");
    console.log(group({ name: a, label: "x" }));
    const b = makeThing("beta");
    console.log(listGroup([a, b]));
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
        "x=thing:alpha\nthing:alpha,thing:beta\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose type parameter is constrained to
/// some other *named* type (`T extends core.SomeType`) -- an extremely
/// common TS generics idiom, and zod's own real shape for e.g.
/// `optional<T extends core.SomeType>(innerType: T): ZodOptional<T>` --
/// used to make `typed_dynamic_declaration` (thaw-cli's `shims.rs`)
/// give up on the *whole* declaration, since `is_reparseable_ts_type`'s
/// constraint allowlist is just a handful of primitive keywords
/// (`string`, `number`, `Date`, ...), falling all the way back to the
/// bare untyped `(argsArray: Json): Json` shim -- silently wrong for an
/// ordinary single-argument call like this one, the same failure mode
/// as an unresolvable parameter or return type already had its own
/// fallback for. Fixed by dropping an unparseable constraint (rendering
/// the type parameter bare) instead of aborting the declaration, and by
/// teaching `supports_generic_native_layout` (thaw-hir) that `JsValue`
/// specializes a generic type parameter exactly like any other
/// fixed-size scalar -- needed here because `T` infers as `JsValue` from
/// `makeThing`'s own `JsValue`-returning result.
#[test]
fn a_generic_function_constrained_to_a_named_type_still_gets_a_typed_declaration() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-named-constraint-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("generic-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n\
         export declare function wrapGeneric<T extends core.SomeType>(inner: T): JsValue;\n\
         export declare function describe(thing: JsValue): string;\n\
         declare namespace core { interface SomeType {} }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return { toString: function () { return 'thing:' + name; } };\n\
         }\n\
         function wrapGeneric(inner) {\n\
             return { toString: function () { return 'wrapped(' + inner.toString() + ')'; } };\n\
         }\n\
         function describe(thing) { return thing.toString(); }\n\
         module.exports.makeThing = makeThing;\n\
         module.exports.wrapGeneric = wrapGeneric;\n\
         module.exports.describe = describe;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing, wrapGeneric, describe } from "generic-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped = wrapGeneric(thing);
    console.log(describe(wrapped));
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
        "wrapped(thing:gadget)\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// End-to-end regression for the bug that turned out to be blocking real
/// zod, not the two generic-declaration gaps fixed just above: zod
/// exports a schema-builder function literally named `undefined`
/// (`z.undefined()`), and `wrap_as_commonjs_module`'s `module.exports`
/// -> `globalThis` copy loop (thaw-bridge) used to abort entirely the
/// moment one key's assignment threw (`globalThis.undefined` is
/// non-writable) -- silently dropping every export enumerated after it
/// too, `after` included, even though it has nothing to do with
/// `undefined`. See
/// `a_key_that_cannot_bind_to_globalthis_does_not_block_later_exports`
/// (thaw-bridge's own tests) for the unit-level regression coverage;
/// this is the same bug reproduced with a real `--use`d package and a
/// real import, the shape that actually surfaced it. Both functions
/// return `JsValue` (an opaque, un-JSON-representable handle, matching
/// real zod's own schema-builder return shape) rather than a plain
/// string -- a `Fallback` function simple enough to synthesize a
/// numeric/string result gets JIT-compiled directly by thaw itself
/// (`jit_export`) and so never actually loads `bundle.js` into QuickJS
/// at all, which would silently skip the very code path this test
/// means to exercise.
#[test]
fn an_export_literally_named_undefined_does_not_block_later_exports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function undefined(): JsValue;\n\
         export declare function after(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "exports.undefined = function() { return { toString: function() { return 'u'; } }; };\n\
         exports.after = function(name) { return { toString: function() { return '[' + name + ']'; } }; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { after } from "undef-kit";
function main(): void {
    const r = after("hi");
    console.log(r);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "[hi]\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A *second*, more insidious bug from the same root cause as the test
/// above -- found while confirming real zod's own `z.undefined()`
/// works (it still doesn't; see [[project_npm_interop_gaps_2]] for why
/// that part is architectural, not fixed here). `typed_dynamic_bare_
/// alias` (the bare/qualified-call-syntax fix) generates a plain,
/// bare-named top-level declaration for *every* Fallback function
/// unconditionally, regardless of whether it's actually imported --
/// including a function literally named `undefined`. thaw-hir's own
/// `Expr::Ident` lowering treats a bare reference to `undefined`
/// specially (the JS literal) *unless* `self.signatures` -- a flat,
/// whole-program table -- already has a real entry under that exact
/// name, in which case it's treated as a reference to *that* function
/// value instead. Since `self.signatures` has no per-call-site
/// disambiguation, declaring a bare `function undefined(...)` *anywhere*
/// silently broke `!= undefined`/`=== undefined` comparisons
/// *everywhere else in the compiled program* -- including inside
/// thaw's own generated arity-dispatch wrapper's `param != undefined`
/// optional-parameter guard for a *different*, otherwise-uninvolved
/// function, which crashed with "numeric conversion is not defined for
/// native type Optional(...)" the moment it tried comparing a real
/// argument against what it thought was the `undefined` literal but was
/// actually a function value.
///
/// Fixed by skipping the bare (non-qualified) form entirely for a name
/// thaw-hir gives this special global meaning to (`undefined`, `NaN`,
/// `Infinity` -- see `shadows_a_thaw_literal_identifier`'s own doc
/// comment) in both `typed_dynamic_bare_alias`'s caller and the older,
/// untyped `generate_shim`/`generate_native_addon_shim` fallback (which
/// has the exact same risk on its own, independent of the newer bare-
/// alias machinery). The package-qualified alias is unaffected --
/// unrelated to this test, since it can never collide with a bare
/// literal reference.
///
/// Exercises exactly the failure shape: a package exports something
/// under the literal name `undefined` (with an optional parameter, so
/// its own wrapper needs a `!= undefined` guard) *and* a second,
/// unrelated function whose own optional-parameter guard needs the
/// *real* `undefined` literal to keep working, *and* the user's own
/// code compares an unrelated value against `undefined` directly --
/// all three used to be silently corrupted by the mere presence of the
/// `undefined`-named export, even without ever calling it.
#[test]
fn an_export_literally_named_undefined_does_not_corrupt_undefined_comparisons_elsewhere() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-comparisons-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Params { message?: string; }\n\
         export declare function undefined(params?: string | Params): string;\n\
         export declare function safe(value?: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.undefined = function(params) { return 'ok:' + JSON.stringify(params ?? null); };\n\
         module.exports.safe = function(value) { return value === undefined ? -1 : value * 2; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as nk from "undef-kit2";
function main(): void {
    const x: number | undefined = undefined;
    console.log(x === undefined);
    console.log(nk.safe());
    console.log(nk.safe(5));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "true\n-1\n10\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A Fallback function literally named `undefined` (real example: zod's
/// own `z.undefined()`) used to be uncallable outright, in addition to
/// the comparison-corruption bug the test above fixes: the runtime
/// dynamic-dispatch mechanism binds every export onto `globalThis` by
/// its own JS-side key first (`generate_module_init`'s own alias-
/// capture snippet used to read `globalThis.{bare_name}` directly), and
/// `globalThis.undefined` can never be reassigned in *any* JS engine --
/// a real ECMAScript restriction, not a thaw bug -- so the qualified key
/// the actual dynamic call looks up by never got bound to anything at
/// all for this one name, no matter what the TS-side declaration looked
/// like (`callDynamic("pkg::undefined", ...)`'s own runtime lookup found
/// nothing).
///
/// Fixed by reading `globalThis.module.exports.{bare_name}` first
/// instead -- `globalThis.module` still holds *this* package's own
/// fresh `{ exports: {} }` at the exact point this capture runs (nothing
/// else has run in between), so `module.exports.undefined` is a
/// perfectly ordinary object-property lookup, immune to the
/// `globalThis.undefined` restriction, regardless of what `bare_name`
/// is. Falls back to the old `globalThis.{bare_name}` read only when
/// that property lookup finds nothing (the one shape it doesn't cover:
/// a CommonJS package whose whole `module.exports`, not a property of
/// it, is the single exported function).
#[test]
fn an_export_literally_named_undefined_is_actually_callable() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-undefined-export-callable-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("undef-kit3");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export interface Params { message?: string; }\n\
         export declare function undefined(params?: string | Params): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.undefined = function(params) { return 'ok:' + JSON.stringify(params ?? null); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as nk from "undef-kit3";
function main(): void {
    console.log(nk.undefined());
    console.log(nk.undefined("hello"));
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
        "ok:null\nok:\"hello\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A `JsValue` receiver (a Fallback return value with no compiled class
/// behind it, e.g. zod's `z.object(...)` returning a live `ZodObject`)
/// couldn't have any of its own methods called at all --
/// `schema.safeParse(data)` failed to even build ("call to unknown
/// function `schema.safeParse`"), since thaw-hir's member-call lowering
/// only recognized a receiver typed as a known native *class*, erroring
/// for anything else instead of falling through to `callDynamicMethod`
/// (an existing low-level intrinsic for calling a named method on a
/// retained value by handle -- previously only a manual escape hatch,
/// never actually wired up to ordinary `.method(...)` syntax).
/// `lower_dynamic_value_method_call` (thaw-hir) fixes this, reusing the
/// same `Json`-laundering `coerce_to_declared` already does everywhere
/// else to build the method's argument array. Exercises both a bound
/// and a fully inline (unbound, chained straight into a property
/// access) call -- the inline shape needs its own fix too, see
/// `console_log_does_not_crash_on_an_inline_dynamic_method_call` below.
#[test]
fn a_method_can_be_called_on_a_jsvalue_returned_by_a_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-method-call-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("method-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return {\n\
                 describe: function() { return { text: 'thing:' + name }; },\n\
                 rename: function(next) { return { text: 'renamed:' + next }; }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "method-kit";
function main(): void {
    const thing = makeThing("gadget");
    const described = thing.describe();
    console.log(described.text);
    console.log(thing.rename("widget").text);
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
        "\"thing:gadget\"\n\"renamed:widget\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `console.log`'s own argument-type lookup (`expr_hir_type`, thaw-llvm)
/// had no entry for `callDynamicMethod` or its sibling manual dynamic-
/// value intrinsics (`getDynamicValue`, `constructDynamicValue`, ...) --
/// thaw-hir's `infer_expr_type` already special-cased these same names,
/// but thaw-llvm's own, separate copy of that classification didn't, so
/// it fell through to `None`. Printing a *bound* result
/// (`const r = thing.describe(); console.log(r);`) worked fine, but
/// passing the call *inline* (no binding) crashed with a segfault --
/// `compile_console_values` mishandling a value it thought had no type.
/// Only reachable at all once `lower_dynamic_value_method_call` (the
/// fix above) started actually generating an inline `callDynamicMethod`
/// call from ordinary `.method()` syntax for the first time.
#[test]
fn console_log_does_not_crash_on_an_inline_dynamic_method_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-inline-dynamic-method-console-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("inline-method-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing() {\n\
             return { describe: function() { return 'no-args-ok'; } };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "inline-method-kit";
function main(): void {
    const thing = makeThing();
    console.log(thing.describe());
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
        "\"no-args-ok\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// `callDynamicMethod` (the fix above routes `.method()` syntax through
/// it) always treats its result as `Json` -- fine for a method that
/// returns plain data (zod's own `.safeParse(...)`), but a method that
/// returns *another* live object (a hypothetical chained schema-builder
/// method returning another schema instance) used to silently produce
/// `{}` instead (a function-only object JSON-stringifies to that)
/// rather than a usable `JsValue`, with no way to ask for the real
/// handle. Fixed by choosing between `callDynamicMethod` and the new
/// `callDynamicMethodHandle` (which retains the result as a handle via
/// `thaw_js_call_method_handle_result` instead of JSON-decoding it)
/// based on this call's own expected-type hint -- the same mechanism an
/// ambiguous `let`/`const` initializer's own type annotation already
/// resolves elsewhere. Exercises both defaults in the same program: an
/// annotated `const wrapped: JsValue = thing.wrap();` gets the real
/// handle (and can have a further method called on *that*, recursing
/// back into the same lowering), while an unannotated call to the same
/// method keeps the old, unchanged behavior.
#[test]
fn a_method_can_return_a_jsvalue_when_the_call_site_asks_for_one() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-returning-method-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return {\n\
                 describe: function() { return 'thing:' + name; },\n\
                 wrap: function() {\n\
                     return { describe: function() { return 'wrapped(thing:' + name + ')'; } };\n\
                 }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit";
function main(): void {
    const thing = makeThing("gadget");
    const wrapped: JsValue = thing.wrap();
    console.log(wrapped.describe());
    const unannotated = thing.wrap();
    console.log(unannotated);
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
        "\"wrapped(thing:gadget)\"\n{}\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// An unannotated method call used as an *object literal field value*
/// (not bound to a `const`, not itself chained further) needs the exact
/// same "keep the real handle" default as a chained-method-call
/// receiver already gets -- real example: zod's own `object({ ...,
/// nickname: string().optional() })`. Before this fix, `.optional()`'s
/// receiver being `JsValue`-typed didn't matter: with no annotation and
/// no further chaining, the field defaulted to the plain JSON-decoding
/// behavior (see `a_method_can_return_a_jsvalue_when_the_call_site_
/// asks_for_one` above), discarding the real handle. That produced an
/// object literal with a field typed plain `Json` holding a content-
/// free snapshot (a schema-builder instance's own state lives behind
/// methods, not serializable fields) -- which a *generic* Fallback
/// function receiving it as part of its inferred type parameter can't
/// specialize for at all (`supports_generic_native_layout` has no case
/// for `Json`, by design: confirmed by direct experiment that loosening
/// it just trades this clean compile error for real zod's own internal
/// validation throwing on the far side of the dynamic call once it
/// doesn't recognize the snapshot as a real schema -- a silent,
/// message-less `exit(1)`).
///
/// Fixed in `lower_object_lit_field_value` (thaw-hir): a field value
/// that's a method call whose receiver is already known to be
/// `JsValue`-typed gets `Some(&HirType::JsValue)` as its own expected-
/// type hint, the same way this session's chained-method-call fix
/// already does for a method call used as *another* method call's own
/// receiver. Verified end-to-end: the field's real handle (with its own
/// methods) survives being embedded in an object literal, passed
/// through a generic Fallback function, and read back out later.
#[test]
fn an_unannotated_method_call_used_as_an_object_literal_field_keeps_its_real_handle() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-object-lit-field-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("shape-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function str(): JsValue;\n\
         export declare function build<T>(shape: T): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.str = function() {\n\
             return {\n\
                 optional: function() {\n\
                     return { toString: function() { return 'optional-str'; } };\n\
                 }\n\
             };\n\
         };\n\
         module.exports.build = function(shape) {\n\
             return {\n\
                 describeNickname: function() {\n\
                     return shape.nickname && typeof shape.nickname.toString === 'function'\n\
                         ? shape.nickname.toString()\n\
                         : 'lost:' + JSON.stringify(shape.nickname);\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { build, str } from "shape-kit";
function main(): void {
    const shape = build({ nickname: str().optional() });
    console.log(shape.describeNickname());
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
        "\"optional-str\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A generic Fallback function whose inferred type parameter is an
/// *array* of `JsValue` (real example: zod's own `union<T extends
/// readonly core.SomeType[]>(options: T): ZodUnion<T>`, called with
/// `[z.string(), z.number()]`, an array literal of two plain schema-
/// builder function calls) used to fail to specialize outright
/// ("cannot specialize for native layout Array(JsValue)") --
/// `supports_generic_native_layout` (thaw-hir) hardcoded its `Array`
/// case to accept only `F64` elements, unlike `Tuple`/`Object`, which
/// already recursed into every element/field's own type. Confirmed
/// first, directly, that a plain non-generic `JsValue[]` parameter
/// already marshals correctly as an ordinary Fallback argument (unlike
/// the earlier `Object`/`Json`-field gap, which really did mask an
/// unimplemented codegen path) -- so this was genuinely just an
/// unnecessarily narrow check, fixed by making `Array` recurse the same
/// way `Tuple` already does.
#[test]
fn a_generic_call_can_specialize_for_an_array_of_jsvalue_elements() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-generic-array-of-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("union-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function str(): JsValue;\n\
         export declare function num(): JsValue;\n\
         export declare function pick<T>(options: T): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.str = function() {\n\
             return { describe: function() { return 'str'; } };\n\
         };\n\
         module.exports.num = function() {\n\
             return { describe: function() { return 'num'; } };\n\
         };\n\
         module.exports.pick = function(options) {\n\
             return {\n\
                 describeAll: function() {\n\
                     return options.map(function(o) { return o.describe(); }).join(',');\n\
                 }\n\
             };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { str, num, pick } from "union-kit";
function main(): void {
    const picked = pick([str(), num()]);
    console.log(picked.describeAll());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"str,num\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// `if`/`while`/`do`/`for`/the ternary's own test/`!x` all used to
/// require an exact `boolean` condition, with no truthiness coercion at
/// all -- real JS lets *any* value be a condition (`truthiness_expr`
/// already backed `Boolean(x)` and `console.assert(x)`, but these six
/// spots never routed through it, each calling `expect_type(&HirType::
/// Bool, ...)` directly instead). Real example: `if (result.success)`
/// against a dynamic-call result's own `Json`-typed `.success` field
/// (`schema.safeParse(...).success`, real zod) -- used to fail outright
/// ("if condition has type Json, expected Bool") even though the exact
/// same value printed or compared fine on its own. Fixed by routing all
/// six through a shared `lower_condition_expr`/direct `truthiness_expr`
/// call instead.
#[test]
fn an_if_condition_accepts_a_json_value_via_truthiness_coercion() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-if-condition-truthiness-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("result-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeResult(ok: boolean): JsValue;\nexport declare function makeValue(ok: boolean): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeResult = function(ok) {\n\
             return { check: function() { return { success: ok }; } };\n\
         };\n\
         module.exports.makeValue = function(ok) { return ok ? { value: 1 } : null; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeResult, makeValue } from "result-kit";
function main(): void {
    console.log(makeValue(false) ? "truthy" : "falsy");
    console.log(makeValue(true) ? "truthy" : "falsy");
    const r = makeResult(true).check();
    if (r.success) {
        console.log("yes");
    } else {
        console.log("no");
    }
    if (!r.success) {
        console.log("negated-yes");
    } else {
        console.log("negated-no");
    }
    const bad = makeResult(false).check();
    console.log(bad.success ? "truthy" : "falsy");
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
        "falsy\ntruthy\nyes\nnegated-no\nfalsy\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A plain property *read* (no call at all) on a `JsValue` receiver used
/// to be entirely unsupported ("unsupported property access `.field` on
/// a value of type JsValue") -- only a *method call* on a `JsValue` was
/// wired up to ordinary syntax (`lower_dynamic_value_method_call`, an
/// earlier session). Real motivating example: real zod's own
/// `ZodError.issues` is deliberately a *non-enumerable* own property (so
/// pretty-printing the error via its own lazy `.message` getter doesn't
/// eagerly serialize every issue) -- meaning an *unannotated* method
/// call's own default JSON-snapshot behavior (a JSON encode can only
/// ever capture enumerable properties) silently loses `.issues`
/// entirely, and there was no other way to reach it at all.
///
/// Fixed by wiring `.property` syntax on a `JsValue` receiver to the
/// existing `getDynamicProperty` intrinsic (`thaw_js_get_property_
/// result`, thaw-quickjs) -- previously only a manual escape hatch,
/// unused by ordinary syntax, the same way `callDynamicMethod` was
/// before *it* got wired up. Reads the property by plain lookup, not by
/// enumeration, so it finds a non-enumerable property correctly.
/// `getDynamicProperty` always hands back a real handle (no JSON-
/// decoding sibling to choose between), so a chained property read
/// (`bad.error.issues.length`) recurses back into the same lowering for
/// free once the outermost receiver is annotated `JsValue` -- confirmed
/// here with only the *outermost* `bad` explicitly annotated, not every
/// intermediate step.
#[test]
fn a_property_can_be_read_on_a_jsvalue_including_a_non_enumerable_one() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jsvalue-property-read-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("prop-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeThing = function() {\n\
             var inner = { visible: 'v', hidden: 'h' };\n\
             Object.defineProperty(inner, 'hidden', { value: 'h', enumerable: false });\n\
             return { detail: inner };\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "prop-kit";
function main(): void {
    const thing: JsValue = makeThing();
    console.log(readDynamicValue(thing.detail.visible));
    console.log(readDynamicValue(thing.detail.hidden));
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
        "\"v\"\n\"h\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A method called directly on the result of *another* method call, with
/// no intermediate `const` binding at all -- `z.string().min(2).max(10)
/// .safeParse(...)`, `dayjs(...).add(10, "day").format(...)` -- used to
/// fail to build ("unsupported member call target") past the very first
/// link. Two gaps, both in thaw-hir's `lower/invocations.rs`, fixed
/// together:
///
/// 1. The member-call receiver-type match had no case at all for a
///    receiver that's itself `<expr>.method(...)` (a `Call` whose callee
///    is a `Member`, not a plain `Ident`) -- only a *plain* function call
///    receiver (`dayjs(...).format(...)`, no further chaining) was
///    handled. Fixed by extracting the whole match into a proper
///    recursive method, `infer_member_receiver_type`: when the receiver
///    is itself a method call, it recurses into *that* call's own
///    receiver, and (since a dynamic method's real return type has no
///    declared shape to look up at all) assumes a method invoked on a
///    `JsValue` receiver also yields another `JsValue` -- matching the
///    same "more of the same object" convention real builder-style
///    chains (zod, dayjs) universally follow.
/// 2. Even once the receiver's *type* was known, its actual *value*
///    still came back wrong: `lower_dynamic_value_method_call` lowered
///    its own receiver expression with no expected-type hint, so a
///    receiver that's itself a dynamic method call defaulted to the
///    JSON-decoding behavior (see `a_method_can_return_a_jsvalue_when_
///    the_call_site_asks_for_one` above) instead of a real handle --
///    silently wrong data, not a build error. Fixed by lowering the
///    receiver through `lower_expr_with_expected_type(_, Some(&HirType::
///    JsValue))` instead of the bare `lower_expr`, so every link in the
///    chain unconditionally asks its own receiver for a real handle,
///    recursively.
///
/// Exercises a three-link-deep fully inline chain (no binding anywhere)
/// to confirm both fixes recurse correctly through more than one hop.
#[test]
fn a_method_can_be_chained_directly_onto_another_methods_call_result() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-dynamic-method-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit2");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(name: string): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeThing(name) {\n\
             return {\n\
                 append: function(part) {\n\
                     return makeThing(name + '.' + part);\n\
                 },\n\
                 describe: function() { return 'thing:' + name; }\n\
             };\n\
         }\n\
         module.exports.makeThing = makeThing;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit2";
function main(): void {
    console.log(makeThing("root").append("a").append("b").describe());
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
        "\"thing:root.a.b\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A chain whose intermediate links are themselves "thenable" (an object
/// with a callable `.then`, not a genuine pending Promise) must not have
/// each intermediate result eagerly resolved via `Promise.resolve()` --
/// doing so invokes `.then` prematurely, executing the chain's side
/// effect before later links (`.where(...)`) ever apply. Real-world
/// example: drizzle-orm's `db.select().from(users).where(cond)`, where
/// every query-builder link is a thenable and only the fully-built,
/// awaited chain should execute the query. Reproduces the bug via a
/// minimal synthetic thenable builder instead of the real npm package.
#[test]
fn a_chained_methods_intermediate_thenable_result_is_not_resolved_early() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-thenable-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-thenable-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeDb(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeQuery(steps) {\n\
             return {\n\
                 from: function(part) {\n\
                     console.log('FROM:' + part);\n\
                     return makeQuery(steps.concat(['from:' + part]));\n\
                 },\n\
                 where: function(part) {\n\
                     console.log('WHERE:' + part);\n\
                     return makeQuery(steps.concat(['where:' + part]));\n\
                 },\n\
                 then: function(resolve, reject) {\n\
                     console.log('EXECUTED:' + steps.join(','));\n\
                     return Promise.resolve(steps.join(',')).then(resolve, reject);\n\
                 }\n\
             };\n\
         }\n\
         function makeDb() {\n\
             return {\n\
                 select: function() {\n\
                     console.log('SELECT');\n\
                     return makeQuery(['select']);\n\
                 }\n\
             };\n\
         }\n\
         module.exports.makeDb = makeDb;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeDb } from "chain-thenable-kit";
async function main(): Promise<void> {
    const db: JsValue = makeDb();
    const result: JsValue = await db.select().from("t").where("c");
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
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "SELECT\nFROM:t\nWHERE:c\nEXECUTED:select,from:t,where:c\nselect,from:t,where:c\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// The "no import at all" bare-name and package-qualified (`pkg.name(...)`)
/// call syntaxes used to always reach `generate_shim`'s own, always-
/// untyped `(argsArray: Json): Json` fallback -- fine for a single-
/// argument function (which happens to already look like `argsArray`),
/// but broken for any Fallback function taking more than one real
/// argument, since the untyped shape expects its *one* parameter to
/// already be a pre-packed JSON array, not real positional arguments.
/// Real example: `--use semver`, calling bare `semver.major("1.2.3",
/// true)` (or even just bare `major(...)`) with no import written at
/// all. `typed_dynamic_bare_alias` fixes this by forwarding both the
/// bare name and the package-qualified alias to whatever properly-typed
/// declaration `typed_dynamic_declaration` already produced for a plain
/// (non-generic, no `...rest`) Fallback function, instead of leaving
/// them pointed at the untyped fallback. No import anywhere in this
/// program at all -- both calls resolve purely through `--use`.
#[test]
fn bare_and_qualified_calls_with_no_import_reach_a_typed_multi_argument_fallback_function() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-bare-qualifier-typed-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("verkit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function major(version: string, precise?: boolean): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.major = function(version, precise) {\n\
             var n = parseInt(String(version).split('.')[0], 10);\n\
             return precise ? n + 0.5 : n;\n\
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"function main(): void {
    console.log(verkit.major("3.5.1", true));
    console.log(major("3.5.1"));
}
"#,
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
        &["verkit".to_string()],
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "3.5\n3\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A two-level member chain (`ns.coerce.number(...)`) through a nested-
/// namespace re-export (`export * as coerce from "...";`) -- real-world
/// example: zod v4's `z.coerce.number()`/`z.iso.datetime()`. Reuses the
/// package.d.ts shape thaw-registry's own flattening produces (see
/// `thaw_bridge::nested_namespace_members`'s doc comment): a synthesized
/// top-level function (`__thaw_ns_coerce_number`, deliberately sharing no
/// name with the package's own top-level `number`, exactly like real
/// zod's `coerce.number` vs. top-level `number`) plus a `declare
/// namespace coerce { export { ... }; }` block recording the real member
/// name it should be reachable under.
///
/// `bundle.js`'s runtime shape is the harder-to-get-right half of this:
/// `coerce.number` is a real *nested* object property (`module.exports =
/// { ..., coerce: { number: fn } }`), not a bare top-level one -- found
/// necessary because a function value reached only through a two-level
/// property chain (`module.exports.coerce.number`), when captured via a
/// *separate*, later `loadScript` call the way an ordinary cross-package
/// collision alias already is, becomes silently uninvokable through the
/// native `callDynamic` FFI boundary (no thrown exception, the whole
/// program just exits 1 with no output at all) despite remaining
/// perfectly callable from JS itself -- confirmed via a minimal, package-
/// agnostic repro. The fix captures a nested-namespace member from
/// *inside* the bundle's own wrapped script instead (right where the
/// ordinary `module.exports` -> `globalThis` copy loop already runs),
/// which this test exercises end to end.
#[test]
fn a_two_level_member_chain_through_a_nested_namespace_reexport_calls_correctly() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-nested-namespace-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("case-kit6");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "import * as ns from \"./lib\";\n\
         export * from \"./lib\";\n\
         export { ns, ns as default };\n\
         \n\
         export declare function number(): number;\n\
         export declare function __thaw_ns_coerce_number(): number;\n\
         declare namespace coerce {\n\
         \x20\x20\x20\x20export { __thaw_ns_coerce_number as number };\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { number: function() { return 0; }, coerce: { number: function() { return 42; } } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { ns } from "case-kit6";
function main(): void {
    console.log(ns.number());
    console.log(ns.coerce.number());
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A real, compiled (native) closure passed as an argument to a dynamic
/// (`JsValue`) method call -- real-world example: zod's `z.number().
/// refine((n: number) => n > 0, {...})`/`.transform(...)`, whose
/// predicate/mapper has no JSON representation at all (`value has type
/// Function([F64], Bool), expected Json`). `coerce_to_declared` (thaw-hir)
/// wraps it in a manual `registerNativeCallback` call, which thaw-llvm's
/// `compile_register_native_callback` turns into a live, retained
/// QuickJS-NG function value: reuses the existing (N-API-oriented but
/// backend-agnostic) `compile_napi_value_callback` adapter, then bridges
/// it into a real callable via a new `thaw_js_register_native_callback`
/// (thaw-quickjs).
///
/// `bundle.js`'s `check` method calls the predicate three times with
/// different arguments to confirm each call round-trips independently
/// (not just a one-shot capture), and the mapper case (`test3`-shaped,
/// folded into this same test) confirms a non-boolean return value
/// marshals correctly too.
///
/// Also confirms a `JsValue` nested inside a *method* call's own argument
/// array works, not just a top-level function call's (`compile_call_
/// dynamic_method`/`compile_call_dynamic_method_handle` previously never
/// set `compiling_quickjs_dynamic_arguments` around their own args
/// marshaling at all, unlike the typed ambient-declaration dispatch path
/// -- confirmed to reproduce the pre-fix "a dynamic (JsValue) value can
/// only be passed as an argument to another QuickJS-backed dynamic call"
/// error via a temporary revert).
#[test]
fn a_native_closure_can_be_passed_as_an_argument_to_a_dynamic_method_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { return pred(5) > 0; }, \
         map: function(pred) { return pred(1) + \",\" + pred(2) + \",\" + pred(3); } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    console.log(holder.check((n: number) => n > 0));
    console.log(holder.check((n: number) => n < 0));
    console.log(holder.map((n: number) => n * 10));
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
        "true\nfalse\n\"10,20,30\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure passed as a dynamic-method-call argument (same shape
/// as the test above), whose declared return type is `Promise<T>` --
/// real-world example: drizzle-orm's `sqlite-proxy` driver,
/// `drizzle(callback)`, where `callback` is always
/// `(sql, params, method) => Promise<{rows}>` since it wraps real I/O.
/// Found via drizzle bug-hunting: passing an `async` predicate crashed at
/// *build* time with `"cannot serialize collection element Promise(F64)
/// to JSON"`.
///
/// Root cause: `compile_napi_value_callback` (the callback bridge, reused
/// for both real N-API addons and this QuickJS Fallback path) pushes the
/// callback's raw return value into its JSON result array using the
/// callback's *declared* return type verbatim -- for an `async` callback
/// that's `HirType::Promise(inner)`, but nothing ever stripped the
/// `Promise` wrapper or drove it to resolution first, and
/// `compile_json_array_push_native` has no match arm for
/// `HirType::Promise` at all. Every `is_async` function -- named or an
/// inline async arrow -- always exposes the same real, resolvable
/// `ThawPromise`-pointer ABI regardless of whether its body actually
/// suspends (`discover_frame_async_functions`'s own documented
/// invariant), so the raw return value here is never a raw unwrapped
/// value in disguise -- just an unresolved promise. Fixed by teaching
/// `compile_napi_value_callback` to detect a `Promise`-typed `ret` and
/// drive it to its resolved value via `drive_promise_to_resolved_value`
/// (the value-taking half of the existing `compile_typed_blocking_await`,
/// already used for an ordinary typed `await` expression) before
/// marshaling the *resolved* type, not `Promise<T>`, to JSON. Confirmed
/// to reproduce the exact pre-fix build error via a temporary revert.
#[test]
fn a_promise_returning_native_closure_can_be_passed_as_a_dynamic_method_call_argument() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-async-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("async-callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n\
         interface Holder {\n\
         \x20\x20\x20\x20check(pred: (n: number) => Promise<number>): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { \
         Promise.resolve(pred(5)).then(function(result) { console.log('got:' + result); }); \
         } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "async-callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    holder.check(async (n: number): Promise<number> => n * 10);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "got:50\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The `async` predicate's `Promise<void>`-resolved counterpart (no
/// meaningful return value at all) -- confirms the null-result path
/// still works correctly once routed through the same Promise-unwrapping
/// branch (real-world example: zod's own `.superRefine((val, ctx) => {
/// ctx.addIssue(...); })`-shaped callbacks, if ever declared `async`).
#[test]
fn a_promise_void_returning_native_closure_argument_still_produces_a_null_result() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-async-void-native-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("async-void-callback-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): Holder;\n\
         interface Holder {\n\
         \x20\x20\x20\x20check(pred: (n: number) => Promise<void>): void;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { makeHolder: function() { \
         return { \
         check: function(pred) { \
         Promise.resolve(pred(5)).then(function(result) { console.log('got:' + result); }); \
         } \
         }; \
         } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder } from "async-void-callback-kit";
function main(): void {
    const holder: JsValue = makeHolder();
    holder.check(async (n: number): Promise<void> => { console.log(n); });
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\ngot:null\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A native closure passed as an argument to an *ordinary top-level
/// Fallback function call* -- not a method call on a `JsValue` receiver
/// (the shape the three tests above cover, `holder.check(pred)`). Real-
/// world example: drizzle-orm's `sqlite-proxy` driver, `drizzle(callback:
/// (sql, params, method) => Promise<{rows}>)` -- `drizzle` is a plain
/// `declare function`, not a method.
///
/// Found via drizzle bug-hunting: this built successfully but crashed at
/// *runtime* with a JS-side `TypeError: cb is not a function` -- the
/// callback argument was silently never written into the JSON args array
/// at all (`compile_typed_dynamic_call`, thaw-llvm's `dynamic_host.rs`,
/// only marshals a typed function argument for the `napi` backend with a
/// `JsValue` return; every other combination -- including any
/// Fallback/QuickJS call -- has a deliberate `continue` that skips the
/// argument's slot instead of erroring, so the real JS side saw `args[0]
/// === undefined`).
///
/// Root cause, two independent gaps found together:
///
/// 1. `typed_dynamic_declaration` (thaw-cli's `shims.rs`) rendered a
///    `Function`/`CallableFunction`-classified parameter as a real
///    callback type in the generated shim for *both* `napi` and
///    Fallback/QuickJS functions alike -- unlike its sibling
///    `supported_class_method_param` (used for class methods), which
///    already restricts this to the `napi` backend. Fixed by applying the
///    same restriction here: for a non-`napi` function, a
///    `Function`/`CallableFunction`-classified parameter is widened to
///    `Json` (matching how `DtsType::Unsupported` is already handled),
///    which routes it through `coerce_to_declared`(`declared == Json`)'s
///    `registerNativeCallback` bridge instead.
/// 2. Once routed there, this specific test still failed at runtime with
///    "JavaScript value handle registry is empty" -- unlike every other
///    test exercising this bridge, this program's *first-ever* touch of a
///    JsValue handle at all is registering the closure itself (no prior
///    `makeHolder(): JsValue`-style call to have bootstrapped anything
///    first). `retain_value` (thaw-quickjs's `api.rs`, the sole write path
///    into the realm's handle registry) assumed the registry (`__thaw_
///    value_handles`/`__thaw_value_handle_live`) already existed, unlike
///    its near-duplicate sibling `thaw_js_get_global`, which already
///    lazily created it on first use. Fixed by moving that lazy-creation
///    into `retain_value` itself (and simplifying `thaw_js_get_global` to
///    just call it), so *every* path that can be a program's first handle
///    registration self-heals the same way.
///
/// Confirmed to reproduce the pre-fix silent-wrong-output symptom (this
/// doesn't fail to *build* -- it must be asserted via the callback's own
/// observable side effect) via a temporary revert of both fixes together.
#[test]
fn a_native_closure_can_be_passed_as_an_argument_to_an_ordinary_fallback_function_call() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-plain-callback-arg-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-arg-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function invoke(cb: (x: number) => number): void;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         invoke: function(cb) { console.log('result:' + cb(21)); } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { invoke } from "callback-arg-kit";
function main(): void {
    invoke((x: number): number => x * 2);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "result:42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// An *optional* callback parameter on an ordinary top-level Fallback
/// function, called with it omitted -- real example: lodash's
/// `filter(collection: string | null | undefined, predicate?:
/// StringIterator<boolean>): string[];` (found while testing the earlier
/// lodash `default`-import fix against real lodash's own full `.d.ts`,
/// via `import * as _ from "lodash"; _.chunk(...)`, which crashed even
/// though `chunk` itself has no callback parameter at all).
///
/// Root cause: an optional callback parameter classifies as `Native(
/// Optional(Function(...)))` -- a *different* `Native` variant from the
/// bare `Native(Function(...))` case the test above covers, so it fell
/// through `typed_dynamic_declaration`'s (thaw-cli's `shims.rs`) widening
/// match arm unwidened. Separately, `typed_dynamic_bare_alias` (the
/// wrapper generated under a function's *bare* name, reached by a plain
/// named import like this test's) had its *own*, independent parameter-
/// type computation with no widening at all, `napi`-aware or not -- so
/// even fixing the first arm alone wasn't enough: calling the bare-name
/// alias with the optional parameter omitted still crashed, since
/// thaw-hir's omitted-trailing-optional-parameter machinery needed to
/// synthesize a value using *this* alias's own (still unwidened)
/// declared type. Both fixed the same way -- widened to `Json` for the
/// non-`napi` backend, matching the existing bare-`Function` case.
#[test]
fn an_optional_callback_parameter_omitted_at_the_call_site_does_not_crash() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-optional-callback-param-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("filter-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function filter(collection: string, predicate?: (char: string, index: number, s: string) => boolean): string[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         filter: function(collection, predicate) { \
         var out = []; \
         for (var i = 0; i < collection.length; i++) { \
         if (!predicate || predicate(collection[i], i, collection)) out.push(collection[i]); \
         } \
         return out; \
         } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { filter } from "filter-kit";
function main(): void {
    console.log(filter("hello").length);
    console.log(filter("hello", (char: string) => char === "l").length);
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "5\n2\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// A dynamic-call argument that's itself a method call chained off a
/// `JsValue` receiver, with no intermediate `const` binding at all --
/// real-world example: zod's `z.string().pipe(z.string().min(3))`,
/// where `.min(3)`'s own result (chained off `z.string()`) is passed
/// straight into `.pipe(...)`'s own argument position. Found via
/// bug-hunting after the native-callback-bridge work: `z.string().
/// pipe(z.string().min(3)).safeParse(...)` silently crashed (exit 1, no
/// output at all -- the same failure shape a missing `JsValue` capture
/// always produces here).
///
/// Root cause: `lower_dynamic_value_method_call`'s own argument-lowering
/// loop called plain `lower_expr` on each argument, with no expected-
/// type hint at all -- so an argument that's itself a further dynamic
/// method call defaulted to the ordinary JSON-decoding snapshot
/// behavior (a content-free `{}`), discarding its real handle, the
/// exact same failure mode `lower_object_lit_field_value` was fixed for
/// at a *different* sink point (an object-literal field, not a method-
/// call argument) earlier in [[project_npm_interop_gaps_2]]. Fixed by
/// giving each argument the same one-shot `Some(&HirType::JsValue)`
/// hint the receiver itself already gets.
///
/// `combine`'s own JS implementation calls a *method* (`.describe()`)
/// on its argument, not just reads a plain data field -- a JSON-
/// stringified snapshot would still carry plain fields like `.tag`
/// correctly (methods just don't serialize), so reading a field alone
/// wouldn't have caught the bug; calling a method the snapshot doesn't
/// have is what actually distinguishes a real live handle from a JSON
/// snapshot here. Confirmed to fail with the pre-fix silent-crash
/// symptom via a temporary revert.
#[test]
fn a_dynamic_call_argument_that_is_itself_a_chained_method_call_stays_live() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-chained-arg-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeThing(): Thing;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function build(n) { \
         return { \
         tag: n, \
         combine: function(other) { return build(this.tag + \":\" + other.describe()); }, \
         describe: function() { return this.tag; } \
         }; \
         } \
         module.exports = { makeThing: function() { return build(\"root\"); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeThing } from "chain-kit";
function main(): void {
    const a: JsValue = makeThing();
    const b: JsValue = a.combine(makeThing().combine(makeThing()));
    console.log(b.describe());
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
        "\"root:root:root\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A `superRefine`-shaped native callback -- real-world example: zod's
/// `z.object({...}).superRefine((val, ctx) => { ctx.addIssue(...); })` --
/// exercising four gaps found and fixed together while bug-hunting after
/// the basic native-callback-bridge work (item 10) landed:
///
/// 1. A callback parameter explicitly typed `JsValue` (`ctx`) -- the
///    bridge previously only marshaled plain JSON-representable
///    parameters into a callback; `compile_json_value_to_native`'s new
///    `HirType::JsValue` case, paired with a `jsvalue_param_mask`
///    thaw-llvm now threads through `thaw_js_register_native_callback`,
///    retains exactly the marked argument positions as a live handle
///    (encoded as the same `{"__thaw_js_handle_id__": N}` marker used
///    for the opposite direction) instead of naively `JSON.stringify`-
///    ing them.
/// 2. A *generic* top-level function's return value used as an inline
///    method-call receiver with no intermediate `const` at all
///    (`object({...}).superRefine(...)`) -- `infer_member_receiver_type`
///    used to exclude *every* generic callee's declared return type
///    (correct when substitution genuinely matters, e.g. `identity<T>(x:
///    T): T`), even when that declared return type was already `JsValue`
///    and thus substitution-independent (an unresolved interface
///    reference like `Schema<T>` can never become JSON-representable
///    no matter what `T` is).
/// 3. A `void`-returning callback (`(val, ctx) => { ctx.addIssue(...); }`,
///    no `return` at all) -- `compile_napi_value_callback`'s adapter used
///    to unconditionally require a real return value.
/// 4. A dynamic call made *from inside* a native callback that was
///    itself invoked *by* an outer dynamic call still on the stack
///    (`ctx.addIssue(...)`, called while the outer `.validate(...)` call
///    that triggered the callback hasn't returned yet) -- `with_context`
///    panicked ("RefCell already borrowed") on the second, reentrant
///    call. Fixed by reusing the `ActiveNapiContext` guard `install_
///    napi_bridge`'s own reentrant call already relies on (despite the
///    "napi" name, a generic "currently active `Ctx`" mechanism, not
///    N-API-specific) via a new `with_active_or_context` (thaw-quickjs).
///
/// `val`'s own numeric fields are read via `Json`/`Number(...)`, not
/// `JsValue` -- deliberately: `val` is plain, JSON-representable data
/// here (matching real zod's own `RefinementCtx` usage, where the
/// *validated value* is ordinary data and only `ctx` itself is a live
/// object with methods), confirming the fix doesn't force every
/// parameter to go through the handle path.
#[test]
fn a_superrefine_shaped_native_callback_with_a_jsvalue_context_parameter_works() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-superrefine-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("super-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function object<T>(shape: T): Schema<T>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeSchema() { \
         return { \
         superRefine: function(check) { \
         return { \
         validate: function(val) { \
         var ctx = { issues: [], addIssue: function(msg) { this.issues.push(msg); } }; \
         check(val, ctx); \
         return ctx.issues; \
         } \
         }; \
         } \
         }; \
         } \
         module.exports = { object: function(shape) { return makeSchema(); } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { object } from "super-kit";
function main(): void {
    const s: JsValue = object({ a: 1, b: 2 }).superRefine((val: Json, ctx: JsValue) => {
        if (Number(val.a) > Number(val.b)) {
            ctx.addIssue("a must be <= b");
        }
    });
    console.log(JSON.stringify(s.validate({ a: 1, b: 2 })));
    console.log(JSON.stringify(s.validate({ a: 3, b: 2 })));
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
        "[]\n[\"a must be <= b\"]\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_async_native_callback_can_reenter_quickjs_while_being_polled() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-reentrant-native-promise-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("callback-promise-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeHolder(): JsValue;\n\
         export declare function run(callback: () => Promise<void>): Promise<void>;\n\
         export declare function runRejected(callback: () => Promise<void>): Promise<string>;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.makeHolder = function() { return { check: function(callback) { return callback('held'); } }; };\n\
         module.exports.run = function(callback) { return new Promise(function(resolve, reject) { setTimeout(function() { Promise.resolve(callback()).then(resolve, reject); }, 0); }); };\n\
         module.exports.runRejected = function(callback) { return Promise.resolve(callback()).then(function() { return 'unexpected'; }, function() { return 'rejected'; }); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeHolder, run, runRejected } from "callback-promise-kit";
const holder: JsValue = makeHolder();
async function main(): Promise<void> {
    await run(async (): Promise<void> => {
        await new Promise<void>((resolve): void => resolve());
        console.log(holder.check((value: string): string => value));
    });
    console.log(await runRejected(async (): Promise<void> => { throw "boom"; }));
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"held\"\nrejected\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Real-world example: hono's `app.get(path, (c) => c.text(...))` --
/// registering a route handler that receives a `JsValue` "context" and
/// returns a live `Response`-shaped object, then reading `.status` off
/// the result of `app.request(path)` (hono's own synchronous testing
/// helper, which invokes the registered handler directly).
///
/// This previously read back `undefined` instead of `200`, with no
/// error at all -- the handler's `return c.text(...)` silently lowered
/// the dynamic method call through the untyped/JSON-decoding dispatch
/// (`callDynamicMethod`, not `callDynamicMethodHandle`), so hono's mock
/// router received a content-free `{}` snapshot instead of the real
/// `Response` handle, and read `.status` off *that*. Two independent
/// gaps, both in thaw-hir, both fixed here:
///
/// 1. `Stmt::Return` (statements/lowering.rs) lowered its argument with
///    plain `lower_expr`, never passing the function's own declared/
///    inferred `ret_type` through as an expected-type hint -- so even an
///    arrow explicitly annotated `(c: JsValue): JsValue => { return c.
///    text(...); }` failed outright ("value has type Json, expected
///    JsValue") until fixed to route through `lower_expr_with_expected_
///    type`.
/// 2. An arrow with *no* return-type annotation at all (the realistic
///    hono handler shape, `(c: JsValue) => c.text(...)` or `(c: JsValue)
///    => { return c.text(...); }`) defaulted its own inferred `ret_type`
///    to `HirType::Dynamic`, starving fix #1's hint of anything useful
///    to propagate. Fixed with a new one-shot `expected_arrow_return_
///    hint`, set by `lower_dynamic_value_method_call`'s own argument-
///    lowering loop (the same place that already hints a chained-call
///    argument as `JsValue`) whenever the argument being lowered is
///    itself an arrow -- letting an unannotated callback passed straight
///    into a dynamic method call default its own return type to
///    `JsValue` instead of `Dynamic`, so a bare tail-position dynamic
///    method call inside it (covering both the braced-body path via fix
///    #1, and the implicit-return expression-body path, which needed its
///    own `lower_expr_with_expected_type` call alongside `Stmt::Return`'s)
///    keeps its real handle.
///
/// Exercises all three handler shapes side by side against independent
/// router instances: an explicit `: JsValue` return annotation, a
/// braced body with no annotation, and an unannotated implicit-return
/// expression body -- confirmed via a temporary revert of both fixes to
/// reproduce the original `undefined` symptom for all three.
#[test]
fn a_dynamic_callback_argument_returning_a_jsvalue_keeps_it_live_even_when_unannotated() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-callback-return-jsvalue-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("router-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function makeRouter(): JsValue;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function makeRouter() { \
         var handler = null; \
         return { \
         get: function(path, h) { handler = h; }, \
         request: function(path) { \
         var ctx = { text: function(body) { return { status: 200, body: body }; } }; \
         return handler(ctx); \
         } \
         }; \
         } \
         module.exports = { makeRouter: makeRouter };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { makeRouter } from "router-kit";
function main(): void {
    const appA: JsValue = makeRouter();
    appA.get('/', (c: JsValue): JsValue => { return c.text('one'); });
    console.log(appA.request('/').status);

    const appB: JsValue = makeRouter();
    appB.get('/', (c) => { return c.text('two'); });
    console.log(appB.request('/').status);

    const appC: JsValue = makeRouter();
    appC.get('/', (c) => c.text('three'));
    console.log(appC.request('/').status);

    const appD: JsValue = makeRouter();
    appD.get('/', (c) => c.text(c.missing || 'fallback'));
    console.log(appD.request('/').body);

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
        "200\n200\n200\n\"fallback\"\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "\"HELLO\"\n");
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
fn fallback_object_callback_is_contextually_typed_and_runs_as_native_code() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-contextual-object-callback-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("server-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export type Handler<T = unknown> = (request: T, toolkit: T, error?: T) => T;\n\
         export interface Route<T = unknown> { method: string; path: string; handler?: Handler<T> | object | undefined; }\n\
         export declare class Server { route<T = unknown>(route: Route<T> | Route<T>[]): void; }\n\
         export declare function server(): Server;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function Server() {} Server.prototype.route = function(route) { console.log(route.handler({ path: route.path }, { response: function(value) { return value; } })); }; function server() { return new Server(); } module.exports = { Server: Server, server: server };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import * as Kit from "server-kit";
function main(): void {
    const server = Kit.server();
    server.route({ method: "GET", path: "/jit", handler: (request, toolkit) => toolkit.response(request.path) });
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
