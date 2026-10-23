// Language-core dispatch contracts: virtual dispatch from a base constructor and base-typed receivers,
// sort comparators on `any`-aliased arrays, class toJSON in JSON.stringify, hidden #private fields,
// and super/static access through `any`. Expected output from node 22.

#[test]
fn json_stringify_runs_class_to_json() {
    let source = r###"class W { v = 1; toJSON(): string { return "TJ"; } }
class Sub extends W { toJSON(): string { return "SUB"; } }
class Holder { a = 1; #p = 2; w: W = new W(); list: W[] = [new W()]; }
function dump(x: W): string { return JSON.stringify(x); }
function main(): void {
  console.log(JSON.stringify(new W()), JSON.stringify([new W()]));
  console.log(JSON.stringify({ w: new W() }));
  console.log(JSON.stringify(new Holder()));
  console.log(dump(new Sub()), dump(new W()));
}
"###;
    assert_eq!(compile_and_run(source, "json_stringify_runs_class_to_json"), "\"TJ\" [\"TJ\"]\n{\"w\":\"TJ\"}\n{\"a\":1,\"w\":\"TJ\",\"list\":[\"TJ\"]}\n\"SUB\" \"TJ\"\n");
}

#[test]
fn private_fields_hidden_from_console_and_for_in() {
    let source = r###"class P {
  a = 1;
  #x = 2;
  b = "s";
  get x(): number { return this.#x; }
}
function main(): void {
  const p = new P();
  console.log(p);
  console.log([p]);
  console.log({ p });
  const seen: string[] = [];
  for (const k in p) seen.push(k);
  console.log(seen.join(), Object.keys(p).join(), p.x);
}
"###;
    assert_eq!(compile_and_run(source, "private_fields_hidden_from_console_and_for_in"), "P { a: 1, b: 's' }\n[ P { a: 1, b: 's' } ]\n{ p: P { a: 1, b: 's' } }\na,b a,b 2\n");
}

#[test]
fn sort_comparator_on_array_aliased_through_any() {
    let source = r###"interface R { k: string }
function main(): void {
  const names: string[] = ["b", "a", "c"];
  const aliasOfNames: any = names;
  names.sort((x, y) => (x < y ? -1 : x > y ? 1 : 0));
  console.log(names.join(), aliasOfNames.join());
  const nums: number[] = [3, 1, 10, 2];
  const aliasOfNums: any = nums;
  nums.sort((x, y) => y - x);
  console.log(nums.join(), aliasOfNums.join());
  const rows: R[] = [{ k: "b" }, { k: "a" }, { k: "c" }];
  const aliasOfRows: any = rows;
  rows.sort((x, y) => (x.k < y.k ? -1 : x.k > y.k ? 1 : 0));
  console.log(rows.map((r) => r.k).join(), aliasOfRows.length);
}
"###;
    assert_eq!(compile_and_run(source, "sort_comparator_on_array_aliased_through_any"), "a,b,c a,b,c\n10,3,2,1 10,3,2,1\na,b,c 3\n");
}

#[test]
fn super_and_static_members_through_any() {
    let source = r###"class Base {
  static count = 0;
  id: number;
  constructor() { this.id = ++Base.count; }
  who(): string { return "b"; }
}
class Der extends Base {
  who(): string { return "d" + super.who(); }
}
function main(): void {
  const D: any = Der;
  console.log(new D().id, D.count);
  const list: Base[] = [new Base(), new Der()];
  const view: any = list;
  console.log(view.map((x: Base) => x.who()).join());
}
"###;
    assert_eq!(compile_and_run(source, "super_and_static_members_through_any"), "1 1\nb,db\n");
}

#[test]
fn virtual_dispatch_in_base_constructor() {
    let source = r###"class A {
  constructor() { this.init(); }
  init(): void { console.log("A.init"); }
  who(): string { return "A"; }
}
class B extends A {
  init(): void { console.log("B.init"); }
  who(): string { return "B"; }
}
class C extends B {
  who(): string { return "C+" + super.who(); }
}
function describe(x: A): string { return x.who(); }
function main(): void {
  new A();
  new B();
  new C();
  const list: A[] = [new A(), new B(), new C()];
  console.log(describe(new C()), list.map((x) => x.who()).join());
}
"###;
    assert_eq!(compile_and_run(source, "virtual_dispatch_in_base_constructor"), "A.init\nB.init\nB.init\nA.init\nB.init\nB.init\nB.init\nC+B A,B,C+B\n");
}
