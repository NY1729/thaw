//! `WebAssembly` as a typeof-safe stub (no wasm engine on thaw-js; the scan rejects real uses).

use crate::Interp;

const SOURCE: &str = include_str!("wasm.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("WebAssembly stub must evaluate"));
}
