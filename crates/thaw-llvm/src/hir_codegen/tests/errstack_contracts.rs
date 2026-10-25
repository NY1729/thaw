// Error identity and props inside `any` containers, and `stack` generation (header, frame shape,
// `Error.stackTraceLimit`, `Error.captureStackTrace`). Expected output is real node 22 output
// (`node --experimental-strip-types`); frame lines are only checked by shape because their file
// paths cannot match.

#[test]
fn error_stack_json_container_props_and_identity() {
    let source = r#"function main(): void {
  const err = new Error("boom") as any;
  err.stack = "Error: boom";
  err.code = "C";
  const o: any = { e: err, n: 1 };
  console.log(o.e.message, o.e.name, o.e.code, o.e.stack);
  o.e.extra = { a: 1 };
  console.log(err.extra, o.e.extra.a, Object.keys(err), Object.keys(o.e));
  const other = new Error("boom") as any;
  other.stack = "Error: boom";
  console.log(o.e === err, o.e === other, o.e !== err, o.e !== other);
  delete o.e.extra;
  console.log("extra" in err, "code" in o.e, o.e.hasOwnProperty("code"));
  o.e.message = "changed";
  console.log(err.message, o.e.message);
  console.log(JSON.stringify(o));
  console.log(o);
  console.log(typeof o.e, o.e instanceof Error, String(o.e));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_json_container_props_and_identity"),
        "boom Error C Error: boom\n{ a: 1 } 1 [ 'code', 'extra' ] [ 'code', 'extra' ]\ntrue false false true\nfalse true true\nchanged changed\n{\"e\":{\"code\":\"C\"},\"n\":1}\n{ e: [Error: boom] { code: 'C' }, n: 1 }\nobject true Error: changed\n"
    );
}

#[test]
fn error_stack_json_arrays_records_and_params() {
    let source = r#"function main(): void {
  const a = new Error("a") as any;
  const b = new Error("a") as any;
  const arr: any[] = [a, b];
  arr[1].tag = "second";
  console.log(a.tag, b.tag, arr[0] === a, arr[1] === a, arr[1] === b, arr.indexOf(b), arr.lastIndexOf(a));
  const nested: any = { a: { list: [a] } };
  nested.a.list[0].deep = 1;
  console.log(a.deep, nested.a.list[0] === a, Object.keys(a));
  const rec: Record<string, any> = {};
  rec["k"] = b;
  rec["k"].viaRec = true;
  console.log(b.viaRec, rec["k"] === b, Object.values(rec)[0] === b, Object.values(rec)[0] === a);
  const typed: Error[] = [new Error("x"), new Error("y")];
  const view: any = typed;
  view[0].tag = "X";
  console.log((typed[0] as any).tag, view[0] === typed[0], view[1] === typed[0]);
  function touch(x: any) { x.viaParam = 1; return x; }
  const r = touch(a);
  console.log(a.viaParam, r === a, r === b);
  const back: Error = view[0] as Error;
  console.log(back === typed[0], back.message, `${view[1]}` + String(view[0]));
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_json_arrays_records_and_params"),
        "undefined second true false true 1 0\n1 true [ 'deep' ]\ntrue true true false\nX true false\n1 true false\ntrue x Error: yError: x\n"
    );
}

#[test]
fn error_stack_header_and_frame_shape() {
    let source = r#"function main(): void {
  const e = new Error("boom");
  const lines = e.stack!.split("\n");
  console.log(lines[0], lines.length > 1, /^    at /.test(lines[1]));
  const t = new TypeError("tt");
  console.log(t.stack!.split("\n")[0], new RangeError("r").stack!.startsWith("RangeError: r"));
  const e1 = new Error("x");
  e1.name = "Renamed";
  console.log(e1.stack!.split("\n")[0]);
  const e2 = new Error("y");
  console.log(e2.stack!.split("\n")[0]);
  e2.message = "late";
  console.log(e2.stack!.split("\n")[0], String(e2));
  class MyErr extends Error { constructor(m: string) { super(m); this.name = "MyErr"; } }
  const m = new MyErr("mm");
  console.log(m.stack!.split("\n")[0], m.stack!.split("\n").length > 1);
  class Plain extends Error {}
  console.log(new Plain("pp").stack!.split("\n")[0]);
  console.log(JSON.stringify(new Error("").stack!.split("\n")[0]));
  const c = new Error("outer", { cause: new Error("inner") });
  console.log(c.stack!.split("\n")[0], (c.cause as Error).stack!.split("\n")[0]);
  e.stack = "custom";
  console.log(e.stack, e.message);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_header_and_frame_shape"),
        "Error: boom true true\nTypeError: tt true\nRenamed: x\nError: y\nError: y Error: late\nMyErr: mm true\nError: pp\n\"Error\"\nError: outer Error: inner\ncustom boom\n"
    );
}

#[test]
fn error_stack_trace_limit() {
    let source = r#"function main(): void {
  console.log(typeof Error.stackTraceLimit, Error.stackTraceLimit);
  Error.stackTraceLimit = 0;
  const zero = new Error("zero");
  console.log(JSON.stringify(zero.stack));
  Error.stackTraceLimit = 1;
  const one = new Error("one");
  console.log(one.stack!.split("\n").length);
  Error.stackTraceLimit = 10;
  console.log(Error.stackTraceLimit, new Error("ten").stack!.split("\n").length > 1);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_trace_limit"),
        "number 10\n\"Error: zero\"\n2\n10 true\n"
    );
}

#[test]
fn error_stack_capture_stack_trace_targets() {
    let source = r#"function main(): void {
  const o: any = {};
  Error.captureStackTrace(o);
  console.log(typeof o.stack, o.stack.split("\n")[0], o.stack.split("\n").length > 1);
  const p: any = { name: "Foo", message: "bar" };
  Error.captureStackTrace(p);
  console.log(p.stack.split("\n")[0]);
  const q: any = { name: "", message: "only" };
  Error.captureStackTrace(q);
  console.log(q.stack.split("\n")[0]);
  const r: any = { message: "m" };
  Error.captureStackTrace(r);
  console.log(r.stack.split("\n")[0]);
  const n = new Error("n");
  Error.captureStackTrace(n);
  console.log(n.stack!.split("\n")[0]);
  class Custom extends Error { constructor(m: string) { super(m); this.name = "Custom"; Error.captureStackTrace(this); } }
  console.log(new Custom("cc").stack!.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "error_stack_capture_stack_trace_targets"),
        "string Error true\nFoo: bar\nonly\nError: m\nError: n\nCustom: cc\n"
    );
}
