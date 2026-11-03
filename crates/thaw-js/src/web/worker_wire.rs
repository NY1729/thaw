//! Worker message wire (worker_wire.js): `__thaw_worker_encode` / `__thaw_worker_decode`, the structured-clone JSON
//! form that crosses worker threads as a string.

use crate::Interp;

const SOURCE: &str = include_str!("worker_wire.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("worker wire prelude must evaluate"));
}
