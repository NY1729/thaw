//! Table-driven tests of the C1 analyzer. Each source defines `f` (a function declaration, or a `var f`
//! holding the function under test); `def_of` runs it and takes `f`'s `FuncDef`.

use super::{analyze, stats_level, CState, CompileMode, FallbackReason, Stats};
use crate::{FuncDef, Interp, Kind, Value};
use std::rc::Rc;

use FallbackReason as R;

fn def_of(src: &str) -> Rc<FuncDef> {
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    let v = it
        .run_sloppy(&format!("{src}\nf"))
        .unwrap_or_else(|_| panic!("source did not run: {src}"));
    let Value::Object(o) = v else {
        panic!("f is not an object: {src}");
    };
    let def = match &*o.kind.borrow() {
        Kind::Closure { def, .. } => def.clone(),
        _ => panic!("f is not a closure: {src}"),
    };
    def
}

/// `name:class` entries (sorted), then `| free: ...`.
fn summary(def: &FuncDef) -> String {
    let plan = analyze(def);
    let mut parts: Vec<String> = plan
        .names
        .iter()
        .map(|n| format!("{}:{}", n.name, n.class.as_str()))
        .collect();
    parts.sort();
    let mut s = parts.join(" ");
    if !plan.free.is_empty() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str("| free: ");
        s.push_str(&plan.free.join(" "));
    }
    s
}

fn verdict(def: &FuncDef) -> Option<FallbackReason> {
    analyze(def).verdict().err()
}

/// (source, expected verdict, expected names and free names).
const CASES: &[(&str, Option<FallbackReason>, &str)] = &[
    // eligible leaf code
    ("function f(a, b) { return a + b }", None, "a:slot b:slot"),
    (
        "function f(a) { var c = a * 2; let d = c + 1; const e = d; return e }",
        None,
        "a:slot c:slot d:slot e:slot",
    ),
    (
        "function f() { return undefinedThing + 1 }",
        None,
        "| free: undefinedThing",
    ),
    (
        "function f(a) { for (let i = 0; i < 3; i++) { let y = a + i; y++ } return a }",
        None,
        "a:slot i:slot y:slot",
    ),
    (
        "function f(a) { var n = 0; while (n < a) { n++ } do { n-- } while (n > 0); return n }",
        None,
        "a:slot n:slot",
    ),
    (
        "function f(a) { if (a) { return 1 } else { return 2 } }",
        None,
        "a:slot",
    ),
    ("function f(a) { return a ? 1 : (2, 3) }", None, "a:slot"),
    ("function f(a) { return a.length }", None, "a:slot"),
    (
        "function f(a) { var x = a[0]; a[1] = x; return a }",
        None,
        "a:slot x:slot",
    ),
    (
        "function f() { var s = 'ab'; s += 'c'; return s }",
        None,
        "s:slot",
    ),
    (
        "function f(a) { return typeof a === 'number' ? -a : ~a }",
        None,
        "a:slot",
    ),
    ("function f(a) { return (a && a) ?? 5 }", None, "a:slot"),
    (
        "function f(a) { var t = a; t = t << 2 >>> 1; return (t & 0xff) | 0 }",
        None,
        "a:slot t:slot",
    ),
    ("function f() { return [1, 2, 3] }", None, ""),
    ("function f() { var o = {}; return o }", None, "o:slot"),
    ("function f() { g = 2; return 1 }", None, "| free: g"),
    (
        "function f(a) { var k = 0; for (;;) { k = k + 1; if (k > a) break; continue } return k }",
        None,
        "a:slot k:slot",
    ),
    (
        "function f() { var x; let y; const z = 1; return [x, y, z] }",
        None,
        "x:slot y:slot z:slot",
    ),
    ("var f = function g(a) { return a }", None, "a:slot"),
    (
        "function f() { var x = 1; { let a = 2; a } return x }",
        None,
        "a:slot x:slot",
    ),
    (
        "function f() { { let a = 1; } return a }",
        None,
        "a:slot | free: a",
    ),
    ("function f() { x = 1; var x; return x }", None, "x:slot"),
    ("function f() { return 1n }", None, ""),
    ("var f = (a) => a + 1", None, "a:slot"),
    ("function f(a) { return a ? [a] : {} }", None, "a:slot"),
    // tier A (plan 2.1): the six programs must report ReadBeforeDecl
    (
        "function f() { x; let x = 1 }",
        Some(R::ReadBeforeDecl),
        "x:slot",
    ),
    (
        "var x = 5; function f() { x; let x = 1; return x }",
        Some(R::ReadBeforeDecl),
        "x:slot",
    ),
    (
        "function f() { return typeof x; let x = 1 }",
        Some(R::ReadBeforeDecl),
        "x:slot",
    ),
    (
        "function f() { let n = 0; while (n < 2) { if (n === 1) { y; } let y = n; n++ } }",
        Some(R::ReadBeforeDecl),
        "n:slot y:slot",
    ),
    (
        "function f(a) { switch (a) { case 1: let q = 1; break; case 2: q; } }",
        Some(R::ReadBeforeDecl),
        "a:slot q:slot",
    ),
    (
        "function f() { let x = x + 1; return x }",
        Some(R::ReadBeforeDecl),
        "x:slot",
    ),
    // further tier A shapes
    (
        "function f() { x = 2; let x; return x }",
        Some(R::ReadBeforeDecl),
        "x:slot",
    ),
    ("function f() { let a = 1; { a } }", None, "a:slot"),
    (
        "function f(x) { for (let x of x) { } }",
        Some(R::ReadBeforeDecl),
        "x:slot x:slot",
    ),
    // switch-case lexical declarations without a read
    (
        "function f(a) { switch (a) { case 1: let q = 1; break; } }",
        Some(R::SwitchLexical),
        "a:slot q:slot",
    ),
    // closures: captured names are Scoped, others Slot
    (
        "function f() { var g = () => x; let x = 1; return g }",
        Some(R::InnerFunction),
        "g:slot x:scoped",
    ),
    (
        "function f() { var g = function () { return h }; var h = 2; return g }",
        Some(R::InnerFunction),
        "g:slot h:scoped",
    ),
    (
        "function f() { for (let i = 0; i < 2; i++) { var g = () => i } return g }",
        Some(R::InnerFunction),
        "g:slot i:scoped",
    ),
    (
        "function f(p) { var g = () => p; return g }",
        Some(R::InnerFunction),
        "g:slot p:scoped",
    ),
    (
        "function f() { var g = () => { let z = 1; return z }; return g }",
        Some(R::InnerFunction),
        "g:slot",
    ),
    (
        "function f(a) { var g = () => a; { let a = 2; a } return g }",
        Some(R::InnerFunction),
        "a:scoped a:scoped g:slot",
    ),
    (
        "function f(a) { { let a = 2; a } return a }",
        None,
        "a:slot a:slot",
    ),
    (
        "function f() { { let k = 1; var g = () => k } return g }",
        Some(R::InnerFunction),
        "g:slot k:scoped",
    ),
    (
        "function f() { var g = function () { let q = 1 }; return g }",
        Some(R::InnerFunction),
        "g:slot",
    ),
    // named function expressions: the self name is a Slot, a reference to it is a blocker
    // self-binding compiles
    ("var f = function g() { return g }", None, "g:slot"),
    ("var f = function g() { return 1 }", None, ""),
    ("var f = function g(g) { return g }", None, "g:slot"),
    ("function f() { var f = 1; return f }", None, "f:slot"),
    // arguments, new.target
    (
        "function f() { return arguments.length }",
        Some(R::Arguments),
        "| free: arguments",
    ),
    (
        "function f() { return () => arguments }",
        Some(R::InnerFunction),
        "",
    ),
    ("function f() { return new.target }", Some(R::NewTarget), ""),
    (
        "function f() { function g() { return arguments } return g }",
        Some(R::InnerFunction),
        "g:slot",
    ),
    // parameter forms
    (
        "function f(a = 1) { return a }",
        Some(R::DefaultParam),
        "a:slot",
    ),
    (
        "function f(...r) { return r }",
        Some(R::RestParam),
        "r:slot",
    ),
    (
        "function f({ a }) { return a }",
        Some(R::Destructuring),
        "a:slot",
    ),
    (
        "function f(a, [b, c]) { return a + b + c }",
        Some(R::Destructuring),
        "a:slot b:slot c:slot",
    ),
    (
        "function f(a = 1, b = a) { return b }",
        Some(R::DefaultParam),
        "a:slot b:slot",
    ),
    // destructuring declarations
    (
        "function f(o) { var { x } = o; return x }",
        Some(R::Destructuring),
        "o:slot x:slot",
    ),
    (
        "function f(o) { let [p, q] = o; return p }",
        Some(R::Destructuring),
        "o:slot p:slot q:slot",
    ),
    // statements and expressions that wait for later PRs
    (
        "function f(o) { for (const k in o) { } }",
        Some(R::ForIn),
        "k:slot o:slot",
    ),
    (
        "function f(o) { for (const x of o) { } }",
        Some(R::ForOf),
        "o:slot x:slot",
    ),
    (
        "function f(o) { for (var k in o) { } return k }",
        Some(R::ForIn),
        "k:slot o:slot",
    ),
    (
        "function f(o) { try { return o } catch (e) { return e } }",
        Some(R::Try),
        "e:slot o:slot",
    ),
    ("function f() { throw 1 }", Some(R::Throw), ""),
    (
        "function f(a) { switch (a) { case 1: return 2; default: return 3 } }",
        Some(R::Switch),
        "a:slot",
    ),
    (
        "function f() { l: for (;;) { break l } }",
        Some(R::Labels),
        "",
    ),
    ("function f(a) { return [...a] }", Some(R::Spread), "a:slot"),
    (
        "function f(a) { return a?.b }",
        Some(R::OptionalChain),
        "a:slot",
    ),
    (
        "function f(a) { return a`x` }",
        Some(R::TaggedTemplate),
        "a:slot",
    ),
    // in/instanceof and regex literals compile
    ("function f() { return /ab/g }", None, ""),
    (
        "function f() { return { m() { return 1 } } }",
        Some(R::ObjectMethod),
        "",
    ),
    (
        "function f(o) { return delete o.x }",
        Some(R::Delete),
        "o:slot",
    ),
    (
        "function f(o) { return 'x' in o }",
        // in/instanceof compile
        None,
        "o:slot",
    ),
    (
        "function f(o) { return o instanceof Array }",
        // in/instanceof compile
        None,
        "o:slot | free: Array",
    ),
    // C3: calls now compile
    ("function f(a) { return a() }", None, "a:slot"),
    (
        "function f() { return new Date() }",
        // C3: calls now compile
        None,
        "| free: Date",
    ),
    // C3: calls now compile
    ("function f(a) { return a.b() }", None, "a:slot"),
    (
        "function f() { return { __proto__: null } }",
        Some(R::ProtoLiteralKey),
        "",
    ),
    (
        "function f() { return import('x') }",
        Some(R::DynamicImport),
        "",
    ),
    // permanent blockers
    (
        "function f(o) { with (o) { return x } }",
        Some(R::With),
        "o:slot | free: x",
    ),
    (
        "function f(a) { return eval('a') }",
        Some(R::DirectEval),
        "a:slot | free: eval",
    ),
    (
        "function f() { return Function('return 1') }",
        Some(R::FunctionCtor),
        "| free: Function",
    ),
    (
        "var f = ({ m() { return super.toString() } }).m",
        Some(R::Super),
        "",
    ),
    (
        "var f = (class { #x = 1; m() { return this.#x } }).prototype.m",
        Some(R::PrivateName),
        "",
    ),
    (
        "function f() { class A {} return A }",
        Some(R::ClassBody),
        "A:slot",
    ),
    (
        "var f = class { constructor(a) { this.a = a } }",
        Some(R::ClassConstructor),
        "a:slot",
    ),
    (
        "function f() { if (true) { function g() {} } return 1 }",
        Some(R::AnnexBBlockFunction),
        "g:slot g:slot",
    ),
    (
        "function f() { { function g() {} } }",
        Some(R::AnnexBBlockFunction),
        "g:slot g:slot",
    ),
    (
        "function f() { l: function g() {} }",
        Some(R::LabeledFunction),
        "g:slot",
    ),
    // generators and async
    ("function* f(a) { yield a }", Some(R::Generator), "a:slot"),
    (
        "async function f(a) { return await a }",
        Some(R::Async),
        "a:slot",
    ),
    ("var f = async (a) => a", Some(R::Async), "a:slot"),
    (
        "function f(a) { outer: for (;;) { inner: for (;;) { break outer } } return a }",
        Some(R::Labels),
        "a:slot",
    ),
];

#[test]
fn analysis_table() {
    let mut failures = Vec::new();
    for (src, want_reason, want_names) in CASES {
        let def = def_of(src);
        let got = (verdict(&def), summary(&def));
        if got != (*want_reason, want_names.to_string()) {
            failures.push(format!(
                "{src}\n    want {want_reason:?} {want_names:?}\n    got  {:?} {:?}",
                got.0, got.1
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Analysis is only a classification: the same source must behave identically under `off` and `on`.
#[test]
fn analysis_table_behaviour_matches_interpreter() {
    for (src, _, _) in CASES {
        let program = format!(
            "{src}\ntry {{ console.log(String(f(1, 2))) }} catch (e) {{ console.log('threw', String(e)) }}"
        );
        let mut off = Interp::with_compile_mode(CompileMode::Off);
        let mut on = Interp::with_compile_mode(CompileMode::On);
        on.compile_after = 1;
        let ok_off = off.run_sloppy(&program).is_ok();
        let ok_on = on.run_sloppy(&program).is_ok();
        assert_eq!(ok_off, ok_on, "{src}");
        assert_eq!(on.out, off.out, "{src}");
        assert!(!off.out.is_empty() || !ok_off, "{src}");
    }
}

#[test]
fn tier_a_six_programs_report_read_before_decl() {
    let programs = [
        "function f() { x; let x = 1 }",
        "var x = 5; function f() { x; let x = 1; return x }",
        "function f() { return typeof x; let x = 1 }",
        "function f() { let n = 0; while (n < 2) { if (n === 1) { y; } let y = n; n++ } }",
        "function f(a) { switch (a) { case 1: let q = 1; break; case 2: q; } }",
        "function f() { let x = x + 1; return x }",
    ];
    for src in programs {
        assert_eq!(verdict(&def_of(src)), Some(R::ReadBeforeDecl), "{src}");
    }
}

#[test]
fn pure_kernel_flag() {
    let cases = [
        ("function f(a, b) { return (a + b) | 0 }", true),
        ("function f(a) { var s = 0; for (var i = 0; i < a.length; i++) { s = s + a[i] } return s }", true),
        ("function f(a) { return a.x }", false),
        ("function f(a) { return a() }", false),
        ("function f() { return [1, 2] }", true),
        ("function f() { return {} }", false),
        ("function f() { g = 1 }", false),
        ("function f() { var g = 1; return g }", true),
        ("function f(a) { return new Array(a) }", false),
        ("function f() { return () => 1 }", false),
        ("function f(a) { return a.length + a[0] }", true),
    ];
    for (src, pure) in cases {
        assert_eq!(analyze(&def_of(src)).pure_kernel, pure, "{src}");
    }
}

#[test]
fn label_table_in_order() {
    let plan = analyze(&def_of(
        "function f(a) { outer: for (;;) { inner: for (;;) { break outer } } return a }",
    ));
    assert_eq!(plan.labels, ["outer", "inner"]);
    let plan = analyze(&def_of("function f() { a: { } b: { } }"));
    assert_eq!(plan.labels, ["a", "b"]);
    assert!(analyze(&def_of("function f() { return 1 }"))
        .labels
        .is_empty());
}

#[test]
fn analysis_is_pure_and_does_not_touch_cstate() {
    let def = def_of("function f(a) { var x = a; return x }");
    let first = summary(&def);
    assert_eq!(summary(&def), first);
    assert!(matches!(&*def.cstate.borrow(), CState::Cold(0)));
}

#[test]
fn stats_follow_the_cstate_transitions() {
    // add: call 1 is cold, call 2 analyses it (eligible), call 3 is an eligible call.
    // fib(5) makes 15 calls: the first is cold, the second analyses it (eligible: self-binding compiles),
    // the other 13 are eligible calls, plus the second call itself.
    let src = "function add(a, b) { return a + b } \
               function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2) } \
               add(1, 2); add(3, 4); add(5, 6); fib(5);";
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.run(src).unwrap();
    let s = &it.cstats;
    assert_eq!(s.calls(), 18);
    // self-binding compiles
    assert_eq!(s.eligible(), 2);
    // C3: calls now compile
    assert_eq!(s.rejected(R::NotYet), 0);
    // self-binding compiles
    assert_eq!(s.calls_eligible(), 16);
    // C3: calls now compile
    assert_eq!(s.calls_rejected(R::NotYet), 0);
}

#[test]
fn stats_level_parsing() {
    assert_eq!(stats_level(None), 0);
    assert_eq!(stats_level(Some("")), 0);
    assert_eq!(stats_level(Some("0")), 0);
    assert_eq!(stats_level(Some("1")), 1);
    assert_eq!(stats_level(Some("2")), 2);
    assert_eq!(stats_level(Some("yes")), 1);
}

#[test]
fn reason_table_is_consistent() {
    for (i, r) in FallbackReason::ALL.iter().enumerate() {
        assert_eq!(*r as usize, i, "{r:?} is out of order");
    }
    assert_eq!(FallbackReason::COUNT, FallbackReason::ALL.len());
    let mut names: Vec<&str> = FallbackReason::ALL.iter().map(|r| r.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        FallbackReason::COUNT,
        "reason names must be unique"
    );
}

#[test]
fn report_lists_only_reasons_seen() {
    let mut s = Stats::with_level(1);
    assert!(s.report().is_empty());
    s.record_rejected(R::With);
    s.record_eligible();
    let text = s.report();
    assert!(text.contains("with"), "{text}");
    assert!(!text.contains("delete"), "{text}");
}
