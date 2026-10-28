//! `AbortSignal` / `AbortController` (spec behaviour; builds on event_target.rs).

use crate::Interp;

const SOURCE: &str = include_str!("abort.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("AbortSignal prelude must evaluate"));
}
