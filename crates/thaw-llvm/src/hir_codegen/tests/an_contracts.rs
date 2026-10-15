// Wave AN: `any` receivers, extracted methods, class instances in `any`. Expected output recorded from node 22.

#[test]
fn an_contract_plain_object_method_called_through_any() {
    let source = r##"const o = { n: 0, inc() { this.n++; return this.n; } };
async function main(): Promise<void> {
  const a: any = o;
  console.log(a.inc(), a.inc(), o.n);
}
"##;
    assert_eq!(
        compile_and_run(source, "an_contract_plain_object_method_called_through_any"),
        "1 2 2\n"
    );
}

// A class instance in `any` is a live view of the native owner, methods included.
#[test]
fn an_contract_class_method_through_any_mutates_the_owner() {
    let source = r##"class C { n = 1; bump(): number { this.n += 1; return this.n; } }
async function main(): Promise<void> {
  const c = new C();
  const a: any = c;
  try { a.bump(); console.log("called"); } catch (e) { console.log("threw"); }
  console.log(c.n, a.n);
}
"##;
    assert_eq!(
        compile_and_run(source, "an_contract_class_method_through_any_mutates_the_owner"),
        "called\n2 2\n"
    );
}

#[test]
fn an_contract_extracted_method_is_callable() {
    let source = r##"const o = { k: 7, double(x: number): number { return x * 2; } };
async function main(): Promise<void> {
  const f = o.double;
  const g = o.double;
  console.log(f(4), typeof f, g(5), o.double(6));
}
"##;
    assert_eq!(
        compile_and_run(source, "an_contract_extracted_method_is_callable"),
        "8 function 10 12\n"
    );
}

#[test]
fn an_contract_async_main_throwing_before_first_await_fails_the_process() {
    let source = r##"async function main(): Promise<void> {
  console.log("before");
  throw new TypeError("boom");
}
"##;
    let (stdout, stderr, code) = compile_and_run_failing(
        source,
        "an_contract_async_main_throwing_before_first_await",
    );
    assert_eq!(stdout, "before\n");
    assert!(stderr.contains("boom"), "{stderr}");
    assert_eq!(code, Some(1));
}
