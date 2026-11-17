//! `node:test` mock timers, the host half (`src/web/test_mock_timers.js`): the `__thaw_test_mock_timers_*` globals the
//! shared node:test wrapper calls. Virtual timers and the mocked `Date` are plain JS over the captured native timers.

use crate::Interp;

const SOURCE: &str = include_str!("test_mock_timers.js");

pub(crate) fn install(interp: &mut Interp) {
    interp
        .run_builtin(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("test mock timers prelude must evaluate"));
}
