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

