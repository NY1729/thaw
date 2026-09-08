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

