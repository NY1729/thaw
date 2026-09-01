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
        "export interface Result { value: number; remaining: number; }\nexport declare function orPop(values: number[]): number;\nexport declare function andPop(values: number[]): number;\nexport declare function nested(values: number[]): number;\nexport declare function object(values: number[]): Result;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orPop = values => values.pop() || values.pop(); module.exports.andPop = values => values.pop() && values.pop(); module.exports.nested = values => values.pop() || values.pop() || values.pop(); module.exports.object = values => ({ value: values.pop() || values.pop(), remaining: values.length });\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orPop, andPop, nested, object } from 'jit-logical';\nfunction main(): void { const orHit = [1, 2, 3]; console.log(orPop(orHit) + ':' + orHit.length); const orMiss = [1, 2, 0]; console.log(orPop(orMiss) + ':' + orMiss.length); const andMiss = [1, 2, 0]; console.log(andPop(andMiss) + ':' + andMiss.length); const andHit = [1, 2, 3]; console.log(andPop(andHit) + ':' + andHit.length); const nestedValues = [1, 4, 0, 0]; console.log(nested(nestedValues) + ':' + nestedValues.length); const nestedHit = [1, 2, 3]; console.log(nested(nestedHit) + ':' + nestedHit.length); const objectValues = [1, 5, 0]; const result = object(objectValues); console.log(result.value + ':' + result.remaining + ':' + objectValues.length); }\n",
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
        "3:2\n2:1\n0:2\n2:1\n4:1\n3:2\n5:1:1\n"
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
        "export declare function add(left: number, right: number): number;\nexport declare function accumulate(left: number, right: number): number;\nexport declare function choose(left: number, right: number): number;\nexport declare function sub(left: number, right: number): number;\nexport declare function double(value: number): number;\nexport declare function negate(value: number): number;\nexport declare function mask(value: number): number;\nexport declare function fallbackOr(value: number, fallback: number): number;\nexport declare function guard(value: number, result: number): number;\nexport declare function magnitude(value: number): number;\nexport declare function roundedRoot(value: number): number;\nexport declare function logarithm(value: number): number;\nexport declare function integerMath(left: number, right: number): number;\nexport declare function numericPredicates(value: number): boolean;\nexport declare function stringPredicate(value: string): boolean;\nexport declare function stringLength(value: string): number;\nexport declare function stringLess(left: string, right: string): boolean;\nexport declare function greet(value: string): string;\nexport declare function matches(value: string): boolean;\nexport declare function find(value: string): number;\nexport declare function transform(value: string): string;\nexport declare function clean(value: string): string;\nexport declare function first(value: string, index: number): string;\nexport declare function code(value: string, index: number): number;\nexport declare function power(base: number, exponent: number): number;\nexport declare function less(left: number, right: number): boolean;\nexport declare function negateFlag(value: boolean): boolean;\nexport declare function remainder(left: number, right: number): number;\nexport declare function minimum(a: number, b: number, c: number): number;\nexport declare function maximum(a: number, b: number, c: number): number;\nexport declare function sum3(a: number, b: number, c: number): number;\nexport declare function answer(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "'use strict'; const SCALE = 2; const double = value => value * SCALE; module.exports = { add: function(left, right) { const sum = left + right; const doubled = sum * SCALE; const delta = left - right; if (left < right) return doubled; return delta; }, accumulate: function(left, right) { let total = left; total += right; total *= SCALE; total--; return total; }, choose: function(left, right) { if (left) { if (right < 0) return 1; return 2; } else if (right) return 3; else return 4; }, sub: (left, right) => left - right, double, negate: value => -value, mask: value => (value & 255) ^ 42, fallbackOr: (value, fallback) => value || fallback, guard: (value, result) => value && result, magnitude: value => Math.abs(value), roundedRoot: value => finish(Math.sqrt(value)), logarithm: value => value + Math.round(Math.PI), integerMath: (left, right) => Math.imul(left, right) + Math.clz32(1) + Math.fround(1), numericPredicates: value => Number.isSafeInteger(value), stringPredicate: value => Number.isNaN(value) || isNaN(value), stringLength: value => value.length, stringLess: (left, right) => left.localeCompare(right) < 0, greet: value => `hello, ${value}!`, matches: value => value.isWellFormed() && value.startsWith('pre', 0) && value.endsWith('fix', 6) && value.includes('ref', 1), find: value => value.indexOf('😀', 1) + value.lastIndexOf('😀', 3), transform: value => value.toWellFormed().toLowerCase().toUpperCase(), clean: value => value.trim().repeat(2).slice(0, 2).substring(1, 0).padStart(3, '0').padEnd(4, '1').replace('0', 'a').replaceAll('1', 'b'), first: (value, index) => value.charAt(index), code: (value, index) => value.charCodeAt(index), power: (base, exponent) => Math.pow(base, exponent), less: (left, right) => left < right, negateFlag: value => !value, remainder: (left, right) => left % right, minimum: (a, b, c) => Math.min(a, b, c), maximum: (a, b, c) => Math.max(a, b, c), sum3: (a, b, c) => a + b + c, answer: () => Math.round(Math.PI) + 39 }; function finish(value) { return Math.round(value); }\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { add, accumulate, choose, sub, double, negate, mask, fallbackOr, guard, magnitude, roundedRoot, logarithm, integerMath, stringLength, stringLess, greet, matches, find, transform, clean, first, code, power, less, negateFlag, remainder, minimum, maximum, sum3, answer } from 'jit-math';\nfunction main(): void { console.log(add(2, 19) + accumulate(1, 2) + choose(1, -1) + sub(1, 1) + double(0) + negate(0) + mask(42) + fallbackOr(0, 42) + guard(0, 42) + magnitude(-1) + roundedRoot(0) + logarithm(8) + integerMath(3, 4) + stringLength('😀') + (stringLess('a', 'b') ? 1 : 0) + stringLength(greet('x')) + (matches('prefix') ? 1 : 0) + find('😀a😀') + stringLength(transform('Ab')) + stringLength(clean(' x ')) + stringLength(first('abc', 0)) + code('A', 0) + power(2, 0) + (less(1, 2) && negateFlag(false) ? 1 : 0) + remainder(5, 2) + minimum(-1, 0, 1) + maximum(-1, 0, 1) + sum3(0, 0, 0) + answer() - 240); }\n",
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
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
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
                                    write: (chunk: string) => boolean;
                                    end: (chunk: string) => boolean;
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
                    response: { statusCode: number; setHeader: (name: string, value: string) => boolean; write: (chunk: string) => boolean; end: (chunk: string) => boolean }
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
