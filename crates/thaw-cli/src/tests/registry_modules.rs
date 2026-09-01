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
        "export declare function safe(value: number): boolean;\nexport declare function stringNan(value: string): boolean;\nexport declare function float(value: string): number;\nexport declare function integer(value: string, radix: number): number;\nexport declare function fixed(value: number, digits: number): string;\nexport declare function precision(value: number, digits: number): string;\nexport declare function radix(value: number, base: number): string;\nexport declare function exponential(value: number, digits: number): string;\nexport declare function exponentialShortest(value: number): string;\nexport declare function isList(values: number[]): boolean;\nexport declare function isValue(value: number): boolean;\nexport declare function count(values: number[]): number;\nexport declare function pick(values: number[], index: number): number | undefined;\nexport declare function pickText(values: string[], index: number): string | undefined;\nexport declare function pickBool(values: boolean[], index: number): boolean | undefined;\nexport declare function getNumber(values: number[], index: number): number | undefined;\nexport declare function getText(values: string[], index: number): string | undefined;\nexport declare function getBool(values: boolean[], index: number): boolean | undefined;\nexport declare function hasNumber(values: number[], needle: number, from: number): boolean;\nexport declare function hasText(values: string[], needle: string, from: number): boolean;\nexport declare function boolIndex(values: boolean[], needle: boolean, from: number): number;\nexport declare function lastNumber(values: number[], needle: number): number;\nexport declare function lastText(values: string[], needle: string, from: number): number;\nexport declare function lastBool(values: boolean[], needle: boolean, from: number): number;\nexport declare function formatNumbers(values: number[], separator: string): string;\nexport declare function formatText(values: string[]): string;\nexport declare function formatBools(values: boolean[], separator: string): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { safe: value => Number.isSafeInteger(value), stringNan: value => Number.isNaN(value) || isNaN(value), float: value => Number.parseFloat(value), integer: (value, radix) => parseInt(value, radix), fixed: (value, digits) => value.toFixed(digits), precision: (value, digits) => value.toPrecision(digits), radix: (value, base) => value.toString(base), exponential: (value, digits) => value.toExponential(digits), exponentialShortest: value => value.toExponential(), isList: values => Array.isArray(values), isValue: value => Array.isArray(value), count: values => values.length, pick: (values, index) => values.at(index), pickText: (values, index) => values.at(index), pickBool: (values, index) => values.at(index), getNumber: (values, index) => values[index], getText: (values, index) => values[index], getBool: (values, index) => values[index], hasNumber: (values, needle, from) => values.includes(needle, from), hasText: (values, needle, from) => values.includes(needle, from), boolIndex: (values, needle, from) => values.indexOf(needle, from), lastNumber: (values, needle) => values.lastIndexOf(needle), lastText: (values, needle, from) => values.lastIndexOf(needle, from), lastBool: (values, needle, from) => values.lastIndexOf(needle, from), formatNumbers: (values, separator) => values.join(separator), formatText: values => values.toString(), formatBools: (values, separator) => values.join(separator) };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { safe, stringNan, float, integer, fixed, precision, radix, exponential, exponentialShortest, isList, isValue, count, pick, pickText, pickBool, getNumber, getText, getBool, hasNumber, hasText, boolIndex, lastNumber, lastText, lastBool, formatNumbers, formatText, formatBools } from 'jit-predicates';\nfunction main(): void { console.log(safe(42)); console.log(stringNan('x')); console.log(float('  -12.5px')); console.log(integer('11', 2)); console.log(fixed(12.5, 2)); console.log(precision(12.5, 3)); console.log(radix(255, 16)); console.log(exponential(12.6, 1)); console.log(exponentialShortest(12.5)); console.log(isList([1])); console.log(isValue(1)); console.log(count([1, 2, 3])); console.log(pick([1, 2, 3], -1) ?? 0); console.log(pickText(['a', 'b'], -1) ?? 'none'); console.log(pickBool([false, true], -1) ?? false); console.log(getNumber([4, 5], 1) ?? 0); console.log(getNumber([4, 5], -1) ?? 9); console.log(getText(['x', 'y'], 0) ?? 'none'); console.log(getBool([false, true], 1) ?? false); console.log(hasNumber([NaN], NaN, 0)); console.log(hasText(['a', 'b'], 'b', 0)); console.log(boolIndex([false, true], true, 0)); console.log(lastNumber([1, 2, 1], 1)); console.log(lastText(['a', 'b', 'a'], 'a', 1)); console.log(lastBool([true, false, true], true, 1)); console.log(formatNumbers([1, 2], '|')); console.log(formatText(['a', 'b'])); console.log(formatBools([true, false], '|')); try { fixed(1, 101); } catch { console.log('range'); } try { exponential(1, 101); } catch { console.log('exp-range'); } }\n",
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
        "true\ntrue\n-12.5\n3\n12.50\n12.5\nff\n1.3e+1\n1.25e+1\ntrue\nfalse\n3\n3\nb\ntrue\n5\n9\nx\ntrue\ntrue\ntrue\n1\n2\n0\n0\n1|2\na,b\ntrue|false\nrange\nexp-range\n"
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
        "export declare function describe(value: string, flag: boolean): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.describe = function(value, flag) { const parsed = Number(value.valueOf()); const rounded = Math.round(parsed).valueOf(); const matched = '421'.includes(rounded); const padded = 'x'.padEnd('2', flag); const replaced = '42'.replace(rounded, flag); const picked = 'abc'.charAt('1'); const repeated = 'x'.repeat('2'); let text = `value=${rounded}`; text += ':'; text += flag.toString(); return text.concat(':', rounded, ':', matched, ':', padded, ':', replaced, ':', picked, ':', repeated, ':', String(rounded > 0)); };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { describe } from 'jit-coercion';\nfunction main(): void { console.log(describe('42.4', true)); }\n",
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
        "value=42:true:42:true:xt:true:b:xx:true\n"
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
