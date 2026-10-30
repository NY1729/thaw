fn storage_allocations(source: &str) -> Vec<bool> {
    struct Allocations(Vec<swc_common::Span>);
    impl Visit for Allocations {
        fn visit_expr(&mut self, expression: &Expr) {
            if matches!(expression, Expr::Array(_) | Expr::Object(_)) {
                self.0.push(swc_common::Spanned::span(expression));
            }
            expression.visit_children_with(self);
        }
    }
    let module = thaw_parser::parse_typescript(source).unwrap();
    let plan = StoragePlan::analyze(&module).unwrap();
    let mut allocations = Allocations(Vec::new());
    module.visit_with(&mut allocations);
    allocations
        .0
        .into_iter()
        .map(|span| plan.is_dynamic(span))
        .collect()
}

#[test]
fn storage_tracks_aliases_fields_captures_calls_returns() {
    assert_eq!(
        storage_allocations(
            r#"
        function identity(value: number[]): number[] { return value; }
        function main(): void {
            const source: number[] = [];
            const holder = { value: identity(source) };
            const read = () => holder.value;
            const escaped: any = read();
            escaped.push(1);
        }
    "#
        ),
        vec![true, true]
    );
}

#[test]
fn storage_handles_recursive_calls() {
    assert_eq!(
        storage_allocations(
            r#"
        function left(value: number[], n: number): number[] {
            return n ? right(value, n - 1) : value;
        }
        function right(value: number[], n: number): number[] { return left(value, n); }
        function main(): void {
            const source: number[] = [];
            const escaped: any = left(source, 2);
        }
    "#
        ),
        vec![true]
    );
}

#[test]
fn storage_distinguishes_shadowed_names() {
    assert_eq!(
        storage_allocations(
            r#"
        function main(): void {
            const value: number[] = [];
            { const value: number[] = []; const exposed: any = value; }
            value.push(1);
        }
    "#
        ),
        vec![false, true]
    );
}

#[test]
fn storage_keeps_native_control_unboxed() {
    assert_eq!(
        storage_allocations(
            r#"
        function sum(values: number[]): number { return values[0] + 1; }
        function main(): void { const values: number[] = [1]; console.log(sum(values)); }
    "#
        ),
        vec![false]
    );
}

#[test]
fn source_contract_lookup_keeps_local_alias_and_nested_class_field_types() {
    let module = thaw_parser::parse_typescript(
        "class Item { value: number = 1; } class Holder { item: Item = new Item(); } class Maybe { value?: number; } abstract class Base { abstract value: number; label: string = 'base'; } class Derived extends Base { value: number = 2; } class Tree { child: any; value: number = 1; } function f() { item; alias; holder.item; new Item(); dynamic; unknown; new Maybe(); new Derived(); new Tree(); fake; }",
    ).unwrap();
    let _storage = StorageScope::enter(&module).unwrap();
    let (mut interfaces, generic_interfaces) = resolve_interfaces(&module).unwrap();
    let classes = collect_native_classes(&module, &mut interfaces, &generic_interfaces).unwrap();
    assert_eq!(classes.len(), 6);
    let tree = interfaces.get("Tree").unwrap().clone();
    let tree_decl = classes.iter().find(|class| class.ident.sym == "Tree").unwrap();
    let ClassMember::ClassProp(child) = &tree_decl.class.body[0] else { panic!("expected child property"); };
    remember_storage_source(storage_key(child.span), &tree);
    let item = interfaces.get("Item").unwrap().clone();
    let holder = interfaces.get("Holder").unwrap().clone();
    let signatures = HashMap::new();
    let enum_values = EnumValues::new();
    let enum_reverse_values = EnumReverseValues::new();
    let mut lowerer = FnLowerer::new(&signatures, &interfaces, &generic_interfaces, &enum_values, &enum_reverse_values, HirType::Void, None);
    lowerer.source_scope.insert("item".into(), item.clone());
    lowerer.source_scope.insert("alias".into(), item.clone());
    lowerer.source_scope.insert("holder".into(), holder);
    lowerer.source_scope.insert("dynamic".into(), HirType::Json);
    let fake = HirType::Object(vec![
        ("__thaw_class_identity_\u{1e}Item".into(), HirType::Str),
        ("value".into(), HirType::Json),
    ]);
    lowerer.source_scope.insert("fake".into(), fake.clone());

    let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = module.body.last().unwrap() else { panic!("expected source fixture function"); };
    let expressions = function.function.body.as_ref().unwrap().stmts.iter().map(|statement| {
        let Stmt::Expr(statement) = statement else { panic!("expected expression statement"); };
        statement.expr.as_ref()
    }).collect::<Vec<_>>();
    for expression in [&expressions[0], &expressions[1], &expressions[2], &expressions[3]] {
        let contract = lowerer.source_contract_type_for_expression(expression).unwrap();
        assert!(matches!(&contract, HirType::Object(fields) if fields.iter().any(|(name, ty)| name == "value" && *ty == HirType::F64)), "{contract:?}");
    }
    assert_eq!(lowerer.source_contract_type_for_expression(expressions[4]), Some(HirType::Json));
    assert_eq!(lowerer.source_contract_type_for_expression(expressions[5]), None);
    let optional = lowerer.source_contract_type_for_expression(expressions[6]).unwrap();
    assert!(matches!(&optional, HirType::Object(fields) if fields.iter().any(|(name, ty)| name == "value" && *ty == HirType::Optional(Box::new(HirType::F64)))));
    let inherited = lowerer.source_contract_type_for_expression(expressions[7]).unwrap();
    assert!(matches!(&inherited, HirType::Object(fields) if fields.iter().any(|(name, ty)| name == "value" && *ty == HirType::F64)));
    assert!(matches!(&inherited, HirType::Object(fields) if fields.iter().any(|(name, ty)| name == "label" && *ty == HirType::Str)));
    let recursive = lowerer.source_contract_type_for_expression(expressions[8]).unwrap();
    assert!(matches!(&recursive, HirType::Object(fields) if fields.iter().any(|(name, ty)| name == "value" && *ty == HirType::F64)));
    assert_eq!(lowerer.source_contract_type_for_expression(expressions[9]), Some(fake));
}

#[test]
fn source_contract_for_this_hides_private_non_arrow_receiver_payload() {
    let module = thaw_parser::parse_typescript("function f() { this; }").unwrap();
    let _storage = StorageScope::enter(&module).unwrap();
    let signatures = HashMap::new();
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let enum_values = EnumValues::new();
    let enum_reverse_values = EnumReverseValues::new();
    let declared = HirType::Object(vec![("value".into(), HirType::F64)]);
    let mut lowerer = FnLowerer::new(
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        HirType::Void,
        None,
    );
    lowerer
        .scope
        .insert("this".into(), HirType::NonArrowReceiver(Box::new(declared.clone())));

    let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = &module.body[0] else {
        panic!("expected source fixture function");
    };
    let Stmt::Expr(statement) = &function.function.body.as_ref().unwrap().stmts[0] else {
        panic!("expected `this` expression");
    };
    assert_eq!(
        lowerer.source_contract_type_for_expression(&statement.expr),
        Some(declared.clone()),
    );

    lowerer.source_scope.insert("this".into(), HirType::Json);
    assert_eq!(
        lowerer.source_contract_type_for_expression(&statement.expr),
        Some(HirType::Json),
        "an explicit source contract still takes precedence over the physical scope type",
    );
}

#[test]
fn storage_tracks_casts_late_writes_and_unknown_calls() {
    assert_eq!(
        storage_allocations(
            r#"
        declare function __thaw_typed_js_65787465726e616c(value: number[]): void;
        function main(): void {
            let first: number[] = [];
            const later: number[] = [];
            const exposed = first as any;
            first = later;
            const other: number[] = [];
            __thaw_typed_js_65787465726e616c(other);
        }
    "#
        ),
        vec![true, true, true]
    );
}

#[test]
fn storage_native_ffi_does_not_expose_owners() {
    assert_eq!(
        storage_allocations(
            "declare function native(values: number[]): void; function main(): void { const xs: number[] = []; native(xs); }"
        ),
        vec![false]
    );
}

#[test]
fn storage_tracks_destructuring_and_accessors() {
    assert_eq!(
        storage_allocations(
            r#"
        function main(): void {
            const source: number[] = [];
            const holder = { get value(): number[] { return source; } };
            const { value } = holder;
            const exposed: any = value;
        }
    "#
        ),
        vec![true, true]
    );
}

#[test]
fn storage_tracks_hoisted_var_and_catch_binding() {
    assert_eq!(
        storage_allocations(
            r#"
        function main(): void {
            const source: number[] = [];
            { var alias = source; }
            const exposed: any = alias;
        }
    "#
        ),
        vec![true]
    );
    assert_eq!(
        storage_allocations(
            r#"
        function main(): void {
            const source: number[] = [];
            try { throw source; } catch (caught) { const exposed: any = caught; }
        }
    "#
        ),
        vec![true]
    );
}

#[test]
fn storage_tracks_constructor_arguments_and_dynamic_builtins() {
    assert_eq!(
        storage_allocations(
            r#"
        class Box { value: number[]; constructor(value: number[]) { this.value = value; } }
        function main(): void { const source: number[] = []; const box: any = new Box(source); }
    "#
        ),
        vec![true]
    );
    assert_eq!(
        storage_allocations(
            r#"
        function main(): void { const source: number[] = []; const proxy = new Proxy(source, {}); }
    "#
        ),
        vec![true, true]
    );
}

#[test]
fn storage_compilation_scope_does_not_leak() {
    let dynamic =
        lower("function main(): void { const xs: number[] = []; const exposed: any = xs; }");
    assert!(matches!(
        &dynamic.functions[0].body[0],
        HirStmt::Let(_, HirType::Json, _)
    ));
    let native = lower("function main(): void { const xs: number[] = []; }");
    assert!(
        matches!(&native.functions[0].body[0], HirStmt::Let(_, HirType::Array(element), _) if **element == HirType::F64)
    );
}

#[test]
fn storage_does_not_alias_unrelated_function_parameters() {
    let program = lower(
        r#"
        declare function __thaw_typed_js_7765616b<T>(value: T): JsValue;
        function double(value: number): number { return value * 2; }
        function main(): void { const reference = __thaw_typed_js_7765616b(double); }
    "#,
    );
    let double = program
        .functions
        .iter()
        .find(|function| function.name == "double")
        .unwrap();
    assert_eq!(double.params[0].ty, HirType::F64);
    assert_eq!(double.ret, HirType::F64);
}

#[test]
fn storage_tracks_returned_callable_aliases() {
    assert_eq!(
        storage_allocations(
            r#"
        function make(xs: number[]): () => number[] { return () => xs; }
        function main(): void { const source: number[] = []; const read = make(source); const exposed: any = read(); }
    "#
        ),
        vec![true]
    );
}

#[test]
fn storage_returned_callable_keeps_dynamic_signature() {
    let program = lower(
        r#"
        function make(xs: number[]): () => number[] { return () => xs; }
        function main(): void { const source: number[] = []; const read = make(source); const exposed: any = read(); }
    "#,
    );
    let make = program
        .functions
        .iter()
        .find(|function| function.name == "make")
        .unwrap();
    assert_eq!(make.params[0].ty, HirType::Json);
    assert_eq!(
        make.ret,
        HirType::Function(Vec::new(), Box::new(HirType::Json))
    );
}

#[test]
fn storage_preserves_native_ffi_class_layout() {
    let program = lower(
        r#"
        class Native { value: number = 1; }
        declare function native_read(value: Native): number;
        function main(): void { const value = new Native(); const exposed: any = value; exposed.value = 7; console.log(native_read(value)); }
    "#,
    );
    let native = program
        .extern_functions
        .iter()
        .find(|function| function.symbol == "native_read")
        .unwrap();
    let HirType::Object(fields) = &native.params[0] else {
        panic!("native object parameter")
    };
    assert_eq!(fields[1], ("value".into(), HirType::F64));
}

#[test]
fn storage_tracks_constructor_parameters() {
    assert_eq!(
        storage_allocations(
            r#"
        class Holder { constructor(value: number[]) { const exposed: any = value; exposed[0] = {}; } }
        function main(): void { const value: number[] = []; const holder = new Holder(value); }
    "#
        ),
        vec![true, true]
    );
}

#[test]
fn storage_class_method_parameters_use_dynamic_storage() {
    let program = lower(
        r#"class Box { run(value: Map<string, number>): void {} } function main(): void { const box = new Box(); const exposed: any = box; }"#,
    );
    let method = program
        .functions
        .iter()
        .find(|function| function.name.contains("run"))
        .unwrap();
    assert_eq!(method.params[1].ty, HirType::Json);
}

#[test]
fn storage_optional_parameter_keeps_omission_tag() {
    let program = lower(
        r#"function inspect(value?: {depth: number}): void {} function main(): void { const exposed: any = inspect; inspect(); }"#,
    );
    let function = program
        .functions
        .iter()
        .find(|function| function.name == "inspect")
        .unwrap();
    assert_eq!(
        function.params[0].ty,
        HirType::Optional(Box::new(HirType::Json))
    );
}

#[test]
fn storage_tracks_unknown_callable_arguments() {
    assert_eq!(
        storage_allocations(
            r#"
        function invoke(callback: any, value: number[]): void { callback(value); }
        function main(): void { const value: number[] = []; invoke(new Function("value", "value[0] = {}"), value); }
    "#
        ),
        vec![true]
    );
}

#[test]
fn storage_native_program_keeps_unboxed_layout() {
    let program = lower(
        r#"
        class Counter { value: number = 0; add(n: number): void { this.value += n; } }
        function read(values: number[]): number { return values[0]; }
        function main(): void { const values: number[] = [1, 2]; const counter = new Counter(); counter.add(read(values)); console.log(counter.value); }
    "#,
    );
    let read = program
        .functions
        .iter()
        .find(|function| function.name == "read")
        .unwrap();
    assert_eq!(read.params[0].ty, HirType::Array(Box::new(HirType::F64)));
    assert_eq!(read.ret, HirType::F64);
    let constructor = program
        .functions
        .iter()
        .find(|function| function.name == class_constructor_symbol("Counter"))
        .unwrap();
    let HirType::Object(fields) = &constructor.ret else {
        panic!("native class layout")
    };
    assert_eq!(fields[1], ("value".into(), HirType::F64));
}

#[test]
fn storage_preserves_source_initializer_diagnostics() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void { const xs: number[] = ["bad"]; const exposed: any = xs; }"#,
    )
    .unwrap();
    assert!(
        lower_module(&module).is_err(),
        "promotion must not erase the number[] annotation"
    );
}

#[test]
fn storage_loop_header_shadows_outer_binding() {
    assert_eq!(
        storage_allocations(
            r#"function main(): void { const xs: number[] = [1]; for (let xs: number[] = []; ; ) { const exposed: any = xs; break; } console.log(xs[0]); }"#
        ),
        vec![false, true]
    );
}

#[test]
fn storage_accepts_native_ffi_return_with_promoted_internal_class() {
    let module = thaw_parser::parse_typescript(r#"class C { value: number = 1; } declare function native_make(): C; function main(): void { const local = new C(); const exposed: any = local; const returned: C = native_make(); console.log(returned.value); }"#).unwrap();
    assert!(
        lower_module(&module).is_ok(),
        "native return must adapt to internal class layout"
    );
}

#[test]
fn storage_preserves_source_call_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"function accept(values: number[]): void {} function main(): void { const call: any = accept; accept(["bad"]); }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_preserves_source_return_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"function value(): number[] { return ["bad"]; } function main(): void { const call: any = value; console.log(value()); }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_preserves_source_class_field_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"class Box { value: number[] = ["bad"]; } function main(): void { const box = new Box(); const exposed: any = box; }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_tracks_nested_dynamic_parameter_slots() {
    assert_eq!(
        storage_allocations(
            r#"function write(holder: {slot: any}): void { holder.slot[0] = {}; } function main(): void { const xs: number[] = [1]; const holder = {slot: xs}; write(holder); }"#
        ),
        vec![true, true, true]
    );
}

#[test]
fn storage_tracks_dynamic_map_value_slots() {
    assert_eq!(
        storage_allocations(
            r#"function main(): void { const xs: number[] = [1]; const map = new Map<string, any>(); map.set("x", xs); map.get("x")[0] = {}; }"#
        ),
        vec![true, true]
    );
}

#[test]
fn storage_tracks_inferred_dynamic_field_writes() {
    assert_eq!(
        storage_allocations(
            r#"function main(): void { const xs: number[] = [1]; const holder = {slot: undefined as any}; holder.slot = xs; holder.slot[0] = {}; }"#
        ),
        vec![true, true, true]
    );
}

#[test]
fn storage_preserves_contextual_empty_array_and_any_assignments() {
    let module = thaw_parser::parse_typescript(r#"function main(): void { const xs: {x: number}[] = []; const exposed: any = xs; const unknown: any = {}; const map: Map<string, number> = unknown; }"#).unwrap();
    assert!(lower_module(&module).is_ok());
}

#[test]
fn storage_source_checks_ignore_generated_accessor_arguments() {
    let module = thaw_parser::parse_typescript(r##"class Base { n=1;get value():number{return this.n;}set value(v:number){this.n=v;} }
class Derived extends Base { get twice():number{return this.n*2;} }
function main():void {const x=new Derived();const y=new Derived();y.n=5;const a:any=x;const b:any=y;console.log(a.value,a.twice);a.value=3;console.log(x.n,a.twice);const proto:any=Object.getPrototypeOf(a);const getter:any=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(proto),"value").get;console.log(getter.call(b));console.log(Object.keys(a).join(","),Object.prototype.hasOwnProperty.call(a,"value"));}"##).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn json_union_typeof_narrowing_handles_both_branches_and_guard_clauses() {
    let module = thaw_parser::parse_typescript(
        r#"
        function both(value: number | string): string {
            const dynamic: any = value;
            if (typeof dynamic === "number") return String(dynamic + 1);
            return dynamic.toUpperCase();
        }
        function guard(value: number | string): string {
            const dynamic: any = value;
            if (typeof dynamic === "number") return "number";
            return dynamic.toUpperCase();
        }
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn json_union_typeof_narrowing_is_cleared_after_a_binding_write() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invalid(value: number | string): string {
            let dynamic: any = value;
            if (typeof dynamic === "number") {
                dynamic = "changed";
                return dynamic.toUpperCase();
            }
            return "unchanged";
        }
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_preserves_source_method_argument_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"class Box { accept(values: number[]): void {} } function main(): void { const box = new Box(); const exposed: any = box; box.accept(["bad"]); }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_preserves_source_function_alias_argument_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"function accept(values: number[]): void {} function main(): void { const alias = accept; const exposed: any = alias; alias(["bad"]); }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_preserves_source_constructor_argument_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"class Box { constructor(values: number[]) { const exposed: any = values; } } function main(): void { const box = new Box(["bad"]); }"#).unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_handoff_async_arrow_source_return() {
    let module = thaw_parser::parse_typescript(r##"async function main(): Promise<void> { let count: number = 0; const advance = async (): Promise<number> => { await Promise.resolve(); count += 1; return count; }; console.log(await advance(), await advance(), count); }"##).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_dynamic_unary_plus() {
    let module = thaw_parser::parse_typescript(r##"function main(): void { const obj = { value: 5, [Symbol.toPrimitive](hint: string): any { if (hint === 'number') return this.value; return `obj(${this.value})`; }, }; console.log(+obj, `${obj}`); }"##).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_union_class_fields() {
    let module = thaw_parser::parse_typescript(r#"class Box { value: number = 1; readAny(): number { const self: any = this; return self.value; } } class Other { value: number = 2; } function readUnion(box: Box | Other | undefined): number { return box.value; } function main(): void { readUnion(undefined); }"#).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_caught_values_keep_dynamic_call_parameters() {
    let program = lower(
        r#"function inspect(error: Error): void { console.log(error.name); } function main(): void { try { throw new TypeError("bad"); } catch (error) { inspect(error); } }"#,
    );
    let function = program
        .functions
        .iter()
        .find(|function| function.name == "inspect")
        .unwrap();
    assert_eq!(function.params[0].ty, HirType::Json);
}

#[test]
fn storage_handoff_issue2_full_program() {
    let module = thaw_parser::parse_typescript(r##"
        class Box {
            value: number = 1;
            read(): number { return this.value; }
            readComputed(): number { return this["value"]; }
            readBranch(use: boolean): number { if (use) { return this.value; } return 0; }
            readArrow(): number { const inner: () => number = (): number => this.value; return inner(); }
            write(next: number): void { this.value = next; }
            readAny(): number { const self: any = this; return self.value; }
        }
        class Other { value: number = 2; }
        function readUnion(box: Box | Other | undefined): number { return box.value; }
        function check(label: string, error: Error): void {
            console.log(label + ":" + error.name + ":" + String(error instanceof TypeError) + ":" + String(error));
        }
        function main(): void {
            const box = new Box();
            const read = box.read;
            const readComputed = box.readComputed;
            const readBranch = box.readBranch;
            const readArrow = box.readArrow;
            const write = box.write;
            const readAny = box.readAny;
            try { read(); } catch (error) { check("read", error); }
            try { readComputed(); } catch (error) { check("computed", error); }
            try { readBranch(true); } catch (error) { check("branch", error); }
            try { readArrow(); } catch (error) { check("arrow", error); }
            try { write(2); } catch (error) { check("write", error); }
            try { readAny(); } catch (error) { check("any", error); }
            try { readUnion(undefined); } catch (error) { check("union", error); }
        }
    "##).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_jit_call_keeps_native_arguments() {
    assert_eq!(
        storage_allocations(
            r#"declare function __thaw_typed_jit_636f756e74(values: number[]): number; function main(): void { const values: number[] = [1]; console.log(__thaw_typed_jit_636f756e74(values)); }"#
        ),
        vec![false]
    );
}

#[test]
fn storage_handoff_typed_js_union_keeps_dynamic_arguments() {
    let module = thaw_parser::parse_typescript(r#"declare function __thaw_typed_js_696e7370656374(value: string | string[]): void; function main(): void { const values: string[] = ["x"]; __thaw_typed_js_696e7370656374(values); }"#).unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_typed_js_rest_wrapper_uses_logical_argument_types() {
    let module = thaw_parser::parse_typescript(r#"
        declare function __thaw_typed_js_6a6f696e__arity_2(first: Json, second: Json): string;
        declare function __thaw_typed_js_6a6f696e__arity_5(first: Json, second: Json, third: Json, fourth: Json, fifth: Json): string;
        function __thaw_typed_wrapper_js_6a6f696e(...inputs: Json[]): string {
            if (inputs.length === 5) {
                return __thaw_typed_js_6a6f696e__arity_5(inputs[0], inputs[1], inputs[2], inputs[3], inputs[4]);
            }
            if (inputs.length === 2) {
                return __thaw_typed_js_6a6f696e__arity_2(inputs[0], inputs[1]);
            }
            return __thaw_typed_js_6a6f696e__arity_2(inputs[0], inputs[1]);
        }
        function main(): string { return __thaw_typed_wrapper_js_6a6f696e("a", "b"); }
    "#).unwrap();
    lower_module(&module).unwrap();

    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_6a6f696e__arity_2(first: Json, second: Json): string;
        function with_prefix(prefix: string, ...inputs: Json[]): string {
            return __thaw_typed_js_6a6f696e__arity_2(inputs[0], inputs[1]);
        }
        function main(): string { return with_prefix("prefix", "a", "b"); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();

    // `.call()` contributes its explicit receiver as physical parameter zero,
    // so the fixed prefix and rest element still map to their proper slots.
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_6a6f696e__arity_2(first: Json, second: Json): string;
        function with_receiver(this: { tag: string }, prefix: string, ...inputs: Json[]): string {
            return __thaw_typed_js_6a6f696e__arity_2(inputs[0], inputs[1]);
        }
        function main(): string { return with_receiver.call({ tag: "x" }, "prefix", "a", "b"); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();

    // Ordinary native rest lowering retains its declared element type.
    let module = thaw_parser::parse_typescript(
        r#"
        function join(...inputs: string[]): string { return inputs.join(","); }
        function main(): string { return join("a", "b"); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn storage_handoff_generator_to_dynamic_lowers_to_protocol_object() {
    let module = thaw_parser::parse_typescript(r#"function* gen(): Generator<any> { yield 1; } function main(): void { const g: any = gen(); }"#).unwrap();
    let program = lower_module(&module).unwrap();
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("ObjectLit"), "{body}");
    for field in ["\"next\"", "\"return\"", "\"throw\""] {
        assert!(body.contains(field), "{field}: {body}");
    }
    // The internal i64 resume-mode producer is only called, never wrapped as a
    // callable: no lambda in `main` takes an `i64` parameter.
    assert!(!body.contains("ty: I64"), "{body}");
}

#[test]
fn storage_handoff_class_to_primitive_arguments_match_its_signature() {
    let program = lower(
        r#"
        class Money {
            amount: number;
            constructor(amount: number) { this.amount = amount; }
            [Symbol.toPrimitive](hint: string): any {
                if (hint === "number") return this.amount;
                return String(this.amount);
            }
        }
        function main(): void {
            const money = new Money(5);
            console.log(+money, `${money}`);
        }
    "#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let hir = format!("{:?}", main.body);
    assert!(
        hir.contains("Json], Json), [Var(\"money\""),
        "toPrimitive signature must retain its lowered Json hint parameter: {hir}"
    );
    assert!(
        !hir.contains("Var(\"money\"), Lit(Str(\"number\"))"),
        "the number hint must be coerced to the signature: {hir}"
    );
    assert!(
        !hir.contains("Var(\"money\"), Lit(Str(\"string\"))"),
        "the string hint must be coerced to the signature: {hir}"
    );
}

#[test]
fn storage_handoff_error_callbacks_use_nominal_receiver_abi() {
    let program = lower(
        r#"
        class MarkerError extends Error {
            constructor(message: string) { super(message); }
        }
        function main(): void {
            try { throw new MarkerError("boom"); }
            catch (error) { console.log(error.message, error instanceof MarkerError); }
        }
    "#,
    );
    let hir = format!("{program:#?}");
    assert!(
        hir.contains("__thaw_class_owner"),
        "nominal Error projection must use the explicit class receiver ABI"
    );
}

#[test]
fn storage_handoff_dynamic_json_error_instanceof_uses_realm_constructor() {
    let program = lower(
        r#"
        function main(): void {
            let caught: any;
            try { throw new TypeError("boom"); }
            catch (error: any) { caught = error; }
            console.log(caught instanceof TypeError);
        }
    "#,
    );
    let hir = format!("{program:#?}");
    assert!(
        hir.contains("__thaw_json_host_instanceof"),
        "Json-backed caught exceptions must use the realm's Error constructor: {hir}"
    );
    assert!(
        hir.contains("TypeError"),
        "the builtin constructor name must be preserved: {hir}"
    );
}

#[test]
fn storage_handoff_typed_js_calls_use_active_nullish_narrowing() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_74616b65(value: number): number;
        function forward(value?: number | null): number {
            if (value != undefined) return __thaw_typed_js_74616b65(value);
            return 0;
        }
        function main(): number { forward(); return forward(2); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();

    // Equality's else branch has the same present-payload narrowing.
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_74616b65(value: number): number;
        function forward(value?: number | null): number {
            if (value == undefined) return 0;
            else return __thaw_typed_js_74616b65(value);
        }
        function main(): number { return forward(2); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();

    // Strict comparison excludes only undefined, so null must remain rejected.
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_74616b65(value: number): number;
        function forward(value?: number | null): number {
            if (value !== undefined) return __thaw_typed_js_74616b65(value);
            return 0;
        }
        function main(): number { return forward(2); }
    "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());

    // Strict comparison with null likewise leaves undefined possible.
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_74616b65(value: number): number;
        function forward(value?: number | null): number {
            if (value !== null) return __thaw_typed_js_74616b65(value);
            return 0;
        }
        function main(): number { return forward(2); }
    "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());

    // An unguarded source-nullish value remains incompatible with the JS ABI.
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_74616b65(value: number): number;
        function forward(value?: number | null): number {
            return __thaw_typed_js_74616b65(value);
        }
        function main(): number { return forward(2); }
    "#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn storage_handoff_quickjs_function_values_keep_source_argument_contracts() {
    for call in [
        "alias([\"wrong\"]);",
        "(alias)([\"wrong\"]);",
        "alias.call(null, [\"wrong\"]);",
        "alias.apply(null, [\"wrong\"]);",
    ] {
        let source = format!(
            r#"
            declare function __thaw_typed_js_617070656e64(values: number[]): number;
            function main(): number {{
                const alias: (values: number[]) => number = __thaw_typed_js_617070656e64;
                {call} return 0;
            }}
            "#
        );
        let module = thaw_parser::parse_typescript(&source).unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("argument") || error.contains("expected"),
            "invalid source arguments must be rejected for `{call}`: {error}"
        );
    }

    let valid_spread = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e6450616972(count: number, values: number[]): number;
        function main(): number {
            const alias = __thaw_typed_js_617070656e6450616972;
            const args: [number, number[]] = [1, [2]];
            return alias(...args);
        }
        "#,
    )
    .unwrap();
    lower_module(&valid_spread).unwrap();

    let invalid_apply_tuple = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e6450616972(count: number, values: number[]): number;
        function main(): number {
            const alias = __thaw_typed_js_617070656e6450616972;
            const args: [number, string[]] = [1, ["wrong"]];
            return alias.apply(null, args);
        }
        "#,
    )
    .unwrap();
    let error = lower_module(&invalid_apply_tuple).unwrap_err();
    assert!(
        error.contains("argument") || error.contains("expected"),
        "typed apply tuples must validate against the logical signature: {error}"
    );
}

#[test]
fn storage_handoff_quickjs_function_value_rejects_contextual_callback_mismatch() {
    let valid = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e64(values: number[]): number;
        function invoke(callback: (values: number[]) => number): number {
            return callback([1]);
        }
        function main(): number {
            const alias: (values: number[]) => number = __thaw_typed_js_617070656e64;
            invoke(alias);
            return invoke((values: number[]) => values.length);
        }
        "#,
    )
    .unwrap();
    lower_module(&valid).unwrap();

    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e64(values: number[]): number;
        const alias: (values: number[]) => number = __thaw_typed_js_617070656e64;
        function invoke(callback: (values: number[]) => number): number {
            return callback([1]);
        }
        function main(): number {
            invoke(alias);
            return invoke((values: string[]) => 0);
        }
        "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("callback") || error.contains("parameter") || error.contains("expected"),
        "the original callback parameter contract must reject string[]: {error}"
    );
}

#[test]
fn storage_handoff_json_to_string_uses_live_method_dispatch() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_6465636f6465(): any;
        function main(): void {
            const buffer: any = __thaw_typed_js_6465636f6465();
            const text: string = buffer.toString();
            console.log(text);
        }
    "#,
    )
    .unwrap();
    let program = lower_module(&module).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(
        body.contains("__thaw_call_selected_dynamic_method"),
        "a Json receiver's actual toString method must be called: {body}"
    );
    assert!(
        body.contains("JsonAsString"),
        "the dynamic method result must be converted to the string ABI: {body}"
    );
}


#[test]
fn storage_handoff_narrowed_union_calls_validate_the_active_member() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_6e6174697665(value: number): number;
        function f(value: boolean | number): number {
            if (typeof value === "number") return __thaw_typed_js_6e6174697665(value);
            return 0;
        }
        function main(): void { f(1); }
    "#,
    )
    .unwrap();
    lower_module(&module).unwrap();

    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_6e6174697665(value: number): number;
        function f(value: boolean): number { return __thaw_typed_js_6e6174697665(value); }
        function main(): void { f(true); }
    "#,
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("expected F64"), "{error}");
}

#[test]
fn storage_handoff_union_source_validation_keeps_contextual_route_handlers() {
    let valid_source = r#"
        type Handler = ((request: any, h: any) => any) | ((request: any, h: any, toolkit: any) => any) | {} | undefined;
        type Options = { auth?: boolean | string } | undefined;
        type Route = {
            method: string;
            path: string;
            handler: Handler;
            vhost: string | string[] | undefined;
            options: Options;
            rules?: any;
        };
        declare function __thaw_typed_js_726f757465(route: Route | Route[]): void;
        function main(): void {
            __thaw_typed_js_726f757465({
                method: "GET",
                path: "/",
                handler: (request, h) => h.response(request.path),
            });
        }
        "#;
    assert_eq!(storage_allocations(valid_source), vec![true]);
    let valid = thaw_parser::parse_typescript(valid_source).unwrap();
    lower_module(&valid).unwrap();

    let wrong_scalar = thaw_parser::parse_typescript(
        r#"
        type Handler = ((request: any, h: any) => any) | ((request: any, h: any, toolkit: any) => any) | {} | undefined;
        type Options = { auth?: boolean | string } | undefined;
        type Route = { method: string; path: string; handler: Handler; vhost: string | string[] | undefined; options: Options; rules?: any };
        declare function __thaw_typed_js_726f757465(route: Route | Route[]): void;
        function main(): void {
            __thaw_typed_js_726f757465({ method: 7, path: "/", handler: (request, h) => h.response(request.path) });
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&wrong_scalar).is_err());

    let wrong_handler = thaw_parser::parse_typescript(
        r#"
        type Handler = ((request: any, h: any) => any) | ((request: any, h: any, toolkit: any) => any) | {} | undefined;
        type Options = { auth?: boolean | string } | undefined;
        type Route = { method: string; path: string; handler: Handler; vhost: string | string[] | undefined; options: Options; rules?: any };
        declare function __thaw_typed_js_726f757465(route: Route | Route[]): void;
        function main(): void {
            __thaw_typed_js_726f757465({ method: "GET", path: "/", handler: 7 });
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&wrong_handler).is_err());

    let missing_required = thaw_parser::parse_typescript(
        r#"
        type Handler = ((request: any, h: any) => any) | ((request: any, h: any, toolkit: any) => any) | {} | undefined;
        type Options = { auth?: boolean | string } | undefined;
        type Route = { method: string; path: string; handler: Handler; vhost: string | string[] | undefined; options: Options; rules?: any };
        declare function __thaw_typed_js_726f757465(route: Route | Route[]): void;
        function main(): void {
            __thaw_typed_js_726f757465({ method: "GET", handler: (request, h) => h.response(request.path) });
        }
        "#,
    )
    .unwrap();
    assert!(lower_module(&missing_required).is_err());
}

#[test]
fn storage_handoff_quickjs_method_keeps_live_json_argument_and_receiver_abi() {
    let program = lower(
        r#"
        type Handler = ((request: any, h: any) => any) | ((request: any, h: any, toolkit: any) => any) | {} | undefined;
        type Route = { method: string; path: string; handler: Handler; vhost: string | string[] | undefined; options: ({ auth?: boolean | string }) | undefined; rules?: any };
        declare function __thaw_typed_js_246d6574686f64766f69642453657276657224726f757465246f7665726c6f61643024617269747931(receiver: JsValue, route: Route | Route[]): void;
        function main(receiver: JsValue): void {
            __thaw_typed_js_246d6574686f64766f69642453657276657224726f757465246f7665726c6f61643024617269747931(receiver, {
                method: "GET",
                path: "/",
                handler: (request, h) => h.response(request.path),
            });
        }
    "#,
    );
    let hir = format!("{program:#?}");
    assert!(
        hir.contains("symbol: \"$methodvoid$Server$route$overload0$arity1\",")
            && hir.contains("JsValue,\n                                Json,"),
        "the nonreceiver live route argument should use the Json handle ABI while the receiver stays JsValue: {hir}"
    );
    assert!(
        hir.contains("Var(\n                                \"receiver\","),
        "the typed method receiver should be retained: {hir}"
    );
}

#[test]
fn storage_handoff_all_settled_keeps_native_promise_arrays() {
    for source in [
        r#"async function f(n: number): Promise<number> { return n; } async function main(): Promise<void> { const ps = [f(1), f(2)]; const settled = await Promise.allSettled(ps); settled.map((value: any) => value.status); }"#,
        r#"async function f(n: number): Promise<number> { return n; } async function main(): Promise<void> { const ps: Promise<number>[] = [f(1), f(2)]; const tail: Promise<number>[] = [f(3)]; const settled = await Promise.allSettled([...ps, ...tail]); settled.map((value: any) => value.status); }"#,
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        lower_module(&module).unwrap_or_else(|error| panic!("{source}: {error}"));
    }

    let module = thaw_parser::parse_typescript(r#"async function f(): Promise<number> { return 1; } function main(): void { const ps = [f()]; const escaped: any = ps; }"#).unwrap();
    assert!(lower_module(&module).unwrap_err().contains("Promise"));
}

#[test]
fn storage_handoff_dynamic_json_parse_retains_its_live_identity() {
    let native = lower(
        r#"function main(): void { const parsed = JSON.parse("[1]"); console.log(parsed[0]); }"#,
    );
    let native_hir = format!(
        "{:?}",
        native
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap()
            .body
    );
    assert!(
        native_hir.contains("Call(Var(\"JSON.parse\")"),
        "unexposed JSON.parse should stay native: {native_hir}"
    );
    assert!(
        !native_hir.contains("retainDynamicJson"),
        "unexposed JSON.parse should not allocate a live handle: {native_hir}"
    );

    let dynamic = lower(
        r#"function main(): void { const parsed = JSON.parse("[1]"); const exposed: any = parsed; exposed[0] = 2; }"#,
    );
    let dynamic_hir = format!(
        "{:?}",
        dynamic
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap()
            .body
    );
    assert!(
        dynamic_hir.contains("retainDynamicJson"),
        "escaping JSON.parse must retain the parsed JS object: {dynamic_hir}"
    );
    assert!(
        dynamic_hir.contains("__thaw_json_from_consumed_dynamic"),
        "the retained handle must be consumed into its shared Json view: {dynamic_hir}"
    );

    let typed = lower(
        r#"function parse(text: string): any { return JSON.parse(text); } function main(): void { const parsed = parse("[1]"); }"#,
    );
    let typed_hir = format!("{typed:#?}");
    assert!(
        typed_hir.contains("name: \"text\"")
            && typed_hir.contains("ty: Json")
            && typed_hir.contains("JsonAsString"),
        "a promoted Json parameter must be converted to the string input required by JSON.parse: {typed_hir}"
    );
}

#[test]
fn storage_handoff_live_json_uses_host_property_operations() {
    let program = lower(
        r#"
        function main(): void {
            const host: any = JSON.parse('{"value":1}');
            Object.defineProperty(host, "value", { enumerable: false });
            Object.getOwnPropertyDescriptors(host);
            JSON.stringify(host, ["value"]);
        }
    "#,
    );
    let hir = format!("{program:#?}");
    assert!(
        hir.contains("__thaw_json_borrowed_handle_id"),
        "live descriptor writes must dispatch by borrowed-handle presence: {hir}"
    );
    assert!(
        hir.contains("getOwnPropertyDescriptors"),
        "Json-backed descriptor queries must use the host builtin: {hir}"
    );
    assert!(
        hir.contains("stringify"),
        "Json-backed stringify must use dynamic builtin dispatch: {hir}"
    );
}

#[test]
fn storage_handoff_json_object_create_and_integrity_use_live_host_path() {
    let program = lower(r#"
        function main(): void {
            const prototype: any = { inherited: 1 };
            const child: any = Object.create(prototype);
            console.log(Object.getPrototypeOf(child) === prototype);
            console.log(Reflect.has(child, "inherited"));
            for (const key in child) console.log(key);

            const nullPrototype: any = Object.create(null);
            console.log(Reflect.has(nullPrototype, "toString"));

            const frozen: any = { value: 1 };
            Object.preventExtensions(frozen);
            console.log(Reflect.isExtensible(frozen));
        }
    "#);
    let hir = format!("{program:#?}");
    assert!(hir.contains("callDynamicMethodHandle"), "Object.create should preserve a live prototype chain: {hir}");
    assert!(hir.contains("__thaw_json_from_consumed_dynamic"), "Object.create should expose a Json host projection: {hir}");
    assert!(hir.contains("__thaw_json_borrowed_handle_id"), "Json host state should dispatch to the live object: {hir}");
    assert!(!hir.contains("__thaw_json_set_prototype"), "Object.create must not use the native-only prototype table: {hir}");
}

#[test]
fn storage_handoff_aggregate_error_copies_promoted_json_array() {
    let program = lower(r#"
        function main(): void {
            const errors: any = [new Error("inner")];
            const aggregate = new AggregateError(errors);
            console.log(aggregate.errors);
        }
    "#);
    let hir = format!("{program:#?}");
    assert!(hir.contains("__thaw_json_array_slice"),
        "promoted array inputs must be copied into AggregateError.errors: {hir}");
    assert!(hir.contains("__thaw_json_is_array"),
        "promoted Json inputs must be checked before the array copy: {hir}");
    assert!(hir.contains("AggregateError errors must be an array"),
        "a non-array Json input must take the TypeError path: {hir}");

    let object_input = lower(r#"
        function main(): void {
            const errors: any = { value: 1 };
            const aggregate = new AggregateError(errors);
        }
    "#);
    assert!(format!("{object_input:#?}").contains("__thaw_json_is_array"),
        "object-shaped Json must be rejected at runtime by the array guard");
}

#[test]
fn storage_handoff_flat_map_return_hint_only_tracks_returned_calls() {
    lower(r#"
        function main(): void {
            const a: any[] = [1];
            const source: any[] = [a];
            const result = source.flatMap((value, index) => {
                a.push(9);
                return index ? a : value;
            });
            console.log(result.length);
        }
    "#);
    lower(r#"
        function main(): void {
            const a: any[] = [1];
            const source: any[] = [a];
            const result = source.flatMap((value, index) => {
                function unrelated(): number { return a.push(9); }
                return index ? a : value;
            });
            console.log(result.length);
        }
    "#);
    let returned_conditional_call = lower(r#"
        declare function live(value: any): any;
        function main(): void {
            const source: any[] = [1, 2];
            const result = source.flatMap((value, index) =>
                index ? live(value) : live(value)
            );
            console.log(result.length);
        }
    "#);
    let hir = format!("{returned_conditional_call:#?}");
    assert!(hir.contains("symbol: \"live\""), "conditional returned calls should lower: {hir}");
}

#[test]
fn storage_handoff_hidden_this_receiver_tracks_dynamic_escape() {
    let program = lower(
        r#"
        function main(): void {
            const lam = function(this: { n: number }, value: number): number {
                return this.n + value;
            };
            const escaped: any = lam;
            const result = lam.call({ n: 5 }, 1);
            const generic = function<T>(this: T, value: T): T { return this; };
            const escapedGeneric: any = generic;
            const genericResult = generic.call({ n: 7 }, { n: 8 });
            console.log(result, genericResult, escaped, escapedGeneric);
        }
    "#,
    );
    let hir = format!(
        "{:?}",
        program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap()
            .body
    );
    assert!(
        hir.contains("FunctionWithThis(Json"),
        "the hidden receiver must use Json storage when it escapes: {hir}"
    );
    assert!(
        hir.contains("HirParam { name: \"__thaw_this\", ty: NonArrowReceiver(Json) }"),
        "generic receiver specialization must also use Json storage: {hir}"
    );
}

#[test]
fn storage_handoff_widens_promise_payload_once_at_json_boundary() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_636f6c6c656374(): Promise<number>;
        function producer(): Promise<number> { return __thaw_typed_js_636f6c6c656374(); }
        function main(): void {
            const escaped: any = producer;
            const pending = producer();
            const first = pending;
            const second = pending;
            console.log(first === second, escaped);
        }
    "#,
    )
    .unwrap();
    let program = lower_module(&module).unwrap();
    let producer = program
        .functions
        .iter()
        .find(|function| function.name == "producer")
        .unwrap();
    assert_eq!(producer.ret, HirType::Promise(Box::new(HirType::Json)));
    let body = format!("{:?}", producer.body);
    assert!(
        body.contains("PromiseThen"),
        "the payload must be converted at the producer boundary: {body}"
    );
    assert!(
        body.contains("F64, Json, false, false"),
        "the fulfilled adapter must preserve rejection and produce Json: {body}"
    );
    assert_eq!(
        body.matches("symbol: \"collect\"").count(),
        1,
        "the Promise producer must execute exactly once: {body}"
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(
        body.contains("Let(\"first\", Promise(Json), Var(\"pending\"))"),
        "reading an already-promoted Promise must preserve its identity: {body}"
    );
    assert!(
        body.contains("Let(\"second\", Promise(Json), Var(\"pending\"))"),
        "repeated reads must not insert another Promise adapter: {body}"
    );
    assert_eq!(
        body.matches("PromiseThen").count(),
        0,
        "ordinary reads of an existing Promise must not create new Promise objects: {body}"
    );

    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_636f6c6c656374(): Promise<number>;
        function main(): void { const pending: Promise<any> = __thaw_typed_js_636f6c6c656374(); }
    "#,
    )
    .unwrap();
    let explicit = lower_module(&module).unwrap();
    let main = explicit
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(
        format!("{:?}", main.body).contains("PromiseThen"),
        "an explicit Promise<any> slot must adapt the payload without converting the Promise itself"
    );
}

#[test]
fn storage_handoff_quickjs_callbacks_validate_every_logical_rest_argument() {
    let valid = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e64(values: number[]): number;
        function invoke(callback: (values: number[]) => number): number {
            return callback([1]);
        }
        function main(): number {
            const alias: (values: number[]) => number = __thaw_typed_js_617070656e64;
            invoke(alias);
            invoke((values: number[]): number => values.length);
            return invoke((...groups: number[][]): number => groups[0].length);
        }
    "#,
    )
    .unwrap();
    lower_module(&valid).unwrap();

    let invalid = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_617070656e6450616972(first: number[], second: string[]): number;
        function invoke2(callback: (first: number[], second: string[]) => number): number {
            return callback([1], ["x"]);
        }
        function main(): number {
            const alias: (first: number[], second: string[]) => number =
                __thaw_typed_js_617070656e6450616972;
            invoke2(alias);
            return invoke2((...groups: number[][]) => groups.length);
        }
    "#,
    )
    .unwrap();
    let error = lower_module(&invalid).unwrap_err();
    assert!(
        error.contains("callback") || error.contains("parameter") || error.contains("expected"),
        "rest callback compatibility must check each logical parameter: {error}"
    );
}

#[test]
fn storage_handoff_promise_callback_rest_mapper_keeps_native_input_type() {
    let module = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_js_7465737452657374(
            callback: (...values: number[]) => number
        ): number;
        function namedRest(...values: number[]): number {
            const escaped: any = values;
            return values[0] + values[1] + values[2];
        }
        function main(): void {
            console.log(__thaw_typed_js_7465737452657374(namedRest));
        }
    "#,
    )
    .unwrap();
    let program = lower_module(&module).unwrap();
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(
        body.contains("Let(\"__thaw_map_element_") && body.contains(", F64,"),
        "the rest-array mapper input must retain its native number type: {body}"
    );
    assert!(
        !body.contains("JsonAsNumber(Var(\"__thaw_map_element_"),
        "the mapper must not decode a native number as a Json box: {body}"
    );
}

#[test]
fn jit_storage_preflight_follows_exposure_through_source_calls() {
    let exposed = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_jit_6d6978(values: number[]): number;
        function forward(values: number[]): number {
            return __thaw_typed_jit_6d6978(values);
        }
        function main(): void {
            const values: number[] = [1, 2];
            const escaped: any = values;
            forward(values);
        }
    "#,
    )
    .unwrap();
    let fallback = jit_calls_requiring_dynamic_storage(&exposed).unwrap();
    assert_eq!(fallback.len(), 1);
    assert!(fallback.contains("__thaw_typed_jit_6d6978"));

    let aliased = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_jit_6d6978(values: number[]): number;
        const call = __thaw_typed_jit_6d6978;
        function main(): void {
            const values: number[] = [1, 2];
            const escaped: any = values;
            call(values);
        }
    "#,
    )
    .unwrap();
    let fallback = jit_calls_requiring_dynamic_storage(&aliased).unwrap();
    assert_eq!(fallback.len(), 1);
    assert!(fallback.contains("__thaw_typed_jit_6d6978"));

    let higher_order = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_jit_6d6978(values: number[]): number;
        function invoke(callback: (values: number[]) => number, values: number[]): number {
            return callback(values);
        }
        function main(): void {
            const values: number[] = [1, 2];
            const escaped: any = values;
            invoke(__thaw_typed_jit_6d6978, values);
        }
    "#,
    )
    .unwrap();
    let fallback = jit_calls_requiring_dynamic_storage(&higher_order).unwrap();
    assert_eq!(fallback.len(), 1);
    assert!(fallback.contains("__thaw_typed_jit_6d6978"));

    let native_only = thaw_parser::parse_typescript(
        r#"
        declare function __thaw_typed_jit_6d6978(values: number[]): number;
        function forward(values: number[]): number {
            return __thaw_typed_jit_6d6978(values);
        }
        function main(): void { forward([1, 2]); }
    "#,
    )
    .unwrap();
    assert!(jit_calls_requiring_dynamic_storage(&native_only)
        .unwrap()
        .is_empty());
}
