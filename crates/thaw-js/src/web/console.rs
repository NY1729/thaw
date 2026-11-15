//! `Console` (`src/web/console.js`, Node 22 shape) and `console.Console`. The only native is
//! `__thaw_console_format`, which is the same `util.format` the global `console.log` uses.

use crate::{Interp, Value};

const SOURCE: &str = include_str!("console.js");

pub(crate) fn install(interp: &mut Interp) {
    let format = interp.native(|it, _, args| Ok(Value::Str(it.format_log(args).into())));
    interp.global.declare("__thaw_console_format", format, true);
    // The console runs on the AST: its prelude closures are never analysed or compiled (`FuncDef::no_compile`).
    interp.no_compile_define = true;
    let console = interp.run(&format!(
        "(function () {{\n{SOURCE}\nreturn globalConsole;\n}})();"
    ));
    interp.no_compile_define = false;
    let console = console.unwrap_or_else(|_| panic!("Console prelude must evaluate"));
    interp.global.declare("console", console, true);
}
