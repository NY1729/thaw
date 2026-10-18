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
fn kb_contract_this_assignment_and_call_order_matches_node() {
    let source = r#"let trace = "";
const assign = function(this: { a: number }, rhs: () => number): number { this.a = rhs(); return 0; };
const literalAssign = function(this: { a: number }, rhs: () => number): number { this["a"] = rhs(); return 0; };
const copy = function(this: { a: number, b: number }): number { this.a = this.b; return 0; };
const computed = function(this: any, key: () => string): any { return this[key()]; };
const dynamicNamedAssign = function(this: any, rhs: () => number): number { this.a = rhs(); return 0; };
const dynamicComputedAssign = function(this: any, key: () => string, rhs: () => number): number { this[key()] = rhs(); return 0; };
const invokeMethod = function(this: { method: (n: number) => number }, arg: () => number): number { return this.method(arg()); };
const invokeAliasMethod = function(this: { method: (n: number) => number }, arg: () => number): number { const self = this; return self.method(arg()); };
const upper = function(this: string): string { const self = this; return self.toUpperCase(); };
const bigintText = function(this: bigint): string { const self = this; return self.toString(); };
const callOwner = { get method(): (n: number) => number { trace += "method "; return (n: number): number => n; } };
const compound = function(this: { a: number }, rhs: () => number): number { this.a += rhs(); return 0; };
const update = function(this: { a: number }): number { this.a++; return 0; };
const nested = function(this: { child: { a: number } }, rhs: () => number): number { this.child.a = rhs(); return 0; };
const freeze = function(this: { a: number }, next: { a: number }): number { let self = this; self.a = (() => { self = next; return 7; })(); return this.a * 10 + self.a; };
function main(): void {
  const rhs = (): number => { trace += "rhs "; return 5; };
  try { assign.call(undefined as any, rhs); } catch (e: any) { console.log("undefined assign " + trace + e.message); }
  trace = "";
  try { assign.call(null as any, rhs); } catch (e: any) { console.log("null assign " + trace + e.message); }
  trace = "";
  try { literalAssign.call(undefined as any, rhs); } catch (e: any) { console.log("literal assign " + trace + e.message); }
  try { copy.call(undefined as any); } catch (e: any) { console.log("copy " + e.message); }
  trace = "";
  try { console.log("computed value " + computed.call(undefined as any, () => { trace += "key "; return "a"; })); } catch (e: any) { console.log("computed " + trace + e.message); }
  trace = "";
  try { dynamicNamedAssign.call(undefined as any, () => { trace += "rhs "; return 1; }); } catch (e: any) { console.log("dynamic named " + trace + e.message); }
  trace = "";
  try { dynamicComputedAssign.call(undefined as any, () => { trace += "key "; return "a"; }, () => { trace += "rhs "; return 1; }); } catch (e: any) { console.log("dynamic computed " + trace + e.message); }
  trace = "";
  try { dynamicComputedAssign.call(null as any, () => { trace += "key "; return "a"; }, () => { trace += "rhs "; return 1; }); } catch (e: any) { console.log("dynamic computed null " + trace + e.message); }
  trace = "";
  try { invokeMethod.call(undefined as any, () => { trace += "arg "; return 1; }); } catch (e: any) { console.log("this method " + trace + e.message); }
  trace = "";
  try { invokeAliasMethod.call(undefined as any, () => { trace += "alias arg "; return 1; }); } catch (e: any) { console.log("alias method " + trace + e.message); }
  trace = "";
  console.log("alias builtin " + upper.call("xy"));
  console.log("bigint builtin " + bigintText.call(3n));
  console.log("call " + callOwner.method((() => { trace += "arg "; return 1; })()) + " " + trace);
  trace = "";
  try { compound.call(undefined as any, rhs); } catch (e: any) { console.log("compound " + e.message); }
  try { update.call(undefined as any); } catch (e: any) { console.log("update " + e.message); }
  trace = "";
  try { nested.call(undefined as any, rhs); } catch (e: any) { console.log("nested " + trace + e.message); }
  try { const copyTarget: { a: number, b: number } = { a: 4, b: 6 }; copy.call(copyTarget); console.log("valid copy " + copyTarget.a); } catch (e: any) { console.log("valid copy error " + e.message); }
  try { console.log("valid computed " + computed.call({ a: 6 }, () => "a")); } catch (e: any) { console.log("valid computed error " + e.message); }
  const original: { a: number } = { a: 1 };
  const next: { a: number } = { a: 2 };
  console.log("frozen target " + freeze.call(original, next) + " " + original.a + " " + next.a);
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_this_assignment_and_call_order_matches_node"),
        "undefined assign rhs Cannot set properties of undefined (setting 'a')\nnull assign rhs Cannot set properties of null (setting 'a')\nliteral assign rhs Cannot set properties of undefined (setting 'a')\ncopy Cannot read properties of undefined (reading 'b')\ncomputed key Cannot read properties of undefined (reading 'a')\ndynamic named rhs Cannot set properties of undefined (setting 'a')\ndynamic computed key rhs Cannot set properties of undefined (setting 'a')\ndynamic computed null key rhs Cannot set properties of null (setting 'a')\nthis method Cannot read properties of undefined (reading 'method')\nalias method Cannot read properties of undefined (reading 'method')\nalias builtin XY\nbigint builtin 3\ncall 1 method arg \ncompound Cannot read properties of undefined (reading 'a')\nupdate Cannot read properties of undefined (reading 'a')\nnested Cannot read properties of undefined (reading 'child')\nvalid copy 6\nvalid computed 6\nfrozen target 72 7 2\n"
    );
}

#[test]
fn kb_contract_valid_receiver_assignment_update_and_nested_targets() {
    let source = r#"const assign = function(this: { a: number, b: number }, rhs: () => number): number { this.a = rhs(); return this.a; };
const update = function(this: { a: number }): number { this.a++; return this.a; };
const nested = function(this: { child: { a: number } }, rhs: () => number): number { this.child.a = rhs(); return this.child.a; };
function main(): void {
  const assigned = { a: 1, b: 2 };
  console.log("assign", assign.call(assigned, () => 7), assigned.a, assigned.b);
  const updated = { a: 2 };
  console.log("update", update.call(updated), updated.a);
  const parent = { child: { a: 3 } };
  console.log("nested", nested.call(parent, () => 8), parent.child.a);
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_valid_receiver_assignment_update_and_nested_targets"),
        "assign 7 7 2\nupdate 3 3\nnested 8 8\n"
    );
}

#[test]
fn kb_contract_object_method_receiver_is_deferred_until_body_access() {
    let source = r#"let effects = 0;
const object = { value: 4, method(): number { effects++; console.log("method before " + effects); return this.value; } };
const extracted = object.method;
function main(): void {
  try { extracted(); } catch (error: any) { console.log(error.message); }
  console.log("effects " + effects);
  try { extracted.call(undefined as any); } catch (error: any) { console.log(error.message); }
  console.log("effects " + effects);
  const invoke: any = new Function("o", "return o.method.call(undefined)");
  const invokeValid: any = new Function("o", "return o.method.call(o)");
  try { invoke(object); } catch (error: any) { console.log("JS invoke error " + error.message); }
  console.log("effects " + effects);
  console.log("JS valid " + invokeValid(object));
  const branch = { value: 9, method(run: boolean): number { if (!run) return 0; return this.value; } };
  console.log("branch skip " + branch.method.call(undefined as any, false));
  try { branch.method.call(null as any, true); } catch (error: any) { console.log("null read " + error.message); }
  console.log("extracted valid " + extracted.call({ value: 7 } as any));
  console.log("direct " + object.method());
  const nested = { value: 5, method(): number { function own(): number { return this.value; } const arrow = () => this.value; return own.call({ value: 7 }) + arrow(); } };
  console.log("nested " + nested.method());
  const optionalRest = { value: 10, method(prefix: string = "x", ...values: number[]): number { return this.value + prefix.length + values.length; } };
  console.log("optional/rest " + optionalRest.method() + " " + optionalRest.method("ab", 1, 2));
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_object_method_receiver_is_deferred_until_body_access"),
        "method before 1\nCannot read properties of undefined (reading 'value')\neffects 1\nmethod before 2\nCannot read properties of undefined (reading 'value')\neffects 2\nmethod before 3\nJS invoke error Cannot read properties of undefined (reading 'value')\neffects 3\nmethod before 4\nJS valid 4\nbranch skip 0\nnull read Cannot read properties of null (reading 'value')\nmethod before 5\nextracted valid 7\nmethod before 6\ndirect 4\nnested 12\noptional/rest 11 14\n");
}

#[test]
fn kb_contract_plain_object_method_value_and_computed_read_keep_live_receiver() {
    let source = r#"const object = {
  n: 0,
  increment(): any { this.n++; return this; },
  read(): number { return this["n"]; }
};
function main(): void {
  console.log(object.increment().increment() === object, object.n, object.read());
  const callFromJavaScript: any = new Function("owner", "return owner.read()");
  console.log(callFromJavaScript(object), object.n);
}"#;
    assert_eq!(
        compile_and_run(source, "kb_contract_plain_object_method_value_live_receiver"),
        "true 2 2\n2 2\n"
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
fn kb_contract_typed_this_value_keeps_actual_kind_and_alias() {
    let source = r#"const actual = function(this: { n: number }): any { return this; };
const kind = function(this: { n: number }): string { return typeof this; };
const alias = function(this: { n: number }): any { const self = this; this.n++; return self; };
const arrow = function(this: { n: number }): any { const read = () => this; return read(); };
function main(): void {
  console.log(kind.call(undefined as any));
  console.log(kind.call(null as any));
  console.log(kind.call(false as any));
  console.log(kind.call(0 as any));
  console.log(kind.call("x" as any));
  console.log(kind.call(1n as any));
  console.log(kind.call(Symbol("s") as any));
  console.log(kind.call("host" as any));
  console.log(kind.call({ n: 1 }));
  const native = { n: 4, extra: 9 };
  const result = actual.call(native);
  console.log(result === native, result.extra);
  const aliasResult = alias.call(native);
  console.log(aliasResult === native, aliasResult.n, native.n);
  console.log(arrow.call(native) === native);
  console.log(actual.call(undefined as any) === undefined);
  const hostKind: any = new Function("fn", "return fn.call(Object.create(null))");
  console.log(hostKind(kind));
  const hostSymbol: any = new Function("fn", "const symbol=Symbol('identity'); return fn.call(symbol)===symbol && fn.call(symbol)!==Symbol('identity')");
  console.log(hostSymbol(actual));
  const hostActual: any = new Function("fn", "value", "return fn.call(value)");
  console.log(hostActual(actual, native) === native);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_typed_this_value_keeps_actual_kind_and_alias"),
        "undefined\nobject\nboolean\nnumber\nstring\nbigint\nsymbol\nstring\nobject\ntrue 9\ntrue 5 5\ntrue\ntrue\nobject\ntrue\ntrue\n");
}

#[test]
fn kb_contract_bare_this_value_keeps_declared_primitive_return_types() {
    let source = r#"const bigintThis = function(this: bigint): bigint { return this; };
const symbolThis = function(this: symbol): symbol { return this; };
function main(): void {
  const value = Symbol("identity");
  console.log(bigintThis.call(12n), symbolThis.call(value) === value);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_value_primitive_returns"), "12n true\n");
}

#[test]
fn kb_contract_bare_this_alias_preserves_typed_primitive_return() {
    let source = r#"const bigintThis = function(this: bigint): bigint { const self: bigint = this; return self; };
function main(): void { console.log(bigintThis.call(12n)); }"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_alias_primitive_return"), "12n\n");
}

#[test]
fn kb_contract_bare_this_alias_preserves_lazy_payload_and_typed_values() {
    let source = r#"const bigintThis = function(this: bigint): bigint { const self = this; return self; };
const symbolThis = function(this: symbol): symbol { const self = this; return self; };
const mutableBigintThis = function(this: bigint): bigint { let self = this; self = this; return self; };
const capturedSymbolThis = function(this: symbol): symbol { const self = this; const read = (): symbol => self; return read(); };
const unused = function(this: { n: number }): number { const self = this; return 3; };
function main(): void {
  const symbol = Symbol("same");
  console.log(bigintThis.call(12n), mutableBigintThis.call(14n));
  console.log(symbolThis.call(symbol) === symbol, capturedSymbolThis.call(symbol) === symbol);
  console.log(unused.call(null as any));
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_alias_private_payload"), "12n 14n\ntrue true\n3\n");
}

#[test]
fn kb_contract_bare_this_alias_chain_and_native_reassignment_keep_actual_owner() {
    let source = r#"const identity = function(this: { n: number }, replacement: { n: number }): any {
  let first = this;
  const second = first;
  first = replacement;
  return second === this ? first : second;
};
function main(): void {
  const original = { n: 1 };
  const replacement = { n: 4, extra: 9 };
  const result = identity.call(original, replacement);
  console.log(result === replacement, result.n, result.extra);
  result.extra = 12;
  console.log(replacement.extra);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_alias_chain_native_reassignment"), "true 4 9\n12\n");
}

#[test]
fn kb_contract_bare_this_alias_reassignment_preserves_reordered_layout() {
    let source = r#"const replace = function(this: { n: number }): any {
  let self = this;
  self = { extra: 9, n: 4 };
  self.n++;
  return self;
};
function main(): void {
  const result = replace.call({ n: 1 });
  console.log(result.n, result.extra);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_alias_reordered_reassignment"), "5 9\n");
}

#[test]
fn kb_contract_bare_this_class_return_preserves_derived_owner() {
    let source = r#"class Base { base = 1; }
class Derived extends Base { extra = 9; }
const identity = function(this: Base): Base { return this; };
function main(): void {
  const actual = new Derived();
  const result = identity.call(actual);
  console.log((result as any) === (actual as any), result instanceof Derived, result.extra);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_bare_this_class_return"), "true true 9\n");
}

#[test]
fn kb_contract_typed_this_reordered_layout_keeps_declared_property_access() {
    let source = r#"const update = function(this: { n: number }): number { this.n++; return this.n; };
function main(): void {
  const reordered = { extra: 9, n: 4 };
  console.log(update.call(reordered), reordered.n, reordered.extra);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_typed_this_reordered_layout_keeps_declared_property_access"), "5 5 9\n");
}

#[test]
fn kb_contract_object_method_this_value_keeps_full_owner_alias() {
    let source = r#"const object = { n: 1, extra: 9, bump(): any { this.n++; return this; } };
function main(): void {
  const result = object.bump();
  console.log(result === object, result.n, result.extra);
  result.extra = 12;
  console.log(object.extra);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_object_method_this_value_keeps_full_owner_alias"), "true 2 9\n12\n");
}

#[test]
fn kb_contract_dynamic_object_method_value_uses_host_this_value() {
    let source = r#"const object = { kind(): string { return typeof this; }, value(): any { return this; } };
const invoke: any = new Function("method", "receiver", "return method.call(receiver)");
function main(): void {
  console.log(invoke(object.kind, 5));
  console.log(invoke(object.value, 5));
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_dynamic_object_method_value_uses_host_this_value"), "number\n5\n");
}

#[test]
fn kb_contract_class_this_value_keeps_derived_full_owner() {
    let source = r#"class Base { value: number = 1; }
class Derived extends Base { extra: number = 9; }
const identity = function(this: Base): any { return this; };
function main(): void {
  const actual = new Derived();
  const result = identity.call(actual);
  console.log(result === actual, result.value, result.extra, result instanceof Derived);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_class_this_value_keeps_derived_full_owner"), "true 1 9 true\n");
}

#[test]
fn kb_contract_class_this_value_preserves_unrelated_actual_owner() {
    let source = r#"class Base { value: number = 1; }
class Other { extra: number = 9; }
const identity = function(this: Base): any { return this; };
function main(): void {
  const actual = new Other();
  const result = identity.call(actual);
  console.log(result === actual, result.extra, result instanceof Other);
}"#;
    assert_eq!(compile_and_run(source, "kb_contract_class_this_value_preserves_unrelated_actual_owner"), "true 9 true\n");
}

#[test]
fn kb_contract_object_method_value_observes_midbody_owner_mutation() {
    let source = r#"let object: any = {};
object = { n: 1, method(): number { const self = this; object.n = 7; return self.n; } };
function main(): void { console.log(object.method(), object.n); }"#;
    assert_eq!(compile_and_run(source, "kb_contract_object_method_value_observes_midbody_owner_mutation"), "7 7\n");
}

#[test]
fn kb_contract_native_object_method_value_aliases_owner_midbody() {
    let source = r#"const object = { n: 1, method(): number { const self = this; this.n = 7; return self.n; } };
function main(): void { console.log(object.method(), object.n); }"#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let hir = format!("{program:#?}");
    assert!(hir.contains("__thaw_register_native_object_projector"), "{hir}");
    assert_eq!(compile_and_run(source, "kb_contract_native_object_method_value_aliases_owner_midbody"), "7 7\n");
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
