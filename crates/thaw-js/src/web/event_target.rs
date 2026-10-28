//! `Event` / `EventTarget` (ported from thaw-quickjs `platform_globals/workers/blob_events.js`).
//! `install` evaluates the port once; `AbortSignal` (abort.rs) builds on it.

use crate::Interp;

const SOURCE: &str = include_str!("event_target.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("Event/EventTarget prelude must evaluate"));
}
