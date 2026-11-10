//! PR A of the inner-function plan (TJ157): analysis bookkeeping only. Name levels, `needs_scope` and the
//! `ScopedBlock` blocker bit are checked here; the verdicts stay pinned by analyze_tests.rs.

use super::{analyze, CompileMode, FallbackReason as R, Interp};
use crate::{FuncDef, Kind, Value};
use std::rc::Rc;

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

/// `name:level` for every declared name (sorted).
fn levels(src: &str) -> String {
    let plan = analyze(&def_of(src));
    let mut v: Vec<String> = plan
        .names
        .iter()
        .map(|n| format!("{}:{}", n.name, n.level))
        .collect();
    v.sort();
    v.join(" ")
}

fn needs(src: &str) -> bool {
    analyze(&def_of(src)).needs_scope
}

#[test]
fn name_levels_are_function_body_or_block() {
    assert_eq!(
        levels("function f(a) { var v = 0; let t = 1; { let u = 2; } return a + t + v }"),
        "a:0 t:1 u:2 v:0"
    );
}

#[test]
fn needs_scope_follows_captures_strictness_and_this() {
    // no inner function: never a frame scope
    assert!(!needs("function f(a) { var b = a; return b }"));
    // a closure naming a local of the function
    assert!(needs("function f() { var n = 0; return () => n }"));
    // a closure naming only globals
    assert!(!needs("function f() { return () => Math }"));
    // a strict function with a closure, even with no captured name
    assert!(needs("function f() { 'use strict'; return () => 1 }"));
    // an arrow in a non-arrow function reads `this`
    assert!(needs("function f() { return () => this }"));
    // an arrow inside an arrow, naming nothing
    assert!(!needs("var f = () => () => 1"));
    // an arrow inside an arrow, naming the outer parameter
    assert!(needs("var f = (x) => () => x"));
}

#[test]
fn scoped_block_is_a_blocker_bit_never_the_verdict() {
    let src =
        "function f() { var fs = []; for (let i = 0; i < 2; i++) { fs.push(() => i) } return fs }";
    let plan = analyze(&def_of(src));
    assert_eq!(plan.verdict(), Err(R::InnerFunction));
    assert_eq!(plan.all_reasons(), vec![R::InnerFunction, R::ScopedBlock]);

    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    it.run(&format!("{src}\nf(); f();")).unwrap();
    let s = &it.cstats;
    assert_eq!(s.rejected(R::InnerFunction), 1);
    assert_eq!(s.rejected(R::ScopedBlock), 0);
    assert_eq!(s.any_functions(R::ScopedBlock), 1);
    assert_eq!(s.sole_functions(R::ScopedBlock), 0);
    assert_eq!(s.compiled(), 0);
    // the new counters stay out of the report while they are zero
    let text = s.report();
    assert!(text.contains("scoped-block"), "{text}");
    assert!(!text.contains("thaw-js compile scopes"), "{text}");
}
