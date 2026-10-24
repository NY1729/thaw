// A mutated `any` callback parameter keeps identity and `instanceof` against the native instance.

#[test]
fn anyclasses_contract_any_callback_param_keeps_identity_and_instanceof_after_a_write() {
    let source = r##"class C { x = 1; inc(): number { this.x += 1; return this.x; } }
class E extends C { y = 2; }
class D { z = 0; }
function main(): void {
  const c = new C(); const e = new E();
  setTimeout((o: any) => { o.x = 10; console.log(o === c, o instanceof C, o instanceof D, o === e, c.x, o.inc()); }, 1, c);
  setTimeout((o: any) => { o.y = 7; console.log(o === e, o instanceof E, o instanceof C, o instanceof D, e.y); }, 2, e);
  setTimeout((o: any) => { o.x = 2; console.log(o === c, o instanceof C, o.x); }, 4, { x: 1 });
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_any_callback_param_keeps_identity_and_instanceof_after_a_write"),
        "true true false false 10 11\ntrue true true false 7\nfalse false 2\n"
    );
}

#[test]
fn anyclasses_contract_any_timer_callback_param_keeps_identity_and_instanceof() {
    let source = r##"class C { x = 1; }
const c = new C();
function main(): void {
  setTimeout((o: any) => { o.x = 5; console.log(o === c, o instanceof C, c.x); }, 1, c);
}
"##;
    assert_eq!(
        compile_and_run(source, "anyclasses_contract_any_timer_callback_param_keeps_identity_and_instanceof"),
        "true true 5\n"
    );
}
