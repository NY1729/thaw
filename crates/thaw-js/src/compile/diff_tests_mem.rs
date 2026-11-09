//! Differential tests of the C2 PR4 subset (plan TJ150 section 7): member reads and writes, updates and
//! compound assignments on members, array and object literals. Each program runs under `off` (reference),
//! `on` and `check`; output and ok-ness must match, and the compiled-function count must reach the minimum.

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

/// Reads: in range, out of range, negative, fractional, string, NaN and -0 keys.
#[test]
fn pr4_array_reads_of_every_key_kind() {
    same(
        r#"
        function rd(a, i) { return a[i] }
        const arr = [10, 20, 30];
        console.log(String(rd(arr, 0)), String(rd(arr, 2)), String(rd(arr, 3)));
        console.log(String(rd(arr, -1)), String(rd(arr, 1.5)), String(rd(arr, "1")));
        console.log(String(rd(arr, NaN)), String(rd(arr, -0)), String(rd(arr, 4294967295)));
        console.log(String(rd(arr, 0)), String(rd(arr, 2)), String(rd(arr, 3)));
        "#,
        1,
    );
}

/// `a.length`, writes in range, at length (append), past length, and writes to `length` itself.
#[test]
fn pr4_array_length_and_writes_past_the_end() {
    same(
        r#"
        function len(a) { return a.length }
        function put2(a, i, v) { a[i] = v; return a.length }
        function setlen(a) { a.length = 1; return a.length }
        const q = [1, 2];
        console.log(String(len(q)), String(put2(q, 2, 'x')), String(put2(q, 5, 'y')));
        console.log(JSON.stringify(q));
        const z = [1, 2, 3];
        console.log(String(setlen(z)), JSON.stringify(z));
        console.log(String(len(q)), String(put2(q, 0, 'first')), JSON.stringify(q));
        "#,
        3,
    );
}

/// Holes: `[1,,3]` and `[,2]` read as undefined and print as null.
#[test]
fn pr4_array_literal_holes() {
    same(
        r#"
        function mk() { return [1, , 3] }
        function hole() { const h = [, 2]; return h[0] }
        console.log(JSON.stringify(mk()), String(mk()[1]), String(hole()));
        console.log(JSON.stringify(mk()), String(mk()[1]), String(hole()));
        "#,
        2,
    );
}

/// Typed arrays: wraparound and clamping come from `set`, the reads from `get_index`.
#[test]
fn pr4_typed_array_reads_and_writes() {
    same(
        r#"
        function setTA(t, i, v) { t[i] = v; return t[i] }
        const i32 = new Int32Array(4);
        const u8 = new Uint8Array(4);
        const f64 = new Float64Array(2);
        console.log(String(setTA(i32, 0, 2147483648)), String(setTA(i32, 1, -1.9)));
        console.log(String(setTA(u8, 0, 256)), String(setTA(u8, 1, -1)), String(setTA(u8, 2, 3.7)));
        console.log(String(setTA(f64, 0, 1 / 3)), String(setTA(i32, 9, 5)), String(setTA(i32, 3, "12")));
        console.log(JSON.stringify(Array.from(i32)), JSON.stringify(Array.from(u8)));
        console.log(JSON.stringify(Array.from(f64)));
        "#,
        1,
    );
}

/// Strings are read by index and length; out-of-range and negative indices are undefined.
#[test]
fn pr4_string_index_and_length() {
    same(
        r#"
        function sc(s, i) { return s[i] }
        function sl(s) { return s.length }
        console.log(String(sc("hey", 1)), String(sc("hey", 10)), String(sc("hey", -1)));
        console.log(String(sl("héllo")), String(sc("hey", 0)), String(sl("")));
        console.log(String(sc("hey", 2)), String(sl("hey")));
        "#,
        2,
    );
}

/// Errors raised by the shared helpers: the messages must be the same as the interpreter's.
#[test]
fn pr4_member_access_on_nullish_bases_throws_the_same_text() {
    same(
        r#"
        function tryit(f) { try { return String(f()) } catch (e) { return e.name + ': ' + e.message } }
        function rdp(o) { return o.x }
        function rdi(o) { return o[0] }
        function wrp(o) { o.x = 1; return 0 }
        function cmpd(o) { o.x += 1; return 0 }
        function nest(m) { return m.a.b[1] }
        console.log(tryit(() => rdp(null)), tryit(() => rdp(undefined)));
        console.log(tryit(() => rdi(null)), tryit(() => wrp(undefined)));
        console.log(tryit(() => cmpd(null)), tryit(() => nest({})));
        console.log(tryit(() => nest({ a: { b: [0, 'x'] } })));
        console.log(tryit(() => rdp(null)), tryit(() => nest({})));
        "#,
        5,
    );
}

/// Frozen and non-extensible targets: the same silent ignore (or throw) as the interpreter.
#[test]
fn pr4_frozen_and_non_extensible_writes() {
    same(
        r#"
        function wf(a, i) { a[i] = 9; return a[i] }
        function wo(o) { o.x = 2; return o.x }
        const fz = Object.freeze([1, 2]);
        const ne = Object.preventExtensions([7]);
        const fo = Object.freeze({ x: 1 });
        console.log(String(wf(fz, 0)), JSON.stringify(fz), String(wf(ne, 5)), JSON.stringify(ne));
        console.log(String(wo(fo)), String(wo(fo)), String(wf(fz, 0)));
        "#,
        2,
    );
}

/// Primitive bases: a property write on a number or string goes through `set` as before.
#[test]
fn pr4_property_access_on_primitive_bases() {
    same(
        r#"
        function tryit(f) { try { return String(f()) } catch (e) { return e.name + ': ' + e.message } }
        function prim(s) { s.x = 1; return s.x }
        function prim2(s) { return s.toFixedX }
        console.log(tryit(() => prim(5)), tryit(() => prim("abc")));
        console.log(tryit(() => prim2(true)), tryit(() => prim(5)));
        "#,
        2,
    );
}

/// Object literals: key order, computed keys, duplicate keys, shorthand, string and numeric keys.
#[test]
fn pr4_object_literal_key_order_and_computed_keys() {
    same(
        r#"
        function mkobj(k, v) { return { b: 1, a: v, [k]: 2, 3: 'three', 'q r': 4, b: 5 } }
        function sh(x, y) { return { x, y, x2: x } }
        const ob = mkobj('zz', 'A');
        console.log(JSON.stringify(ob), Object.keys(ob).join(','));
        console.log(JSON.stringify(mkobj(1, 'B')), Object.keys(mkobj(1, 'B')).join(','));
        console.log(JSON.stringify(sh(1, 'two')), JSON.stringify(sh(undefined, null)));
        "#,
        2,
    );
}

/// Compound, logical and update forms on array elements and properties.
#[test]
fn pr4_compound_logical_and_update_on_members() {
    same(
        r#"
        function bump(o, i) { o[i] += 2; o.n++; ++o.n; o.p ||= 7; o.q &&= 8; o.r ??= 9; return o[i] }
        function post(o, i) { const a = o[i]++; const b = ++o[i]; return [a, b, o[i]] }
        function bi(o) { const a = o.x++; return a + ':' + o.x }
        function asg(o) { return o.x = 5 }
        function asg2(a) { return a[0] += 1 }
        const ba = [1, 2, 3];
        console.log(String(bump(ba, 1)), JSON.stringify(ba));
        const bo = { n: 0, p: 0, q: 1, r: null };
        console.log(String(bump(bo, 0)), JSON.stringify(bo));
        console.log(JSON.stringify(post([5, 6], 0)), JSON.stringify(post({ x: '7' }, 'x')));
        console.log(bi({ x: 1n }), String(asg({})), String(asg2([4])));
        console.log(bi({ x: 1n }), String(asg({})), String(asg2([4])));
        "#,
        5,
    );
}

/// Numeric and string keys name the same property.
#[test]
fn pr4_numeric_and_string_keys_are_the_same_property() {
    same(
        r#"
        function kk(o) { o[1] = 'a'; o["1"] += 'b'; return o[1] }
        const k1 = {};
        const k2 = { "1": 'z' };
        console.log(kk(k1), Object.keys(k1).join(), kk(k2), Object.keys(k2).join());
        console.log(kk({}), kk({ "1": 'z' }));
        "#,
        1,
    );
}

/// Evaluation order: object, then key, then right-hand side (and read-modify-write reads before the right side).
#[test]
fn pr4_evaluation_order_object_key_then_value() {
    same(
        r#"
        var trace = '';
        function ord(o) { (trace += 'o', o)[(trace += 'k', 'p')] = (trace += 'r', 1); return trace }
        function ord2(o) { (trace += 'o', o)[(trace += 'k', 0)] += (trace += 'r', 1); return trace }
        console.log(ord({}));
        console.log(ord({}));
        console.log(ord2([10]));
        console.log(ord2([10]));
        "#,
        2,
    );
}

/// Getters and setters on an object passed in are run by `get`/`set`.
#[test]
fn pr4_accessors_on_an_argument_object() {
    same(
        r#"
        var log = [];
        const obj = { get x() { return 5 }, set x(v) { log.push(v) } };
        function gs(o) { o.x = 3; return o.x }
        console.log(String(gs(obj)), JSON.stringify(log));
        console.log(String(gs(obj)), JSON.stringify(log));
        "#,
        1,
    );
}

/// A Proxy argument: the traps run through `get`/`set`.
#[test]
fn pr4_proxy_argument_runs_its_traps() {
    same(
        r#"
        const px = new Proxy({ a: 1 }, {
            get(t, k) { return 'P' + String(k) },
            set(t, k, v) { t[k] = v * 2; return true }
        });
        function pg(o) { o.z = 4; return o.a }
        console.log(String(pg(px)));
        console.log(String(pg(px)));
        "#,
        1,
    );
}

/// A function that calls something stays on the AST (no compile), with the same output.
#[test]
fn pr4_member_call_stays_on_the_ast() {
    same(
        r#"
        function callsOut(o) { return o.toString() }
        console.log(String(callsOut([1, 2])), String(callsOut({})));
        console.log(String(callsOut([1, 2])), String(callsOut({})));
        "#,
        0,
    );
}
