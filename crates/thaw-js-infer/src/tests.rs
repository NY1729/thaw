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
    // a base class from outside the package: the instance has fields we cannot see
    let s = "class A { constructor(){ this.a = 1; } m(){ return this.a; } } class B extends Ext { n(){ return this.a; } } \
             new A().m(); new B().n();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter().find(|f| f.name == "n").unwrap().reasons[0],
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

fn with_evidence(pkg: &str, program: &str) -> Config {
    let mut ev = crate::evidence::Evidence::default();
    ev.add_program(pkg, program);
    Config {
        evidence: Some(std::sync::Arc::new(ev)),
        ..Default::default()
    }
}

#[test]
fn smoke_program_calls_type_exported_params() {
    let cfg = with_evidence("pkg", "import { add } from 'pkg'; add(1, 2); add(3);");
    let r = fnr(
        "exports.add = function(a, b){ return a + (b || 0); };",
        cfg,
        "add",
    );
    assert_eq!(r.slots[0].1, "number");
    assert_eq!(r.slots[1].1, "number | undefined");
    assert!(r.trust & crate::TRUST_EVIDENCE != 0);
    // namespace / default-import member calls and README blocks count too
    let mut ev = crate::evidence::Evidence::default();
    ev.add_readme(
        "pkg",
        "text\n```js\nconst p = require('pkg');\np.greet('x');\n```\n",
    );
    let cfg = Config {
        evidence: Some(std::sync::Arc::new(ev)),
        ..Default::default()
    };
    let r = fnr(
        "exports.greet = function(n){ return 'hi ' + n; };",
        cfg,
        "greet",
    );
    assert_eq!(r.slots[0].1, "string");
}

#[test]
fn cross_module_calls_in_a_bundle_are_evidence() {
    let s = r#"module.exports = (function(require){
var __thaw_bundle_entry_key = "a";
var __thaw_bundle_factories = {
  "a": function(module, exports, require){ var u = require('./b'); u.twice(4); },
  "b": function(module, exports, require){ exports.twice = function(x){ return x * 2; }; }
};
var __thaw_bundle_require_maps = { "a": {"./b": "b"}, "b": {} };
})(globalThis.require);"#;
    let r = fnr(s, Config::default(), "twice");
    assert_eq!(r.slots[0].1, "number");
    assert!(r.trust & crate::TRUST_EVIDENCE != 0);
}

#[test]
fn named_function_callbacks_get_element_types() {
    let s = "function dbl(x){ return x * 2; } var ys = [1,2,3].map(dbl);";
    let r = fnr(s, Config::default(), "dbl");
    assert_eq!(r.slots[0].1, "number");
    assert!(r.closed);
    // passed to an unknown receiver: parameters are `any`
    let s = "function g(x){ return x; } mystery.each(g);";
    assert_eq!(fnr(s, Config::default(), "g").slots[0].1, "any");
}

#[test]
fn default_values_are_hint_evidence() {
    let s = "exports.f = function(a, b = 5){ return a + b; };";
    let h = fnr(
        s,
        Config {
            hint_params: true,
            ..Default::default()
        },
        "f",
    );
    assert_eq!(h.slots[1].1, "number");
    assert!(h.trust & crate::TRUST_HINT != 0);
    assert_eq!(fnr(s, Config::default(), "f").slots[1].1, "any");
}

#[test]
fn functions_stored_in_objects_and_passed_around_are_followed() {
    let s = "var m = { add: function(a, b){ return a + b; }, neg: function(x){ return -x; } }; \
             var r = m.add(1, 2); var q = m.neg(3);";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let add = r.iter().find(|f| f.name == "add").unwrap();
    assert_eq!(add.slots[0].1, "number");
    assert_eq!(add.ret, "number");
    assert!(add.closed);
    // higher order: closures returned from a function and called later
    let s = "function compose(f, g){ return function(x){ return f(g(x)); }; } \
             var h = compose(function(a){ return a + 1; }, function(b){ return b * 2; }); var n = h(3);";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let inner = r
        .iter()
        .find(|f| f.slots.iter().any(|s| s.0 == "x"))
        .unwrap();
    assert_eq!(inner.slots[0].1, "number");
    let a = r
        .iter()
        .find(|f| f.slots.iter().any(|s| s.0 == "a"))
        .unwrap();
    assert_eq!(a.slots[0].1, "number");
}

#[test]
fn escaping_function_values_get_any_params() {
    let s = "on('x', function(e){ return e; });";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter()
            .find(|f| f.slots.iter().any(|s| s.0 == "e"))
            .unwrap()
            .slots[0]
            .1,
        "any"
    );
    // stored in an array that reaches unknown code
    let s = "var cbs = []; cbs.push(function(z){ return z; }); mystery(cbs);";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter()
            .find(|f| f.slots.iter().any(|s| s.0 == "z"))
            .unwrap()
            .slots[0]
            .1,
        "any"
    );
    // returned from an exported function: called by external code
    let s = "exports.mk = function(){ return function(y){ return y; }; };";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter()
            .find(|f| f.slots.iter().any(|s| s.0 == "y"))
            .unwrap()
            .slots[0]
            .1,
        "any"
    );
}

#[test]
fn es_hierarchy_with_super_shares_one_record() {
    let s = "class A { constructor(x){ this.x = x; } get(){ return this.x; } } \
             class B extends A { constructor(x, y){ super(x); this.y = y; } sum(){ return this.x + this.y + super.get(); } } \
             var b = new B(1, 2); var s = b.sum(); var g = b.get();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let sum = r.iter().find(|f| f.name == "sum").unwrap();
    assert_eq!(sum.ret, "number");
    assert!(sum.this_typed);
    // B's own field is only set by B's constructor: `number | undefined` for the shared record
    let get = r.iter().find(|f| f.name == "get").unwrap();
    assert_eq!(get.ret, "number");
    let ctor_b = r.iter().filter(|f| f.name == "constructor").nth(1).unwrap();
    assert_eq!(ctor_b.slots[0].1, "number");
}

#[test]
fn constructor_function_hierarchy_via_inherits_and_call() {
    let s =
        "function Base(n){ this.n = n; } Base.prototype.dbl = function(){ return this.n * 2; }; \
             function Kid(n, k){ Base.call(this, n); this.k = k; } \
             Kid.prototype = Object.create(Base.prototype); \
             Kid.prototype.both = function(){ return this.dbl() + this.k; }; \
             var kd = new Kid(2, 3); var r = kd.both();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let both = r.iter().find(|f| f.name == "both").unwrap();
    assert_eq!(both.ret, "number");
    assert!(both.this_typed);
    let dbl = r.iter().find(|f| f.name == "dbl").unwrap();
    assert_eq!(dbl.ret, "number");
}

#[test]
fn instanceof_in_a_hierarchy_does_not_exclude_siblings() {
    let s = "class A { constructor(){ this.v = 1; } } class B extends A { constructor(){ super(); } } \
             exports.f = function(x){ if (x instanceof B) { return 1; } var y = x; return y; }; new B();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    let f = r.iter().find(|f| f.name == "f").unwrap();
    assert_eq!(f.slots.iter().find(|s| s.0 == "y").unwrap().1, "any");
}

#[test]
fn node_builtin_modules_are_typed() {
    let s = "var path = require('path'); var fs = require('node:fs'); var os = require('os'); \
             function f(a){ var p = path.join(a, 'x'); var t = fs.readFileSync(p, 'utf8'); \
             var n = process.argv.length; var e = os.EOL; var d = path.parse(p); return t + e + d.base + n; } f('q');";
    let r = fnr(s, Config::default(), "f");
    let get = |n: &str| r.slots.iter().find(|s| s.0 == n).unwrap().1.clone();
    assert_eq!(get("p"), "string");
    assert_eq!(get("t"), "string");
    assert_eq!(get("n"), "number");
    assert_eq!(get("e"), "string");
    assert_eq!(
        get("d"),
        "{ base: string; dir: string; ext: string; name: string; root: string }"
    );
    assert_eq!(r.ret, "string");
    assert!(r.closed);
}

#[test]
fn pure_builtin_calls_do_not_make_arguments_escape() {
    let s = "function f(){ var a = [1]; console.log(a); var b = Math.PI * 2; a.push(2); return a; } f();";
    let r = fnr(s, Config::default(), "f");
    assert_eq!(r.slots.iter().find(|s| s.0 == "a").unwrap().1, "number[]");
    assert_eq!(r.slots.iter().find(|s| s.0 == "b").unwrap().1, "number");
    // a table miss is an unknown call: arguments escape
    let s =
        "var fs = require('fs'); function g(){ var a = [1]; fs.readFile('x', a); return a; } g();";
    assert_eq!(fnr(s, Config::default(), "g").class, Class::Any);
    // third-party modules are `any` with their own reason
    let s = "function h(){ var x = require('left-pad'); return x; } h();";
    assert_eq!(fnr(s, Config::default(), "h").reasons[0], Reason::Require);
}

#[test]
fn trusted_functions_carry_an_entry_guard() {
    let cfg = with_dts("declare function add(a: number, b?: string): number;");
    let r = fnr(
        "exports.add = function(a, b){ return a + (b ? 1 : 0); };",
        cfg,
        "add",
    );
    let g = r.guard.unwrap();
    assert!(g.contains("typeof a === 'number'"), "{g}");
    assert!(
        g.contains("typeof b === 'string'") && g.contains("b === undefined"),
        "{g}"
    );
    // `this` of a modelled class is guarded by a brand check
    let s = "class P { constructor(){ this.v = 1; } get(){ return this.v; } } new P().get();";
    let (r, _) = analyze_source(s, Config::default()).unwrap();
    assert_eq!(
        r.iter().find(|f| f.name == "get").unwrap().guard.as_deref(),
        Some("this instanceof P")
    );
    // a provable function needs no guard
    let r = fnr(
        "function f(n){ return n + 1; } f(1);",
        Config::default(),
        "f",
    );
    assert!(r.guard.is_none());
}

#[test]
fn reports_serialise_to_json() {
    let cfg = with_dts("declare function add(a: number): number;");
    let (r, s) = analyze_source("exports.add = function(a){ return a + 1; };", cfg).unwrap();
    let j = crate::report_json(&r, &s);
    assert!(j.contains("\"typed_functions\":[{\"name\":\"add\""), "{j}");
    assert!(j.contains("\"guard\":\"typeof a === 'number'\""), "{j}");
    assert!(j.starts_with("{\"summary\":{\"functions\":1,"), "{j}");
}

#[test]
fn bundled_modules_export_records_that_other_modules_can_follow() {
    let s = r#"module.exports = (function(require){
var __thaw_bundle_entry_key = "a";
var __thaw_bundle_factories = {
  "a": function(module, exports, require){ var c = require('./b'); exports.twice = function(n){ return n * c.k; }; },
  "b": function(module, exports, require){ exports.k = 4; exports.name = 'b'; }
};
var __thaw_bundle_require_maps = { "a": {"./b": "b"}, "b": {} };
})(globalThis.require);"#;
    // `c.k` is a number, so `twice` is fully typed once its parameter comes from the contract
    let cfg = with_dts("declare function twice(n: number): number;");
    let r = fnr(s, cfg, "twice");
    assert_eq!(r.ret, "number");
    assert!(r.closed, "{:?}", r);
    // a dynamic `require(x)` makes every module record visible to unknown code
    let s2 = s.replace(
        "exports.twice =",
        "var dyn = require(name); exports.twice =",
    );
    let cfg = with_dts("declare function twice(n: number): number;");
    let r = fnr(&s2, cfg, "twice");
    assert!(!r.closed);
}

#[test]
fn evidence_types_identifiers_from_annotations_and_initialisers() {
    let mut ev = crate::evidence::Evidence::default();
    ev.add_program(
        "pkg",
        "import { add } from 'pkg'; const n = 5; let s: string = 'a'; function main(k: number) { add(n, k); add(n, s); }",
    );
    let calls = &ev.calls["add"];
    assert_eq!(calls[0], vec![crate::dts::DT::Num, crate::dts::DT::Num]);
    assert_eq!(calls[1], vec![crate::dts::DT::Num, crate::dts::DT::Str]);
}

#[test]
fn dts_free_function_matches_a_function_on_a_plain_local_object() {
    let cfg = with_dts("declare function merge(a: number, b: number): number;");
    let r = fnr(
        "var utils = {}; utils.merge = function(a, b){ return a + b; }; exports.u = utils;",
        cfg,
        "merge",
    );
    assert!(r.dts);
    assert_eq!(r.slots[0].1, "number");
}

#[test]
fn aot_clone_inlines_constants_and_helpers() {
    let s = r#"module.exports = (function(require){
var __thaw_bundle_entry_key = "a";
var __thaw_bundle_factories = {
  "a": function(module, exports, require){
    var c = require('./b');
    var SCALE = 3;
    function clampIt(v) { return v < 0 ? 0 : v; }
    exports.scaled = scaled;
    function scaled(n) { return clampIt(Math.trunc(n * c.k)) * SCALE; }
    exports.shout = function (s) { return s.toUpperCase() + '!'; };
    exports.bad = function (s) { return /x/.test(s); };
  },
  "b": function(module, exports, require){ const k = (exports.k = 4); }
};
var __thaw_bundle_require_maps = { "a": {"./b": "b"}, "b": {} };
})(globalThis.require);"#;
    let cfg = with_dts(
        "declare function scaled(n: number): number; declare function shout(s: string): string; \
         declare function bad(s: string): boolean;",
    );
    let clones = crate::clone::extract_clones(s, cfg, "__aot_").unwrap();
    let names: Vec<_> = clones.iter().map(|c| c.exports[0].clone()).collect();
    assert_eq!(names, vec!["scaled", "shout"], "{clones:#?}");
    let scaled = &clones[0];
    assert_eq!(scaled.params, vec![crate::clone::Prim::Num]);
    assert!(scaled.ts.contains("var SCALE = 3;"), "{}", scaled.ts);
    assert!(
        scaled.ts.contains("function clampIt(v: number)"),
        "{}",
        scaled.ts
    );
    assert!(scaled.ts.contains("Math.trunc(n * 4)"), "{}", scaled.ts);
    assert!(!scaled.ts.contains("__thaw_export"));
}

#[test]
fn dts_default_export_and_babel_exports_are_recognised() {
    // `export default function clamp` in the declarations, `module.exports = clamp` in the bundle
    let cfg = with_dts("export default function clamp(a: number, lo: number, hi: number): number;");
    let r = fnr("function clamp(a, lo, hi){ return a < lo ? lo : a > hi ? hi : a; } module.exports = clamp;", cfg, "clamp");
    assert!(r.dts);
    assert_eq!(r.slots[0].1, "number");
    assert!(r.exports.contains(&"<default>".to_string()));
    // Babel's UMD helper exports through a parameter called `_exports`
    let cfg = with_dts("export function getRounds(h: string): number;");
    let r = fnr("(function(_exports){ _exports.getRounds = getRounds; function getRounds(h){ return h.length; } })({});", cfg, "getRounds");
    assert!(
        r.exports.contains(&"getRounds".to_string()),
        "{:?}",
        r.exports
    );
}

#[test]
fn clones_support_self_recursion_and_reject_runtime_state() {
    let s = "exports.fib = function fib(n){ return n < 2 ? n : fib(n - 1) + fib(n - 2); }; \
             var cache = {}; exports.cached = function(n){ return cache[n]; };";
    let cfg = with_dts(
        "declare function fib(n: number): number; declare function cached(n: number): number;",
    );
    let clones = crate::clone::extract_clones(s, cfg, "__a_").unwrap();
    assert_eq!(clones.len(), 1, "{clones:#?}");
    assert!(
        clones[0].ts.contains("__a_0_fib(n - 1) + __a_0_fib(n - 2)"),
        "{}",
        clones[0].ts
    );
}
