//! `gc_registry` tests: the registry holds exactly the live object records of the thread, its entries point back
//! at their nodes, scopes register only when captured, and the census counts closure captures.
use super::*;
use crate::gc_registry::{check_invariants, counts};
use crate::mem_trace::{census_for_tests, census_snapshot, live_counts};

/// Registered objects equal the live object records; every entry and `reg_idx` agree.
fn assert_objects_match_live(at: &str) {
    let live = live_counts();
    assert_eq!(
        counts().0 as isize,
        live.0,
        "{at}: registered objects differ from live object records"
    );
    check_invariants().unwrap_or_else(|e| panic!("{at}: {e}"));
}

/// Registry `(objects, scopes)` left by one realm after its drop, measured from `before`.
fn registry_residue(before: (usize, usize), run: Option<&str>) -> (isize, isize) {
    {
        let mut it = Interp::new();
        if let Some(src) = run {
            it.run(src).unwrap_or_else(|_| panic!("script must run"));
        }
    }
    let after = counts();
    (
        after.0 as isize - before.0 as isize,
        after.1 as isize - before.1 as isize,
    )
}

#[test]
fn registry_matches_live_objects_through_a_realm() {
    drop(Interp::new()); // warm-up: one-time per-thread state
    assert_objects_match_live("before realm");
    {
        let mut it = Interp::new();
        assert_objects_match_live("after new");
        it.run(
            "var a = [1, 2, { x: 3 }]; var f = function () { return a; }; var s = 'x' + a.length;",
        )
        .unwrap_or_else(|_| panic!("script must run"));
        assert_objects_match_live("after script with arrays, objects and closures");
    }
    assert_objects_match_live("after realm drop");
}

#[test]
fn swap_remove_keeps_the_moved_entry_index_right() {
    let _it = Interp::new();
    let base = counts().0;
    let mut keep: Vec<Obj> = (0..50).map(|_| new_obj(None, Kind::Plain)).collect();
    assert_eq!(counts().0, base + 50);
    check_invariants().unwrap_or_else(|e| panic!("{e}"));
    for round in 0..200usize {
        // drop from the middle (moves the last entry into the hole), then add one
        let idx = (round * 7 + 3) % keep.len();
        keep.swap_remove(idx);
        check_invariants().unwrap_or_else(|e| panic!("after drop {round}: {e}"));
        keep.push(new_obj(None, Kind::Plain));
        check_invariants().unwrap_or_else(|e| panic!("after add {round}: {e}"));
        assert_eq!(counts().0, base + keep.len());
    }
    keep.clear();
    assert_eq!(counts().0, base);
}

#[test]
fn plain_call_frames_are_not_registered() {
    let mut it = Interp::new();
    it.run("function plain(x) { var y = x + 1; return y; } for (var i = 0; i < 200; i++) { plain(i); }")
        .unwrap_or_else(|_| panic!("script must run"));
    // the declaration captures the global scope, which the prelude's closures already registered
    let base = counts().1;
    it.run("for (var i = 0; i < 200; i++) { var z = plain(i) + 1; { let b = z; } }")
        .unwrap_or_else(|_| panic!("script must run"));
    assert_eq!(
        counts().1,
        base,
        "uncaptured call and block scopes must not be registered"
    );
}

#[test]
fn captured_scopes_register_and_release_with_their_closures() {
    let mut it = Interp::new();
    let base = counts().1;
    it.run("function mk() { let a = 1; return function () { return a; }; } var k1 = mk(); var k2 = mk();")
        .unwrap_or_else(|_| panic!("script must run"));
    // one frame per call of `mk`, captured by the closure it returns
    assert_eq!(
        counts().1,
        base + 2,
        "two captured frames must be registered"
    );
    it.run("k1 = null; k2 = null;")
        .unwrap_or_else(|_| panic!("script must run"));
    assert_eq!(counts().1, base, "freed closures must release their frames");
    check_invariants().unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn cycle_free_script_leaves_the_empty_realm_residue_in_the_registry() {
    drop(Interp::new()); // warm-up
    let before = counts();
    let empty = registry_residue(before, None);
    let before = counts();
    let script = "function add(a, b) { return a + b; } var sum = add(1, 2);";
    let with_script = registry_residue(before, Some(script));
    assert_eq!(
        with_script, empty,
        "a script without cycles left more registered nodes than an empty realm"
    );
    assert_objects_match_live("after cycle-free scripts");
}

#[test]
fn cyclic_script_leaves_the_registry_equal_to_the_live_counts() {
    drop(Interp::new()); // warm-up
    let before_live = live_counts();
    let before = counts();
    let script = "function mk() { var o = {}; o.f = function () { return o; }; return o; } \
                  for (var i = 0; i < 1000; i++) { mk(); }";
    {
        let mut it = Interp::new();
        it.run(script)
            .unwrap_or_else(|_| panic!("cycle script must run"));
        assert_objects_match_live("while the cycles are alive");
    }
    // the cycles are unreachable from the global object, so teardown cannot free them: they stay registered
    assert_objects_match_live("after the realm drop");
    let live = live_counts();
    let residue = (live.0 - before_live.0, live.1 - before_live.1);
    let reg = counts();
    assert_eq!(
        (
            reg.0 as isize - before.0 as isize,
            reg.1 as isize - before.1 as isize
        ),
        residue,
        "registry residue must equal the live residue"
    );
    assert!(
        residue.0 >= 2000 && residue.1 >= 1000,
        "leaked cycles must be seen: {residue:?}"
    );
}

#[test]
fn census_counts_the_captured_chain_against_the_free_names() {
    let mut it = Interp::new();
    census_for_tests(true);
    let before = census_snapshot();
    // `outer` is declared in the global scope: its chain has no bindings. The inner closure captures
    // `outer`'s frame, which binds `used` (named by the inner body) and `unused` (not named): mixed.
    it.run("function outer() { let used = 1; let unused = 2; return function () { return used; }; } outer();")
        .unwrap_or_else(|_| panic!("script must run"));
    let after = census_snapshot();
    let d: Vec<usize> = (0..5).map(|i| after[i] - before[i]).collect();
    assert_eq!(
        d,
        vec![2, 1, 0, 0, 1],
        "[closures, chain_empty, used_only, unused_only, mixed]"
    );
    census_for_tests(false);
}

#[test]
fn census_line_has_the_documented_format() {
    let line = crate::mem_trace::census_line("unit");
    assert!(
        line.starts_with("thaw-js mem-trace unit: closures_created="),
        "bad line: {line}"
    );
    for key in [
        " chain_empty=",
        " used_only=",
        " unused_only=",
        " mixed=",
        " live_closures_holder_in_cycle=",
    ] {
        assert!(line.contains(key), "missing {key:?} in {line}");
    }
}
