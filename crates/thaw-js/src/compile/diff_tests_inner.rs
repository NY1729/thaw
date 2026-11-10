//! Differential tests of compiled functions with inner closures (plan TJ157, PR B: function expressions and
//! arrows whose captured names are parameters or `var`s). Each program runs under `off` (the reference), `on`
//! and `check`; output and ok-ness must match, and the `on` run must compile the function under test, so a
//! program cannot pass by staying on the AST. Refused shapes stay on the AST and must say why.

use super::*;
use crate::Interp;

fn interp(src_units: &[&str], mode: CompileMode) -> Interp {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    for src in src_units {
        it.run_sloppy(src)
            .unwrap_or_else(|e| panic!("{mode:?}: {src}: {e:?}"));
    }
    it
}

/// Runs `units` (one realm, one unit after another) under all three modes, asserts identical output, and
/// asserts the `on` run compiled at least `min_compiled` functions. Returns the reference output.
fn same_units(units: &[&str], min_compiled: u64) -> Vec<String> {
    let want = interp(units, CompileMode::Off).out.clone();
    for mode in [CompileMode::On, CompileMode::Check] {
        let it = interp(units, mode);
        assert_eq!(it.out, want, "{mode:?}: {units:?}");
        if mode == CompileMode::On {
            assert!(
                it.cstats.compiled() >= min_compiled,
                "compiled {} < {min_compiled}: {units:?}",
                it.cstats.compiled()
            );
        }
    }
    want
}

fn same(src: &str, min_compiled: u64) -> Vec<String> {
    same_units(&[src], min_compiled)
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn counter_factory_matches_off() {
    // two independent counters; a closure outlives the call that made it
    let src = r#"
        function mk() { var n = 0; return function () { return ++n } }
        var a = mk(), b = mk();
        console.log("" + a() + "," + a() + "," + b() + "," + a() + "," + b());
    "#;
    assert_eq!(same(src, 1), strs(&["1,2,1,3,2"]));
    let it = interp(&[src], CompileMode::On);
    assert!(it.cstats.scope_frames() > 0, "no per-call scope was built");
    assert!(
        it.cstats.closures_created() > 0,
        "no compiled closure was created"
    );
    assert!(it.cstats.compiled_calls() > 0);
}

#[test]
fn shared_mutable_var_matches_off() {
    // two closures over one `var`; the outer reads it after calling one of them
    let src = r#"
        function pair() {
            var x = 1;
            var inc = function () { x = x + 1 };
            var get = function () { return x };
            inc(); inc();
            var seen = get();
            return seen + ":" + x;
        }
        console.log(pair());
        console.log(pair());
    "#;
    assert_eq!(same(src, 1), strs(&["3:3", "3:3"]));
}

#[test]
fn captured_names_reassigned_after_creation_matches_off() {
    // the closure reads a parameter and a `var` as they are when it runs, not when it was created
    let src = r#"
        function late(p) {
            var g = function () { return p + q };
            var q = 5;
            p = 10;
            q = q + 1;
            return g();
        }
        console.log(late(1));
    "#;
    assert_eq!(same(src, 1), strs(&["16"]));
}

#[test]
fn closure_over_param_and_curried_arrow_matches_off() {
    let src = r#"
        function adder(x) { return function (y) { return x + y } }
        var add5 = adder(5);
        console.log(add5(3));
        var curry = x => y => x + y;
        console.log(curry(2)(3));
    "#;
    assert_eq!(same(src, 1), strs(&["8", "5"]));
}

#[test]
fn inner_calls_and_loop_matches_off() {
    let src = r#"
        function sum(n) {
            var acc = 0;
            var add = function (i) { acc = acc + i };
            for (var i = 0; i < n; i++) add(i);
            return acc;
        }
        console.log(sum(5));
        function two() {
            var a = 1;
            var f = function () { return a * 2 };
            var g = function () { return f() + 1 };
            return g();
        }
        console.log(two());
    "#;
    assert_eq!(same(src, 1), strs(&["10", "3"]));
}

#[test]
fn callbacks_to_natives_match_off() {
    let src = r#"
        function scale(k) { return [1, 2, 3].map(function (x) { return x * k }) }
        console.log(scale(10).join(","));
        function sorted(p) { return [3, 1, 2].sort(function (a, b) { return (a - b) * p }) }
        console.log(sorted(1).join(","));
        function total() { var acc = 0; [1, 2, 3].forEach(function (x) { acc += x }); return acc }
        console.log(total());
        function red(k) { return [1, 2, 3].reduce(function (s, x) { return s + x * k }, 0) }
        console.log(red(2));
    "#;
    assert_eq!(same(src, 1), strs(&["10,20,30", "1,2,3", "6", "12"]));
}

#[test]
fn this_in_arrows_matches_off() {
    let src = r#"
        function F() { this.v = 1; return () => this.v }
        var g = F();
        console.log("" + g());
        var h = new F();
        console.log("" + h());
        var o = { v: 7, m: function () { return () => this.v } };
        console.log("" + o.m()());
        function S() { "use strict"; return () => this }
        console.log("" + (S()() === undefined));
        function W() { this.q = 4; return () => () => this.q }
        var w = W();
        console.log("" + w()());
    "#;
    same(src, 1);
}

#[test]
fn named_function_expression_matches_off() {
    // the self name is used by the outer body only (a slot), the closure does not name it
    let src = r#"
        var f = function nm(n) {
            var cb = function () { return 1 };
            return n ? nm(n - 1) + cb() : 0;
        };
        console.log("" + f(3));
    "#;
    assert_eq!(same(src, 1), strs(&["3"]));
}

#[test]
fn recursion_with_closures_matches_off() {
    // every call holds its own scope: each `g` returns its own call's `k`
    let src = r#"
        function rec(n) {
            var k = n;
            var g = function () { return k };
            if (n === 0) return g();
            return rec(n - 1) + g();
        }
        console.log("" + rec(3));
    "#;
    assert_eq!(same(src, 1), strs(&["6"]));
}

#[test]
fn duplicate_and_redeclared_params_match_off() {
    let src = r#"
        function dup(a, a) { var g = function () { return a }; return g() }
        console.log("" + dup(1, 2));
        function vp(p) { var p; var g = function () { return p }; return g() }
        console.log("" + vp(9));
    "#;
    assert_eq!(same(src, 1), strs(&["2", "9"]));
}

#[test]
fn scoped_name_operators_match_off() {
    // read before the `var`, typeof, `++`, `+=`, `||=`, and a write from the closure
    let src = r#"
        function ops() {
            var before = t;
            var t = 3;
            var c = function () { t = t + 1; return t };
            return [typeof t, before === undefined, t++, (t += 1), c(), (t ||= 9), t].join(",");
        }
        console.log(ops());
    "#;
    assert_eq!(same(src, 1), strs(&["number,true,3,5,6,6,6"]));
}

#[test]
fn cross_unit_closure_matches_off() {
    // the closure is created by a compiled function of an earlier unit (another run id)
    same_units(
        &[
            "function mk() { var n = 0; return function () { return ++n } }",
            "var c = mk(); console.log('' + c() + c());",
        ],
        1,
    );
}

#[test]
fn closure_creation_and_prototype_match_off() {
    let src = r#"
        function make() { return function () { return 1 } }
        var n = 0;
        for (var i = 0; i < 1000; i++) { var f = make(); if (f !== make()) n++; }
        console.log("" + n);
        function mkn() { var k = 1; return function (a, b) { return k } }
        var C = mkn();
        console.log("" + (typeof C.prototype) + C.length + (C.name === "") + (new C() instanceof C));
    "#;
    assert_eq!(same(src, 1), strs(&["1000", "object2truetrue"]));
}

#[test]
fn throw_from_callback_matches_off() {
    let src = r#"
        function boom(k) {
            var g = function () { if (k > 0) throw new Error("bad " + k); return 0 };
            return g();
        }
        function caller() { try { boom(2) } catch (e) { return e.message } return "none" }
        console.log(caller());
    "#;
    assert_eq!(same(src, 1), strs(&["bad 2"]));
}

// ----- negative controls -----

#[test]
fn closure_naming_only_globals_has_no_scope() {
    // the closure names no declared name: the outer compiles, and no per-call scope is built
    let src = r#"
        function g() { var q = function () { return Math.max(1, 2) }; return q() }
        console.log("" + g());
        console.log("" + g());
    "#;
    assert_eq!(same(src, 1), strs(&["2", "2"]));
    let it = interp(&[src], CompileMode::On);
    assert_eq!(it.cstats.scope_frames(), 0);
    assert!(it.cstats.compiled() >= 1);
}

/// A refused shape: it matches `off` and the outer function records `reason`. A function with a rejection
/// reason is never compiled; the inner function on its own may well compile, so `compiled()` is not asserted.
fn refused(src: &str, reason: FallbackReason) {
    same(src, 0);
    let it = interp(&[src], CompileMode::On);
    assert!(
        it.cstats.rejected(reason) >= 1,
        "no {reason:?} rejection for: {src}"
    );
}

#[test]
fn refused_captured_for_let_stays_on_ast() {
    refused(
        "function f() { var fs = []; for (let i = 0; i < 2; i++) { fs.push(function () { return i }) } return fs.length } console.log('' + f());",
        FallbackReason::InnerFunction,
    );
}

#[test]
fn refused_captured_block_let_stays_on_ast() {
    refused(
        "function f() { var g; { let k = 1; g = function () { return k } } return g() } console.log('' + f());",
        FallbackReason::InnerFunction,
    );
}

#[test]
fn refused_captured_catch_param_stays_on_ast() {
    refused(
        "function f() { try { throw 1 } catch (e) { return function () { return e } } } console.log('' + f()());",
        FallbackReason::Try,
    );
}

#[test]
fn refused_captured_self_name_stays_on_ast() {
    refused(
        "function f(n) { return n ? 0 : function () { return f } } console.log('' + (f(0) === f));",
        FallbackReason::SelfBinding,
    );
}

#[test]
fn refused_inner_method_stays_on_ast() {
    refused(
        "function f() { var o = { m() { return 1 } }; return o.m() } console.log('' + f());",
        FallbackReason::ObjectMethod,
    );
}

#[test]
fn refused_inner_generator_stays_on_ast() {
    refused(
        "function f() { var g = function* () { yield 1 }; return g().next().value } console.log('' + f());",
        FallbackReason::InnerFunction,
    );
}

#[test]
fn refused_inner_async_stays_on_ast() {
    refused(
        "function f() { var g = async function () { return 1 }; return typeof g } console.log('' + f());",
        FallbackReason::InnerFunction,
    );
}

#[test]
fn refused_inner_class_stays_on_ast() {
    refused(
        "function f() { var C = class { m() { return 1 } }; return new C().m() } console.log('' + f());",
        FallbackReason::ClassBody,
    );
}

#[test]
fn refused_inner_declaration_stays_on_ast() {
    refused(
        "function f() { function g() { return 1 } return g() } console.log('' + f());",
        FallbackReason::InnerFunction,
    );
}
