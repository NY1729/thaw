//! `ReadableStream` / `WritableStream` (default controllers; no tee, pipeThrough or byte readers).

use crate::Interp;

const SOURCE: &str = include_str!("streams.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("streams prelude must evaluate"));
}
