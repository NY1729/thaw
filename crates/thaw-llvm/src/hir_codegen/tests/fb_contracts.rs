
// Wave FB: owner-backed element writes and identity through `any` views. Expected output recorded from node 22.

#[test]
fn fb_contract_class_and_array_elements_write_through_to_the_native_owner() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const nat: C[] = [new C()];
  const v: any = nat;
  const e = new C();
  v[1] = e;
  e.bump();
  console.log(v.length, nat.length, nat[1] === e, nat[1].n, v[1] === e, v[1].n);
  v.push(new C());
  console.log(nat.length, nat[2].n);
  const d = new C();
  const x: any = d;
  const y: any = [d];
  console.log(x === d, d === x, y[0] === d, y[0] === x, x !== d);
  const grid: number[][] = [[1]];
  const gv: any = grid;
  const inner = [5, 6];
  gv[0] = inner;
  inner.push(7);
  console.log(grid[0].length, inner.length, grid[0] === inner, gv[0] === inner);
  gv.push([9]);
  console.log(grid.length, grid[1][0]);
  const ref: any = nat;
  console.log(ref === nat, nat === ref);
}
"####;
    assert_eq!(compile_and_run(source, "fb_contract_class_and_array_elements_write_through_to_the_native_owner"), "2 2 true 2 true 2\n3 1\ntrue true true true false\n3 3 true true\n2 9\ntrue true\n");
}

#[test]
fn fb_contract_top_level_any_slots_alias_native_arrays() {
    let source = r####"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
const nums: number[] = [1];
const alias: any = nums;
nums.push(2);
const nat: C[] = [new C()];
const view: any = nat;
view[1] = new C();
console.log(alias.length, view.length, nat.length, nat[1].n);
async function main(): Promise<void> {}
"####;
    assert_eq!(compile_and_run(source, "fb_contract_top_level_any_slots_alias_native_arrays"), "2 2 2 1\n");
}

// Expected output from tsc (experimentalDecorators) on node 22; node itself cannot run decorators.
#[test]
fn fb_contract_decorated_class_instance_in_a_local_any_slot_is_a_live_view() {
    let source = r####"function Dec(): ClassDecorator { return (t) => {}; }
@Dec()
class D { n = 1; hi(): string { return "hi" + this.n; } }
async function main(): Promise<void> {
  const d = new D();
  const a: any = d;
  console.log(typeof a.hi, a instanceof D, a.n);
  d.n = 5;
  console.log(a.n);
  console.log(a.hi());
  a.n = 7;
  console.log(d.n, a === d);
}
"####;
    assert_eq!(compile_and_run(source, "fb_contract_decorated_class_instance_in_a_local_any_slot_is_a_live_view"), "function true 1\n5\nhi5\n7 true\n");
}
