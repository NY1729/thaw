//! Worker stdio streams (worker_stream.js): the private `__thaw_worker_stream` global (Readable, Writable, hex bytes),
//! installed in every realm before worker_threads.js and the worker bootstrap use it.

use crate::Interp;

const SOURCE: &str = include_str!("worker_stream.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("worker stream prelude must evaluate"));
}
