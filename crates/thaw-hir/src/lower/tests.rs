use super::*;
use crate::HirType;

fn lower(source: &str) -> HirProgram {
    let module = thaw_parser::parse_typescript(source).expect("parse error");
    lower_module(&module).expect("lowering error")
}

include!("tests/async.rs");
include!("tests/classes.rs");
include!("tests/control_flow.rs");
include!("tests/core.rs");
include!("tests/ffi.rs");
include!("tests/types.rs");
include!("tests/values.rs");
