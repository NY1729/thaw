//! Tests of the C2 PR6 numeric nodes (`num.rs`): the operator closures against `binary` over a corner table,
//! and differential programs (Off, On, Check) for the fused shapes. The fused shapes use only the public
//! interpreter surface, so the test file stays a sibling of the existing differential tests.

use super::num::{bit_nm, spec, Leaf, Side};
use super::*;
use crate::{bigint, Interp, Scope, Value};
use thaw_parser::ast as a;

type B = a::BinaryOp;

const BLOWFISH_LEAF: &str = include_str!("../../../../benchmarks/jsengine/blowfish_leaf.js");

fn run(src: &str, mode: CompileMode) -> (Vec<String>, bool, u64) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it.cstats.compiled())
}

/// Off, On and Check must print the same lines and agree on ok-ness; On must compile at least `min` functions.
fn same(src: &str, min: u64) -> Vec<String> {
    let (want, ok_want, _) = run(src, CompileMode::Off);
    assert!(!want.is_empty(), "reference printed nothing: {src}");
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, compiled) = run(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            compiled >= min,
            "{mode:?}: compiled {compiled} < {min}: {src}"
        );
    }
    want
}

/// `Object.is`-style equality for the values the corner table produces.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits()
        }
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::BigInt(x), Value::BigInt(y)) => **x == **y,
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
        _ => false,
    }
}

fn corner_values() -> Vec<Value> {
    let nums = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -1.5,
        2147483647.0,
        2147483648.0,
        -2147483648.0,
        -2147483649.0,
        4294967295.0,
        4294967296.0,
        4294967297.0,
        1e19,
        -1e19,
        9007199254740993.0,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e10,
        31.0,
        32.0,
        33.0,
    ];
    let mut v: Vec<Value> = nums.iter().map(|&n| Value::Number(n)).collect();
    v.push(Value::str("5"));
    v.push(Value::str("abc"));
    v.push(Value::str(""));
    v.push(Value::Bool(true));
    v.push(Value::Bool(false));
    v.push(Value::Null);
    v.push(Value::Undefined);
    v.push(bigint::big(num_bigint::BigInt::from(3)));
    v
}

#[test]
fn pr6_operator_nodes_match_binary_over_the_corner_table() {
    // one realm for the whole table: every `binary` call is the reference
    let mut it = Interp::with_compile_mode(CompileMode::Off);
    let mut f = Frame {
        slots: Vec::new(),
        scope: Scope::new(None),
        this: Value::Undefined,
        ret: Value::Undefined,
    };
    let vals = corner_values();
    let spec_ops = [
        B::Add,
        B::Sub,
        B::Mul,
        B::Div,
        B::Mod,
        B::Lt,
        B::LtEq,
        B::Gt,
        B::GtEq,
        B::EqEqEq,
        B::NotEqEq,
        B::EqEq,
        B::NotEq,
    ];
    for op in spec_ops {
        for (xi, x) in vals.iter().enumerate() {
            for (yi, y) in vals.iter().enumerate() {
                let want = it.binary(op, x, y);
                let ex = spec(op, Leaf::Const(x.clone()), Leaf::Const(y.clone()));
                let got = super::unbox(ex(&mut it, &mut f));
                match (want, got) {
                    (Ok(w), Ok(g)) => assert!(same_value(&w, &g), "{op:?} at {xi},{yi}"),
                    (Err(_), Err(_)) => {}
                    (w, g) => panic!("{op:?}: reference {:?} vs node {:?}", w.is_ok(), g.is_ok()),
                }
            }
        }
    }
    let bit_ops = [
        B::BitAnd,
        B::BitOr,
        B::BitXor,
        B::LShift,
        B::RShift,
        B::ZeroFillRShift,
    ];
    for op in bit_ops {
        for (xi, x) in vals.iter().enumerate() {
            for (yi, y) in vals.iter().enumerate() {
                // the Number-result node is only built when one side is a Number or the operator is `>>>`
                let numeric = matches!(x, Value::Number(_)) || matches!(y, Value::Number(_));
                if !numeric && op != B::ZeroFillRShift {
                    continue;
                }
                let want = it.binary(op, x, y);
                let node = bit_nm(
                    op,
                    Side::L(Leaf::Const(x.clone())),
                    Side::L(Leaf::Const(y.clone())),
                );
                let got = super::unbox(node(&mut it, &mut f));
                match (want, got) {
                    (Ok(Value::Number(w)), Ok(g)) => {
                        assert_eq!(w.to_bits(), g.to_bits(), "{op:?} at {xi},{yi}")
                    }
                    (Err(_), Err(_)) => {}
                    (w, g) => panic!("{op:?}: reference {:?} vs node {:?}", w.is_ok(), g.is_ok()),
                }
            }
        }
    }
}

#[test]
fn pr6_binary_and_compare_shapes_match_off() {
    same(
        r#"
const vals = [0, -0, 1, 2.5, NaN, Infinity, "5", "", null, undefined, true, 3n, 2147483648, -2147483649, 4294967296];
function f(a, b) {
  let s = a + b;
  let d = a - b;
  let m = a * b;
  let q = a / b;
  let r = a % b;
  let c = (a < b) ? 1 : 0;
  let e = a === b;
  let n = a !== b;
  return (""+(s)) + "," + (""+(1 / d)) + "," + (""+(m)) + "," + (""+(q)) + "," + (""+(r)) + "," + (""+(c)) + "," + (""+(e)) + "," + (""+(n)) + "," + (""+(a == b)) + "," + (""+(a >= b));
}
for (const x of vals) {
  for (const y of vals) {
    try { console.log(f(x, y)); } catch (e) { console.log("E " + e.message); }
  }
}
"#,
        1,
    );
}

#[test]
fn pr6_bitwise_number_nodes_and_index_reads_match_off() {
    same(
        r#"
const S = [];
for (let i = 0; i < 1024; i++) S.push((i * 2654435761) | 0);
const P = [1, 2, 3, 4];
function g(l, r) {
  let t = S[l >>> 24];
  t = (t + S[0x100 | ((l >> 16) & 0xff)]) | 0;
  t ^= S[0x200 | ((l >> 8) & 0xff)];
  r ^= t ^ P[1];
  return (""+(t)) + "," + (""+(r)) + "," + (""+((l << 31) >> 1)) + "," + (""+((l >>> 0) | 0)) + "," + (""+(l >> 40)) + "," + (""+(l & -1)) + "," + (""+(l ^ 2147483648)) + "," + (""+(l | 0)) + "," + (""+(r >>> 3));
}
const ls = [0, -0, 1, -1, 2147483648, -2147483649, 3735928559, NaN, Infinity, "7", 1.5, true, null, undefined, 4294967296];
for (const l of ls) {
  for (const r of [0, 9, -7, "3", NaN]) {
    try { console.log(g(l, r)); } catch (e) { console.log("E " + e.message); }
  }
}
function big(x) { return (""+(x >>> 0)) + "," + (""+(x >>> 1)); }
try { console.log(big(3n)); } catch (e) { console.log("E " + e.message); }
const arr = [10, 20, 30, 40, 50, 60, 70, 80, "x"];
const ta = new Int32Array(8);
ta[3] = -5;
function h(a, i) {
  return (""+(a[i >>> 0])) + "|" + (""+(a[i | 0])) + "|" + (""+(a[i & 7])) + "|" + (""+(a[i + 0.5])) + "|" + (""+(a[i - 2])) + "|" + (""+(a[(i | 0) + 1])) + "|" + (""+(a[1 << i]));
}
for (const i of [0, 3, 8, -1, 1.5, NaN, 4294967295, 2147483649, "2", 9]) {
  console.log(h(arr, i));
  console.log(h(ta, i));
}
"#,
        1,
    );
}

#[test]
fn pr6_compound_and_update_slots_match_off() {
    same(
        r#"
function c(n) {
  let s = 0; let t = "a"; let u = n;
  s += n; s -= 1; s *= 3; s /= 2; s %= 7; s |= 5; s ^= 3; s &= 255; s <<= 2; s >>= 1; s >>>= 0; s **= 2;
  t += n; t += "x"; t += null;
  u++; u--; ++u; u += u; u -= "1";
  let k = 0;
  for (let i = 0; i < 5; i++) { k++; k += i; }
  return (""+(s)) + "|" + t + "|" + (""+(u)) + "|" + (""+(k));
}
for (const n of [0, 1, -3, 2.5, "4", null, undefined, 4294967296, NaN, 3n]) {
  try { console.log(c(n)); } catch (e) { console.log("E " + e.message); }
}
function al(n) {
  let s = "ab"; let t = s; s += "c"; s += n;
  let q = "x"; let out = [0, 0, 0];
  for (let i = 0; i < 3; i++) { q += i; out[i] = q; }
  return t + "|" + s + "|" + q + "|" + out[0] + ";" + out[1] + ";" + out[2];
}
console.log(al(1));
console.log(al("z"));
function cc(v) { const k = v; k += 1; return k; }
function cu(v) { const k = v; k++; return k; }
function cs(v) { let k = v; k ^= 1; k += 2; return k; }
for (const v of [1, "2"]) {
  for (const fn of [cc, cu, cs]) {
    try { console.log(fn(v)); } catch (e) { console.log("E " + e.message); }
  }
}
"#,
        1,
    );
}

#[test]
fn pr6_conditions_and_update_statements_match_off() {
    same(
        r#"
function cnd(a, b) {
  let r = "";
  if (a < b) r += "lt";
  if (!(a >= b)) r += "!ge";
  if (a && b) r += "&";
  if (a || b) r += "|";
  if (!a) r += "!a";
  r += (a === b) ? "eq" : "ne";
  let i = 0;
  while (i < 3 && a != b) i++;
  do { i++; } while (i < 2 && a);
  for (let j = 0; a > j && j < 2; j++) r += j;
  let v = a;
  v++; v--; ++v; v--;
  return r + "|" + i + "|" + (""+(v));
}
const cv = [0, -0, 1, NaN, "", "0", null, undefined, true, "abc"];
for (const a of cv) {
  for (const b of cv) {
    try { console.log(cnd(a, b)); } catch (e) { console.log("E " + e.message); }
  }
}
"#,
        1,
    );
}

#[test]
fn pr6_blowfish_leaf_checksum_matches_the_reference() {
    let (want, ok, _) = run(BLOWFISH_LEAF, CompileMode::Off);
    assert!(ok);
    let (got, ok_on, compiled) = run(BLOWFISH_LEAF, CompileMode::On);
    assert!(ok_on);
    assert_eq!(got, want);
    assert!(compiled >= 2, "compiled {compiled}");
    assert_eq!(
        want,
        vec!["blowfish_leaf 1459210893 1459210893".to_string()]
    );
}
