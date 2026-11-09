//! Cursor-based UTF-16 access on `JsStr` (`len16`, `unit_at`, `unit_to_byte`, `slice_units`).
//! Every result is checked against an independent oracle built here (not `builtins::str_units`),
//! and the char-walk counter bounds the work for sequential, backward and jittered access.

use crate::jsstr::char_walk_steps;
use crate::JsStr;

/// The string layer stores a lone surrogate unit `u` as the char `LONE + (u - 0xD800)`.
const LONE: u32 = 0x10F800;

fn lone(unit: u16) -> char {
    char::from_u32(LONE + u32::from(unit) - 0xD800).unwrap()
}

fn lcg(seed: u64) -> u64 {
    seed.wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
}

/// Oracle: the UTF-16 units of `text`.
fn oracle_units(text: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        let cp = c as u32;
        if cp >= LONE {
            out.push((cp - LONE + 0xD800) as u16);
        } else if cp >= 0x10000 {
            let v = cp - 0x10000;
            out.push((0xD800 + (v >> 10)) as u16);
            out.push((0xDC00 + (v & 0x3FF)) as u16);
        } else {
            out.push(cp as u16);
        }
    }
    out
}

/// Oracle: the text of a unit slice, with a lone surrogate in the stored form.
fn oracle_text(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied())
        .map(|r| r.unwrap_or_else(|e| lone(e.unpaired_surrogate())))
        .collect()
}

/// Oracle: `table[ui]` = byte of the first char starting at or after unit `ui`, else the length.
fn oracle_unit_to_byte_table(text: &str, n: usize) -> Vec<usize> {
    let mut table = vec![text.len(); n + 3];
    let mut next = 0;
    let mut u = 0;
    for (b, c) in text.char_indices() {
        for slot in table.iter_mut().take(u + 1).skip(next) {
            *slot = b;
        }
        next = u + 1;
        u += oracle_units(&c.to_string()).len();
    }
    table
}

/// Index access as the JS paths do it: a negative index is never looked up.
fn at(js: &JsStr, i: i64) -> Option<u16> {
    usize::try_from(i).ok().and_then(|i| js.unit_at(i))
}

/// Every index in -1..=len+2 (ascending, then descending: two different cursor histories).
fn check_all_indices(js: &JsStr, text: &str) {
    let want = oracle_units(text);
    let n = want.len();
    assert_eq!(js.len16(), n, "len16");
    let table = oracle_unit_to_byte_table(text, n);
    let mut order: Vec<i64> = (-1..=n as i64 + 2).collect();
    for _pass in 0..2 {
        for &i in &order {
            let w = usize::try_from(i).ok().and_then(|i| want.get(i).copied());
            assert_eq!(at(js, i), w, "unit_at({i}) of {n} units");
            if let Ok(ui) = usize::try_from(i) {
                assert_eq!(js.unit_to_byte(ui), table[ui], "unit_to_byte({ui})");
            }
        }
        order.reverse();
    }
}

/// Slices: every pair for short strings, a grid of starts (both parities) and lengths otherwise.
fn check_slices(js: &JsStr, text: &str) {
    let want = oracle_units(text);
    let n = want.len();
    let mut starts: Vec<usize> = if n <= 60 {
        (0..=n).collect()
    } else {
        let step = n / 60;
        let mut v: Vec<usize> = (0..=n).step_by(step).collect();
        v.extend((0..=n).step_by(step).map(|x| x + 1).filter(|&x| x <= n));
        v.extend([0, 1, 2, n - 2, n - 1, n]);
        v
    };
    starts.sort_unstable();
    starts.dedup();
    for &a in &starts {
        let ends: Vec<usize> = if n <= 60 {
            (a..=n).collect()
        } else {
            [0usize, 1, 2, 3, 7, 1000, n]
                .iter()
                .map(|len| (a + len).min(n))
                .collect()
        };
        for b in ends {
            assert_eq!(
                js.slice_units(a, b),
                oracle_text(&want[a..b]),
                "slice_units({a}, {b}) of {n} units"
            );
        }
    }
}

fn short_cases() -> Vec<String> {
    vec![
        String::new(),
        "a".into(),
        "é😀x".into(),
        "😀".into(),
        lone(0xD83D).to_string(),
        lone(0xDE00).to_string(),
        format!("x{}y", lone(0xD83D)),
        format!("{}{}", lone(0xDE00), lone(0xD83D)),
        format!("😀{}", lone(0xDE00)),
    ]
}

/// Long enough for the `Buf` representation (the cursor lives there).
fn long_cases() -> Vec<String> {
    vec![
        "ab".repeat(200),
        "é".repeat(300),
        "ÿé中".repeat(100),
        "a😀b".repeat(80),
        format!("{}{}", "é".repeat(300), lone(0xD83D)),
        format!("{}{}", "é".repeat(300), lone(0xDE00)),
        format!("{}{}{}", "x".repeat(299), lone(0xDE00), lone(0xD83D)),
        format!("{}{}", "x".repeat(300), '😀'),
        format!("{}{}{}", "x".repeat(120), lone(0xD83D), "y".repeat(200)),
        format!("{}é", "ab".repeat(200)),
        // 120k units, 90k chars
        "é😀a".repeat(30_000),
    ]
}

#[test]
fn unit_access_matches_the_oracle_for_every_index() {
    for text in short_cases().into_iter().chain(long_cases()) {
        let js = JsStr::from(text.as_str());
        check_all_indices(&js, &text);
        check_slices(&JsStr::from(text.as_str()), &text);
    }
}

#[test]
fn seam_merged_buffer_matches_the_oracle() {
    // a trailing lone high surrogate, then a lone low one merged into an astral char by push_joined
    let mut js = JsStr::from("x".repeat(299) + "é");
    let mut text = "x".repeat(299) + "é";
    check_all_indices(&js, &text);
    js.push_joined(&lone(0xD83D).to_string());
    text.push(lone(0xD83D));
    check_all_indices(&js, &text);
    js.push_joined(&lone(0xDE00).to_string());
    text.pop();
    text.push('😀');
    assert_eq!(&*js, text.as_str());
    check_all_indices(&js, &text);
    check_slices(&js, &text);
}

#[test]
fn cursor_resets_after_push_joined() {
    let mut js = JsStr::from("x".repeat(299) + "é");
    let mut text = "x".repeat(299) + "é";
    // leave the cursor on the last char, then append: the old cursor must not be reused
    assert_eq!(js.unit_at(299), Some(0xE9));
    js.push_joined("😀a");
    text.push_str("😀a");
    assert_eq!(js.cursor_for_test(), Some((0, 0)));
    check_all_indices(&js, &text);

    // now leave the cursor on a lone high surrogate at the end, then merge it away
    js.push_joined(&lone(0xD83D).to_string());
    text.push(lone(0xD83D));
    assert_eq!(js.unit_at(js.len16() - 1), Some(0xD83D));
    js.push_joined(&lone(0xDE00).to_string());
    text.pop();
    text.push('😀');
    assert_eq!(js.cursor_for_test(), Some((0, 0)));
    assert_eq!(&*js, text.as_str());
    check_all_indices(&js, &text);
}

#[test]
fn forward_scan_is_linear_in_the_string_length() {
    let text = "é😀a".repeat(30_000);
    let want = oracle_units(&text);
    let n = want.len();
    let js = JsStr::from(text.as_str());
    let base = char_walk_steps();
    let mut i = 0;
    while i < js.len16() {
        assert_eq!(js.unit_at(i), Some(want[i]));
        i += 1;
    }
    let steps = char_walk_steps() - base;
    assert_eq!(i, n);
    // len16 walks every char once; the scan steps over every char once more
    assert!(
        steps <= 2 * n,
        "forward scan: {steps} char steps for {n} units"
    );
}

#[test]
fn backward_scan_is_linear_in_the_string_length() {
    let text = "é😀a".repeat(30_000);
    let want = oracle_units(&text);
    let n = want.len();
    let js = JsStr::from(text.as_str());
    let base = char_walk_steps();
    for i in (0..n).rev() {
        assert_eq!(js.unit_at(i), Some(want[i]));
    }
    let steps = char_walk_steps() - base;
    // len16, one forward walk to the end, then the backward walk
    assert!(
        steps <= 3 * n,
        "backward scan: {steps} char steps for {n} units"
    );
}

#[test]
fn jittered_access_stays_linear() {
    let text = "é😀a".repeat(30_000);
    let want = oracle_units(&text);
    let n = want.len();
    let js = JsStr::from(text.as_str());
    let base = char_walk_steps();
    for i in 0..n {
        if i + 2 < n {
            assert_eq!(js.unit_at(i + 2), Some(want[i + 2]));
        }
        assert_eq!(js.unit_at(i), Some(want[i]));
    }
    let steps = char_walk_steps() - base;
    // Measured: about 4.5 steps per unit for this text (each read crosses 1-2 chars, twice). Linear is the point:
    // a walk from the start would cost billions of steps here.
    assert!(
        steps <= 6 * n,
        "jittered scan: {steps} char steps for {n} units"
    );
}

#[test]
fn random_access_pays_the_distance_walked() {
    // Documented ceiling: a random jump walks from the cursor or the start, so its cost is the
    // distance, not 1. The bound only asserts that: 200 random reads cost far more than 200 steps.
    let text = "é😀a".repeat(30_000);
    let want = oracle_units(&text);
    let n = want.len();
    let js = JsStr::from(text.as_str());
    let base = char_walk_steps();
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let accesses = 200;
    for _ in 0..accesses {
        seed = lcg(seed);
        let i = (seed >> 33) as usize % n;
        assert_eq!(js.unit_at(i), Some(want[i]));
    }
    let steps = char_walk_steps() - base;
    assert!(
        steps > accesses * 1000,
        "random access: only {steps} char steps for {accesses} reads"
    );
}

#[test]
fn cursor_always_sits_on_a_char_boundary() {
    let text = format!("{}{}é😀{}", "x".repeat(300), lone(0xD83D), "ÿ😀".repeat(50));
    let want = oracle_units(&text);
    let n = want.len();
    let js = JsStr::from(text.as_str());
    let mut seed = 12_345_u64;
    for _ in 0..2000 {
        seed = lcg(seed);
        let i = (seed >> 33) as usize % (n + 3);
        let _ = js.unit_at(i);
        let _ = js.unit_to_byte(i);
        let a = i.min(n);
        let _ = js.slice_units(a, (a + 3).min(n));
        let (u, b) = js.cursor_for_test().expect("a long string has a cursor");
        assert!(
            text.is_char_boundary(b),
            "cursor byte {b} is not a char start"
        );
        assert_eq!(oracle_units(&text[..b]).len(), u, "cursor unit vs byte {b}");
    }
}
