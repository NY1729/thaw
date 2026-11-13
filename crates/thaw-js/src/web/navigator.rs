//! `Navigator` / `navigator` (`src/web/navigator.js`). The only native is the host CPU count, which
//! `hardwareConcurrency` reports the way Node does (`os.availableParallelism()`).

use crate::Interp;

const SOURCE: &str = include_str!("navigator.js");

pub(crate) fn install(interp: &mut Interp) {
    let par = interp.native(|_, _, _| {
        let n = std::thread::available_parallelism().map_or(1, |n| n.get());
        Ok(crate::Value::Number(n as f64))
    });
    interp
        .global
        .declare("__thaw_available_parallelism", par, true);
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("Navigator prelude must evaluate"));
}
