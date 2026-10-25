use crate::*;

fn fnr(src: &str, cfg: Config, f: &str) -> FnReport {
    let (r, _) = analyze_source(src, cfg).unwrap();
    r.into_iter()
        .find(|x| x.name == f)
        .unwrap_or_else(|| panic!("no fn {f}"))
}

fn slot(src: &str, f: &str, v: &str) -> String {
    let r = fnr(src, Config::default(), f);
    r.slots
        .iter()
        .find(|s| s.0 == v)
        .unwrap_or_else(|| panic!("no slot {v}: {:?}", r.slots))
        .1
        .clone()
}

#[test]
fn only_numbers_is_number() {
    let s = "function f(){ var x = 1; x = 2; x += 3; x++; return x; } f();";
    assert_eq!(slot(s, "f", "x"), "number");
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.ret, "number");
    assert_eq!(r.class, Class::Concrete);
    assert!(r.closed);
}

#[test]
fn number_and_string_is_union() {
    let s = "function f(c){ var x = 1; if (c) x = 'a'; return x; } f(true);";
    assert_eq!(slot(s, "f", "x"), "number | string");
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.class, Class::Union);
    assert_eq!(r.ret, "number | string");
}

#[test]
fn unknown_is_any_with_reason() {
    let s = "function f(){ var x = foo(); return x; } f();";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(slot(s, "f", "x"), "any");
    assert_eq!(r.class, Class::Any);
    assert_eq!(r.reasons[0], Reason::UnknownGlobal);
    assert!(!r.closed);
}

#[test]
fn loop_counter_and_params_from_call_sites() {
    let s = "function sum(n){ var s = 0; for (var i = 0; i < n; i++) { s += i; } return s; } sum(10); sum(20);";
    assert_eq!(slot(s, "sum", "n"), "number");
    assert_eq!(slot(s, "sum", "i"), "number");
    assert_eq!(slot(s, "sum", "s"), "number");
    assert!(fnr(s, Config::default(), "sum").closed);
}

#[test]
fn mixed_call_sites_make_param_union() {
    let s = "function id(x){ return x; } id(1); id('a');";
    assert_eq!(slot(s, "id", "x"), "number | string");
    assert_eq!(fnr(s, Config::default(), "id").ret, "number | string");
}

#[test]
fn string_concat_and_bitops() {
    let s = "function f(a){ var s = ''; s += 'a'; s += 1; var k = a | 0; var p = +a; return s + k + p; } f('7');";
    assert_eq!(slot(s, "f", "s"), "string");
    assert_eq!(slot(s, "f", "k"), "number");
    assert_eq!(slot(s, "f", "p"), "number");
    assert_eq!(fnr(s, Config::default(), "f").ret, "string");
}

#[test]
fn shadowing_is_per_function() {
    let s = "function outer(){ var x = 'a'; function inner(){ var x = 1; return x; } return x + inner(); } outer();";
    assert_eq!(slot(s, "inner", "x"), "number");
    assert_eq!(slot(s, "outer", "x"), "string");
}

#[test]
fn arrays_and_aliasing_are_sound() {
    let s = "function f(){ var a = [1,2]; a.push(3); return a; } f();";
    assert_eq!(slot(s, "f", "a"), "number[]");
    // pushing a string through an alias widens the original too
    let s = "function g(){ var a = [1]; var b = a; b.push('x'); return a; } g();";
    assert_eq!(slot(s, "g", "a"), "(number | string)[]");
    // passing an array to unknown code kills the element type
    let s = "function h(){ var a = [1]; mystery(a); return a; } h();";
    assert_eq!(fnr(s, Config::default(), "h").class, Class::Any);
}

#[test]
fn out_of_bounds_read_is_optional() {
    let s = "function f(){ var a = [1,2]; var v = a[5]; return v; } f();";
    assert_eq!(slot(s, "f", "v"), "number | undefined");
    assert_eq!(fnr(s, Config::default(), "f").class, Class::Nullable);
}

#[test]
fn fixed_shape_objects() {
    let s = "function f(){ var o = {x: 1, y: 'a'}; o.x = 2; return o.x + o.x; } f();";
    assert_eq!(slot(s, "f", "o"), "{ x: number; y: string }");
    // a late property is `T | undefined`, a dynamic key kills the shape
    let s = "function g(k){ var o = {}; o.a = 1; return o; } g('k');";
    assert_eq!(slot(s, "g", "o"), "{ a: number | undefined }");
    let s = "function h(k){ var o = {a: 1}; o[k] = 2; return o; } h('k');";
    assert_eq!(fnr(s, Config::default(), "h").class, Class::Any);
}

#[test]
fn escaped_function_params_are_any_unless_hinted() {
    let s = "exports.f = f; function f(a){ return a * 2; }";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.slots[0].1, "any");
    assert_eq!(r.reasons[0], Reason::Escaped);
    let h = fnr(
        s,
        Config {
            hint_params: true,
            ..Default::default()
        },
        "f",
    );
    assert_eq!(h.slots[0].1, "number");
    assert!(h.closed);
}

#[test]
fn this_arguments_eval_force_any() {
    let r = fnr("function f(){ return this; } f();", Config::default(), "f");
    assert_eq!(r.reasons[0], Reason::This);
    let r = fnr(
        "function f(){ return arguments.length; } f();",
        Config::default(),
        "f",
    );
    assert_eq!(r.reasons[0], Reason::Arguments);
    let r = fnr(
        "function f(s){ return eval(s); } f('1');",
        Config::default(),
        "f",
    );
    assert_eq!(r.reasons[0], Reason::Eval);
}

#[test]
fn recursion_reaches_fixpoint() {
    let s = "function fib(n){ return n < 2 ? n : fib(n - 1) + fib(n - 2); } fib(10);";
    let r = fnr(s, Config::default(), "fib");
    assert_eq!(r.ret, "number");
    assert!(r.closed);
}

#[test]
fn uninitialised_var_is_undefined_unless_optimistic() {
    let s = "function f(c){ var x; if (c) x = 1; return x; } f(true);";
    assert_eq!(slot(s, "f", "x"), "number | undefined");
    let o = fnr(
        s,
        Config {
            optimistic_uninit: true,
            ..Default::default()
        },
        "f",
    );
    assert_eq!(o.slots.iter().find(|s| s.0 == "x").unwrap().1, "number");
}

#[test]
fn annotate_writes_typescript() {
    let s = "function add(a, b){ var t; t = a + b; return t; }\nadd(1, 2);\nvar g = x => x;\ng(1);";
    let out = annotate(
        s,
        Config {
            optimistic_uninit: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        out,
        "function add(a: number, b: number){ var t: number; t = a + b; return t; }\nadd(1, 2);\nvar g = (x: number) => x;\ng(1);"
    );
}

#[test]
fn definite_assignment_drops_undefined() {
    // `var s; var i; s = 0; for (i = 0; ...)` : both are assigned before any other mention
    let s =
        "function f(n){ var s; var i; s = 0; for (i = 0; i < n; i++) { s += i; } return s; } f(3);";
    assert_eq!(slot(s, "f", "s"), "number");
    assert_eq!(slot(s, "f", "i"), "number");
    // a conditional first assignment may leave it undefined
    let s = "function g(c){ var x; if (c) { x = 1; } return x; } g(true);";
    assert_eq!(slot(s, "g", "x"), "number | undefined");
    // a closure mention (hoisted, callable early) defeats it
    let s = "function h(){ function r(){ return x; } var x; x = 1; return r(); } h();";
    assert_eq!(slot(s, "h", "x"), "number | undefined");
}
