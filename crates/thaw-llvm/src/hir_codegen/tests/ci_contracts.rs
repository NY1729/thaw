// Wave CI: class instances held in `any` keep their methods and identity. Expected output recorded from node 22.

#[test]
fn ci_contract_class_instance_in_any_is_a_live_view_with_methods() {
    let source = r##"class C {
  n = 1;
  tag = "x";
  bump(): number { this.n += 1; return this.n; }
  plus(a: number, b: number): number { return a + b + this.n; }
}
async function main(): Promise<void> {
  const c = new C();
  const a: any = c;
  console.log(a.n);
  console.log(a.bump(), a.bump());
  console.log(c.n, a.n, a.plus(1, 2));
  a.n = 10;
  console.log(c.n, c.bump());
  console.log(JSON.stringify(a));
  console.log(a);
  console.log(Object.keys(a));
  console.log(a instanceof C, a === a);
}
"##;
    assert_eq!(
        compile_and_run(source, "ci_contract_class_instance_in_any_is_a_live_view_with_methods"),
        "1\n2 3\n3 3 6\n10 11\n{\"n\":11,\"tag\":\"x\"}\nC { n: 11, tag: 'x' }\n[ 'n', 'tag' ]\ntrue true\n"
    );
}

#[test]
fn ci_contract_inherited_async_rest_and_thenable_methods_through_any() {
    let source = r##"class Base { id = 7; who(): string { return "base" + this.id; } }
class D extends Base {
  n = 1;
  then(res: (v: number) => void): void { res(this.n * 10); }
  async later(x: number): Promise<number> { return x + this.n; }
  greet(name: string, ...rest: number[]): string { return name + this.n + rest.length; }
}
function viaParam(x: any): string { return x.greet("p"); }
async function main(): Promise<void> {
  const d = new D();
  const a: any = d;
  console.log(a.who(), a.greet("x", 1, 2));
  console.log(await a.later(5));
  console.log(await a);
  console.log(a.constructor.name, a instanceof Base);
  const b: any = d;
  console.log(a === b);
  console.log(viaParam(d));
  const list: any[] = [d, new D()];
  console.log(list.map((e: any) => e.greet("m")).join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "ci_contract_inherited_async_rest_and_thenable_methods_through_any"),
        "base7 x12\n6\n10\nD true\ntrue\np10\nm10,m10\n"
    );
}

