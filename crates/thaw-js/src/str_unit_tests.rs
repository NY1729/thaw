//! Differential and work-count tests for UTF-16 unit access on strings (`str_len`,
//! `str_unit_at`, `str_slice`). The oracle is `str_units`, a full decode of the string; the
//! functions under test must agree with it for every index, including past the end.

use crate::builtins::{
    str_len, str_len_with, str_slice, str_slice_with, str_unit_at_with, str_units, units_str,
};
use crate::jsstr::{ascii_scans, char_walk_steps};
use crate::{Interp, JsStr};

/// A lone surrogate unit as the string layer stores it (`LONE + (unit - 0xD800)`).
fn lone(unit: u16) -> char {
    char::from_u32(0x10F800 + u32::from(unit) - 0xD800).unwrap()
}

/// Indices to probe: every index around the ends, and a stride through the middle for long input.
fn probe_indices(n: usize, full: bool) -> Vec<usize> {
    if full {
        (0..=n + 2).collect()
    } else {
        let mut v: Vec<usize> = (0..=n + 2).step_by(997).collect();
        v.extend(0..3.min(n + 3));
        v.extend(n.saturating_sub(3)..=n + 2);
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// Checks the `&str` wrappers and the cached-flag (`_with`) forms the JS paths use.
fn check_against_oracle(js: &JsStr, full: bool) {
    let text: &str = js;
    let ascii = js.is_ascii_cached();
    let units = str_units(text);
    let n = units.len();
    assert_eq!(str_len(text), n, "str_len of {text:?}");
    assert_eq!(str_len_with(text, ascii), n, "str_len_with of {text:?}");
    for i in probe_indices(n, full) {
        assert_eq!(
            str_unit_at_with(text, ascii, i),
            units.get(i).copied(),
            "str_unit_at_with({i}) of a string with {n} units"
        );
    }
    let ends = probe_indices(n, full);
    for &a in &ends {
        let a = a.min(n);
        for b in [a, a + 1, a + 2, n] {
            let b = b.min(n).max(a);
            if !full && b - a > 3 && a != 0 && b != n {
                continue;
            }
            let want = units_str(&units[a..b]);
            assert_eq!(str_slice(text, a, b), want, "str_slice({a}, {b})");
            assert_eq!(
                str_slice_with(text, ascii, a, b),
                want,
                "str_slice_with({a}, {b}) of a string with {n} units"
            );
        }
    }
}

#[test]
fn unit_access_matches_the_oracle_for_short_strings() {
    let cases: Vec<String> = vec![
        String::new(),
        "a".into(),
        "abc".into(),
        "é".into(),
        "ÿé中".into(),
        "😀".into(),
        "a😀b".into(),
        "é😀x".into(),
        lone(0xD83D).to_string(),
        lone(0xDE00).to_string(),
        format!("x{}y", lone(0xD83D)),
        format!("{}{}", lone(0xDE00), lone(0xD83D)),
        format!("😀{}", lone(0xDE00)),
    ];
    for text in &cases {
        check_against_oracle(&JsStr::from(text.as_str()), true);
    }
}

#[test]
fn unit_access_matches_the_oracle_for_long_strings() {
    // ASCII, and a Buf whose surrogate seam was merged by `push_joined`
    let ascii = JsStr::from("ab".repeat(200));
    check_against_oracle(&ascii, false);
    let mut seam = JsStr::from("x".repeat(300));
    seam.push_joined(&lone(0xD83D).to_string());
    seam.push_joined(&lone(0xDE00).to_string());
    assert_eq!(str_len(&seam), 302);
    check_against_oracle(&seam, false);
    // 100k units: ASCII head, then one BMP char and one astral char near the end
    let big = JsStr::from(format!("{}é😀{}", "a".repeat(99_000), "b".repeat(1_000)));
    check_against_oracle(&big, false);
}

#[test]
fn ascii_path_scans_the_string_once() {
    // Work counter: before the cached flag, `s.length` and `s[i]` each re-scanned the whole
    // payload (2n scans for an n-char loop, measured on the pre-change code). Now the flag is
    // computed once per string and reused by every later call.
    let s = JsStr::from("a".repeat(100_000));
    let before = ascii_scans();
    let mut sum = 0u64;
    for i in 0..s.len() {
        let ascii = s.is_ascii_cached();
        if i < str_len_with(&s, ascii) {
            sum += u64::from(str_unit_at_with(&s, ascii, i).unwrap());
        }
    }
    assert_eq!(sum, 97 * 100_000);
    assert_eq!(ascii_scans() - before, 1);
    assert_eq!(char_walk_steps(), 0, "ASCII access takes no char steps");
}

#[test]
fn sign_nan_and_fraction_indices_keep_their_meaning() {
    // Node: NaN for charCodeAt(-1), charCodeAt(NaN); fraction is truncated for charCodeAt and
    // charAt; s[-1] and s[1.5] are undefined; charAt out of range is ""; length counts units.
    let src = "var s = 'é😀x';\
        console.log(s.length, s.charCodeAt(-1), s.charCodeAt(NaN), s.charCodeAt(1.5), s.charCodeAt(4), s.charCodeAt(5));\
        console.log(s.charAt(-1) === '', s.charAt(5) === '', s.charAt(1.5).charCodeAt(0), s[-1] === undefined, s[1.5] === undefined, s[4] === undefined);\
        console.log(s[0].charCodeAt(0), s[1].charCodeAt(0), s[2].charCodeAt(0), s[3], s.slice(1, 3).length, s.substring(3).charCodeAt(0));";
    let mut it = Interp::new();
    assert!(it.run(src).is_ok(), "script failed");
    assert_eq!(
        it.out.join("|"),
        "4 NaN NaN 55357 NaN NaN|true true 55357 true true true|233 55357 56832 x 2 120"
    );
}
