//! Tests of the C4 PR2 element read (plan TJ153 section 3.2): `slot[pure key]` through `elem.rs`. Programs run under
//! Off (the reference), On and Check; output and ok-ness must match. The counters prove the Number-element fast
//! path ran for an Array or a typed array, and that it stays off for holes, out-of-range and non-index keys,
//! proxies, getters, strings, `null` elements and `undefined` bases. The gate is checked through `elem_built`.

use super::*;
use crate::Interp;

fn run(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

/// Off, On and Check print the same lines, agree on ok-ness, and On compiles at least `min` functions.
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

/// Every element kind over the corner keys, read through a member read, a bitwise node and an addition.
#[test]
fn elem_corner_table_reads_match_off() {
    same(
        r#"
function rd(s, k) { return s[k]; }
function bx(s, k) { return s[k] ^ 0; }
function sum(s, k) { let t = 0; t = t + s[k]; return t; }
const keys = [0, 1, 2, 3, -1, -0, 0.5, 1.5, NaN, Infinity, 2147483648, 4294967295, 4294967296,
  "0", "01", "1.5", "-0", "Infinity", true, null, undefined, { valueOf() { return 1; } }];
const makers = [
  () => [1, 2, 3],
  () => new Int8Array([1, -2, 3]),
  () => new Uint8Array([255, 2, 3]),
  () => new Uint8ClampedArray([300, 2.5, 3]),
  () => new Int16Array([-5, 6, 7]),
  () => new Uint16Array([65535, 6, 7]),
  () => new Int32Array([-1, 2147483647, 3]),
  () => new Uint32Array([4294967295, 2, 3]),
  () => new Float32Array([1.5, 0.1, 3]),
  () => new Float64Array([1.5, NaN, Infinity]),
  () => [1, , 3],
  () => [null, undefined, "x"],
  () => new BigInt64Array([1n, 2n]),
];
for (const mk of makers) {
  for (const k of keys) {
    for (const r of [rd, bx, sum]) {
      try { console.log(String(r(mk(), k))); } catch (e) { console.log("E " + e.message); }
    }
  }
}
"#,
        3,
    );
}

/// Holes, frozen, sealed and non-extensible arrays, a read-only length, a getter on an index, an inherited
/// index, a Proxy, a string, a number, `undefined` and `null` bases, and a key with `valueOf`.
#[test]
fn elem_holes_frozen_proxies_and_bases_match_off() {
    same(
        r#"
function rd(s, k) { return s[k]; }
function sum(s, k) { let t = 0; t = t + s[k]; return t; }
const log = [];
const fz = Object.freeze([4, 5]);
const sealed = Object.seal([6, 7]);
const ne = Object.preventExtensions([8, 9]);
const ro = [1, 2];
Object.defineProperty(ro, "length", { writable: false });
const idx = [1, 2, 3];
Object.defineProperty(idx, 1, { get() { return 99; } });
const holes = [1, , 3];
const z = [10, 20];
const px = new Proxy([1, 2, 3], {
  get(t, k, r) { log.push("get " + String(k)); return Reflect.get(t, k, r); },
});
const key = { valueOf() { log.push("valueOf"); return 1; } };
console.log(String(rd(fz, 1)), String(rd(sealed, 0)), String(rd(ne, 1)), String(rd(ro, 0)));
console.log(String(rd(idx, 1)), String(rd(holes, 1)), String(rd(holes, 2)), String(rd(z, -0)));
console.log(String(rd(px, 1)), log.join(","));
console.log(String(rd("abc", 1)), String(rd(5, 0)), String(rd(z, key)), log.join(","));
console.log(String(sum(z, 1)), String(sum(holes, 1)), String(sum(idx, 1)));
Array.prototype[5] = "proto";
console.log(String(rd([0, 1], 5)));
try { rd(undefined, 0); } catch (e) { console.log("E " + e.message); }
try { rd(null, 0); } catch (e) { console.log("E " + e.message); }
try { rd(undefined, key); } catch (e) { console.log("E " + e.message, log.join(",")); }
"#,
        2,
    );
}

/// The gate refuses keys that can write a slot (`s[(s = x, 0)]`, `s[i++]`): the program is the same as Off, and no
/// element node is built for them.
#[test]
fn elem_gate_refuses_writing_keys_and_keeps_the_order() {
    let src = r#"
function g1(s, i) { return s[(i = 1, i)]; }
function g2(s, i) { return s[i++]; }
function g3(s, i) { return s[(s = [9, 8], 0)]; }
function g4(s, i) { return s[(i += 1)]; }
function g5(s, i) { return s[i = 2]; }
const a = [1, 2, 3];
let i = 0;
console.log(String(g1(a, 0)), String(g2(a, 0)), String(g3(a, 0)), String(g4(a, 0)), String(g5(a, 0)));
const b = [5, 6, 7];
console.log(String(g1(b, 0)), String(g3(b, 1)), String(g4(b, 0)), String(g5(b, 0)));
"#;
    let want = same(src, 5);
    let (_, ok, it) = run(src, CompileMode::On);
    assert!(ok);
    assert_eq!(
        it.cstats.elem_built(),
        0,
        "no writing key may build an element node"
    );
    assert_eq!(it.cstats.elem_fast(), 0);
    assert!(!want.is_empty());
}

/// Evaluation order: a key that reads a variable changed by an earlier statement, a compound read of the same
/// element, reassignment of the base after it is read, and BigInt operands of a bitwise node.
#[test]
fn elem_order_compound_and_bigint_operands_match_off() {
    same(
        r#"
function o1(s, t) { t = 2; return s[t]; }
function o2(a, n) { let r = a[0] + a[n]; n = 1; return r + a[n]; }
function o3(a, n) { let r = a[n] + (a[n] = 40); return r; }
function o4(a) { return a[0] ^ a[1]; }
function o5(a, n) { n += (n = 5); return a[n] + a[0]; }
function o6(a) { return a[0] | a[1] & a[2]; }
const arr = [3, 4, 5];
const big = [1n, 2n];
const obj = { 0: "zero", 1: "one" };
console.log(String(o1(arr, 0)), String(o2(arr, 1)), String(o3([1, 2], 1)), String(o5(arr, 1)));
console.log(String(o6(arr)), String(o1(obj, 1)), String(o2(obj, 0)));
for (const f of [o4, o6]) {
  try { console.log(String(f(big))); } catch (e) { console.log("E " + e.message); }
  try { console.log(String(f([1, 2n, 3]))); } catch (e) { console.log("E " + e.message); }
}
console.log(String(o4([7, 2])), String(o1(arr, 2)));
"#,
        5,
    );
}

/// Counters: a Number element of an Array or a typed array takes the fast path, and every other key or base takes
/// the helper. The program is one call of `get`, so each case is one Interp.
#[test]
fn elem_counters_and_negative_controls() {
    // (base expression, key expression, fast path expected)
    let cases: &[(&str, &str, bool)] = &[
        ("new Int32Array([5, 6, 7])", "1", true),
        ("[1, 2, 3]", "2", true),
        ("[1, 2, 3]", "-0", true),
        ("new Float64Array([0.5])", "0", true),
        ("[1, , 3]", "1", false),
        ("[1, 2, 3]", "7", false),
        ("new Int32Array(2)", "5", false),
        ("[1, 2, 3]", "1.5", false),
        ("[1, 2, 3]", "-1", false),
        ("[1, 2, 3]", "NaN", false),
        ("[1, 2, 3]", "\"1\"", false),
        ("[1, 2, 3]", "2 ** 32", false),
        ("undefined", "0", false),
        ("new Proxy([1, 2, 3], {})", "1", false),
        // thaw-js ignores an accessor defined on an array index (Node answers 9, the interpreter answers 2), so
        // the compiled read agrees with the interpreter on the stored element. Not a compiler divergence.
        (
            "Object.defineProperty([1, 2, 3], 1, { get() { return 9; } })",
            "1",
            true,
        ),
        ("\"abc\"", "1", false),
        ("5", "0", false),
        ("[null]", "0", false),
    ];
    for &(base, key, fast) in cases {
        let src = format!(
            "function get(s, k) {{ return s[k]; }}\nconst s = {base};\nconsole.log(String(get(s, {key})));\n"
        );
        let (want, ok_want, _) = run(&src, CompileMode::Off);
        let (got, ok, it) = run(&src, CompileMode::On);
        assert_eq!(got, want, "{base} [{key}]");
        assert_eq!(ok, ok_want, "{base} [{key}]");
        assert_eq!(it.cstats.elem_built(), 1, "{base} [{key}]: one gated read");
        let (f, s) = (it.cstats.elem_fast(), it.cstats.elem_slow());
        if fast {
            assert_eq!((f, s), (1, 0), "{base} [{key}]: fast path expected");
        } else {
            assert_eq!((f, s), (0, 1), "{base} [{key}]: helper expected");
        }
    }
}

/// The hot kind: a loop over an Int32Array and over a plain Array reads each element on the fast path.
#[test]
fn elem_fast_path_runs_per_iteration_for_hot_kinds() {
    for base in ["new Int32Array([1, 2, 3])", "[1, 2, 3]"] {
        let src = format!(
            "function sum(a, n) {{ let t = 0; let i = 0; while (i < n) {{ t = t + a[i]; i = i + 1; }} return t; }}\n\
             console.log(String(sum({base}, 3)));\n"
        );
        let (want, ok_want, _) = run(&src, CompileMode::Off);
        let (got, ok, it) = run(&src, CompileMode::On);
        assert_eq!((got.clone(), ok), (want, ok_want), "{base}");
        assert_eq!(got, vec!["6".to_string()], "{base}");
        assert!(
            it.cstats.elem_fast() >= 3,
            "{base}: {}",
            it.cstats.elem_fast()
        );
        assert_eq!(it.cstats.elem_slow(), 0, "{base}");
    }
}

/// A blowfish-shaped round over an Int32Array and over a plain Array with seeded inputs: Off, On and Check agree.
#[test]
fn elem_blowfish_round_matches_off_over_seeded_inputs() {
    same(
        r#"
let seed = 12345;
function rnd() { seed = (seed * 1103515245 + 12345) % 2147483648; return seed; }
const S = new Int32Array(1024);
const A = new Array(1024);
for (let i = 0; i < 1024; i++) { const v = rnd() | 0; S[i] = v; A[i] = v; }
function F(S, x) {
  return (((S[x >>> 24] + S[256 + ((x >>> 16) & 255)]) ^ S[512 + ((x >>> 8) & 255)]) + S[768 + (x & 255)]) | 0;
}
function rounds(S, l, r) {
  for (let i = 0; i < 16; i++) { l = l ^ S[i]; r = r ^ F(S, l); const t = l; l = r; r = t; }
  return [l, r];
}
let out = 0;
for (let n = 0; n < 1000; n++) {
  const p = rounds(S, rnd() | 0, rnd() | 0);
  const q = rounds(A, p[0], p[1]);
  out = (out + q[0] + q[1]) | 0;
}
console.log(String(out));
"#,
        3,
    );
}
