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
