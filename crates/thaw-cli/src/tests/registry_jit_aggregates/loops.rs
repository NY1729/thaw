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

