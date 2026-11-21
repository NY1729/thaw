//! The worker realm's builtin table: `__thaw_builtin_source` (eventloop.rs) reads thaw-node-builtins, and
//! `__thaw_builtin_exports` (web/builtin_modules.js) evaluates each source once per realm.

use super::*;

/// Runs `src` on a fresh Interp and returns its console lines (as worker_small_gaps_tests.rs does).
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    it.run_event_loop().expect("event loop runs");
    std::mem::take(&mut it.out)
}

#[test]
fn builtin_exports_evaluate_the_path_source() {
    assert_eq!(
        lines("console.log(__thaw_builtin_exports('path').join('a', 'b'));"),
        ["a/b"]
    );
}

#[test]
fn builtin_exports_accept_the_node_prefix_and_cache_per_realm() {
    assert_eq!(
        lines(
            "console.log(__thaw_builtin_exports('node:path') === __thaw_builtin_exports('path'));"
        ),
        ["true"]
    );
}

#[test]
fn builtin_path_posix_requires_node_path_through_the_table() {
    assert_eq!(
        lines("console.log(__thaw_builtin_exports('path/posix') === __thaw_builtin_exports('path').posix);"),
        ["true"]
    );
}

#[test]
fn builtin_exports_of_an_unknown_name_is_undefined() {
    assert_eq!(
        lines("console.log(__thaw_builtin_exports('no-such-builtin'));"),
        ["undefined"]
    );
}
