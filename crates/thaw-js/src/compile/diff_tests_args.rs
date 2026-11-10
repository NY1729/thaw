//! Tests of the pooled argument buffers of compiled calls (`Interp::arg_pool`, `call.rs` `eval_args`, plan TJ158
//! PR1). Programs run under Off (the reference), On and Check; output and ok-ness must match. `arg_pool_hits`
//! proves a recycled buffer was taken, and the pool is empty and bounded after every run.

use super::run::ARG_POOL_CAP;
use super::*;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

fn check_pool(it: &Interp, mode: CompileMode) {
    assert!(
        it.arg_pool.iter().all(|v| v.is_empty()),
        "{mode:?}: a pooled argument buffer kept a value"
    );
    assert!(it.arg_pool.len() <= ARG_POOL_CAP, "{mode:?}: pool grew");
}

/// Off, On and Check print the same; On and Check compile at least `min_compiled` functions.
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
        check_pool(&it, mode);
    }
    want
}

#[test]
fn args_duplicate_parameter_names_equal_off() {
    same(
        "function dup(a, a) { return a }\n\
         console.log(dup(1, 2), dup(3));\n\
         for (var i = 0; i < 3; i++) console.log(dup(i, i + 10));",
        1,
    );
}

#[test]
fn args_more_and_fewer_than_parameters_equal_off() {
    same(
        "function one(a) { return a }\n\
         function two(a, b) { return b }\n\
         function call3() { return one(1, 2, 3) }\n\
         console.log(one(1, 2, 3), one(), two(4), call3(), call3());",
        1,
    );
}

#[test]
fn args_self_reference_slot_equal_off() {
    same(
        "var fact = function h(n) { return n <= 1 ? 1 : n * h(n - 1) };\n\
         function f5() { return fact(5) }\n\
         console.log(fact(6), f5(), f5());",
        1,
    );
}

#[test]
fn args_recursion_nests_buffers_without_aliasing_equal_off() {
    same(
        "function add(a, b) { return a + b }\n\
         function sumTo(n) { return n === 0 ? 0 : add(n, sumTo(n - 1)) }\n\
         function pair(n) { return add(sumTo(n), add(n, sumTo(n - 1))) }\n\
         console.log(sumTo(30), sumTo(10), pair(8), pair(8));",
        2,
    );
}

#[test]
fn args_throw_mid_argument_list_equal_off() {
    same(
        "function two(a, b) { return a + b }\n\
         function g(x) { return two(1, x.p) }\n\
         var r = '';\n\
         for (var i = 0; i < 3; i++) { try { g(undefined) } catch (e) { r += e.name } }\n\
         console.log(r, g({ p: 2 }));",
        1,
    );
}

#[test]
fn args_nested_throw_with_live_outer_buffer_equal_off() {
    same(
        "function p(a, b) { return a + b }\n\
         function q(x) { return p(x, p(1, x.y)) }\n\
         var r = '';\n\
         for (var i = 0; i < 3; i++) { try { q(undefined) } catch (e) { r += e.name } }\n\
         console.log(r, q({ y: 1 }));",
        1,
    );
}

#[test]
fn args_getter_reentrancy_during_argument_evaluation_equal_off() {
    same(
        "function f(a) { return a * 2 }\n\
         var o = { get p() { return f(5) } };\n\
         function h(a, b) { return a + b }\n\
         function g(o) { return h(1, o.p) }\n\
         console.log(g(o), g(o), h(2, f(3)));",
        2,
    );
}

#[test]
fn args_arguments_using_callee_stays_on_the_ast_equal_off() {
    same(
        "function args(a) { return arguments.length + a }\n\
         function callsArgs() { return args(1, 2, 3) }\n\
         var s = 0;\n\
         for (var i = 0; i < 5; i++) s += args(i, 1);\n\
         console.log(s, callsArgs(), callsArgs());",
        1,
    );
}

#[test]
fn args_sloppy_this_equal_off() {
    same(
        "function st() { return typeof this }\n\
         var o = { m: function (a) { return this === o && a } };\n\
         function callst() { return st() }\n\
         function callm() { return o.m(7) }\n\
         console.log(callst(), callm(), callst());",
        1,
    );
}

#[test]
fn args_callee_throw_in_body_equal_off() {
    same(
        "function bad(a) { return a.x.y }\n\
         function callBad(v) { return bad(v) }\n\
         var r = '';\n\
         for (var i = 0; i < 5; i++) { try { callBad({}) } catch (e) { r += e.name } }\n\
         console.log(r, typeof callBad);",
        1,
    );
}

#[test]
fn args_not_callable_and_not_a_constructor_equal_off() {
    same(
        "var notf = 5;\n\
         function cn() { return notf(1, 2) }\n\
         function mk(x) { return new x(1) }\n\
         try { cn() } catch (e) { console.log(e.name, e.message) }\n\
         try { mk(5) } catch (e) { console.log(e.name) }\n\
         console.log(typeof mk(function K(a) { this.a = a }).a);",
        1,
    );
}

#[test]
fn args_infinite_recursion_with_arguments_equal_off() {
    same(
        "function inf(a) { return inf(a + 1) + 1 }\n\
         try { inf(0) } catch (e) { console.log(e.name) }\n\
         try { inf(0) } catch (e) { console.log(e.name) }\n\
         console.log('after');",
        1,
    );
}

/// A loop inside a compiled function calls a compiled callee: the buffer is recycled, the pool stays empty
/// between calls and bounded.
#[test]
fn arg_pool_is_reused_empty_and_bounded() {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    let _ = it.run_sloppy(
        "function f(a, b) { var t = a + b; return t }\n\
         function drive() { var s = 0; for (var i = 0; i < 1000; i++) s += f(i, 1); return s }\n\
         console.log(String(drive()));",
    );
    assert_eq!(it.out, vec!["500500".to_string()]);
    assert!(it.cstats.compiled() >= 2, "f and drive must compile");
    assert!(
        it.cstats.arg_pool_hits() >= 900,
        "reused buffers: {}",
        it.cstats.arg_pool_hits()
    );
    check_pool(&it, CompileMode::On);
    assert!(!it.arg_pool.is_empty(), "a buffer was returned to the pool");
}

/// Negative control: calls made from AST code (top level) take the generic path and never a pooled buffer.
#[test]
fn arg_pool_untouched_by_ast_call_sites() {
    let (out, ok, it) = run_mode(
        "function f(a) { return a + 1 }\n\
         var s = 0;\n\
         for (var i = 0; i < 100; i++) s += f(i);\n\
         console.log(String(s));",
        CompileMode::On,
    );
    assert!(ok);
    assert_eq!(out, vec!["5050".to_string()]);
    assert!(it.cstats.compiled() >= 1, "f must compile");
    assert_eq!(it.cstats.arg_pool_hits(), 0);
    check_pool(&it, CompileMode::On);
}
