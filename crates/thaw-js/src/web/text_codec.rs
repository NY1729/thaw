//! `TextEncoder` / `TextDecoder` (ported from thaw-quickjs `platform_globals/text_encoding.js`).
//! Plain JS on top of the typed-array and DataView intrinsics; `install` evaluates it once.

use crate::Interp;

const SOURCE: &str = include_str!("text_codec.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("TextEncoder/TextDecoder prelude must evaluate"));
}
