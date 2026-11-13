//! `Performance` / `performance` and the user-timing classes (`src/web/performance.js`). The clock is
//! the host's `__thaw_perf_now` and the loop stats are `__thaw_loop_stats`, both registered by the event loop.

use crate::Interp;

const SOURCE: &str = include_str!("performance.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("Performance prelude must evaluate"));
}
