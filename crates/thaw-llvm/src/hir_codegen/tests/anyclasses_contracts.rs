// A mutated `any` callback parameter keeps identity and `instanceof` against the native instance.

#[test]
fn anyclasses_contract_any_callback_param_keeps_identity_and_instanceof_after_a_write() {
    let source = r##"class C { x = 1; inc(): number { this.x += 1; return this.x; } }
class E extends C { y = 2; }
class D { z = 0; }
function main(): void {
  const c = new C(); const e = new E();
  setTimeout((o: any) => { o.x = 10; console.log(o === c, o instanceof C, o instanceof D, o === e, c.x, o.inc()); }, 1, c);
  setTimeout((o: any) => { o.y = 7; console.log(o === e, o instanceof E, o instanceof C, o instanceof D, e.y); }, 2, e);
  setTimeout((o: any) => { o.x = 2; console.log(o === c, o instanceof C, o.x); }, 4, { x: 1 });
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_any_callback_param_keeps_identity_and_instanceof_after_a_write"),
        "true true false false 10 11\ntrue true true false 7\nfalse false 2\n"
    );
}

#[test]
fn anyclasses_contract_any_timer_callback_param_keeps_identity_and_instanceof() {
    let source = r##"class C { x = 1; }
const c = new C();
function main(): void {
  setTimeout((o: any) => { o.x = 5; console.log(o === c, o instanceof C, c.x); }, 1, c);
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_any_timer_callback_param_keeps_identity_and_instanceof"),
        "true true 5\n"
    );
}

#[test]
fn anyclasses_contract_timer_native_collection_arguments_keep_identity() {
    let source = r##"function main(): void {
  const a = [1];
  const m = new Map<string, number>([["x", 1]]);
  let evaluationOrder = "";
  function arrayArgument(): number[] { evaluationOrder += "a"; return a; }
  function mapArgument(): Map<string, number> { evaluationOrder += "m"; return m; }
  setTimeout((first: any, second: any) => {
    console.log("evaluation", evaluationOrder, first === a, second === m);
  }, 1, arrayArgument(), mapArgument());
  setTimeout((value: any, repeated: any) => {
    console.log("array before", value === a, repeated === a, a[0]);
    value[0] = 9;
    console.log("array after", a[0]);
  }, 1, a, a);
  setTimeout((value: any) => {
    console.log("array timer2", value === a, value[0]);
  }, 1, a);
  setTimeout((value: any) => {
    console.log("map before", value === m, m.get("x"));
    value.set("x", 9);
    console.log("map after", m.get("x"));
  }, 1, m);
  a[0] = 2;
  m.set("x", 2);
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_timer_native_collection_arguments_keep_identity"),
        "evaluation am true true\narray before true true 2\narray after 9\narray timer2 true 9\nmap before true 2\nmap after 9\n"
    );
}

#[test]
fn anyclasses_contract_typed_timer_native_collection_arguments_keep_identity() {
    let source = r##"function register(): void {
  const a = [1];
  const m = new Map<string, number>([["x", 1]]);
  const s = new Set<number>(); s.add(1);
  function namedArrayCallback(value: number[]): void {
    console.log("array named", value === a, value[0]);
  }
  setTimeout((value: number[]) => {
    console.log("array", value === a, a[0]);
    value[0] = 9;
    console.log("array after", a[0]);
  }, 1, a);
  setTimeout(namedArrayCallback, 1, a);
  setTimeout((value: Map<string, number>) => {
    console.log("map", value === m, m.get("x"));
    value.set("x", 9);
    console.log("map after", m.get("x"));
  }, 1, m);
  setTimeout((value: Set<number>) => {
    console.log("set", value === s, s.has(2));
    value.add(9);
    console.log("set after", s.has(9));
  }, 1, s);
  a[0] = 2;
  m.set("x", 2);
  s.add(2);
}
function main(): void { register(); }
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_typed_timer_native_collection_arguments_keep_identity"),
        "array true 2\narray after 9\narray named true 9\nmap true 2\nmap after 9\nset true true\nset after true\n"
    );
}

#[test]
fn anyclasses_contract_timer_primitive_arguments_keep_conversion() {
    let source = r##"async function main(): Promise<void> {
  await new Promise<void>((resolve) => setTimeout((left, right) => {
    console.log(String(left) + ":" + String(right));
    resolve();
  }, 1, "x", 2));
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_timer_primitive_arguments_keep_conversion"),
        "x:2\n"
    );
}
