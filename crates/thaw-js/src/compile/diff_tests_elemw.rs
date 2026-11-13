//! Tests of the C4 PR3 element writes (plan TJ153 section 3.4): `slot[pure key] = v`, compound, logical and update
//! forms through `elem.rs`. Programs run under Off (the reference), On and Check; output and ok-ness must match.
//! The store counters prove the plain-store fast path ran for a Number into an in-range element of an Array or a
//! typed array, and stayed off for holes, growth, frozen / non-extensible / length-readonly arrays, non-Number
//! values, non-index keys, proxies and non-object bases.

use super::*;
use crate::Interp;

fn run(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Off, On and Check print the same lines and agree on ok-ness; On compiles at least `min` functions.
fn same(src: &str, min: u64) -> Vec<String> {
    let (want, ok_want, _) = run(src, CompileMode::Off);
    assert!(!want.is_empty(), "reference printed nothing: {src}");
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, it) = run(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            it.cstats.compiled() >= min,
            "{mode:?}: compiled {} < {min}: {src}",
            it.cstats.compiled()
        );
    }
    want
}

/// Every element kind over the corner keys and the corner values, written and then shown.
#[test]
fn elemw_corner_table_writes_match_off() {
    let src = r#"
function wr(s, k, v) { s[k] = v; return s; }
function show(s) { let r = ""; for (let i = 0; i < s.length; i++) { r = r + String(s[i]) + ";"; } return r; }
const keys = [0, 1, 2, -1, -0, 0.5, 1.5, NaN, Infinity, 4294967296,
  "0", "1", "01", "1.5", "-0", "Infinity", true, null, undefined, { valueOf() { return 1; } }];
const vals = [0, -1, 255, 256, 1.5, -1.5, 2147483648, 4294967296, 9007199254740992, NaN, Infinity,
  -Infinity, -0, 300, 2.5, 3.5, "7", true, null, undefined, { valueOf() { return 6; } }];
const makers = [
  () => [1, 2, 3],
  () => new Int8Array(3),
  () => new Uint8Array(3),
  () => new Uint8ClampedArray(3),
  () => new Int16Array(3),
  () => new Uint16Array(3),
  () => new Int32Array(3),
  () => new Uint32Array(3),
  () => new Float32Array(3),
  () => new Float64Array(3),
  () => [1, , 3],
];
for (const mk of makers) {
  for (const k of keys) {
    for (const v of vals) {
      try { console.log(show(wr(mk(), k, v))); } catch (e) { console.log("E " + e.message); }
    }
  }
}
// index 2**31 and 2**32-1 only on typed arrays: a plain Array write there grows its dense storage (Off too)
for (const mk of makers.slice(1, 10)) {
  for (const k of [2147483648, 4294967295]) {
    for (const v of vals) {
      try { console.log(show(wr(mk(), k, v))); } catch (e) { console.log("E " + e.message); }
    }
  }
}
"#;
    same(src, 1);
    let (_, _, it) = run(src, CompileMode::On);
    assert!(it.cstats.elem_store_fast() > 0, "fast stores ran");
    assert!(it.cstats.elem_store_slow() > 0, "helper stores ran");
}

/// Frozen, sealed and non-extensible arrays, a read-only length, an index accessor, a Proxy with a set trap, holes,
/// growth, a string, a number, `undefined` and `null` bases, a key with `valueOf` that runs before the value.
#[test]
fn elemw_arrays_and_bases_match_off() {
    same(
        r#"
function wr(s, k, v) { s[k] = v; return s; }
function show(s) { let r = ""; for (let i = 0; i < s.length; i++) { r = r + String(s[i]) + ";"; } return r; }
const log = [];
const fz = Object.freeze([4, 5]);
const sealed = Object.seal([6, 7]);
const ne = Object.preventExtensions([8, 9]);
const ro = [1, 2];
Object.defineProperty(ro, "length", { writable: false });
const acc = [1, 2, 3];
Object.defineProperty(acc, 1, { set(v) { log.push("set " + v); } });
const px = new Proxy([1, 2, 3], { set(t, k, v, r) { log.push("trap " + String(k)); return Reflect.set(t, k, v, r); } });
const holes = [1, , 3];
const z = [10, 20];
const key = { valueOf() { log.push("key"); return 1; } };
function wk(s, k, v) { s[k] = (log.push("rhs"), v); return s; }
try { console.log(show(wr(fz, 0, 9)), show(wr(sealed, 0, 9)), show(wr(ne, 2, 9)), show(wr(ro, 0, 9)), show(wr(ro, 2, 9))); } catch (e) { console.log("E " + e.message); }
try { console.log(show(wr(acc, 1, 7)), log.join(",")); } catch (e) { console.log("E " + e.message); }
console.log(show(wr(holes, 1, 5)), show(wr(holes, 2, 6)));
console.log(show(wr(z, 5, 1)), z.length, show(wr(z, 1.5, 3)), show(wr(z, -1, 3)), show(wr(z, "1", 4)), show(wr(z, "x", 4)));
try { console.log(show(wr(px, 1, 8)), log.join(",")); } catch (e) { console.log("E " + e.message); }
try { wr(undefined, 0, 1); } catch (e) { console.log("E " + e.message); }
try { wr(null, 0, 1); } catch (e) { console.log("E " + e.message); }
console.log(String(wr("abc", 1, "x")), String(wr(5, 0, 1)));
try { wk(z, key, 4); console.log(show(z), log.join(",")); } catch (e) { console.log("E " + e.message); }
Array.prototype[5] = "proto";
console.log(show(wr([0, 1], 5, 2)), show(wr([0, 1], 3, 2)));
"#,
        1,
    );
}

/// Compound, update, logical assignments and the order of the key, the current value, the right-hand side and the
/// store, over Arrays, typed arrays, BigInt elements, holes and strings. The base is captured before the key and the
/// right-hand side, so reassigning the base in the right-hand side or the key changes nothing.
#[test]
fn elemw_compound_update_and_order_match_off() {
    same(
        r#"
function show(s) { let r = ""; for (let i = 0; i < s.length; i++) { r = r + String(s[i]) + ";"; } return r; }
function ca(a, i, x) { a[i] ^= x; return a; }
function cp(a, i, x) { a[i] += x; return a; }
function cm(a, i) { a[i] -= 1; return a; }
function up(a, i) { return a[i]++; }
function pu(a, i) { return ++a[i]; }
function dn(a, i) { return a[i]--; }
function pd(a, i) { return --a[i]; }
function la(a, i, x) { a[i] ||= x; return a; }
function lb(a, i, x) { a[i] &&= x; return a; }
function ln(a, i, x) { a[i] ??= x; return a; }
function dbl(a, i) { a[i] += a[i]; return a; }
function rd(a, i) { a[i] = a[i] + (a[i] = 40); return a; }
function sw(a, i) { a[i] += (a[i] = 5); return a; }
function rb(a, i) { a[i] = (a = [9, 9], 4); return a; }
function rk(a, i) { a[(a = [7, 7], 0)] = 1; return a; }
const log = [];
const key = { valueOf() { log.push("key"); return 0; } };
function ord1(a, k) { a[k] += (log.push("rhs"), 2); return a; }
function ord2(a, k) { a[k] = (log.push("rhs"), 2); return a; }
function ord3(a, k) { a[k]++; return a; }
const g = { get v() { log.push("get"); return 2; } };
function gs(a, i) { a[i] = g.v; a[i] += g.v; return a; }
const makers = [
  () => [1, 2, 3],
  () => new Uint8Array([255, 1, 7]),
  () => new Float32Array([0.5, 2, 3]),
  () => new Int32Array([7, -1, 2]),
  () => [1, , 3],
  () => [1n, 2n, 3n],
  () => ["a", "b", null],
];
const ops = [ca, cp, cm, up, pu, dn, pd, la, lb, ln, dbl, rd, sw, rb, rk, gs];
for (const mk of makers) {
  for (const i of [0, 1, 2, 5]) {
    for (const op of ops) {
      const a = mk();
      try { const r = op(a, i, 3); console.log(r === a ? "self" : String(r), show(a)); } catch (e) { console.log("E " + e.message, show(a)); }
    }
  }
}
const oa = [1, 2];
ord1(oa, key); ord2(oa, key); ord3(oa, key);
console.log(show(oa), log.join(","));
const tb = new Uint8Array([255, 0]);
console.log(String(up(tb, 0)), String(pu(tb, 0)), String(dn(tb, 1)), String(pd(tb, 1)), show(tb));
try { console.log(show(ca([1n, 2n], 0, 1))); } catch (e) { console.log("E " + e.message); }
try { console.log(show(cp([1, 2], 0, 1n))); } catch (e) { console.log("E " + e.message); }
"#,
        1,
    );
}

/// The bit kernel of a blowfish round over an Int32Array and over a plain Array (holes first, then filled), with
/// element stores and compounds in the loop and seeded inputs. Off, On and Check agree.
#[test]
fn elemw_blowfish_kernel_matches_off_over_seeded_inputs() {
    let src = r#"
let seed = 12345;
function rnd() { seed = (seed * 1103515245 + 12345) % 2147483648; return seed; }
const S = new Int32Array(1024);
const A = new Array(1024);
for (let i = 0; i < 1024; i++) { const v = rnd() | 0; S[i] = v; A[i] = v; }
function F(T, x) {
  return (((T[x >>> 24] + T[256 + ((x >>> 16) & 255)]) ^ T[512 + ((x >>> 8) & 255)]) + T[768 + (x & 255)]) | 0;
}
function enc(T, l, r) {
  for (let i = 0; i < 16; i++) { l = l ^ T[i]; r = r ^ F(T, l); const t = l; l = r; r = t; }
  return [l, r];
}
function upd(T, i, l) { T[i] ^= l; T[i + 1] += 3; T[i + 2]++; T[i + 3] = T[i + 3] + 1; return T[i]; }
let out = 0;
for (let n = 0; n < 1000; n++) {
  const p = enc(S, rnd() | 0, rnd() | 0);
  const q = enc(A, p[0], p[1]);
  out = (out + q[0] + q[1]) | 0;
  upd(S, n & 1000, q[0]);
  upd(A, n & 1000, q[1]);
}
let cs = 0;
for (let i = 0; i < 1024; i++) { cs = (cs + S[i] + (A[i] | 0)) | 0; }
console.log(String(out), String(cs));
"#;
    same(src, 3);
    let (_, _, it) = run(src, CompileMode::On);
    assert!(it.cstats.elem_store_fast() > 0);
}

/// Counters, one store per case: the plain-store fast path runs for a Number into an in-range element of an Array,
/// a typed array, and the same with a Number-valued key; every other case takes `set_index` / `set`. The program is
/// one call of `w`, so each case is one Interp per mode.
#[test]
fn elemw_store_counters_and_negative_controls() {
    // (base expression, key expression, value expression, fast path expected)
    let cases: &[(&str, &str, &str, bool)] = &[
        ("new Int32Array([5, 6, 7])", "1", "9", true),
        ("new Uint8Array([1, 2, 3])", "0", "300", true),
        ("new Float64Array([0.5, 1])", "1", "2.5", true),
        ("new Uint8ClampedArray([1, 2])", "1", "2.5", true),
        ("[1, 2, 3]", "2", "7", true),
        ("[1, 2, 3]", "-0", "7", true),
        ("[null, 2]", "0", "7", true),
        ("[1, , 3]", "1", "7", false),
        ("[1, 2, 3]", "3", "7", false),
        ("[1, 2, 3]", "10", "7", false),
        ("new Int32Array(2)", "5", "1", false),
        ("[1, 2, 3]", "1.5", "7", false),
        ("[1, 2, 3]", "-1", "7", false),
        ("[1, 2, 3]", "NaN", "7", false),
        ("[1, 2, 3]", "\"1\"", "7", false),
        ("[1, 2, 3]", "2 ** 32", "7", false),
        ("[1, 2, 3]", "1", "\"x\"", false),
        ("[1, 2, 3]", "1", "2n", false),
        ("[1, 2, 3]", "1", "undefined", false),
        ("[1, 2, 3]", "1", "{ valueOf() { return 4; } }", false),
        ("Object.freeze([1, 2, 3])", "1", "7", false),
        ("Object.preventExtensions([1, 2, 3])", "1", "7", false),
        ("Object.seal([1, 2, 3])", "1", "7", false),
        (
            "(() => { const a = [1, 2, 3]; Object.defineProperty(a, \"length\", { writable: false }); return a; })()",
            "1",
            "7",
            false,
        ),
        ("new Proxy([1, 2, 3], {})", "1", "7", false),
        ("\"abc\"", "1", "7", false),
        ("5", "0", "7", false),
        ("undefined", "0", "7", false),
        ("null", "0", "7", false),
        // a Number store throws ToBigInt's TypeError on the slow path; a BigInt store is slow too (no f64 form)
        ("new BigInt64Array(2)", "1", "7", false),
        ("new BigUint64Array(2)", "1", "7n", false),
    ];
    for &(base, key, val, fast) in cases {
        let src = format!(
            "function w(s, k, v) {{ s[k] = v; return 0; }}\n\
             function show(s) {{ let r = \"\"; for (let i = 0; i < 3; i++) {{ r = r + String(s[i]) + \";\"; }} return r; }}\n\
             const s = {base};\n\
             try {{ console.log(String(w(s, {key}, {val})), show(s)); }} catch (e) {{ console.log(\"E \" + e.message); }}\n"
        );
        let (want, ok_want, _) = run(&src, CompileMode::Off);
        let (got, ok, it) = run(&src, CompileMode::On);
        assert_eq!(got, want, "{base} [{key}] = {val}");
        assert_eq!(ok, ok_want, "{base} [{key}] = {val}");
        assert!(it.cstats.elem_built() >= 1, "{base} [{key}]: a gated store");
        let (f, s) = (it.cstats.elem_store_fast(), it.cstats.elem_store_slow());
        if fast {
            assert_eq!(
                (f, s),
                (1, 0),
                "{base} [{key}] = {val}: fast store expected"
            );
        } else {
            assert_eq!(
                (f, s),
                (0, 1),
                "{base} [{key}] = {val}: helper store expected"
            );
        }
    }
}

/// Compound stores: `a[i] += 1` and `a[i]++` on an Int32Array are two plain stores per call; on a hole the first
/// store grows the cell through the helper and the update that follows is a plain store.
#[test]
fn elemw_compound_store_counts() {
    let src = "function cp(a, i) { a[i] += 1; a[i]++; return 0; }\n\
               const t = new Int32Array([1, 2, 3]);\n\
               const u = [1, , 3];\n\
               console.log(String(cp(t, 1)), String(t[1]));\n\
               console.log(String(cp(u, 1)), String(u[1]));\n";
    same(src, 1);
    let (_, ok, it) = run(src, CompileMode::On);
    assert!(ok);
    assert_eq!(
        it.cstats.elem_store_fast(),
        3,
        "two typed stores and one array store"
    );
    assert_eq!(it.cstats.elem_store_slow(), 1, "the hole store");
}
