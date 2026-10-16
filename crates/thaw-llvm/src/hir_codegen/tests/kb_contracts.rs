// Simple unconditional receiver accesses match Node 22; ambiguous bodies keep the generic error.

#[test]
fn kb_contract_bare_call_of_this_function_reports_first_property() {
    let source = r##"const lam = function (this: { n: number }, k: number): number { return k + this.n; };
const setter = function (this: { n: number }): void { this.n = 3; };
async function main(): Promise<void> {
  try { console.log(lam(1)); } catch (e: any) { console.log("E1 " + e.message); }
  try { setter(); } catch (e: any) { console.log("E2 " + e.message); }
  console.log(lam.call({ n: 5 }, 1));
  try { console.log(lam.call(null as any, 1)); } catch (e: any) { console.log("E3 " + e.message); }
}
"##;
    assert_eq!(
        compile_and_run(source, "kb_contract_bare_call_of_this_function_reports_first_property"),
        "E1 Cannot read properties of undefined (reading 'n')\nE2 Cannot set properties of undefined (setting 'n')\n6\nE3 Cannot read properties of null (reading 'n')\n"
    );
}

#[test]
fn kb_contract_receiver_messages_do_not_guess_from_compile_order() {
    let source = r#"function main(): void {
      const branch = function(this: {a:number,b:number}, flag:boolean):number { if(flag) return this.a; return this.b; };
      const arrow = function(this: {a:number,b:number}):number { const unused = () => this.a; return this.b; };
      const assign = function(this: {a:number,b:number}):void { this.a = this.b; };
      try { branch(false); } catch(e:any) { console.log(e.message); }
      try { arrow(); } catch(e:any) { console.log(e.message); }
      try { assign(); } catch(e:any) { console.log(e.message); }
    }"#;
    assert_eq!(compile_and_run(source, "kb_contract_receiver_messages_do_not_guess_from_compile_order"),
      "Incompatible function receiver\nIncompatible function receiver\nCannot read properties of undefined (reading 'b')\n");
}
