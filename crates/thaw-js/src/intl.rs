//! `Intl` for thaw-js (cargo feature `intl`): the same `platform_globals/intl.js` polyfill thaw-quickjs
//! runs, over the same icu4x natives (`thaw_intl::js_native`, a leaf crate with no thaw-runtime/std dep).

use crate::{arg, Interp, Value};

const POLYFILL: &str = include_str!("../../thaw-quickjs/src/quickjs/platform_globals/intl.js");

impl Interp {
    pub(crate) fn install_intl(&mut self) {
        std::hint::black_box(thaw_intl::keep_c_exports());
        let g = self.global.clone();
        for &name in thaw_intl::JS_NATIVES {
            let f = self.native(move |it, _, a| {
                let strs: Vec<String> = (0..a.len())
                    .map(|i| it.to_str(&arg(a, i)).to_string())
                    .collect();
                let out = thaw_intl::js_native(name, &strs).unwrap_or_default();
                Ok(match name {
                    "currency_fraction_digits" if out.is_empty() => Value::Undefined,
                    "currency_fraction_digits" | "collator_compare" => {
                        Value::Number(out.parse::<f64>().unwrap_or(0.0))
                    }
                    _ => Value::str(&out),
                })
            });
            g.declare(&format!("__thaw_intl_{name}"), f, true);
        }
        self.run(&format!("(function () {{\n{POLYFILL}"))
            .unwrap_or_else(|_| panic!("thaw-js Intl polyfill must evaluate"));
    }
}
