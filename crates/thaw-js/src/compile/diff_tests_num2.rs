//! Tests of the C4 PR1 operand evaluation (plan TJ153 section 3.1): bitwise nodes over non-literal operands
//! (`bit_ex`), `Num2` operands, in-place Number slots (`store_slot`, `fused`, `discard`). Programs run under Off
//! (the reference), On and Check; output and ok-ness must match. The counters prove the Number-Number fast path
//! ran, and that it stays off for BigInt and string operands.

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

/// Every bitwise and shift operator over the corner values, with both operands non-literal (`bit_ex`).
#[test]
fn num2_bit_operators_over_the_corner_table_match_off() {
    same(
        r#"
function ops(a, b) {
  return "" + (a ^ b) + "," + (a & b) + "," + (a | b) + "," + (a << b) + "," + (a >> b) + "," + (a >>> b);
}
const vs = [0, -0, 1, -1, 0.5, -1.5, 2147483647, 2147483648, -2147483649, 4294967295, 4294967296,
  NaN, Infinity, -Infinity, 31, 32, 33, -33, 1e10, "7", "0x10", "", " 3 ", true, false, null,
  undefined, 5n, -3n];
for (const a of vs) {
  for (const b of vs) {
    try { console.log(ops(a, b)); } catch (e) { console.log("E " + e.message); }
  }
}
"#,
        1,
    );
}

/// Operands are evaluated once, left first: `valueOf` runs in source order, and the results agree with Off.
#[test]
fn num2_operands_evaluate_once_in_order() {
    same(
        r#"
const log = [];
const o1 = { valueOf() { log.push("o1"); return 6; } };
const o2 = { valueOf() { log.push("o2"); return 3; } };
const o3 = { valueOf() { log.push("o3"); return 1; } };
function pair(a, b) { return (a ^ b) + (a | b); }
function trip(a, b, c) { return (a ^ b[0]) + (c ^ a) + (b[0] & c); }
function nest(a, b) { return ((a ^ b) & (b ^ a)) >>> 1; }
console.log(String(pair(o1, o2)), log.join(" "));
console.log(String(trip(o1, [o2], o3)), log.join(" "));
console.log(String(nest(o2, o3)), log.join(" "));
console.log(String(pair(o3, 7)), log.join(" "));
"#,
        3,
    );
}

/// Compound assignments and plain stores on slots: the old value is read before the right-hand side, a
/// Number slot is updated in place, and a non-Number value takes the generic path.
#[test]
fn num2_slot_compound_and_store_match_off() {
    same(
        r#"
function c1(n) { n += (n = 5); return n }
function c2(n, m) { n ^= m; n |= 1; n <<= 2; n >>>= 1; n >>= 1; return n }
function c3(s) { s += 1; s += "x"; s ^= 3; return s }
function c4(n) { n++; ++n; n--; n = n ^ 1; return n }
function c5(a, b) { let x = a ^ b; x = x & 7; x = (x | 0) + 1; x -= a ^ 2; return x }
function c6(a) { let x; x = 3 ^ a; x = x ^ a; return x }
function c7(a) { let x = 0; x = a ^ 1; x = x + (a ^ x); return x }
function c8(a) { let x = a; x = a ^ a; x *= 2; x %= 3; return x }
const cases = [0, -0, 3, 2147483648, NaN, "4", 5n, null, undefined, true, 1.5];
for (const v of cases) {
  for (const f of [c1, c2, c3, c4, c5, c6, c7, c8]) {
    try { console.log(String(f(v, v))); } catch (e) { console.log("E " + e.message); }
  }
}
"#,
        8,
    );
}

/// Counters: the Number-Number fast path runs for a compiled expression with bitwise nodes, and the
/// BigInt or string call takes the generic path (the counter does not move) with the same output as Off.
#[test]
fn num2_fast_path_counters_and_negative_controls() {
    let numbers = r#"
function f(a, b, c) { return (a ^ b[0]) + (c ^ a) }
const arr = [6];
console.log(String(f(3, arr, 5)));
console.log(String(f(-1, arr, 2147483648)));
"#;
    let mixed = r#"
function f(a, b, c) { return (a ^ b[0]) + (c ^ a) }
const arr = [6];
console.log(String(f(3, arr, 5)));
console.log(String(f(-1, arr, 2147483648)));
try { console.log(String(f(3n, [6n], 5n))); } catch (e) { console.log("E " + e.message); }
try { console.log(String(f("3", arr, 5))); } catch (e) { console.log("E " + e.message); }
"#;
    let (want_n, ok_n, it_n) = run(numbers, CompileMode::On);
    assert!(ok_n);
    assert!(it_n.cstats.compiled() >= 1, "f must compile");
    assert!(
        it_n.cstats.bit_ex_built() >= 2,
        "both bitwise nodes take bit_ex: {}",
        it_n.cstats.bit_ex_built()
    );
    let hits_n = it_n.cstats.nm_fast_hits();
    assert!(hits_n > 0, "numbers must take the fast path");

    let (want_m, _, _) = run(mixed, CompileMode::Off);
    let (got_m, ok_m, it_m) = run(mixed, CompileMode::On);
    let (check_m, ok_check, _) = run(mixed, CompileMode::Check);
    assert!(ok_m && ok_check);
    assert_eq!(got_m, want_m);
    assert_eq!(check_m, want_m);
    assert_eq!(&got_m[..2], &want_n[..], "the numeric calls print the same");
    assert_eq!(
        it_m.cstats.nm_fast_hits(),
        hits_n,
        "BigInt, string and mixed calls must not hit the Number path"
    );
}
