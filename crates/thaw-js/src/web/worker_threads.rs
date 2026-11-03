//! `node:worker_threads` main-thread subset (worker_threads.js): the module object thaw-js bundles for the builtin.

use crate::Interp;

const SOURCE: &str = include_str!("worker_threads.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(SOURCE)
        .unwrap_or_else(|_| panic!("worker_threads prelude must evaluate"));
}
