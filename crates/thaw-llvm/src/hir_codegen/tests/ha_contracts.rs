// Wave HA: methods in array literals held as `any`, compound assignment on dynamic members,
// named callbacks mutating an `any` parameter, tagged templates whose tag is a member of an `any`
// value, and a callback returning its own `any` parameter. Expected output from node 22.

#[test]
fn ha_contract_methods_in_any_array_literals_keep_their_receiver() {
    let source = r##"async function main(): Promise<void> {
  const m: any = [{ v: 1, inc() { this.v++; return this.v; } }];
  console.log(m[0].inc(), m[0].v);
  const n: any = [{ k: 2, dbl: function () { this.k *= 2; return this.k; } }, { k: 3, tri() { return this.k * 3; } }];
  console.log(n[0].dbl(), n[0].k, n[1].tri());
  const p: any = { rows: [{ c: 0, bump() { this.c += 5; return this.c; } }] };
  console.log(p.rows[0].bump(), p.rows[0].c);
  const q: any = [[{ z: 1, up() { this.z++; return this.z; } }]];
  console.log(q[0][0].up(), q[0][0].z);
}
"##;
    assert_eq!(
        compile_and_run(source, "ha_contract_methods_in_any_array_literals_keep_their_receiver"),
        "2 2\n4 4 9\n5 5\n2 2\n"
    );
}

#[test]
fn ha_contract_named_callbacks_mutate_any_parameters() {
    let source = r##"async function main(): Promise<void> {
  const rows: any = [{ n: 0, list: [1] }, { n: 0, list: [2] }];
  const f = (x: any) => { x.n = 1; };
  rows.forEach(f);
  const g = (x: any) => x.list.push(7);
  rows.map(g);
  function h(x: any) { x.n += 10; return x.n; }
  const ns = rows.map(h);
  const ok = rows.filter((x: any) => x.n > 5).length;
  console.log(JSON.stringify(rows), JSON.stringify(ns), ok);
}
"##;
    assert_eq!(
        compile_and_run(source, "ha_contract_named_callbacks_mutate_any_parameters"),
        "[{\"n\":11,\"list\":[1,7]},{\"n\":11,\"list\":[2,7]}] [11,11] 2\n"
    );
}

#[test]
fn ha_contract_compound_assignment_on_dynamic_members() {
    let source = r##"async function main(): Promise<void> {
  const o: any = { n: 1, s: "a", z: 0, u: null };
  o.n += 4;
  o.s += "b";
  o.n *= 2;
  o.z ||= 9;
  o.u ??= "set";
  o["n"] -= 1;
  console.log(JSON.stringify(o));
}
"##;
    assert_eq!(
        compile_and_run(source, "ha_contract_compound_assignment_on_dynamic_members"),
        "{\"n\":9,\"s\":\"ab\",\"z\":9,\"u\":\"set\"}\n"
    );
}

#[test]
fn ha_contract_tag_that_is_a_member_of_an_any_value() {
    let source = r##"function tag(s: any, ...v: any[]): string { return s.raw.join("|") + v.length; }
function mk(): any { return { t: tag }; }
async function main(): Promise<void> {
  console.log(tag`a${1}b${2}c`);
  console.log(mk().t`x${1}y`);
  const o: any = { t: tag, deep: { t: tag } };
  console.log(o.t`p${1}q`, o.deep.t`r${2}s${3}t`);
}
"##;
    assert_eq!(
        compile_and_run(source, "ha_contract_tag_that_is_a_member_of_an_any_value"),
        "a|b|c2\nx|y1\np|q1 r|s|t2\n"
    );
}

#[test]
fn ha_contract_callback_returns_its_own_any_parameter() {
    let source = r##"async function main(): Promise<void> {
  const g: any = (globalThis as any);
  g.__ha_lit = { list: [1, 2, 3], names: ["a", "b"], objs: [{ k: 1 }] };
  const o: any = g.__ha_lit;
  console.log(JSON.stringify(o.list.map((x: any) => x)), JSON.stringify(o.names.map((s: any) => s)));
  console.log(JSON.stringify(o.list.map((x: any) => [x])), JSON.stringify(o.objs.map((x: any) => ({ v: x }))));
}
"##;
    assert_eq!(
        compile_and_run(source, "ha_contract_callback_returns_its_own_any_parameter"),
        "[1,2,3] [\"a\",\"b\"]\n[[1],[2],[3]] [{\"v\":{\"k\":1}}]\n"
    );
}
