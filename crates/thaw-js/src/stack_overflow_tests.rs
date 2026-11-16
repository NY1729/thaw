//! Infinite recursion on a configured stack must throw a catchable `RangeError`, not abort the process. Each case runs
//! on its own thread with a small stack (the way a worker or the host runs its `Interp`), recurses until the stack
//! guard trips, then checks that the interpreter still works: a second overflow and a plain call both succeed.

use super::*;

/// Stack of the test thread; `configure_stack` gets the same size, so the guard trips well before the real end.
const SMALL_STACK: usize = 16 << 20;

/// Runs `src` on a fresh Interp configured for `SMALL_STACK` and returns its console lines.
fn run_small_stack(src: String) -> Vec<String> {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            let mut it = Interp::new();
            it.configure_stack(SMALL_STACK);
            match it.run(&src) {
                Ok(_) => {}
                Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
                Err(_) => panic!("stray completion"),
            }
            std::mem::take(&mut it.out)
        })
        .expect("thread spawns")
        .join()
        .expect("thread joins")
}

/// `setup` declares the recursive functions; `trigger` overflows. The trigger runs twice, so a guard left
/// disabled after the first RangeError would show up as an abort on the second.
fn overflow_then_recover(setup: &str, trigger: &str) {
    let src = format!(
        "{setup}\n\
         for (let i = 0; i < 2; i++) {{ try {{ {trigger}; console.log('no throw'); }} catch (e) {{ console.log(e instanceof RangeError, e.message); }} }}\n\
         function ok() {{ return 7; }}\n\
         console.log(ok());"
    );
    assert_eq!(
        run_small_stack(src),
        [
            "true Maximum call stack size exceeded",
            "true Maximum call stack size exceeded",
            "7",
        ]
    );
}

#[test]
fn stack_overflow_self_recursion_throws_range_error() {
    overflow_then_recover("function f() { return f(); }", "f()");
}

#[test]
fn stack_overflow_mutual_recursion_throws_range_error() {
    overflow_then_recover(
        "function a() { return b(); } function b() { return a(); }",
        "a()",
    );
}

#[test]
fn stack_overflow_through_array_map_throws_range_error() {
    overflow_then_recover("function f() { return [1].map(f); }", "f()");
}

#[test]
fn stack_overflow_through_sort_comparator_throws_range_error() {
    overflow_then_recover("function f() { return [2, 1].sort(() => f()); }", "f()");
}

#[test]
fn stack_overflow_through_self_calling_getter_throws_range_error() {
    overflow_then_recover("const o = { get x() { return this.x; } };", "o.x");
}

#[test]
fn stack_overflow_through_proxy_get_trap_throws_range_error() {
    overflow_then_recover(
        "const p = new Proxy({}, { get(t, k, r) { return r[k]; } });",
        "p.x",
    );
}

// The recursion goes through the binary `+` (which propagates errors). Unary `+`, template literals and `String()`
// convert through the infallible `num`/`to_str`, which turn a nested RangeError into NaN or "[object Object]".
#[test]
fn stack_overflow_through_valueof_throws_range_error() {
    overflow_then_recover("const o = { valueOf() { return this + 1; } };", "o + 1");
}

#[test]
fn stack_overflow_through_tostring_throws_range_error() {
    overflow_then_recover("const o = { toString() { return this + ''; } };", "o + ''");
}

#[test]
fn stack_overflow_through_class_constructor_throws_range_error() {
    overflow_then_recover("class A { constructor() { new A(); } }", "new A()");
}

#[test]
fn stack_overflow_through_reflect_apply_throws_range_error() {
    overflow_then_recover("function f() { return Reflect.apply(f, null, []); }", "f()");
}

#[test]
fn stack_overflow_through_bind_throws_range_error() {
    overflow_then_recover("function f() { return f.bind(null)(); }", "f()");
}
