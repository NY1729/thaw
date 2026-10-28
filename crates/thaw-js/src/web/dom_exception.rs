//! `DOMException` (WHATWG; Node-compatible name/code/constants), plain JS on the Error intrinsic.

use crate::Interp;

const SOURCE: &str = include_str!("dom_exception.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("DOMException prelude must evaluate"));
}
