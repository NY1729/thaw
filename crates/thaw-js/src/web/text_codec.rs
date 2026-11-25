//! `TextEncoder` / `TextDecoder`.
//! Plain JS on top of the typed-array and DataView intrinsics; `install` evaluates it once.

use crate::Interp;

const SOURCE: &str = include_str!("text_codec.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run_builtin(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("TextEncoder/TextDecoder prelude must evaluate"));
}
