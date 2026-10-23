// A native Map/Set copied into an `any` object literal stays one live object: later writes
// through the variable show through the literal, and the reverse. Expected output from node 22.

#[test]
fn mapalias_contract_map_in_any_literal_shares_later_writes() {
    let source = r##"const m = new Map<string, number>();
m.set("a", 1);
const o: any = { m };
m.set("b", 2);
console.log(o.m.size, o.m.get("b"));
"##;
    assert_eq!(
        compile_and_run(
            source,
            "mapalias_contract_map_in_any_literal_shares_later_writes"
        ),
        "2 2\n"
    );
}

#[test]
fn mapalias_contract_set_in_any_literal_shares_later_writes() {
    let source = r##"const s = new Set<number>();
s.add(1);
const p: any = { s };
s.add(2);
console.log(p.s.size, p.s.has(2));
"##;
    assert_eq!(
        compile_and_run(
            source,
            "mapalias_contract_set_in_any_literal_shares_later_writes"
        ),
        "2 true\n"
    );
}

#[test]
fn mapalias_contract_map_in_any_literal_in_function_shares_later_writes() {
    let source = r##"function main(): void {
  const m = new Map<string, number>();
  m.set("a", 1);
  const o: any = { m };
  m.set("b", 2);
  console.log(o.m.size, o.m.get("b"));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "mapalias_contract_map_in_any_literal_in_function_shares_later_writes"
        ),
        "2 2\n"
    );
}

#[test]
fn mapalias_contract_collection_in_nested_and_array_any_literals_stays_live() {
    let source = r##"const m = new Map<string, number>();
m.set("a", 1);
const o1: any = { m: m, t: 1 };
m.set("b", 2);
console.log(o1.m.size, o1.m.get("b"));
const m2 = new Map<string, number>();
m2.set("a", 1);
const o2: any = { inner: { m2 } };
m2.set("b", 2);
console.log(o2.inner.m2.size);
const s = new Set<number>();
const o3: any = { s };
s.add(5);
console.log(o3.s.size, o3.s.has(5));
const m4 = new Map<string, number>();
const arr: any = [{ m4 }];
m4.set("q", 9);
console.log(arr[0].m4.get("q"), arr[0].m4.size);
const m5 = new Map<string, number>();
const o5: any = { m5 };
o5.m5.set("w", 1);
console.log(m5.size, m5.has("w"));
const m6 = new Map<string, number>();
m6.set("a", 1);
function f(x: any): any { return x; }
const o6: any = f({ m6 });
m6.set("b", 2);
console.log(o6.m6.size);
"##;
    assert_eq!(
        compile_and_run(
            source,
            "mapalias_contract_collection_in_nested_and_array_any_literals_stays_live"
        ),
        "2 2\n2\n1 true\n9 1\n1 true\n2\n"
    );
}

#[test]
fn mapalias_contract_caught_error_identity_row8_probe() {
    let source = r##"const err = new Error("boom");
try { throw err; } catch (e) { console.log(e === err, (e as Error).message); }
function f(): boolean { const x = new TypeError("t"); try { throw x; } catch (e) { return e === x; } }
console.log(f());
"##;
    assert_eq!(
        compile_and_run(source, "mapalias_contract_caught_error_identity_row8_probe"),
        "true boom\ntrue\n"
    );
}
