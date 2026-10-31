// Function identity: method refs, array search, Map/Set/Weak keys. Expected output recorded from node 22.

#[test]
fn function_identity_method_refs_are_interned() {
    let source = r##"class K { v: number = 1; m(): number { return this.v; } static s(): number { return 2; } }
function main(): void {
  const k = new K();
  console.log(k.m === k.m, K.s === K.s, k.m === new K().m);
  const a: () => number = k.m;
  const b: () => number = k.m;
  console.log(a === b);
}
"##;
    assert_eq!(
        compile_and_run(source, "function_identity_method_refs_are_interned"),
        "true true true\ntrue\n"
    );
}

#[test]
fn function_identity_array_search_finds_functions() {
    let source = r##"function f(x: number): number { return x; }
function g(): void {}
function g2(): void {}
function main(): void {
  const l = [g, g2];
  console.log(l.indexOf(g), l.includes(g), l.includes(g2), l.lastIndexOf(g2));
  const l2: Array<(x: number) => number> = [f];
  l2.splice(l2.indexOf(f), 1);
  console.log(l2.length, l2.indexOf((x: number) => x));
}
"##;
    assert_eq!(
        compile_and_run(source, "function_identity_array_search_finds_functions"),
        "0 true true 1\n0 -1\n"
    );
}

#[test]
fn function_identity_set_of_functions() {
    let source = r##"function g(): void {}
function main(): void {
  const s = new Set<() => void>();
  s.add(g); s.add(g);
  const lam = (): void => {};
  s.add(lam);
  console.log(s.size, s.has(g), s.has(lam), s.has(() => {}));
  s.delete(g);
  console.log(s.size, s.has(g));
  const sf = new Set<Function>();
  sf.add(g);
  console.log(sf.size, sf.has(g));
}
"##;
    assert_eq!(
        compile_and_run(source, "function_identity_set_of_functions"),
        "2 true true false\n1 false\n1 true\n"
    );
}

#[test]
fn function_identity_map_keyed_by_functions() {
    let source = r##"class K { v: number = 1; m(): number { return this.v; } }
function f(x: number): number { return x; }
function mk(): () => number { return () => 1; }
function main(): void {
  const m = new Map<() => number, string>();
  const c1 = mk(), c2 = mk();
  m.set(c1, "a"); m.set(c2, "b"); m.set(c1, "c");
  console.log(m.size, m.get(c1), m.get(c2));
  const mm = new Map<(x: number) => number, string>();
  mm.set(f, "f");
  console.log(mm.get(f), mm.has(f), mm.has((x: number) => x));
  const k = new K();
  const ms = new Map<() => number, string>();
  ms.set(k.m, "meth");
  console.log(ms.get(k.m));
}
"##;
    assert_eq!(
        compile_and_run(source, "function_identity_map_keyed_by_functions"),
        "2 c b\nf true false\nmeth\n"
    );
}

#[test]
fn function_identity_weak_collections_of_functions() {
    let source = r##"function g(): void {}
function main(): void {
  const lam = (): void => {};
  const w = new WeakSet<() => void>();
  w.add(g);
  console.log(w.has(g), w.has(lam));
  const wm = new WeakMap<() => void, number>();
  wm.set(g, 5);
  console.log(wm.get(g), wm.has(lam));
}
"##;
    assert_eq!(
        compile_and_run(source, "function_identity_weak_collections_of_functions"),
        "true false\n5 false\n"
    );
}
