//! `MessageChannel` / `MessagePort` / `BroadcastChannel` (pure JS on EventTarget + structuredClone; messaging.js).
//! The `__thaw_is_message_port` / `__thaw_message_port_postmessage_with_uncloneable_checks` globals are what
//! `node:worker_threads` reads (registry/builtins/system/workers.rs).

use crate::Interp;

const SOURCE: &str = include_str!("messaging.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("MessageChannel prelude must evaluate"));
}
