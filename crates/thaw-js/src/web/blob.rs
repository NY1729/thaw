//! Minimal `Blob` / `File` (in-memory bytes; no streams).

use crate::Interp;

const SOURCE: &str = include_str!("blob.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("Blob prelude must evaluate"));
}
