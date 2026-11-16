//! `node:async_hooks` (AsyncLocalStorage, AsyncResource, createHook) prelude; builds on the async context bridge.

use crate::Interp;

const SOURCE: &str = include_str!("async_hooks.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run_builtin(SOURCE)
        .unwrap_or_else(|_| panic!("async_hooks prelude must evaluate"));
}
