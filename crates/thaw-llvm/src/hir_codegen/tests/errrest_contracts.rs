// Remaining Error-object gaps (cause printing, constructor options, stack on class instances,
// uncaught reporting, catch bindings). Expected output is real node 22 output
// (`node --experimental-strip-types`); stack frames are faked through `.stack` assignments so the
// expectations do not depend on file paths.

#[test]
fn errrest_cause_error_chain_nests_with_stacks() {
    let source = r#"function main(): void {
  const c1 = new Error("inner") as any;
  c1.stack = "Error: inner\n    at a (x:1:1)\n    at b (x:2:1)";
  const e1 = new Error("outer", { cause: c1 }) as any;
  e1.stack = "Error: outer\n    at c (x:3:1)\n    at b (x:2:1)";
  console.log(e1);
  console.log([e1]);
  console.log({ e: e1 });
  const e5 = new Error("nested", { cause: e1 }) as any;
  e5.stack = "Error: nested\n    at n (x:7:1)";
  console.log(e5);
  const e6 = new Error("withprop", { cause: c1 }) as any;
  e6.stack = "Error: withprop\n    at n (x:7:1)";
  e6.code = "E1";
  console.log(e6);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_cause_error_chain_nests_with_stacks"),
        "Error: outer\n    at c (x:3:1)\n    at b (x:2:1) {\n  [cause]: Error: inner\n      at a (x:1:1)\n      at b (x:2:1)\n}\n[\n  Error: outer\n      at c (x:3:1)\n      at b (x:2:1) {\n    [cause]: Error: inner\n        at a (x:1:1)\n        at b (x:2:1)\n  }\n]\n{\n  e: Error: outer\n      at c (x:3:1)\n      at b (x:2:1) {\n    [cause]: Error: inner\n        at a (x:1:1)\n        at b (x:2:1)\n  }\n}\nError: nested\n    at n (x:7:1) {\n  [cause]: Error: outer\n      at c (x:3:1)\n      at b (x:2:1) {\n    [cause]: Error: inner\n        at a (x:1:1)\n        at b (x:2:1)\n  }\n}\nError: withprop\n    at n (x:7:1) {\n  code: 'E1',\n  [cause]: Error: inner\n      at a (x:1:1)\n      at b (x:2:1)\n}\n"
    );
}

#[test]
fn errrest_cause_primitive_and_object_values_print_in_cause_slot() {
    let source = r#"function main(): void {
  const o: any = { code: 5 };
  const e2 = new Error("o2", { cause: o }) as any;
  e2.stack = "Error: o2\n    at c (x:3:1)";
  console.log(e2);
  const e3 = new Error("o3", { cause: "str" }) as any;
  e3.stack = "Error: o3\n    at c (x:3:1)";
  console.log(e3);
  const e4 = new Error("o4", { cause: 42 }) as any;
  e4.stack = "Error: o4\n    at c (x:3:1)";
  console.log([e4]);
  const e5 = new TypeError("o5", { cause: true }) as any;
  e5.stack = "TypeError: o5\n    at c (x:3:1)";
  console.log(e5);
  const e6 = new Error("o6", { cause: null }) as any;
  e6.stack = "Error: o6\n    at c (x:3:1)";
  console.log(e6);
  const e7 = new Error("o7", { cause: [1, "x"] }) as any;
  e7.stack = "Error: o7\n    at c (x:3:1)";
  console.log(e7);
  const e8 = new Error("o8", { cause: { a: { b: [1, 2] } } as any }) as any;
  e8.stack = "Error: o8\n    at c (x:3:1)";
  console.log(e8);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_cause_primitive_and_object_values_print_in_cause_slot"),
        "Error: o2\n    at c (x:3:1) {\n  [cause]: { code: 5 }\n}\nError: o3\n    at c (x:3:1) {\n  [cause]: 'str'\n}\n[\n  Error: o4\n      at c (x:3:1) {\n    [cause]: 42\n  }\n]\nTypeError: o5\n    at c (x:3:1) {\n  [cause]: true\n}\nError: o6\n    at c (x:3:1) {\n  [cause]: null\n}\nError: o7\n    at c (x:3:1) {\n  [cause]: [ 1, 'x' ]\n}\nError: o8\n    at c (x:3:1) {\n  [cause]: { a: { b: [Array] } }\n}\n"
    );
}

#[test]
fn errrest_cause_stackless_errors_print_bracketed() {
    let source = r#"function main(): void {
  const a = new Error("plain", { cause: "x" }) as any;
  a.stack = "Error: plain";
  console.log(a);
  const inner = new RangeError("deep") as any;
  inner.stack = "RangeError: deep";
  const b = new Error("wrap", { cause: inner }) as any;
  b.stack = "Error: wrap";
  console.log(b);
  console.log([b]);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_cause_stackless_errors_print_bracketed"),
        "[Error: plain] { [cause]: 'x' }\n[Error: wrap] { [cause]: [RangeError: deep] }\n[ [Error: wrap] { [cause]: [RangeError: deep] } ]\n"
    );
}

#[test]
fn errrest_aggregate_error_prints_cause_and_errors() {
    let source = r#"function main(): void {
  console.log(new AggregateError([new Error("a"), new TypeError("b")], "multi"));
  const one = new Error("a") as any;
  one.stack = "Error: a\n    at d (x:4:1)";
  const two = new TypeError("b") as any;
  two.stack = "TypeError: b\n    at e (x:5:1)";
  console.log(new AggregateError([one, two], "multi"));
  const ag2 = new AggregateError([1, "x"], "m2", { cause: "s" });
  console.log(ag2);
  console.log(ag2.cause, ag2.errors, ag2.message, ag2.name);
  const inner = new Error("why") as any;
  inner.stack = "Error: why\n    at w (x:6:1)";
  console.log(new AggregateError([1], "m3", { cause: inner }));
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_aggregate_error_prints_cause_and_errors"),
        "[AggregateError: multi] { [errors]: [ [Error: a], [TypeError: b] ] }\n[AggregateError: multi] {\n  [errors]: [\n    Error: a\n        at d (x:4:1),\n    TypeError: b\n        at e (x:5:1)\n  ]\n}\n[AggregateError: m2] { [cause]: 's', [errors]: [ 1, 'x' ] }\ns [ 1, 'x' ] m2 AggregateError\n[AggregateError: m3] {\n  [cause]: Error: why\n      at w (x:6:1),\n  [errors]: [ 1 ]\n}\n"
    );
}

#[test]
fn errrest_error_options_accept_any_cause_value() {
    let source = r#"function main(): void {
  const e = new Error("m", { cause: "why" });
  console.log(e.cause, "cause" in e, Object.getOwnPropertyNames(e));
  const n = new Error("m", { cause: 42 }) as any;
  console.log(n.cause, typeof n.cause);
  const o = { code: 5 };
  const p = new Error("m", { cause: o }) as any;
  console.log(p.cause, p.cause.code, Object.keys(p));
  const arr = new Error("m", { cause: [1, 2] }) as any;
  console.log(arr.cause);
  const opts: any = { cause: "dyn" };
  const d = new TypeError("m", opts) as any;
  console.log(d.cause, d instanceof TypeError);
  const noCause = new Error("m", {}) as any;
  console.log(noCause.cause, "cause" in noCause, Object.getOwnPropertyNames(noCause));
  const none = new Error("m") as any;
  console.log(none.cause);
  const nested = new Error("m", { cause: new Error("in") }) as any;
  console.log(nested.cause.message, nested.cause instanceof Error);
  const callForm = Error("m", { cause: { k: 1 } }) as any;
  console.log(callForm.cause);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_error_options_accept_any_cause_value"),
        "why true [ 'stack', 'message', 'cause' ]\n42 number\n{ code: 5 } 5 []\n[ 1, 2 ]\ndyn true\nundefined false [ 'stack', 'message' ]\nundefined\nin true\n{ k: 1 }\n"
    );
}

#[test]
fn errrest_error_cause_property_semantics() {
    let source = r#"function main(): void {
  const e = new Error("x", { cause: { code: 5 } }) as any;
  console.log(e.cause, e.cause.code, Object.keys(e), Object.getOwnPropertyNames(e), "cause" in e);
  console.log(JSON.stringify(e), String(e), e.message);
  e.cause = 7;
  console.log(e.cause, Object.keys(e));
  delete e.cause;
  console.log("cause" in e, e.cause);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_error_cause_property_semantics"),
        "{ code: 5 } 5 [] [ 'stack', 'message', 'cause' ] true\n{} Error: x x\n7 []\nfalse undefined\n"
    );
}

#[test]
fn errrest_stack_on_record_with_declared_slot() {
    let source = r#"class M { x = 1; stack?: string; }
class N { name = "Custom"; message = "note"; stack: string = ""; }
function main(): void {
  const m = new M();
  console.log(m.stack);
  Error.captureStackTrace(m);
  console.log(typeof m.stack, (m.stack as string).split("\n")[0]);
  const n = new N();
  Error.captureStackTrace(n);
  console.log(n.stack.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_stack_on_record_with_declared_slot"),
        "undefined\nstring Error\nCustom: note\n"
    );
}

#[test]
fn errrest_stack_on_record_without_slot_reads_undefined_until_captured() {
    let source = r#"class M { x = 1; }
class P { name = "Named"; message = "hello"; }
function main(): void {
  const m = new M();
  console.log(m.stack === undefined, typeof m.stack);
  Error.captureStackTrace(m);
  console.log(typeof m.stack, String(m.stack).split("\n")[0]);
  const p = new P();
  Error.captureStackTrace(p);
  console.log(String(p.stack).split("\n")[0]);
  const q = new M();
  q.stack = "custom";
  console.log(q.stack);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_stack_on_record_without_slot_reads_undefined_until_captured"),
        "true undefined\nstring Error\nNamed: hello\ncustom\n"
    );
}

#[test]
fn errrest_stack_on_error_subclass_instances() {
    let source = r#"class MyErr extends Error {
  constructor(msg: string) {
    super(msg);
    this.name = "MyErr";
    Error.captureStackTrace(this, MyErr);
  }
}
class Quiet extends Error {
  constructor(msg: string) { super(msg); }
}
class Named extends TypeError {
  constructor(msg: string) { super(msg); this.name = "Named"; }
}
function main(): void {
  const e = new MyErr("boom");
  console.log(typeof e.stack, e.stack.split("\n")[0]);
  console.log(String(e), e.message, e.name, e instanceof MyErr, e instanceof Error);
  const q = new Quiet("calm");
  console.log(q.stack.split("\n")[0]);
  q.name = "Renamed";
  console.log(q.stack.split("\n")[0], String(q));
  const n = new Named("lazy");
  console.log(n.stack.split("\n")[0]);
  n.message = "changed";
  console.log(n.stack.split("\n")[0]);
  Error.captureStackTrace(n);
  console.log(n.stack.split("\n")[0]);
  const s = new Quiet("custom");
  s.stack = "replaced";
  console.log(s.stack);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_stack_on_error_subclass_instances"),
        "string MyErr: boom\nMyErr: boom boom MyErr true true\nError: calm\nError: calm Renamed: calm\nNamed: lazy\nNamed: lazy\nNamed: changed\nreplaced\n"
    );
}

#[test]
fn errrest_stack_survives_throw_of_subclass() {
    let source = r#"class MyErr extends Error {
  constructor(msg: string) {
    super(msg);
    this.name = "MyErr";
    Error.captureStackTrace(this, this.constructor);
  }
}
function main(): void {
  try { throw new MyErr("boom"); } catch (x: any) {
    console.log(String(x.stack).split("\n")[0], x.name, x instanceof MyErr);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_stack_survives_throw_of_subclass"),
        "MyErr: boom MyErr true\n"
    );
}

#[test]
fn errrest_catch_binding_writes() {
    let source = r#"function main(): void {
  try { throw new Error("boom"); } catch (e: any) {
    e.code = "E1";
    e.extra = { a: 1 };
    console.log(e.code, e.extra, Object.keys(e), e.message);
    console.log(JSON.stringify(e));
    e.code = "E2";
    console.log(e.code, e instanceof Error, e.message);
    delete e.extra;
    console.log("extra" in e, e.extra, Object.keys(e));
  }
  try { throw new TypeError("t"); } catch (e) {
    (e as any).status = 404;
    console.log((e as any).status, e instanceof TypeError, (e as Error).message);
  }
  try { throw new Error("boom"); } catch (e: any) {
    e.message = "changed";
    e.name = "Custom";
    console.log(String(e), e.message, e.name, Object.keys(e));
    e.stack = "my stack";
    console.log(e.stack);
  }
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_catch_binding_writes"),
        "E1 { a: 1 } [ 'code', 'extra' ] boom\n{\"code\":\"E1\",\"extra\":{\"a\":1}}\nE2 true boom\nfalse undefined [ 'code' ]\n404 true t\nCustom: changed changed Custom [ 'name' ]\nmy stack\n"
    );
}

#[test]
fn errrest_json_stringify_error_subclass_instances() {
    let source = r#"class AppError extends Error {
  constructor(msg: string) { super(msg); this.name = "AppError"; }
}
class Coded extends Error {
  code: number;
  constructor(msg: string, code: number) { super(msg); this.code = code; }
}
class Plain extends Error {
  constructor(msg: string) { super(msg); }
}
function main(): void {
  const a = new AppError("bad");
  console.log(JSON.stringify(a));
  console.log(JSON.stringify({ err: a }));
  console.log(JSON.stringify([a]));
  console.log(Object.keys(a));
  const c = new Coded("bad", 3);
  console.log(JSON.stringify(c), JSON.stringify({ c }));
  const p = new Plain("bad");
  console.log(JSON.stringify(p), JSON.stringify([p, 1]));
  console.log(JSON.stringify(new Error("x")));
  console.log(JSON.stringify({ e: new TypeError("y") }));
  console.log(JSON.stringify([new Error("z"), 1]));
  const w: any = new Error("q");
  w.code = 1;
  console.log(JSON.stringify({ w }), JSON.stringify(w, null, 2));
  console.log(JSON.stringify(new AggregateError([1], "m")));
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest_json_stringify_error_subclass_instances"),
        "{\"name\":\"AppError\"}\n{\"err\":{\"name\":\"AppError\"}}\n[{\"name\":\"AppError\"}]\n[ 'name' ]\n{\"code\":3} {\"c\":{\"code\":3}}\n{} [{},1]\n{}\n{\"e\":{}}\n[{},1]\n{\"w\":{\"code\":1}} {\n  \"code\": 1\n}\n{}\n"
    );
}

/// An uncaught throw exits 1 and reports `Uncaught: <inspected value>`. Node prints the source line
/// excerpt first (skipped here: a native binary has no source positions) and then the value the
/// way `util.inspect` shows it; an Error with a `cause` or own properties prints in the same
/// nested form as `console.log` (without frames, so the header is bracketed like a stackless
/// Error in Node), and a plain Error stays `Name: message`.
#[test]
fn errrest_uncaught_throw_reports_inspected_value() {
    let cases: [(&str, &str, &str); 8] = [
        ("function main(): void { throw new Error(\"boom\"); }", "Uncaught: Error: boom\n", "error"),
        (
            "class AppError extends Error { constructor(m: string) { super(m); this.name = \"AppError\"; } }\nfunction main(): void { throw new AppError(\"bad\"); }",
            "Uncaught: AppError: bad\n",
            "subclass",
        ),
        ("function main(): void { throw \"plain string\"; }", "Uncaught: plain string\n", "string"),
        (
            "function main(): void { throw { code: 5, msg: \"obj\" }; }",
            "Uncaught: { code: 5, msg: 'obj' }\n",
            "object",
        ),
        ("function main(): void { throw 42; }", "Uncaught: 42\n", "number"),
        ("function main(): void { throw null; }", "Uncaught: null\n", "null"),
        (
            "function main(): void { throw new TypeError(\"tt\", { cause: new Error(\"root\") }); }",
            "Uncaught: [TypeError: tt] { [cause]: [Error: root] }\n",
            "cause",
        ),
        (
            "function main(): void { const e: any = new Error(\"withprops\"); e.code = \"E_X\"; throw e; }",
            "Uncaught: [Error: withprops] { code: 'E_X' }\n",
            "props",
        ),
    ];
    for (source, expected, label) in cases {
        let (stdout, stderr, code) = compile_and_run_failing(source, &format!("errrest_uncaught_{label}"));
        assert_eq!(stdout, "", "{label}");
        assert_eq!(stderr, expected, "{label}");
        assert_eq!(code, Some(1), "{label}");
    }
}
