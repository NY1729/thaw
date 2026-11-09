//! Check-mode tests (plan TJ150 section 7, PR5). The oracle must pass on real pure kernels; the failpoint proves
//! it can fail; calls it must skip (object arguments, impure functions) run the compiled code alone.

use super::check::CHECK_FAILPOINT;
use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Off, On and Check must print the same lines, and On and Check must compile at least `min_compiled`.
fn same(src: &str, min_compiled: u64) -> Vec<String> {
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    assert!(!want.is_empty(), "reference printed nothing: {src}");
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

/// Turns the check failpoint on for the lifetime of the guard (it is thread-local; the guard resets it).
struct Failpoint;

impl Failpoint {
    fn on() -> Failpoint {
        CHECK_FAILPOINT.with(|c| c.set(true));
        Failpoint
    }
}

impl Drop for Failpoint {
    fn drop(&mut self) {
        CHECK_FAILPOINT.with(|c| c.set(false));
    }
}

#[test]
fn check_passes_a_blowfish_shaped_kernel_on_typed_and_plain_tables() {
    let out = same(
        r#"
        function encipher(S, l, r) {
          var xl = l, xr = r;
          for (var i = 0; i < 16; i = i + 2) {
            xl = xl ^ S[i & 15];
            xr = xr ^ ((((S[(xl >>> 24) & 255] + S[((xl >>> 16) & 255) + 256]) ^ S[((xl >>> 8) & 255) + 512]) + S[(xl & 255) + 768]) | 0);
            xr = xr ^ S[(i + 1) & 15];
            xl = xl ^ ((((S[(xr >>> 24) & 255] + S[((xr >>> 16) & 255) + 256]) ^ S[((xr >>> 8) & 255) + 512]) + S[(xr & 255) + 768]) | 0);
          }
          return [xl, xr];
        }
        var T = new Int32Array(1024);
        var A = [];
        for (var k = 0; k < 1024; k++) { T[k] = k * 40503 - 1000000; A.push(k * 40503 - 1000000); }
        var o = encipher(T, 19088743, -2);
        console.log(String(o[0]), String(o[1]));
        o = encipher(A, 5, 6);
        console.log(String(o[0]), String(o[1]));
        o = encipher(T, 19088743, -2);
        console.log(String(o[0]), String(o[1]));
        "#,
        1,
    );
    assert_eq!(out.len(), 3);
}

#[test]
fn check_sees_argument_writes_through_both_runs() {
    // the AST run writes its copy and the compiled run writes the real argument: both must end the same way
    let out = same(
        r#"
        function m(a) { a[0] = 9; a[1] += 2; return a[0] + a[1] }
        var x = [1, 2];
        var y = new Int32Array([4, 5]);
        console.log(String(m(x)), String(x[0]), String(x[1]));
        console.log(String(m(y)), String(y[0]), String(y[1]));
        console.log(String(m(x)), String(x[0]), String(x[1]));
        "#,
        1,
    );
    assert_eq!(out, lines(&["13 9 4", "16 9 7", "15 9 6"]));
}

#[test]
fn check_reports_a_mismatch_with_the_check_message() {
    let src = r#"
        function f(a) { return a + 1 }
        try { console.log(String(f(1))) } catch (e) { console.log(e.name + ": " + e.message) }
        try { console.log(String(f(2))) } catch (e) { console.log(e.name + ": " + e.message) }
        "#;
    let _fp = Failpoint::on();
    let (got, _, _) = run_mode(src, CompileMode::Check);
    assert_eq!(got.len(), 2);
    for line in &got {
        assert!(
            line.starts_with("Error: thaw-js compile check mismatch in f (")
                && line.ends_with("): result"),
            "{line}"
        );
    }
    // the failpoint lives in the check path only: the same program under `on` is unaffected
    let (on, ok, _) = run_mode(src, CompileMode::On);
    assert!(ok);
    assert_eq!(on, lines(&["2", "3"]));
}

#[test]
fn check_skips_calls_with_object_arguments() {
    let _fp = Failpoint::on();
    let (got, ok, it) = run_mode(
        r#"
        function h(a, o) { return a + 1 }
        console.log(String(h(1, {})));
        console.log(String(h(1, {})));
        "#,
        CompileMode::Check,
    );
    assert!(ok);
    assert_eq!(got, lines(&["2", "2"]));
    assert_eq!(it.cstats.compiled(), 1);
}

#[test]
fn check_runs_impure_functions_without_comparing() {
    let _fp = Failpoint::on();
    let out = same(
        r#"
        function k(a) { return a.x }
        console.log(String(k("abc")));
        console.log(String(k("abc")));
        "#,
        1,
    );
    assert_eq!(out, lines(&["undefined", "undefined"]));
}
