//! Compiled `try`/`catch`/`finally`, `for-of` and `for-in`: programs run under `off` (the reference), `on` and
//! `check` with `compile_after = 1`; output must match and the functions must really be compiled.

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

#[test]
fn try_catch_finally_completions() {
    same(
        r#"
        function a(x) { try { if (x) throw new Error('boom'); return 'ok' } catch (e) { return 'caught ' + e.message } }
        function b(x) { try { return 'try' } finally { console.log('fin ' + x) } }
        function c() { try { return 1 } finally { return 2 } }
        function d() { try { throw 1 } catch (e) { throw 2 } finally { console.log('d fin') } }
        function e1() { for (let i = 0; i < 4; i++) { try { if (i == 1) continue; if (i == 3) break; console.log('body ' + i) } finally { console.log('f' + i) } } return 'done' }
        function f() { try { throw 1 } finally { return 'swallowed' } }
        function g() { try { try { throw new TypeError('in') } finally { console.log('inner fin') } } catch (e) { return e.name } }
        function h() { try { null.x } catch ({ message }) { return typeof message } }
        function i() { try { throw 5 } catch { return 'no param' } }
        function j() { let n = 0; try { n = 1; throw 0 } catch (e) { n += 10 } finally { n += 100 } return n }
        function k() { try { return (() => { throw 3 })() } catch (e) { return 'k' + e } }
        function l() { var r = []; for (const x of [1, 2]) { try { r.push(x); if (x == 1) continue } finally { r.push('f') } } return r.join() }
        function m() { try { throw new RangeError('x') } catch (e) { var e2 = e; return e2 instanceof RangeError } }
        console.log(a(0), a(1), b(1), c(), f(), g(), h(), i(), j(), k(), l(), m());
        try { d() } catch (e) { console.log('d threw ' + e) }
        console.log(e1());
        "#,
        8,
    );
}

#[test]
fn catch_shadowing_and_var() {
    same(
        r#"
        function a() { var e = 'outer'; try { throw 'inner' } catch (e) { e = 'changed' } return e }
        function b() { try { throw 'x' } catch (e) { var e = 'assigned'; } return typeof e }
        function c(e) { try { throw 'y' } catch (e) { } return e }
        function d() { let r = ''; try { throw 1 } catch (z) { let w = z + 1; r += w } try { throw 2 } catch (z) { let w = z + 2; r += w } return r }
        console.log(a(), b(), c('p'), d());
        "#,
        2,
    );
}

#[test]
fn for_of_arrays_strings_and_iterators() {
    let out = same(
        r#"
        function a(xs) { let s = 0; for (const x of xs) s += x; return s }
        function b(s) { const out = []; for (let ch of s) out.push(ch); return out.join('|') }
        function c(m) { const out = []; for (const [k, v] of m) out.push(k + '=' + v); return out.join() }
        function d(xs) { for (const x of xs) { if (x > 2) return 'found ' + x } return 'none' }
        function e1(xs) { let n = 0; for (var x of xs) { if (x == 2) continue; if (x == 4) break; n += x } return n + ':' + x }
        function f(o) { let r = ''; for (const { k, v = 'dv' } of o) r += k + v; return r }
        function g(xs) { let last; for (last of xs); return last }
        function* gen() { try { yield 1; yield 2; yield 3 } finally { console.log('gen closed') } }
        function h() { let n = 0; for (const x of gen()) { n += x; if (x == 2) break } return n }
        function i() { for (const x of gen()) { throw new Error('thrown ' + x) } }
        function j() { for (const x of gen()) { return 'ret ' + x } }
        function k(xs) { const out = []; for (const x of xs) { xs.length < 5 && xs.push(x + 10); out.push(x) } return out.join() }
        const custom = { [Symbol.iterator]() { let n = 0; return { next() { n++; return { done: n > 3, value: n } }, return() { console.log('ret called'); return {} } } } };
        function l(x) { let s = 0; for (const v of x) { s += v; if (s > 2) break } return s }
        console.log(a([1, 2, 3]), b('héy'), c(new Map([[1, 2], [3, 4]])), d([1, 2, 3, 4]), d([]), e1([1, 2, 3, 4, 5]), f([{ k: 'a' }, { k: 'b', v: 'x' }]), g([5, 6]));
        console.log(h()); try { i() } catch (e) { console.log(e.message) } console.log(j());
        console.log(k([1, 2]), l(custom), l([1, 1, 1]));
        for (const bad of [5, null, undefined, {}]) { try { a(bad) } catch (e) { console.log(e.name + ': ' + e.message) } }
        "#,
        10,
    );
    assert!(
        out[0].starts_with("6 h|é|y 1=2,3=4 found 3 none 4:4"),
        "{}",
        out[0]
    );
}

#[test]
fn for_in_keys_and_heads() {
    let out = same(
        r#"
        function a(o) { const ks = []; for (const k in o) ks.push(k); return ks.join() }
        function b(o) { let n = 0; for (var k in o) { if (k == 'b') continue; n++ } return n + k }
        function c(o) { let k; for (k in o) { if (k == 'a') break } return k }
        function d(o) { const r = []; for (const k in o) { delete o[k]; r.push(k) } return r.join() }
        function proto() { function P() { this.own = 1 } P.prototype.inh = 2; return a(new P()) }
        console.log(a({ a: 1, b: 2 }), a([7, 8]), a('xy'), a(null), a(undefined), a(5), b({ a: 1, b: 2, c: 3 }), c({ z: 1, a: 2 }), d({ p: 1, q: 2 }), proto());
        "#,
        4,
    );
    assert!(out[0].starts_with("a,b 0,1 0,1 "), "{}", out[0]);
}

/// User iterators are pulled lazily and closed on break, return and throw (the recorded Node output).
#[test]
fn lazy_iteration_matches_off() {
    same(
        include_str!("../../tests/differential/lazy_iteration.js"),
        1,
    );
}
