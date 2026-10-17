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
    allocations.0.into_iter().map(|span| plan.is_dynamic(span)).collect()
}

#[test]
fn storage_tracks_aliases_fields_captures_calls_returns() {
    assert_eq!(storage_allocations(r#"
        function identity(value: number[]): number[] { return value; }
        function main(): void {
            const source: number[] = [];
            const holder = { value: identity(source) };
            const read = () => holder.value;
            const escaped: any = read();
            escaped.push(1);
        }
    "#), vec![true, true]);
}

#[test]
fn storage_handles_recursive_calls() {
    assert_eq!(storage_allocations(r#"
        function left(value: number[], n: number): number[] {
            return n ? right(value, n - 1) : value;
        }
        function right(value: number[], n: number): number[] { return left(value, n); }
        function main(): void {
            const source: number[] = [];
            const escaped: any = left(source, 2);
        }
    "#), vec![true]);
}

#[test]
fn storage_distinguishes_shadowed_names() {
    assert_eq!(storage_allocations(r#"
        function main(): void {
            const value: number[] = [];
            { const value: number[] = []; const exposed: any = value; }
            value.push(1);
        }
    "#), vec![false, true]);
}

#[test]
fn storage_keeps_native_control_unboxed() {
    assert_eq!(storage_allocations(r#"
        function sum(values: number[]): number { return values[0] + 1; }
        function main(): void { const values: number[] = [1]; console.log(sum(values)); }
    "#), vec![false]);
}

#[test]
fn storage_tracks_casts_late_writes_and_unknown_calls() {
    assert_eq!(storage_allocations(r#"
        declare function __thaw_typed_js_65787465726e616c(value: number[]): void;
        function main(): void {
            let first: number[] = [];
            const later: number[] = [];
            const exposed = first as any;
            first = later;
            const other: number[] = [];
            __thaw_typed_js_65787465726e616c(other);
        }
    "#), vec![true, true, true]);
}

#[test]
fn storage_native_ffi_does_not_expose_owners() {
    assert_eq!(storage_allocations("declare function native(values: number[]): void; function main(): void { const xs: number[] = []; native(xs); }"), vec![false]);
}

#[test]
fn storage_tracks_destructuring_and_accessors() {
    assert_eq!(storage_allocations(r#"
        function main(): void {
            const source: number[] = [];
            const holder = { get value(): number[] { return source; } };
            const { value } = holder;
            const exposed: any = value;
        }
    "#), vec![true, true]);
}

#[test]
fn storage_tracks_hoisted_var_and_catch_binding() {
    assert_eq!(storage_allocations(r#"
        function main(): void {
            const source: number[] = [];
            { var alias = source; }
            const exposed: any = alias;
        }
    "#), vec![true]);
    assert_eq!(storage_allocations(r#"
        function main(): void {
            const source: number[] = [];
            try { throw source; } catch (caught) { const exposed: any = caught; }
        }
    "#), vec![true]);
}

#[test]
fn storage_tracks_constructor_arguments_and_dynamic_builtins() {
    assert_eq!(storage_allocations(r#"
        class Box { value: number[]; constructor(value: number[]) { this.value = value; } }
        function main(): void { const source: number[] = []; const box: any = new Box(source); }
    "#), vec![true]);
    assert_eq!(storage_allocations(r#"
        function main(): void { const source: number[] = []; const proxy = new Proxy(source, {}); }
    "#), vec![true, true]);
}

#[test]
fn storage_compilation_scope_does_not_leak() {
    let dynamic = lower("function main(): void { const xs: number[] = []; const exposed: any = xs; }");
    assert!(matches!(&dynamic.functions[0].body[0], HirStmt::Let(_, HirType::Json, _)));
    let native = lower("function main(): void { const xs: number[] = []; }");
    assert!(matches!(&native.functions[0].body[0], HirStmt::Let(_, HirType::Array(element), _) if **element == HirType::F64));
}

#[test]
fn storage_does_not_alias_unrelated_function_parameters() {
    let program = lower(r#"
        declare function __thaw_typed_js_7765616b<T>(value: T): JsValue;
        function double(value: number): number { return value * 2; }
        function main(): void { const reference = __thaw_typed_js_7765616b(double); }
    "#);
    let double = program.functions.iter().find(|function| function.name == "double").unwrap();
    assert_eq!(double.params[0].ty, HirType::F64);
    assert_eq!(double.ret, HirType::F64);
}

#[test]
fn storage_tracks_returned_callable_aliases() {
    assert_eq!(storage_allocations(r#"
        function make(xs: number[]): () => number[] { return () => xs; }
        function main(): void { const source: number[] = []; const read = make(source); const exposed: any = read(); }
    "#), vec![true]);
}

#[test]
fn storage_returned_callable_keeps_dynamic_signature() {
    let program = lower(r#"
        function make(xs: number[]): () => number[] { return () => xs; }
        function main(): void { const source: number[] = []; const read = make(source); const exposed: any = read(); }
    "#);
    let make = program.functions.iter().find(|function| function.name == "make").unwrap();
    assert_eq!(make.params[0].ty, HirType::Json);
    assert_eq!(make.ret, HirType::Function(Vec::new(), Box::new(HirType::Json)));
}

#[test]
fn storage_preserves_native_ffi_class_layout() {
    let program = lower(r#"
        class Native { value: number = 1; }
        declare function native_read(value: Native): number;
        function main(): void { const value = new Native(); const exposed: any = value; exposed.value = 7; console.log(native_read(value)); }
    "#);
    let native = program.extern_functions.iter().find(|function| function.symbol == "native_read").unwrap();
    let HirType::Object(fields) = &native.params[0] else { panic!("native object parameter") };
    assert_eq!(fields[1], ("value".into(), HirType::F64));
}

#[test]
fn storage_tracks_constructor_parameters() {
    assert_eq!(storage_allocations(r#"
        class Holder { constructor(value: number[]) { const exposed: any = value; exposed[0] = {}; } }
        function main(): void { const value: number[] = []; const holder = new Holder(value); }
    "#), vec![true, true]);
}

#[test]
fn storage_class_method_parameters_use_dynamic_storage() {
    let program = lower(r#"class Box { run(value: Map<string, number>): void {} } function main(): void { const box = new Box(); const exposed: any = box; }"#);
    let method = program.functions.iter().find(|function| function.name.contains("run")).unwrap();
    assert_eq!(method.params[1].ty, HirType::Json);
}

#[test]
fn storage_optional_parameter_keeps_omission_tag() {
    let program = lower(r#"function inspect(value?: {depth: number}): void {} function main(): void { const exposed: any = inspect; inspect(); }"#);
    let function = program.functions.iter().find(|function| function.name == "inspect").unwrap();
    assert_eq!(function.params[0].ty, HirType::Optional(Box::new(HirType::Json)));
}

#[test]
fn storage_tracks_unknown_callable_arguments() {
    assert_eq!(storage_allocations(r#"
        function invoke(callback: any, value: number[]): void { callback(value); }
        function main(): void { const value: number[] = []; invoke(new Function("value", "value[0] = {}"), value); }
    "#), vec![true]);
}

#[test]
fn storage_native_program_keeps_unboxed_layout() {
    let program = lower(r#"
        class Counter { value: number = 0; add(n: number): void { this.value += n; } }
        function read(values: number[]): number { return values[0]; }
        function main(): void { const values: number[] = [1, 2]; const counter = new Counter(); counter.add(read(values)); console.log(counter.value); }
    "#);
    let read = program.functions.iter().find(|function| function.name == "read").unwrap();
    assert_eq!(read.params[0].ty, HirType::Array(Box::new(HirType::F64)));
    assert_eq!(read.ret, HirType::F64);
    let constructor = program.functions.iter().find(|function| function.name == class_constructor_symbol("Counter")).unwrap();
    let HirType::Object(fields) = &constructor.ret else { panic!("native class layout") };
    assert_eq!(fields[1], ("value".into(), HirType::F64));
}

#[test]
fn storage_preserves_source_initializer_diagnostics() {
    let module = thaw_parser::parse_typescript(r#"function main(): void { const xs: number[] = ["bad"]; const exposed: any = xs; }"#).unwrap();
    assert!(lower_module(&module).is_err(), "promotion must not erase the number[] annotation");
}

#[test]
fn storage_loop_header_shadows_outer_binding() {
    assert_eq!(storage_allocations(r#"function main(): void { const xs: number[] = [1]; for (let xs: number[] = []; ; ) { const exposed: any = xs; break; } console.log(xs[0]); }"#), vec![false, true]);
}

#[test]
fn storage_accepts_native_ffi_return_with_promoted_internal_class() {
    let module = thaw_parser::parse_typescript(r#"class C { value: number = 1; } declare function native_make(): C; function main(): void { const local = new C(); const exposed: any = local; const returned: C = native_make(); console.log(returned.value); }"#).unwrap();
    assert!(lower_module(&module).is_ok(), "native return must adapt to internal class layout");
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
    assert_eq!(storage_allocations(r#"function write(holder: {slot: any}): void { holder.slot[0] = {}; } function main(): void { const xs: number[] = [1]; const holder = {slot: xs}; write(holder); }"#), vec![true, true, true]);
}

#[test]
fn storage_tracks_dynamic_map_value_slots() {
    assert_eq!(storage_allocations(r#"function main(): void { const xs: number[] = [1]; const map = new Map<string, any>(); map.set("x", xs); map.get("x")[0] = {}; }"#), vec![true, true]);
}

#[test]
fn storage_tracks_inferred_dynamic_field_writes() {
    assert_eq!(storage_allocations(r#"function main(): void { const xs: number[] = [1]; const holder = {slot: undefined as any}; holder.slot = xs; holder.slot[0] = {}; }"#), vec![true, true, true]);
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
