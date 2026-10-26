// Limits left by the first Error-object round: AggregateError own keys, non-enumerable `stack` on
// plain objects, string methods on caught-error properties, `delete`/`in` on catch bindings,
// Error subclass key order, `{ cause: undefined }`, and uncaught / unhandled-rejection reports.
// Expected output is real node 22 output (`node --experimental-strip-types`).

#[test]
fn errrest2_aggregate_error_own_keys_match_node() {
    let source = r#"function main(): void {
  const ag = new AggregateError([1, 2], "m", { cause: "c" });
  console.log(Object.keys(ag), Object.getOwnPropertyNames(ag), Object.entries(ag), JSON.stringify(ag));
  console.log(Object.getOwnPropertyNames(new AggregateError([1], "m")), Object.values(ag), Reflect.ownKeys(ag));
  for (const k in ag) console.log("forin", k);
  console.log(Object.assign({}, ag), { ...ag });
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_aggregate_error_own_keys_match_node"),
        "[] [ 'stack', 'message', 'cause', 'errors' ] [] {}\n[ 'stack', 'message', 'errors' ] [] [ 'stack', 'message', 'cause', 'errors' ]\n{} {}\n"
    );
}

#[test]
fn errrest2_capture_stack_trace_on_plain_objects_is_non_enumerable() {
    let source = r#"function main(): void {
  const o: any = {};
  Error.captureStackTrace(o);
  console.log(typeof o.stack, o.stack.split("\n")[0], Object.keys(o), JSON.stringify(o));
  const p: any = { a: 1, name: "N", message: "msg" };
  Error.captureStackTrace(p);
  console.log(p.stack.split("\n")[0], Object.keys(p), Object.getOwnPropertyNames(p), JSON.stringify(p));
  console.log(p);
  const q: any = {};
  Error.captureStackTrace(q);
  delete q.stack;
  q.stack = "visible";
  console.log(Object.keys(q), Object.getOwnPropertyNames(q), JSON.stringify(q));
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_capture_stack_trace_on_plain_objects_is_non_enumerable"),
        "string Error [] {}\nN: msg [ 'a', 'name', 'message' ] [ 'a', 'name', 'message', 'stack' ] {\"a\":1,\"name\":\"N\",\"message\":\"msg\"}\n{ a: 1, name: 'N', message: 'msg' }\n[ 'stack' ] [ 'stack' ] {\"stack\":\"visible\"}\n"
    );
}

#[test]
fn errrest2_string_methods_on_caught_error_properties() {
    let source = r#"class MyErr extends Error { constructor(m: string) { super(m); this.name = "MyErr"; } }
function main(): void {
  try { throw new Error("boom"); } catch (e: any) {
    console.log(e.message.toUpperCase(), e.stack.split("\n")[0], e.message.includes("oo"), e.message.slice(1), e.message.indexOf("o"));
    console.log(e.name.startsWith("Er"), e.message.replace("b", "B"), e.message.trim().length, e.message.split("o"));
  }
  try { throw new MyErr("boom"); } catch (e: any) {
    console.log(e.message.toUpperCase(), e.stack.split("\n")[0], e.name.startsWith("My"), e.message.slice(1), e.message.indexOf("o"));
  }
  const c = new Error("outer", { cause: new Error("inner") });
  console.log((c.cause as Error).message.toUpperCase(), (c.cause as Error).stack!.split("\n")[0]);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_string_methods_on_caught_error_properties"),
        "BOOM Error: boom true oom 1\ntrue Boom 4 [ 'b', '', 'm' ]\nBOOM MyErr: boom true oom 1\nINNER Error: inner\n"
    );
}

#[test]
fn errrest2_delete_stack_and_in_on_caught_errors() {
    let source = r#"function main(): void {
  try { throw new Error("boom"); } catch (e: any) {
    console.log("stack" in e, "message" in e, "zzz" in e);
    delete e.stack;
    console.log(e.stack, "stack" in e, Object.getOwnPropertyNames(e));
  }
  const f: any = new Error("x");
  delete f.message;
  console.log(f.message === "", String(f));
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_delete_stack_and_in_on_caught_errors"),
        "true true false\nundefined false [ 'message' ]\ntrue Error\n"
    );
}

#[test]
fn errrest2_json_stringify_key_order_of_error_subclasses() {
    let source = r#"class A extends Error {
  code: number;
  constructor(m: string) { super(m); this.name = "A"; this.code = 3; }
}
class B extends Error {
  code = 4;
  constructor(m: string) { super(m); this.name = "B"; }
}
function main(): void {
  console.log(JSON.stringify(new A("x")), Object.keys(new A("x")));
  console.log(JSON.stringify(new B("x")), Object.keys(new B("x")));
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_json_stringify_key_order_of_error_subclasses"),
        "{\"code\":3,\"name\":\"A\"} [ 'code', 'name' ]\n{\"code\":4,\"name\":\"B\"} [ 'code', 'name' ]\n"
    );
}

#[test]
fn errrest2_cause_undefined_is_an_own_property() {
    let source = r#"function main(): void {
  const e = new Error("m", { cause: undefined }) as any;
  console.log("cause" in e, e.cause, Object.getOwnPropertyNames(e));
  e.stack = "Error: m\n    at x (y:1:1)";
  console.log(e);
  const t = new TypeError("t", { cause: undefined });
  console.log(Object.getOwnPropertyNames(t), "cause" in t);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_cause_undefined_is_an_own_property"),
        "true undefined [ 'stack', 'message', 'cause' ]\nError: m\n    at x (y:1:1) {\n  [cause]: undefined\n}\n[ 'stack', 'message', 'cause' ] true\n"
    );
}

/// Unhandled rejections and timer throws exit 1. An Error prints like an uncaught throw
/// (`Name: message`, the frames of the embedded runtime are dropped); any other reason adds Node's
/// `UnhandledPromiseRejection` description. Node also prints the source excerpt and frames, which a
/// native binary cannot reproduce.
#[test]
fn errrest2_unhandled_rejections_and_timer_throws_report_like_node() {
    let reason = |text: &str| {
        format!("UnhandledPromiseRejection: This error originated either by throwing inside of an async function without a catch block, or by rejecting a promise which was not handled with .catch(). The promise rejected with the reason \"{text}\". {{\n  code: 'ERR_UNHANDLED_REJECTION'\n}}\n")
    };
    let cases: [(&str, String, &str); 7] = [
        ("function main(): void { Promise.reject(\"plain reason\"); }", format!("Uncaught: plain reason\n{}", reason("plain reason")), "string"),
        ("function main(): void { Promise.reject({ code: 5 }); }", format!("Uncaught: {{ code: 5 }}\n{}", reason("#<Object>")), "object"),
        ("function main(): void { Promise.reject(undefined); }", format!("Uncaught: undefined\n{}", reason("undefined")), "undefined"),
        ("function main(): void { Promise.reject(new Error(\"rej\")); }", "Uncaught: Error: rej\n".to_string(), "error"),
        ("function main(): void { setTimeout(() => { throw new Error(\"timer\"); }, 0); }", "Error: timer\n".to_string(), "timer"),
        ("function main(): void { setTimeout(() => { throw \"str\"; }, 0); }", "str\n".to_string(), "timer_string"),
        ("async function main(): Promise<void> { await Promise.resolve(); throw new RangeError(\"late\"); }", "Uncaught: RangeError: late\n".to_string(), "async"),
    ];
    for (source, expected, label) in cases {
        let (stdout, stderr, code) = compile_and_run_failing(source, &format!("errrest2_reject_{label}"));
        assert_eq!(stdout, "", "{label}");
        assert_eq!(stderr, expected, "{label}");
        assert_eq!(code, Some(1), "{label}");
    }
}


#[test]
fn errrest2_in_operator_keeps_inherited_error_properties_and_throws_for_primitives() {
    let source = r#"function main(): void {
  try { throw { code: 5 }; } catch (e: any) {
    console.log("code" in e, "toString" in e);
  }
  try { throw new Error("boom"); } catch (e: any) {
    console.log("name" in e, "toString" in e);
  }
  function check(value: any): void {
    let converted = false;
    const key: any = { toString(): string { converted = true; return "code"; } };
    try { throw value; } catch (e: any) {
      try { console.log(key in e); }
      catch (inner: any) { console.log(inner instanceof TypeError, !converted); }
    }
  }
  check("plain");
  check(5);
  check(null);
  check(undefined);
}
"#;
    assert_eq!(
        compile_and_run(source, "errrest2_in_operator_keeps_inherited_error_properties_and_throws_for_primitives"),
        "true true\ntrue true\ntrue true\ntrue true\ntrue true\ntrue true\n"
    );
}
