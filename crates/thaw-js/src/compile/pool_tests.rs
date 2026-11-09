//! Frame pool of compiled calls (plan TJ150 section 7, PR7): slot buffers are reused, emptied on every exit,
//! and a nested entry never shares a live buffer.

use super::*;

/// A realm in `On` mode that compiles on the first call, after running `src` (output in `it.out`).
fn on(src: &str) -> Interp {
    let mut it = Interp::with_compile_mode(CompileMode::On);
    it.compile_after = 1;
    let _ = it.run_sloppy(src);
    it
}

#[test]
fn pool_reuses_one_buffer_and_keeps_it_empty() {
    let it = on("var s = 0; function f(a) { var t = a + 1; return t; }\n\
        for (var i = 0; i < 1000; i++) s += f(i);\n\
        console.log(String(s));");
    assert_eq!(it.out, vec!["500500".to_string()]);
    assert!(it.cstats.compiled() >= 1, "f must compile");
    assert!(it.cstats.compiled_calls() >= 1000);
    assert!(
        it.frame_pool.len() <= 1,
        "pool grew to {}",
        it.frame_pool.len()
    );
    assert!(it.frame_pool.iter().all(|v| v.is_empty()));
}

#[test]
fn thrown_error_leaves_no_values_in_the_pool() {
    let it = on("function f(a) { var t = a; return t.x.y; }\n\
        var r = 'none';\n\
        try { f({}); } catch (e) { r = e.name; }\n\
        try { f(1); } catch (e) { r = r + '/' + e.name; }\n\
        console.log(r);");
    assert_eq!(it.out, vec!["TypeError/TypeError".to_string()]);
    assert!(it.cstats.compiled() >= 1, "f must compile");
    assert!(it.frame_pool.iter().all(|v| v.is_empty()));
}

#[test]
fn nested_entry_takes_its_own_buffer() {
    // `g` (compiled) reads a getter that calls `f` (compiled) while `g`'s frame is live
    let it = on("function f(a) { var t = a * 2; return t; }\n\
        var o = { get p() { return f(5); } };\n\
        function g(o) { var k = o.p; return k + 1; }\n\
        console.log(String(g(o)));\n\
        console.log(String(g(o)));");
    assert_eq!(it.out, vec!["11".to_string(), "11".to_string()]);
    assert!(it.cstats.compiled() >= 2, "f and g must compile");
    assert_eq!(it.frame_pool.len(), 2);
    assert!(it.frame_pool.iter().all(|v| v.is_empty()));
}
