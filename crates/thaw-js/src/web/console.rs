//! `Console` (`src/web/console.js`, Node 22 shape) and `console.Console`. The only native is
//! `__thaw_console_format`, which is the same `util.format` the global `console.log` uses.

use crate::{Interp, Value};

// A constant, so the text has one address and a vm realm can share its parse (`Interp::run_prelude`).
const WRAPPED: &str = concat!(
    "(function () {\n",
    include_str!("console.js"),
    "\nreturn globalConsole;\n})();"
);

pub(crate) fn install(interp: &mut Interp) {
    let format = interp.native(|it, _, args| Ok(Value::Str(it.format_log(args).into())));
    interp.global.declare("__thaw_console_format", format, true);
    // The console runs on the AST: its prelude closures are never analysed or compiled (`FuncDef::no_compile`).
    interp.no_compile_define = true;
    let console = interp.run_prelude(WRAPPED);
    interp.no_compile_define = false;
    let console = console.unwrap_or_else(|_| panic!("Console prelude must evaluate"));
    interp.global.declare("console", console, true);
}
