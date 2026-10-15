// Wave S: language-semantics contracts. Expected output recorded from node 22.

#[test]
fn s_contract_continue_in_try_does_not_catch_the_update() {
    let source = r##"function upd(): number { throw new Error("boom"); }
function main() {
  let i = 0;
  try { for (i = 0; i < 3; i += upd()) { try { continue; } catch (e) { console.log("inner"); } } } catch (e) { console.log("outer"); }
  try { for (let j = 0; j < 3; j += upd()) { try { continue; } catch (e) { console.log("inner"); } } } catch (e) { console.log("outer"); }
}
"##;
    assert_eq!(compile_and_run(source, "s_contract_continue_try"), "outer\nouter\n");
}

#[test]
fn s_contract_continue_break_through_try_keep_targets() {
    let source = r##"function main() {
  const out: string[] = [];
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      try {
        if (j === 1) continue;
        if (i === 1) continue outer;
        if (i === 2) break outer;
        out.push(`${i}${j}`);
      } finally { out.push("f"); }
    }
  }
  const xs = [1, 2, 3];
  let n = 0;
  for (const x of xs) { try { if (x === 2) continue; if (x === 3) break; n += x; } catch (e) { n = -1; } }
  let k = 0;
  do { try { k++; if (k < 3) continue; } catch (e) { k = 100; } } while (k < 3);
  console.log(out.join(","), n, k);
}
"##;
    assert_eq!(compile_and_run(source, "s_contract_continue_targets"), "00,f,f,02,f,f,f 1 3\n");
}

#[test]
fn s_contract_object_assign_copies_properties_one_at_a_time() {
    let source = r##"function main() {
  const t: any = { a: 0 };
  const s = { a: 1, get b(): number { console.log(t.a); return 2; } };
  Object.assign(t, s);
  console.log(t.b);
  const t2: any = { a: 0 };
  const s2 = { a: 1, get b(): number { throw new Error("getter"); }, c: 3 };
  try { Object.assign(t2, s2); } catch (e: any) { console.log("threw", e.message); }
  console.log(t2.a, t2.b, t2.c);
  const u: any = {};
  Object.assign(u, { x: 1, y: "s" }, { y: 2, z: true });
  console.log(JSON.stringify(u));
}
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_object_assign"),
        "1\n2\nthrew getter\n1 undefined undefined\n{\"x\":1,\"y\":2,\"z\":true}\n"
    );
}

#[test]
fn s_contract_tagged_template_with_void_tag_and_extra_substitutions() {
    let source = r##"let log = "";
function side(n: number): number { log += n; return n; }
function vtag(s: any, ...v: any[]): void { console.log(s.length, v.length); }
function vtag2(s: string[], v: number): void { console.log(s.length, v); }
function one(s: any): any { return s; }
function main() {
  vtag`a${1}x`;
  vtag2`a${1}x`;
  const r = one`a${side(1)}b${side(2)}c`;
  console.log(log, r.length, r[0], r[1], r[2]);
}
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_tagged_template_void"),
        "2 1\n2 1\n12 3 a b c\n"
    );
}

#[test]
fn s_contract_join_applies_symbol_to_primitive() {
    let source = r##"function main() {
  const item = { [Symbol.toPrimitive](_h: string): string { return 'x'; } };
  const plain = { a: 1 };
  console.log([item, item].join('-'), [item].join(''), [plain].join(''));
}
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_join_to_primitive"),
        "x-x x [object Object]\n"
    );
}

#[test]
fn s_contract_named_class_expression_keeps_its_own_name() {
    let source = r##"const Outer = class Inner { static who(): string { return Inner.name; } };
const Anon = class { };
function main() { console.log(Outer.name, Outer.who(), Anon.name); }
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_named_class_expr"),
        "Inner Inner Anon\n"
    );
}

#[test]
fn s_contract_array_rest_turns_holes_into_undefined_elements() {
    let source = r##"function main() {
  const source: number[] = [1, 2, 3];
  delete source[0];
  const [...rest] = source;
  console.log(0 in rest, rest.length, rest[0], rest[1]);
  const anys: any[] = [1, 2, 3];
  delete anys[0];
  const [...anyRest] = anys;
  console.log(0 in anyRest, anyRest.length);
  let tail: number[] = [];
  [, ...tail] = [7, 8, 9];
  console.log(tail.join(), tail.length);
  const [first, ...others] = [1, 2, 3, 4];
  console.log(first, others.join());
}
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_array_rest_holes"),
        "true 3 undefined 2\ntrue 3\n8,9 2\n1 2,3,4\n"
    );
}

#[test]
fn s_contract_flat_keeps_undefined_elements_of_optional_arrays() {
    let source = r##"function main() {
  const a: (number[] | undefined)[] = [[1, 2], undefined, [3]];
  const f = a.flat();
  console.log(JSON.stringify(f), f.length, f[2], f[3]);
  const none: (string[] | undefined)[] = [undefined, undefined];
  console.log(JSON.stringify(none.flat()), none.flat().length);
  const sparse: (number[] | undefined)[] = [];
  sparse[2] = [9];
  console.log(JSON.stringify(sparse.flat()));
}
"##;
    assert_eq!(
        compile_and_run(source, "s_contract_flat_optional"),
        "[1,2,null,3] 4 undefined 3\n[null,null] 2\n[9]\n"
    );
}
