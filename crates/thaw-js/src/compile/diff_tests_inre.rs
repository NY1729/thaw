//! Differential tests of `in`, `instanceof` and regex literals in compiled functions (plan TJ156, PR3a). Same
//! contract as `diff_tests_calls.rs`: `off` is the reference, `on` and `check` must print the same lines, and each
//! compiled mode must compile at least one function, so a program cannot pass by staying on the AST.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

fn same(src: &str, min_compiled: u64) -> Vec<String> {
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    assert!(
        !want.is_empty() || !ok_want,
        "reference printed nothing: {src}"
    );
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, it) = run_mode(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            it.cstats.compiled() >= min_compiled,
            "{mode:?}: compiled {} < {min_compiled}: {src}",
            it.cstats.compiled()
        );
    }
    want
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn inre_in_own_inherited_holes_and_key_coercion() {
    let out = same(
        r#"
        function has(o, k) { return k in o }
        var base = { inh: 1 };
        var child = Object.create(base);
        child.own = 2;
        var arr = [1, , 3];
        console.log(has(child, 'own'), has(child, 'inh'), has(child, 'nope'), has(child, 'hasOwnProperty'));
        console.log(has(arr, 0), has(arr, 1), has(arr, 2), has(arr, 5), has(arr, 'length'), has(arr, '2'));
        console.log(has(arr, 0.5), has(child, 0), has(arr, -1), has(child, null), has(child, undefined));
        console.log(has(child, 'own'), has(arr, 1));
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&[
            "true true false true",
            "true false true false true true",
            "false false false false false",
            "true false",
        ])
    );
}

#[test]
fn inre_in_array_index_loop_reads_each_slot() {
    let out = same(
        r#"
        function count(a) { var n = 0; for (var i = 0; i < 4; i++) { if (i in a) n++ } return n }
        console.log(count([5, 6]), count([, , 1]), count([]));
        console.log(count([5, 6]), count([, , 1]), count([]));
        "#,
        1,
    );
    assert_eq!(out, lines(&["2 1 0", "2 1 0"]));
}

#[test]
fn inre_in_non_object_right_side_throws_the_interpreter_text() {
    let out = same(
        r#"
        function g(s) { try { return 'length' in s } catch (e) { return e.name + ': ' + e.message } }
        console.log(g('abc'), g(5), g(null), g(undefined), g(true));
        console.log(g('abc'), g(5), g(null), g(undefined), g(true));
        "#,
        1,
    );
    assert!(out[0].contains("TypeError"), "{}", out[0]);
    assert_eq!(out[0], out[1]);
}

#[test]
fn inre_in_evaluates_left_then_right_and_calls_proxy_has() {
    let out = same(
        r#"
        var log = [];
        var p = new Proxy({ a: 1 }, { has(t, k) { log.push('has:' + String(k)); return k in t } });
        function has(o, k) { return k in o }
        function lhs() { log.push('L'); return 'a' }
        function rhs() { log.push('R'); return p }
        function ord() { return lhs() in rhs() }
        console.log(has(p, 'a'), has(p, 'b'), has(p, 'a'));
        console.log(ord(), log.join(','));
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&["true false true", "true has:a,has:b,has:a,L,R,has:a",])
    );
}

#[test]
fn inre_instanceof_chain_primitives_and_callables() {
    let out = same(
        r#"
        function A() {}
        function B() {}
        B.prototype = Object.create(A.prototype);
        function f(o, C) { return o instanceof C }
        console.log(f(new B(), A), f(new B(), B), f(new A(), B), f(1, A), f(null, A));
        console.log(f({}, Object), f([], Array), f(new B(), function () {}), f(1, () => 1));
        console.log(f(new B(), A), f(new B(), B), f(new A(), B), f(1, A), f(null, A));
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&[
            "true true false false false",
            "true true false false",
            "true true false false false",
        ])
    );
}

#[test]
fn inre_instanceof_symbol_has_instance_is_called_per_evaluation() {
    let out = same(
        r#"
        var hits = 0;
        var Odd = {};
        Odd[Symbol.hasInstance] = function (v) { hits++; return typeof v === 'number' && v % 2 === 1 };
        function f(v) { return v instanceof Odd }
        console.log(f(1), f(2), f('x'), hits);
        console.log(f(3), hits);
        "#,
        1,
    );
    assert_eq!(out, lines(&["true false false 3", "true 4"]));
}

/// Negative control: `hasInstance` runs user code, so the function is impure and `check` must not run it twice.
#[test]
fn inre_instanceof_is_impure_so_check_mode_runs_the_hook_once() {
    let out = same(
        r#"
        var hits = 0;
        var H = {};
        H[Symbol.hasInstance] = function () { hits++; return true };
        function f(v) { return v instanceof H }
        for (var i = 0; i < 3; i++) f(1);
        console.log(hits);
        "#,
        1,
    );
    assert_eq!(out, lines(&["3"]));
}

#[test]
fn inre_in_is_impure_so_check_mode_runs_the_proxy_trap_once() {
    let out = same(
        r#"
        var hits = 0;
        var p = new Proxy({}, { has(t, k) { hits++; return false } });
        function f(o) { return 'x' in o }
        for (var i = 0; i < 3; i++) f(p);
        console.log(hits, f({ x: 1 }));
        "#,
        1,
    );
    assert_eq!(out, lines(&["3 true"]));
}

#[test]
fn inre_regex_literal_is_a_fresh_object_per_evaluation() {
    let out = same(
        r#"
        function mk() { return /a/g }
        var r1 = mk(), r2 = mk();
        console.log(r1 === r2, r1 instanceof RegExp, r1.source, r1.flags, r1.global, r1.lastIndex);
        function inc() { return /x/ }
        var a = inc();
        a.lastIndex = 5;
        console.log(inc().lastIndex, a.lastIndex, inc() === inc());
        "#,
        1,
    );
    assert_eq!(out, lines(&["false true a g true 0", "0 5 false"]));
}

#[test]
fn inre_regex_lastindex_state_is_not_shared_between_calls() {
    let out = same(
        r#"
        function step(s) {
            var re = /a/g;
            var m = re.exec(s);
            return m === null ? 'none' : String(m.index) + '@' + re.lastIndex;
        }
        console.log(step('aXa'), step('aXa'), step('bb'));
        console.log(step('aXa'), step('aXa'), step('bb'));
        "#,
        1,
    );
    assert_eq!(out, lines(&["0@1 0@1 none", "0@1 0@1 none"]));
}

#[test]
fn inre_regex_flags_and_source_escapes() {
    let out = same(
        r#"
        function fl() { return /a\/b/im }
        function plain() { return /[0-9]+/ }
        var r = fl();
        console.log(r.source, r.flags, r.ignoreCase, r.multiline, r.test('xA/B'));
        console.log(plain().test('abc'), plain().test('a1'), plain().source);
        "#,
        1,
    );
    assert_eq!(
        out,
        lines(&["a\\/b im true true true", "false true [0-9]+"])
    );
}

#[test]
fn inre_regex_in_a_loop_with_test_and_replace() {
    let out = same(
        r#"
        function count(s) {
            var n = 0;
            for (var i = 0; i < 3; i++) {
                if (/o/.test(s)) n++;
                n += s.replace(/o/g, '0').length;
            }
            return n;
        }
        console.log(count('foo'), count('bar'));
        console.log(count('foo'), count('bar'));
        "#,
        1,
    );
    assert_eq!(out, lines(&["12 9", "12 9"]));
}

/// Negative control: `#p in o` stays a blocker of its own (`PrivateName`), so the output must still match `off`.
#[test]
fn inre_private_name_in_stays_on_the_ast() {
    let src = r#"
        class C { #p = 1; static has(o) { return #p in o } }
        console.log(C.has(new C()), C.has({}));
        console.log(C.has(new C()), C.has({}));
        "#;
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    let (got, ok, it) = run_mode(src, CompileMode::On);
    assert_eq!((got, ok), (want, ok_want));
    assert!(it.cstats.rejected(FallbackReason::PrivateName) >= 1);
}
