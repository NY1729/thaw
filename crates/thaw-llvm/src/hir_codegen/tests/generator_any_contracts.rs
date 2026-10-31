// Generator stored as `any`. Expected output recorded from node 22.

#[test]
fn generator_any_next_return_throw_protocol() {
    let source = r##"function* gen() { yield 1; yield 2; return 3; }
function main(): void {
  const g: any = gen();
  console.log(g.next(), g.next(), g.next(), g.next());
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_next_return_throw_protocol"),
        "{ value: 1, done: false } { value: 2, done: false } { value: 3, done: true } { value: undefined, done: true }\n"
    );
}

#[test]
fn generator_any_for_of_and_spread() {
    let source = r##"function* gen() { yield 1; yield 2; }
function main(): void {
  const g: any = gen();
  for (const x of g) console.log(x);
  const h: any = gen();
  console.log([...h].length);
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_for_of_and_spread"),
        "1\n2\n2\n"
    );
}

#[test]
fn generator_any_return_value_identity_and_printing() {
    let source = r##"function* gen() { yield 1; yield 2; return 3; }
function main(): void {
  const g: any = gen();
  g.next();
  console.log(g.return(9), g.next());
  console.log(g, String(g), typeof g, typeof g[Symbol.iterator], g[Symbol.iterator]() === g);
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_return_value_identity_and_printing"),
        "{ value: 9, done: true } { value: undefined, done: true }\nObject [Generator] {} [object Generator] object function true\n"
    );
}

#[test]
fn generator_any_param_and_array_slot() {
    let source = r##"function* gen() { const x = yield 1; console.log("got", x); yield x * 2; }
function use(g: any): any { g.next(); return g.next(21).value; }
function main(): void {
  console.log(use(gen()));
  const arr: any[] = [gen()];
  console.log([...arr[0]].length);
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_param_and_array_slot"),
        "got 21\n42\ngot undefined\n2\n"
    );
}

#[test]
fn generator_any_throw_into_any_generator() {
    let source = r##"function* gen() { try { yield 1; } catch (e: any) { console.log("caught", e.message); yield 7; } }
function main(): void {
  const g: any = gen();
  g.next();
  console.log(g.throw(new Error("x")));
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_throw_into_any_generator"),
        "caught x\n{ value: 7, done: false }\n"
    );
}

#[test]
fn generator_any_yield_star_over_any_generator() {
    let source = r##"function* inner() { yield 10; yield 11; return 12; }
function* outer(g: any): Generator<any, void, unknown> { const r = yield* g; console.log("r", r); yield 99; }
function main(): void {
  for (const v of outer(inner() as any)) console.log(v);
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_yield_star_over_any_generator"),
        "10\n11\nr 12\n99\n"
    );
}

#[test]
fn generator_any_for_of_break_runs_finally() {
    let source = r##"function* early() { try { yield 1; yield 2; } finally { console.log("closed"); } }
function main(): void {
  const e: any = early();
  for (const v of e) { console.log(v); break; }
  console.log(e.next());
}
"##;
    assert_eq!(
        compile_and_run(source, "generator_any_for_of_break_runs_finally"),
        "1\nclosed\n{ value: undefined, done: true }\n"
    );
}
