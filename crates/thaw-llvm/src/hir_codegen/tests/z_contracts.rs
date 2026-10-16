// Wave Z: language-semantics leftovers. Expected output recorded from node 22.

#[test]
fn z_contract_default_argument_closures_share_the_parameter_cell() {
    let source = r##"function f(a: number = 1, g: () => number = () => a): number { a = 2; return g(); }
function mk(x: number = 0, inc: () => number = () => ++x): () => number { return inc; }
async function main(): Promise<void> {
  console.log(f());
  console.log(f(5));
  const i1 = mk();
  const i2 = mk();
  console.log(i1(), i1(), i2());
  console.log(mk(10)());
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_default_argument_closures_share_the_parameter_cell"),
        "2\n2\n1 2 1\n11\n"
    );
}

#[test]
fn z_contract_static_write_through_subclass_shadows_the_base() {
    let source = r##"class A { static s: number = 1; static t: string = 'a'; }
class B extends A {}
class C extends B {}
async function main(): Promise<void> {
  B.s = 10;
  console.log(A.s, B.s, C.s);
  C.s = 20;
  console.log(A.s, B.s, C.s);
  A.s = 5;
  console.log(A.s, B.s, C.s);
  B.t += 'x';
  console.log(A.t, B.t, C.t);
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_static_write_through_subclass_shadows_the_base"),
        "1 10 10\n1 10 20\n5 10 20\na ax ax\n"
    );
}

#[test]
fn z_contract_template_object_identity_raw_and_frozen() {
    let source = r##"function tag(s: TemplateStringsArray, ...v: number[]): TemplateStringsArray { return s; }
const f = (): TemplateStringsArray => tag`a${1}b\n`;
async function main(): Promise<void> {
  console.log(f() === f(), tag`a${1}b` === tag`a${1}b`);
  console.log(Object.isFrozen(f()), f().raw[1], f()[1].length, f().raw[1].length, f().length);
  console.log(Array.isArray(f().raw), Object.isFrozen(f().raw));
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_template_object_identity_raw_and_frozen"),
        "true false\ntrue b\\n 2 3 2\ntrue true\n"
    );
}

#[test]
fn z_contract_group_by_visits_array_holes() {
    let source = r##"async function main(): Promise<void> {
  const a: (number | undefined)[] = [1, , 3];
  const g = Object.groupBy(a, (x) => (x === undefined ? 'u' : 'n'));
  console.log(JSON.stringify(g));
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_group_by_visits_array_holes"),
        "{\"n\":[1,3],\"u\":[null]}\n"
    );
}

#[test]
fn z_contract_frozen_array_mutations_throw() {
    let source = r##"async function main(): Promise<void> {
  const a: number[] = Object.freeze([1, 2]) as number[];
  try { a[0] = 9; } catch (e) { console.log('write throws'); }
  console.log(a[0], a.length);
  for (const op of ['push', 'pop', 'shift', 'unshift', 'reverse', 'sort', 'fill', 'splice']) {
    try {
      if (op === 'push') a.push(3);
      else if (op === 'pop') a.pop();
      else if (op === 'shift') a.shift();
      else if (op === 'unshift') a.unshift(0);
      else if (op === 'reverse') a.reverse();
      else if (op === 'sort') a.sort();
      else if (op === 'fill') a.fill(0);
      else a.splice(0, 1);
      console.log(op, 'ok');
    } catch (e) {
      console.log(op, 'throws', e instanceof TypeError);
    }
  }
  console.log(a.join(), Object.isFrozen(a));
  const s: number[] = Object.seal([1, 2]) as number[];
  s[0] = 7;
  try { s.push(3); } catch (e) { console.log('sealed push throws'); }
  console.log(s.join());
  const live: number[] = [1, 2];
  live[0] = 9;
  live.push(3);
  console.log(live.join());
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_frozen_array_mutations_throw"),
        "write throws\n1 2\npush throws true\npop throws true\nshift throws true\nunshift throws true\nreverse throws true\nsort throws true\nfill throws true\nsplice throws true\n1,2 true\nsealed push throws\n7,2\n9,2,3\n"
    );
}

#[test]
fn z_contract_default_argument_closures_in_every_callable_shape() {
    let source = r##"class K {
  v: number;
  constructor(a: number = 4, g: () => number = () => a) { a = 5; this.v = g(); }
  m(a: number = 6, g: () => number = () => a): number { a = 7; return g(); }
  static s(a: string = 'x', g: () => string = () => a): string { a = a + 'y'; return g(); }
}
async function am(a: number = 8, g: () => number = () => a): Promise<number> { a = 9; return g(); }
function many(a: number, b: number = 2, g: () => number = () => a + b, c: number = 4): number { b = 20; return g() + c; }
async function main(): Promise<void> {
  console.log(new K().v, new K(1).v, new K().m(), K.s());
  console.log(await am());
  console.log(many(1), many(1, 5), many(1, 5, () => 100), many(1, 5, () => 100, 6));
}
"##;
    assert_eq!(
        compile_and_run(source, "z_contract_default_argument_closures_in_every_callable_shape"),
        "5 5 7 xy\n9\n25 25 104 106\n"
    );
}
