// Wave LV: native arrays and nested class instances held in `any` are live views. Expected output recorded from node 22.

#[test]
fn lv_contract_native_array_aliases_both_ways_through_any() {
    let source = r####"async function main(): Promise<void> {
  const arr: number[] = [1, 2];
  const a: any = arr;
  a.push(3);
  console.log(arr.length, arr.join(","));
  arr.push(4);
  console.log(a.length, a[3]);
  a[0] = 9;
  console.log(arr[0]);
  console.log(a.map((x: number) => x * 2).join(","));
  console.log(Array.isArray(a), JSON.stringify(a));
}
"####;
    assert_eq!(compile_and_run(source, "lv_contract_native_array_aliases_both_ways_through_any"), "3 1,2,3\n4 4\n9\n18,4,6,8\ntrue [9,2,3,4]\n");
}

#[test]
fn lv_contract_array_methods_and_holes_through_an_any_view() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const arr: number[] = [3, 1, 2];
  const a: any = arr;
  console.log(a);
  console.log(a.length, a[1], a[5]);
  a.sort();
  console.log(arr.join(","));
  console.log(a.pop(), arr.length);
  console.log(a.shift(), arr.join(","));
  a.unshift(7, 8);
  console.log(arr.join(","));
  a.splice(1, 1, 5, 6);
  console.log(arr.join(","));
  a.length = 2;
  console.log(arr.join(","), arr.length);
  a[4] = 9;
  console.log(arr.length);
  for (const x of a) console.log(x);
  console.log(Object.keys(a).join(","));
  console.log(a.indexOf(9), a.includes(7), a.reverse().join(","));
  console.log(JSON.stringify(a), String(a));
  const names: string[] = ["x", "y"];
  const n: any = names;
  n.push("z");
  console.log(names.join(""), n.join("-"), n.map((s: string) => s + "!").join(""));
  const same: any = arr;
  console.log(a === same);
  const cs: C[] = [new C(), new C()];
  const l: any = cs;
  const e0 = l[0]; console.log(e0.bump(), cs[0].n, l.length);
  const grid: number[][] = [[1], [2]];
  const g: any = grid;
  const g0 = g[0]; g0.push(5);
  console.log(grid[0].length, JSON.stringify(g));
  let t: any = 0;
  t = arr;
  t.push(100);
  console.log(arr[arr.length - 1]);
}
"####;
    assert_eq!(compile_and_run(source, "lv_contract_array_methods_and_holes_through_an_any_view"), "[ 3, 1, 2 ]\n3 1 undefined\n1,2,3\n3 2\n1 2\n7,8,2\n7,5,6,2\n7,5 2\n5\n7\n5\nundefined\nundefined\n9\n0,1,4\n4 true 9,,,5,7\n[9,null,null,5,7] 9,,,5,7\nxyz x-y-z x!y!z!\ntrue\n2 2 2\n2 [[1,5],[2]]\n100\n");
}

#[test]
fn lv_contract_array_view_survives_indirection_through_params_returns_fields_and_literals() {
    let source = r####"class Box { items: any = null; }
function addOne(x: any): void { x.push(1); }
function lenOf(x: any): number { return x.length; }
function make(a: number[]): any { return a; }
async function main(): Promise<void> {
  const arr: number[] = [5];
  addOne(arr);
  console.log(arr.length, lenOf(arr));
  const m = make(arr);
  m.push(2);
  console.log(arr.length);
  const b = new Box();
  b.items = arr;
  b.items.push(3);
  console.log(arr.length);
  const o: any = { list: arr };
  o.list.push(4);
  console.log(arr.length);
  const holder: any[] = [arr];
  holder[0].push(5);
  console.log(arr.length);
}
"####;
    assert_eq!(compile_and_run(source, "lv_contract_array_view_survives_indirection_through_params_returns_fields_and_literals"), "2 2\n3\n4\n5\n6\n");
}

#[test]
fn lv_contract_class_instances_nested_in_literals_and_arrays_stay_live() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const c = new C();
  const o: any = { inner: c };
  console.log(typeof o.inner.bump);
  console.log(o.inner.n);
  c.n = 5;
  console.log(o.inner.n);
  console.log(o.inner.bump(), c.n);
  const l: any = [c];
  console.log(l[0].n, typeof l[0].bump);
}
"####;
    assert_eq!(compile_and_run(source, "lv_contract_class_instances_nested_in_literals_and_arrays_stay_live"), "function\n1\n5\n6 6\n6 function\n");
}

#[test]
fn lv_contract_class_field_holding_a_class_instance_stays_live_through_any() {
    let source = r####"class Inner { v = 1; inc(): number { this.v++; return this.v; } }
class Outer { inner: Inner = new Inner(); name = "o"; }
async function main(): Promise<void> {
  const o = new Outer();
  const a: any = o;
  console.log(a.inner.v, typeof a.inner.inc);
  console.log(a.inner.inc(), o.inner.v);
}
"####;
    assert_eq!(compile_and_run(source, "lv_contract_class_field_holding_a_class_instance_stays_live_through_any"), "1 function\n2 2\n");
}
