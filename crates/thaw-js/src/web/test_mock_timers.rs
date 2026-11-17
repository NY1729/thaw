//! `node:test` mock timers, the host half (`src/web/test_mock_timers.js`): the `__thaw_test_mock_timers_*` globals the
//! shared node:test wrapper calls. Virtual timers and the mocked `Date` are plain JS over the captured native timers.
//! The mocked `Date` also reaches the compiled lane: `__thaw_test_mock_clock(ms)` mirrors the mocked clock into the
//! runtime's mock cell (`thaw_mock_clock_set`), and `__thaw_test_mock_clock()` restores the wall clock.

use crate::{arg, Interp, Value};

const SOURCE: &str = include_str!("test_mock_timers.js");

pub(crate) fn install(interp: &mut Interp) {
    let clock = interp.native(|it, _, args| {
        match arg(args, 0) {
            Value::Undefined | Value::Null => set_native_clock(None),
            value => set_native_clock(Some(it.num(&value))),
        }
        Ok(Value::Undefined)
    });
    interp.global.declare("__thaw_test_mock_clock", clock, true);
    interp
        .run_builtin(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("test mock timers prelude must evaluate"));
}

#[cfg(test)]
thread_local! {
    /// What the compiled-lane hook was last asked to do: `Some(Some(ms))` set, `Some(None)` clear, `None` never called.
    static NATIVE_CLOCK: std::cell::Cell<Option<Option<f64>>> = const { std::cell::Cell::new(None) };
}

/// The last mock-cell request this thread made (tests only; the runtime symbol is absent there).
#[cfg(test)]
pub(crate) fn recorded_native_clock() -> Option<Option<f64>> {
    NATIVE_CLOCK.with(std::cell::Cell::get)
}

/// Mirrors the mocked clock into the compiled runtime's mock cell: `Some(ms)` mocks `Date.now()` and `new Date()`
/// to `ms`, `None` restores the wall clock. The runtime's `thaw_mock_clock_set` / `thaw_mock_clock_clear` are
/// looked up by name, like the other optional runtime symbols; a no-op when the runtime is not linked.
fn set_native_clock(ms: Option<f64>) {
    #[cfg(test)]
    NATIVE_CLOCK.with(|cell| cell.set(Some(ms)));
    #[cfg(all(unix, not(test)))]
    // SAFETY: each symbol, when present, is the compiled runtime's `extern "C"` mock-cell hook with this signature.
    unsafe {
        match ms {
            Some(ms) => {
                let sym = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_mock_clock_set".as_ptr());
                if !sym.is_null() {
                    std::mem::transmute::<*mut std::ffi::c_void, extern "C" fn(f64)>(sym)(ms);
                }
            }
            None => {
                let sym = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_mock_clock_clear".as_ptr());
                if !sym.is_null() {
                    std::mem::transmute::<*mut std::ffi::c_void, extern "C" fn()>(sym)();
                }
            }
        }
    }
}
