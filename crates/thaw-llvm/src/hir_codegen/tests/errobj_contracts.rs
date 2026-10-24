// Error values are framed strings, so their own properties live in a runtime side table keyed by the
// framed pointer (`thaw_error_set_prop` & co). Expected output is real node 22 output
// (`node --experimental-strip-types`); `error_obj_equal_text_errors_known_bad` pins the one known divergence.

#[test]
fn error_obj_props_roundtrip_in_delete() {
    let source = r#"function main(): void {
  const e = new Error("boom") as any;
  e.code = "E_X";
  e.status = 404;
  e.meta = { a: [1, 2], ok: true };
  console.log(Object.keys(e), e.code, e.status, e.meta, e.message, String(e));
  console.log("code" in e, "nope" in e, "message" in e);
  delete e.code;
  console.log(Object.keys(e), "code" in e, e.code);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_props_roundtrip_in_delete"),
        "[ 'code', 'status', 'meta' ] E_X 404 { a: [ 1, 2 ], ok: true } boom Error: boom\ntrue false true\n[ 'status', 'meta' ] false undefined\n"
    );
}

#[test]
fn error_obj_independent_props() {
    let source = r#"function main(): void {
  const a = new Error("x") as any;
  const b = new Error("x") as any;
  a.tag = "A";
  b.tag = "B";
  console.log(a.tag, b.tag, Object.keys(a), Object.keys(b));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_independent_props"),
        "A B [ 'tag' ] [ 'tag' ]\n"
    );
}

#[test]
fn error_obj_name_and_message_assignment() {
    let source = r#"function main(): void {
  const e = new TypeError("t") as any;
  e.name = "Custom";
  console.log(e.name, String(e), e.message, Object.keys(e));
  e.message = "m2";
  console.log(e.message, String(e), e.stack.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_name_and_message_assignment"),
        "Custom Custom: t t [ 'name' ]\nm2 Custom: m2 Custom: m2\n"
    );
}

#[test]
fn error_obj_stack_serialized_on_first_read() {
    let source = r#"function main(): void {
  const e = new Error("s") as any;
  console.log(e.stack.split("\n")[0]);
  e.message = "late";
  console.log(e.stack.split("\n")[0], String(e));
  e.stack = "custom stack";
  console.log(e.stack, Object.keys(e));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_stack_serialized_on_first_read"),
        "Error: s\nError: s Error: late\ncustom stack []\n"
    );
}

#[test]
fn error_obj_object_assign_sources() {
    let source = r#"function main(): void {
  const e = Object.assign(new Error("m"), { a: 1, b: "two" }, { c: true }) as any;
  console.log(Object.keys(e), e.a, e.b, e.c);
  const j: any = { d: [1, 2] };
  Object.assign(e, j);
  console.log(Object.keys(e), e.d);
  console.log(Object.getOwnPropertyNames(e));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_object_assign_sources"),
        "[ 'a', 'b', 'c' ] 1 two true\n[ 'a', 'b', 'c', 'd' ] [ 1, 2 ]\n[ 'stack', 'message', 'a', 'b', 'c', 'd' ]\n"
    );
}

#[test]
fn error_obj_props_survive_throw_typed() {
    let source = r#"function main(): void {
  try {
    const x = new RangeError("r") as any;
    x.code = "ERANGE";
    x.n = 3;
    throw x;
  } catch (c: any) {
    console.log(c.code, c.n, c.message, c.name, c instanceof RangeError);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_props_survive_throw_typed"),
        "ERANGE 3 r RangeError true\n"
    );
}

#[test]
fn error_obj_computed_keys_and_stringify() {
    let source = r#"function main(): void {
  const e = new Error("p") as any;
  e[`k${1}`] = "v";
  const key = "dyn";
  e[key] = 2;
  console.log(Object.keys(e), e.k1, e.dyn);
  console.log(JSON.stringify(e), JSON.stringify({ err: 1 }), JSON.stringify("s"));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_computed_keys_and_stringify"),
        "[ 'k1', 'dyn' ] v 2\n{\"k1\":\"v\",\"dyn\":2} {\"err\":1} \"s\"\n"
    );
}

#[test]
fn error_obj_missing_props_and_string_keys() {
    let source = r#"function main(): void {
  console.log((new Error("zz") as any).nothing, ("abc" as any).foo);
  console.log(Object.keys("ab"), Object.getOwnPropertyNames("ab"));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_missing_props_and_string_keys"),
        "undefined undefined\n[ '0', '1' ] [ '0', '1', 'length' ]\n"
    );
}

#[test]
fn error_obj_aliases_share_props() {
    let source = r#"function main(): void {
  const e = new Error("outer") as any;
  e.code = 1;
  const f = e;
  f.code = 2;
  console.log(e.code, e === f);
  const g = new Error("outer") as any;
  g.code = 7;
  console.log(e.code, g.code);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_aliases_share_props"),
        "2 true\n2 7\n"
    );
}

#[test]
fn error_obj_plain_string_assignment_throws() {
    let source = r#"function main(): void {
  const s: any = "plain";
  try {
    (s as any).x = 1;
    console.log("no throw");
  } catch (e: any) {
    console.log(e.name, e.message);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_plain_string_assignment_throws"),
        "TypeError Cannot create property 'x' on string 'plain'\n"
    );
}

#[test]
fn error_obj_nested_values() {
    let source = r#"function main(): void {
  const e = new Error("n") as any;
  const copy = { ...{ q: 1 } };
  e.count = 1;
  e.list = [1, 2, 3];
  console.log(e.count, e.list.length);
  console.log(Object.getOwnPropertyNames(e), Object.keys(e).length);
  const err2: Error = new Error("typed");
  (err2 as any).who = "me";
  console.log((err2 as any).who, err2.message);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_nested_values"),
        "1 3\n[ 'stack', 'message', 'count', 'list' ] 2\nme typed\n"
    );
}

#[test]
fn error_obj_reassigned_binding_and_nullish() {
    let source = r#"function main(): void {
  let e = new Error("o") as any;
  e.code = "A";
  e = new Error("o") as any;
  console.log(e.code, Object.keys(e));
  e.code = "B";
  console.log(e.code);
  e.x = undefined;
  e.y = null;
  console.log(Object.keys(e), e.y);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_reassigned_binding_and_nullish"),
        "undefined []\nB\n[ 'code', 'x', 'y' ] null\n"
    );
}

#[test]
fn error_obj_catch_binding_write() {
    let source = r#"function main(): void {
  try {
    throw new Error("t");
  } catch (e: any) {
    e.code = "Z";
    console.log(e.code, e.message);
    try { throw e; } catch (f: any) { console.log(f.code, f === e); }
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_catch_binding_write"),
        "Z t\nZ true\n"
    );
}

#[test]
fn error_obj_caught_props_and_keys() {
    let source = r#"function thrower(): void {
  const e = new Error("deep") as any;
  e.info = { n: 1 };
  throw e;
}
function main(): void {
  try { thrower(); } catch (e: any) { console.log(e.info, e.message, Object.keys(e)); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_caught_props_and_keys"),
        "{ n: 1 } deep [ 'info' ]\n"
    );
}

#[test]
fn error_obj_keys_of_caught_values() {
    let source = r#"function main(): void {
  try { throw { a: 1, b: 2 }; } catch (e: any) { console.log(Object.keys(e)); }
  try { throw "ab"; } catch (e: any) { console.log(Object.keys(e)); }
  try { throw new Error("x"); } catch (e: any) { console.log(Object.keys(e)); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_keys_of_caught_values"),
        "[ 'a', 'b' ]\n[ '0', '1' ]\n[]\n"
    );
}

#[test]
fn error_obj_keys_and_plain_error() {
    let source = r#"function main(): void {
  const e: Error = new Error("boom");
  (e as any).code = "E_X";
  console.log(Object.keys(e), (e as any).code, e.message, String(e));
  const plain = new Error("p");
  console.log(Object.keys(plain), Object.getOwnPropertyNames(plain));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_keys_and_plain_error"),
        "[ 'code' ] E_X boom Error: boom\n[] [ 'stack', 'message' ]\n"
    );
}

#[test]
fn error_obj_message_assignment() {
    let source = r#"function main(): void {
  const e = new Error("x");
  e.message = "changed";
  console.log(e.message, String(e), e.stack!.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_message_assignment"),
        "changed Error: changed Error: changed\n"
    );
}

#[test]
fn error_obj_object_assign() {
    let source = r#"function main(): void {
  console.log((Object.assign(new Error("c"), { extra: 1 }) as any).extra);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_object_assign"),
        "1\n"
    );
}

#[test]
fn error_obj_props_survive_throw() {
    let source = r#"function main(): void {
  try { const x: any = new Error("x"); x.foo = {a:1}; throw x; } catch (c: any) { console.log(c.foo, c.message); }
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_props_survive_throw"),
        "{ a: 1 } x\n"
    );
}

#[test]
fn error_obj_json_stringify() {
    let source = r#"function main(): void {
  const e = new Error("m") as any; e.code = "C";
  console.log(JSON.stringify(e), JSON.stringify(new Error("z")));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_json_stringify"),
        "{\"code\":\"C\"} {}\n"
    );
}

#[test]
fn error_obj_json_stringify_variants() {
    let source = r#"function main(): void {
  console.log(JSON.stringify(new Error("z")));
  const e = new Error("m");
  console.log(JSON.stringify(e));
  console.log(JSON.stringify("plain"));
  (e as any).k = 1;
  console.log(JSON.stringify(e));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_json_stringify_variants"),
        "{}\n{}\n\"plain\"\n{\"k\":1}\n"
    );
}

#[test]
fn error_obj_capture_stack_trace() {
    let source = r#"function main(): void {
  const e = new Error("a");
  Error.captureStackTrace(e);
  console.log(typeof e.stack, e.stack!.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_capture_stack_trace"),
        "string Error: a\n"
    );
}

// Pins today's behavior for the identity work (E2): `===` through throw/catch is true, but two distinct
// Errors with equal text also compare equal (Node prints `false` there). Flip the second `true` in E2.
#[test]
fn error_obj_equal_text_errors_known_bad() {
    let source = r#"function main(): void {
  const a = new Error("x");
  try { throw a; } catch (e: any) { console.log(e === a, a === a); }
  const b = new Error("x");
  const c = new Error("x");
  // Known-bad today: distinct Errors with equal text compare strictly equal (Node: false).
  console.log(b === c, String(b), `${c}`);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_obj_equal_text_errors_known_bad"),
        "true true\ntrue Error: x Error: x\n"
    );
}
