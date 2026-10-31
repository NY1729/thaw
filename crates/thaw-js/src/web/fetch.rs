//! Fetch classes (`Headers`, `FormData`, `Request`, `Response`) and the full web streams
//! (`tee`, `pipeThrough`, `TransformStream`, `TextEncoderStream`, ...), ported from the
//! QuickJS platform globals. Installed before `streams.js`, whose minimal version then no-ops.

use crate::Interp;

const SOURCE: &str = include_str!("fetch.js");

pub(crate) fn install(interp: &mut Interp) {
    if let Err(why) = interp.run(&format!("(function () {{\n{SOURCE}\n}})();")) {
        let why = match why {
            crate::Ctl::Throw(v) => interp.to_str(&v).to_string(),
            crate::Ctl::Unsupported(f) => format!("unsupported {f}"),
            _ => "stray completion".into(),
        };
        panic!("fetch prelude must evaluate: {why}");
    }
}
