//! `WebAssembly`. Without the `wasm` feature: a typeof-safe stub (no wasm engine; the scan keeps
//! real uses on QuickJS). With it: the real namespace (web/webassembly.js) over the `__thaw_wasm_*`
//! natives, which are installed first.

use crate::Interp;
#[cfg(feature = "wasm")]
use crate::Value;

#[cfg(not(feature = "wasm"))]
const STUB: &str = include_str!("wasm.js");
#[cfg(feature = "wasm")]
const REAL: &str = include_str!("webassembly.js");

fn run(interp: &mut Interp, source: &str) {
    interp
        .run(source)
        .unwrap_or_else(|_| panic!("WebAssembly shim must evaluate"));
}

/// `__thaw_wasm_mark_native(fn)`: `fn.toString()` prints `function name() { [native code] }`.
#[cfg(feature = "wasm")]
fn install_mark_native(interp: &mut Interp) {
    let mark = interp.native(|_, _, args| {
        if let Value::Object(o) = crate::arg(args, 0) {
            o.native_text.set(true);
        }
        Ok(Value::Undefined)
    });
    interp
        .global
        .clone()
        .declare("__thaw_wasm_mark_native", mark, true);
}

pub(crate) fn install(interp: &mut Interp) {
    #[cfg(feature = "wasm")]
    {
        crate::web_wasm_natives::install(interp);
        install_mark_native(interp);
        run(interp, REAL);
    }
    #[cfg(not(feature = "wasm"))]
    run(interp, STUB);
}
