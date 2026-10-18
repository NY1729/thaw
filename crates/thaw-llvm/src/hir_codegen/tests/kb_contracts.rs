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

#[test]
fn kb_contract_receiver_validation_occurs_at_actual_access() {
    let source = r#"let effects = 0;
const read = function(this: { a: number, b: number }, label: string): number {
  effects++;
  console.log(label + " before " + effects);
  return this.b;
};
const branch = function(this: { a: number, b: number }, flag: boolean): string {
  if (flag) { console.log("branch access"); return String(this.b); }
  console.log("branch skip"); return "ok";
};
const arrow = function(this: { a: number, b: number }): () => number {
  console.log("arrow created"); return () => this.b;
};
const assign = function(this: { a: number, b: number }): number { this.a = this.b; return this.a; };
const validRead = function(this: { n: number }): number { return this.n; };
const object = { b: 4, method(): number { console.log("method before"); return this.b; } };
const extracted = object.method;
function main(): void {
  try { read.call(undefined as any, "undefined"); } catch (error: any) { console.log("undefined read error " + error.message); }
  try { read.call(null as any, "null"); } catch (error: any) { console.log("null read error " + error.message); }
  console.log("valid read value " + validRead.call({ n: 7 }));
  try { console.log("skip value " + branch.call(undefined as any, false)); }
  catch (error: any) { console.log("skip error " + error.message); }
  try { branch.call(undefined as any, true); } catch (error: any) { console.log("branch error " + error.message); }
  try { arrow.call(undefined as any); } catch (error: any) { console.log("unused arrow error " + error.message); }
  const holder = { later: (): number => 0 };
  try { holder.later = arrow.call(undefined as any); } catch (error: any) { console.log("arrow creation error " + error.message); }
  try { holder.later(); } catch (error: any) { console.log("arrow later error " + error.message); }
  try { assign.call(undefined as any); } catch (error: any) { console.log("assignment error " + error.message); }
  try { extracted(); } catch (error: any) { console.log("extracted error " + error.message); }
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_receiver_validation_occurs_at_actual_access"),
        "undefined before 1\nundefined read error Cannot read properties of undefined (reading 'b')\nnull before 2\nnull read error Cannot read properties of null (reading 'b')\nvalid read value 7\nbranch skip\nskip value ok\nbranch access\nbranch error Cannot read properties of undefined (reading 'b')\narrow created\narrow created\narrow later error Cannot read properties of undefined (reading 'b')\nassignment error Cannot read properties of undefined (reading 'b')\nmethod before\nextracted error Cannot read properties of undefined (reading 'b')\n"
    );
}

#[test]
fn kb_contract_bare_this_value_preserves_nullish_receiver() {
    let source = r#"const value = function(this: { n: number }): any { return this; };
function main(): void {
  console.log(value.call(null as any) === null);
  console.log(value.call(undefined as any) === undefined);
  console.log(value.call({ n: 3 }).n);
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_bare_this_value_preserves_nullish_receiver"),
        "true\ntrue\n3\n"
    );
}

#[test]
fn kb_contract_typeof_this_uses_runtime_receiver_value() {
    let source = r#"const typeOfThis = function(this: { n: number }): string { return typeof this; };
function main(): void {
  try { console.log(typeOfThis.call(undefined as any)); } catch (error: any) { console.log(error.message); }
  try { console.log(typeOfThis.call(null as any)); } catch (error: any) { console.log(error.message); }
  console.log(typeOfThis.call({ n: 3 }));
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_typeof_this_uses_runtime_receiver_value"),
        "undefined\nobject\nobject\n"
    );
}

#[test]
fn kb_contract_ordinary_this_access_is_lazy_inside_body_catch() {
    let source = r#"let effects = 0;
const read = function(this: { value: number }, run: boolean): string {
  effects++;
  if (!run) return "skipped";
  try { return String(this.value); }
  catch (error: any) { return "caught " + error.message; }
};
const delayed = function(this: { value: number }): () => string {
  const unused = () => this.value;
  return () => {
    try { return String(this.value); }
    catch (error: any) { return "arrow caught " + error.message; }
  };
};
function main(): void {
  console.log(read.call(undefined as any, false) + " effects " + effects);
  console.log(read.call(null as any, true) + " effects " + effects);
  console.log(delayed.call(undefined as any)());
  console.log(read.call({ value: 8 }, true));
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_ordinary_this_access_is_lazy_inside_body_catch"),
        "skipped effects 1\ncaught Cannot read properties of null (reading 'value') effects 2\narrow caught Cannot read properties of undefined (reading 'value')\n8\n");
}
