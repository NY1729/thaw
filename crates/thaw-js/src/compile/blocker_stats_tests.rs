//! PR0 blocker bookkeeping: every blocker of a rejected function is in its mask, and the stats count
//! each reason as first blocker, as any blocker, and as the sole blocker, with the calls of each.

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

/// Runs `src` with compilation on, analysing every function on its first call.
fn run_on(src: &str) -> Interp {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    it.run(src).unwrap();
    it
}

#[test]
fn two_blockers_are_both_any_and_neither_is_sole() {
    // `switch` comes first, so it is the first blocker; `for-of` is the second.
    let src = "function f(a) { switch (a) { case 1: return 2; default: return 3 } \
               for (const x of a) { } return 0 }";
    let plan = analyze(&def_of(src));
    assert_eq!(plan.verdict(), Err(R::Switch));
    assert_eq!(plan.all_reasons(), vec![R::Switch, R::ForOf]);

    let it = run_on(&format!("{src}\nf(1); f(2);"));
    let s = &it.cstats;
    assert_eq!(s.calls(), 2);
    assert_eq!(s.rejected(R::Switch), 1);
    assert_eq!(s.rejected(R::ForOf), 0);
    assert_eq!(s.any_functions(R::Switch), 1);
    assert_eq!(s.any_functions(R::ForOf), 1);
    assert_eq!(s.sole_functions(R::Switch), 0);
    assert_eq!(s.sole_functions(R::ForOf), 0);
    // calls go to every blocker (any), to none as the sole one
    assert_eq!(s.calls_rejected(R::Switch), 2);
    assert_eq!(s.calls_any(R::Switch), 2);
    assert_eq!(s.calls_any(R::ForOf), 2);
    assert_eq!(s.calls_sole(R::Switch), 0);
    assert_eq!(s.calls_sole(R::ForOf), 0);
}

#[test]
fn one_blocker_is_sole() {
    let src = "function f(a) { switch (a) { case 1: return 2; default: return 3 } } \
               f(1); f(2); f(3);";
    let it = run_on(src);
    let s = &it.cstats;
    assert_eq!(s.rejected(R::Switch), 1);
    assert_eq!(s.any_functions(R::Switch), 1);
    assert_eq!(s.sole_functions(R::Switch), 1);
    assert_eq!(s.any_functions(R::ForOf), 0);
    assert_eq!(s.calls_any(R::Switch), 3);
    assert_eq!(s.calls_sole(R::Switch), 3);
    let text = s.report();
    assert!(text.contains("thaw-js compile blockers"), "{text}");
    assert!(
        text.lines().any(|l| l.trim_start().starts_with("switch")
            && l.contains("any=1")
            && l.contains("sole=1")),
        "{text}"
    );
}

#[test]
fn clean_function_adds_nothing() {
    let src = "function f(a) { return a + 1 } f(1); f(2);";
    assert_eq!(analyze(&def_of(src)).blockers, 0);
    let it = run_on(src);
    let s = &it.cstats;
    assert_eq!(s.eligible(), 1);
    for &r in R::ALL {
        assert_eq!(s.any_functions(r), 0, "{r:?}");
        assert_eq!(s.sole_functions(r), 0, "{r:?}");
        assert_eq!(s.calls_any(r), 0, "{r:?}");
        assert_eq!(s.calls_sole(r), 0, "{r:?}");
    }
    assert!(!s.report().contains("blockers"), "{}", s.report());
}

#[test]
fn tier_a_read_before_decl_is_in_the_mask() {
    // `z` is read before its `let` takes effect: a tier A blocker, and the only one.
    let src = "function f() { const y = z; let z = 1; return y }";
    let plan = analyze(&def_of(src));
    assert_eq!(plan.verdict(), Err(R::ReadBeforeDecl));
    assert_eq!(plan.all_reasons(), vec![R::ReadBeforeDecl]);
}
