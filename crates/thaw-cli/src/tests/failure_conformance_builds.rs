fn run_conformance_program(name: &str, source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    let _ = std::fs::remove_dir_all(dir);
    (
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
        result.status.code(),
    )
}

/// `Promise.reject()` rejects with `undefined` (Node 22: `caught undefined`, then `UR undefined`).
#[test]
fn conformance_promise_reject_without_argument_rejects_with_undefined() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-promise-reject",
        r#"process.on("unhandledRejection", (r: any) => { console.log("UR", r); });
Promise.reject();
Promise.reject().catch((e: any) => console.log("caught", e));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "caught undefined\nUR undefined\n");
}

/// A block-bodied callback without a value `return` satisfies a `() => any` parameter (it yields `undefined`).
#[test]
fn conformance_void_callback_satisfies_any_result() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-void-callback",
        r#"function t(f: () => any): void {
  try { const r = f(); console.log("OK", String(r)); } catch (e: any) { console.log("ERR", e.constructor.name); }
}
const o: any = undefined;
t(() => { o.x = 1; });
t(() => { return; });
t(() => "x".repeat(-1));
t(() => new ArrayBuffer(-1));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ERR TypeError\nOK undefined\nERR RangeError\nERR RangeError\n"
    );
}

/// `(0, eval)(code)` is an indirect eval, like `eval(code)`.
#[test]
fn conformance_indirect_eval_through_a_sequence_expression() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-indirect-eval",
        r#"function t(f: () => any): void {
  try { const r = f(); console.log("OK", String(r)); } catch (e: any) { console.log("ERR", e.constructor.name); }
}
t(() => (0, eval)("1+1"));
t(() => (0, eval)("nopeVar"));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "OK 2\nERR ReferenceError\n");
}

/// A `new Promise` executor that cannot resolve (no parameters, or `resolve` unused) is `Promise<void>`.
#[test]
fn conformance_new_promise_without_a_resolver_builds() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-promise-no-resolver",
        r#"const never = new Promise(() => {});
const rejOnly = new Promise((_, r) => setTimeout(() => r(new Error("slow")), 5));
never.then(() => console.log("never"));
rejOnly.catch((e: any) => console.log("rejected", e.message));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rejected slow\n");
}

/// `http.createServer()` without a listener, and `res.writeHead(n).end()` without a chunk.
#[test]
fn conformance_http_createserver_without_listener_and_chained_end() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-http-optional",
        r#"import * as http from 'node:http';
const bare = http.createServer();
bare.listen(0, () => { bare.close(); });
const srv = http.createServer((req: any, res: any) => { res.writeHead(405).end(); });
srv.listen(0, '127.0.0.1', () => {
  http.get({ port: (srv.address() as any).port, host: '127.0.0.1' }, (r: any) => {
    r.on('data', () => {});
    r.on('end', () => { console.log(r.statusCode); srv.close(); });
  });
});
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "405\n");
}
