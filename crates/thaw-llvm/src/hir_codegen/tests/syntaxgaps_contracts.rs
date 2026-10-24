#[test]
fn syntaxgaps_iterator_explicit_call() {
    let source = r###"class R { n = 0; [Symbol.iterator]() { const self = this; return { next() { self.n++; return { done: self.n > 2, value: self.n }; } }; } }
function main(): void {
  const a = [10, 20];
  const it = a[Symbol.iterator]();
  console.log(JSON.stringify(it.next()), JSON.stringify(it.next()), JSON.stringify(it.next()));
  const s = "hé";
  const si = s[Symbol.iterator]();
  console.log(si.next().value, si.next().value, si.next().done);
  const m = new Map<string, number>([["a", 1], ["b", 2]]);
  const mi = m[Symbol.iterator]();
  console.log(JSON.stringify(mi.next().value), mi.next().done, mi.next().done);
  const st = new Set<number>([5, 6]);
  const sti = st[Symbol.iterator]();
  console.log(sti.next().value, sti.next().value, sti.next().done);
  const ri = new R()[Symbol.iterator]();
  console.log(JSON.stringify(ri.next()), JSON.stringify(ri.next()), JSON.stringify(ri.next()));
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_iterator_explicit_call"), "{\"value\":10,\"done\":false} {\"value\":20,\"done\":false} {\"done\":true}\nh é true\n[\"a\",1] false true\n5 6 true\n{\"done\":false,\"value\":1} {\"done\":false,\"value\":2} {\"done\":true,\"value\":3}\n");
}

#[test]
fn syntaxgaps_optional_chain_call_forms() {
    let source = r###"function main(): void {
  second();
  const o: any = { a: { b() { return 1; }, c() { return 2; } }, f() { return 3; } };
  console.log(o?.a?.["c"](), o?.a?.b(), o.f?.());
  const n: any = null;
  console.log(n?.a?.["c"](), n?.a?.b(), n?.f?.(), o.g?.());
  const t: { a?: { b(): number }, f?: () => number } = { a: { b: () => 7 } };
  console.log(t?.a?.b(), t.f?.(), t.a?.b?.());
}
function second(): void {
  const o: any = { a: { c() { return 2; } }, f() { return 3; } };
  console.log(o["f"](), o.a["c"](), o?.a["c"](), o.a?.["c"]());
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_optional_chain_call_forms"), "3 2 2 2\n2 1 3\nundefined undefined undefined undefined\n7 undefined 7\n");
}

#[test]
fn syntaxgaps_delete_optional_chain() {
    let source = r###"function main(): void {
  const o: any = { a: { b: 1, c: 2 }, k: 5 };
  console.log(delete o?.a.b, JSON.stringify(o));
  console.log(delete o?.["k"], JSON.stringify(o));
  const n: any = null;
  console.log(delete n?.a.b, delete n?.["k"]);
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_delete_optional_chain"), "true {\"a\":{\"c\":2},\"k\":5}\ntrue {\"a\":{\"c\":2}}\ntrue true\n");
}

#[test]
fn syntaxgaps_forward_const_closure() {
    let source = r###"function main(): void {
  let c = 0;
  const f = () => g() + 1;
  function h() { return k(2); }
  const g = () => 10;
  const k = (x: number) => x * 3;
  console.log(f(), h());
  const inc = () => { c++; return twice(); };
  const twice = () => c * 2;
  console.log(inc(), inc());
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_forward_const_closure"), "11 6\n2 4\n");
}

#[test]
fn syntaxgaps_local_classes_capture_free() {
    let source = r###"function make(): string {
  class Counter {
    static made = 0;
    n: number;
    constructor() { this.n = 1; Counter.made++; }
    inc() { this.n += 2; return this.n; }
    static twice() { return Counter.made * 2; }
  }
  class Sub extends Counter { inc() { return super.inc() + 1; } }
  const c = new Sub(); new Counter();
  return [c.inc(), Counter.twice(), c instanceof Counter].join(",");
}
function main(): void {
  console.log(make());
  {
    class B { v = 3; get d() { return this.v * 2; } }
    console.log(new B().d);
  }
  for (let i = 0; i < 2; i++) { class L { x = 7; } console.log(new L().x + i); }
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_local_classes_capture_free"), "4,4,true\n6\n7\n8\n");
}

#[test]
fn syntaxgaps_string_destructuring() {
    let source = r###"function main(): void {
  const [a, b] = "hi";
  const { length } = "abc";
  console.log(a, b, length);
  const [x, ...rest] = "xyz";
  console.log(x, rest.join("-"));
  let p = "", q = "";
  [p, q] = "uv";
  console.log(p, q);
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_string_destructuring"), "h i 3\nx y-z\nu v\n");
}

#[test]
fn syntaxgaps_this_alias_in_method() {
    let source = r###"class R {
  n = 0;
  mk() { const self = this; const o = { next() { self.n++; return { done: self.n > 2, value: self.n }; } }; return o; }
}
function main(): void {
  const it = new R().mk();
  console.log(JSON.stringify(it.next()), JSON.stringify(it.next()), JSON.stringify(it.next()));
}
"###;
    assert_eq!(compile_and_run(source, "syntaxgaps_this_alias_in_method"), "{\"done\":false,\"value\":1} {\"done\":false,\"value\":2} {\"done\":true,\"value\":3}\n");
}
