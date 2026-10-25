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

fn with_dts(d: &str) -> Config {
    Config {
        dts: Some(std::sync::Arc::new(crate::dts::DtsIndex::parse(d).unwrap())),
        ..Default::default()
    }
}

#[test]
fn dts_types_escaped_function_params() {
    let cfg = with_dts("declare function add(a: number, b?: number): number;");
    let s = "exports.add = function(a, b){ return a + (b || 0); };";
    let r = fnr(s, cfg, "add");
    assert!(r.dts);
    assert_eq!(r.slots[0].1, "number");
    assert_eq!(r.slots[1].1, "number | undefined");
    assert_eq!(r.ret, "number");
    // without the d.ts the same function is `any` (Escaped)
    assert_eq!(fnr(s, Config::default(), "add").reasons[0], Reason::Escaped);
}

#[test]
fn dts_methods_use_owner_aliases_and_structs() {
    let d = "type V = string | number; interface Opt { long: boolean; n?: number }\n\
             declare class Foo { bar(x: V, o: Opt): string; baz(xs: string[]): number }";
    let s = "function Foo(){} Foo.prototype.bar = function(x, o){ return String(x); };\n\
             Foo.prototype.baz = function(xs){ return xs.length; };";
    let (reps, _) = analyze_source(s, with_dts(d)).unwrap();
    let bar = reps.iter().find(|r| r.name == "bar").unwrap();
    assert_eq!(bar.slots[0].1, "number | string");
    assert_eq!(bar.slots[1].1, "{ long: boolean; n: number | undefined }");
    let baz = reps.iter().find(|r| r.name == "baz").unwrap();
    assert_eq!(baz.slots[0].1, "string[]");
    assert_eq!(baz.class, Class::Concrete);
}

#[test]
fn dts_any_is_declared_any() {
    let cfg = with_dts("declare function f(x: any, y: unknown): void;");
    let r = fnr("exports.f = function(x, y){ return x; };", cfg, "f");
    assert_eq!(r.slots[0].1, "any");
    assert_eq!(r.reasons[0], Reason::Declared);
}

#[test]
fn array_callbacks_get_element_types() {
    let s = "function f(){ var xs = [1,2,3]; var ys = xs.map(function(x, i){ return x * 2 + i; }); \
             var t = xs.reduce(function(a, b){ return a + b; }, 0); var big = xs.filter(function(x){ return x > 1; }); \
             var names = ['a','b'].map(function(n){ return n.length; }); return t + ys.length + big.length + names[0]; } f();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let f = r.iter().find(|x| x.name == "f").unwrap();
    let get = |n: &str| f.slots.iter().find(|s| s.0 == n).unwrap().1.clone();
    assert_eq!(get("ys"), "number[]");
    assert_eq!(get("t"), "number");
    assert_eq!(get("big"), "number[]");
    assert_eq!(get("names"), "number[]");
    // the callbacks themselves are typed (params from the element type, index is number)
    let cb = r
        .iter()
        .find(|x| x.slots.iter().any(|s| s.0 == "i"))
        .unwrap();
    assert_eq!(cb.slots[0].1, "number");
    assert_eq!(cb.slots[1].1, "number");
    assert!(cb.closed);
}

#[test]
fn callbacks_to_unknown_receivers_stay_any() {
    let s = "function f(o){ return o.map(function(x){ return x + 1; }); } f({});";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let cb = r
        .iter()
        .find(|x| x.slots.iter().any(|s| s.0 == "x"))
        .unwrap();
    assert_eq!(cb.slots[0].1, "any");
    assert_eq!(cb.reasons[0], Reason::Escaped);
}

#[test]
fn es_class_fields_are_a_struct() {
    let s = "class P { constructor(x, y){ this.x = x; this.y = y; this.tag = 'p'; } \
             len(){ return this.x * this.x + this.y * this.y; } label(){ return this.tag + this.x; } } \
             var p = new P(3, 4); var n = p.len(); var l = p.label();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let len = r.iter().find(|f| f.name == "len").unwrap();
    assert!(len.this_typed);
    assert_eq!(len.ret, "number");
    assert_eq!(len.class, Class::Concrete);
    assert!(len.closed);
    let label = r.iter().find(|f| f.name == "label").unwrap();
    assert_eq!(label.ret, "string");
    // the constructor is closed-world (only `new P(..)`): params from the call
    let ctor = r.iter().find(|f| f.name == "constructor").unwrap();
    assert_eq!(ctor.slots[0].1, "number");
    assert!(ctor.this_typed);
}

#[test]
fn function_style_class_with_prototype_methods() {
    let s = "function Counter(start){ this.n = start; this.log = []; } \
             Counter.prototype.inc = function(){ this.n = this.n + 1; this.log.push(this.n); return this.n; }; \
             var c = new Counter(0); c.inc(); var k = c.inc();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let inc = r.iter().find(|f| f.name == "inc").unwrap();
    assert_eq!(inc.ret, "number");
    assert!(inc.this_typed && inc.closed);
}

#[test]
fn late_fields_are_optional_and_inheritance_is_not_modelled() {
    let s =
        "class A { constructor(){ this.a = 1; } init(){ this.b = 'x'; } get(){ return this.b; } } \
             var a = new A(); a.init(); var v = a.get();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter().find(|f| f.name == "get").unwrap().ret,
        "string | undefined"
    );
    let s = "class A { constructor(){ this.a = 1; } m(){ return this.a; } } class B extends A { n(){ return this.a; } } \
             new A().m(); new B().n();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter().find(|f| f.name == "m").unwrap().reasons[0],
        Reason::This
    );
}

#[test]
fn extracted_method_makes_this_unsafe() {
    let s = "function F(){ this.v = 1; } F.prototype.get = function(){ return this.v; }; \
             var f = new F(); var g = f.get; var r = g();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter().find(|f| f.name == "get").unwrap().reasons[0],
        Reason::This
    );
}

#[test]
fn typeof_narrowing_types_branches_of_an_any_param() {
    // `a` is `any` (escaped), but inside each branch it is known
    let s = "exports.f = function(a){ if (typeof a === 'string') { var n = a.length; return n; } \
             if (typeof a !== 'number') { return -1; } var m = a * 2; return m; };";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.slots[0].1, "any");
    let get = |n: &str| r.slots.iter().find(|s| s.0 == n).unwrap().1.clone();
    assert_eq!(get("n"), "number");
    assert_eq!(get("m"), "number");
    assert_eq!(r.ret, "number");
}

#[test]
fn null_and_truthiness_narrowing() {
    let s = "function f(xs){ var v = xs[0]; if (v === undefined) { return 0; } var w = v; return w; } f([1]);";
    assert_eq!(slot(s, "f", "w"), "number");
    let s = "function g(c){ var x = c ? 1 : undefined; var y = x ? x + 1 : 0; return y; } g(true);";
    assert_eq!(slot(s, "g", "y"), "number");
    let s = "function h(o){ var r = o && o.n; return r; } h({n: 1});";
    assert_eq!(slot(s, "h", "r"), "number");
}

#[test]
fn narrowing_is_blocked_by_assignment_in_the_region() {
    let s = "exports.f = function(a){ if (typeof a === 'string') { a = 5; var n = a; return n; } return 0; };";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.slots.iter().find(|s| s.0 == "n").unwrap().1, "any");
}

#[test]
fn array_isarray_and_instanceof_narrow() {
    let s = "exports.f = function(a){ if (Array.isArray(a)) { var n = a.length; return n; } return 0; };";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.slots.iter().find(|s| s.0 == "n").unwrap().1, "number");
    let s = "class P { constructor(){ this.v = 1; } get(){ return this.v; } } \
             exports.g = function(x){ if (x instanceof P) { var r = x.get(); return r; } return 0; }; new P().get();";
    let r = fnr(s, Config::default(), "g");
    assert_eq!(r.slots.iter().find(|s| s.0 == "r").unwrap().1, "number");
}

#[test]
fn literal_types_survive_in_const_and_returns_but_widen_in_variables() {
    let s = "function mode(c){ return c ? 'fast' : 'slow'; } mode(true);";
    assert_eq!(fnr(s, Config::default(), "mode").ret, "\"fast\" | \"slow\"");
    let s = "function f(){ const k = 'a'; var m = 'a'; var n = 1; n = 2; return k + m; } f();";
    assert_eq!(slot(s, "f", "k"), "\"a\"");
    assert_eq!(slot(s, "f", "m"), "string");
    assert_eq!(slot(s, "f", "n"), "number");
    let s =
        "function g(k){ if (k === 'a') { const x = k; return x; } return 'b'; } g('a'); g('b');";
    assert_eq!(slot(s, "g", "x"), "\"a\"");
}
