// Wave FD: arrays nested in `any` literals have dynamic element semantics. Expected output from node 22.

#[test]
fn fd_contract_array_in_any_literal_takes_any_element_type() {
    let source = r##"async function main(): Promise<void> {
  const h: any = { list: [1] };
  h.list.push("x");
  h.list.push({ a: 1 });
  h.list.push(null);
  h.list.push(undefined);
  h.list.push([2, 3]);
  console.log(h.list.length, JSON.stringify(h.list), typeof h.list[1], h.list[2].a);
  h.list[0] = "z";
  h.list[7] = true;
  console.log(h.list.length, h.list[0], h.list[7], h.list[6]);
  h.list.unshift("u", { b: 2 });
  console.log(h.list.length, h.list[0], h.list[1].b, h.list[2]);
  h.list.splice(1, 1, "s", [9]);
  console.log(h.list.length, h.list[1], h.list[2][0]);
}
"##;
    assert_eq!(
        compile_and_run(source, "fd_contract_array_in_any_literal_takes_any_element_type"),
        "6 [1,\"x\",{\"a\":1},null,null,[2,3]] string 1\n8 z true undefined\n10 u 2 z\n11 s 9\n"
    );
}

#[test]
fn fd_contract_fill_and_nested_arrays_in_any_literal() {
    let source = r##"async function main(): Promise<void> {
  const m: any = { arr: [1, 2, 3] };
  m.arr.fill("f");
  console.log(m.arr.join(","));
  m.arr.fill({ q: 1 }, 1, 2);
  console.log(m.arr[0], m.arr[1].q, m.arr[2]);
  const w: any = { s: ["a"], n: [[1]] };
  w.s.push(5);
  w.n.push("str");
  w.n[0].push("in");
  console.log(JSON.stringify(w));
}
"##;
    assert_eq!(
        compile_and_run(source, "fd_contract_fill_and_nested_arrays_in_any_literal"),
        "f,f,f\nf 1 f\n{\"s\":[\"a\",5],\"n\":[[1,\"in\"],\"str\"]}\n"
    );
}

#[test]
fn fd_contract_arrays_in_any_literals_across_sinks() {
    let source = r##"function f(o: any): any { o.list.push("p"); return o; }
function g(): any { return { inner: { list: [1] } }; }
async function main(): Promise<void> {
  const h: any = {};
  h.x = { list: [1] };
  h.x.list.push("a");
  console.log(JSON.stringify(h));
  const arrs: any = [{ l: [1] }];
  arrs[0].l.push("b");
  console.log(JSON.stringify(arrs));
  console.log(JSON.stringify(f({ list: [1] })));
  const r = g();
  r.inner.list.push("c");
  console.log(JSON.stringify(r));
  const base = { k: 1 };
  const s: any = { ...base, list: [1, 2] };
  s.list.push("d");
  console.log(JSON.stringify(s), s.k);
  const t: any = { list: [1], n: { deep: [[1]] } };
  t.n.deep[0].push("e");
  t.n.deep.push("f");
  t.list.length = 1;
  t.list.splice(0, 0, "g");
  console.log(JSON.stringify(t), Array.isArray(t.list), t.list instanceof Array);
  const u: any = { list: [3, 1, 2] };
  u.list.sort();
  console.log(u.list.map((v: number) => v * 2).join(","), u.list.indexOf(2), [...u.list].length);
}
"##;
    assert_eq!(
        compile_and_run(source, "fd_contract_arrays_in_any_literals_across_sinks"),
        "{\"x\":{\"list\":[1,\"a\"]}}\n[{\"l\":[1,\"b\"]}]\n{\"list\":[1,\"p\"]}\n{\"inner\":{\"list\":[1,\"c\"]}}\n{\"k\":1,\"list\":[1,2,\"d\"]} 1\n{\"list\":[\"g\",1],\"n\":{\"deep\":[[1,\"e\"],\"f\"]}} true true\n2,4,6 1 3\n"
    );
}

#[test]
fn fd_contract_any_literal_with_array_keeps_native_aliases_and_class_identity() {
    let source = r##"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const c = new C();
  const arr: number[] = [1, 2];
  const o: any = { list: [1], c, arr };
  console.log(o.c.bump(), c.n, o.c === c);
  o.arr.push(3);
  console.log(arr.length, o.arr.length);
  o.list.push(c);
  o.list[1].bump();
  console.log(c.n, o.list[1] === c);
  console.log(JSON.stringify(o.list.length), typeof o.list[1].bump);
}
"##;
    assert_eq!(
        compile_and_run(source, "fd_contract_any_literal_with_array_keeps_native_aliases_and_class_identity"),
        "2 2 true\n3 3\n3 true\n2 function\n"
    );
}
