//! Typed intermediate representation and TypeScript lowering for Thaw.
//! Lowering resolves native layouts and call signatures, normalizes classes
//! and control flow, and represents closures, tagged values, promises, FFI,
//! and dynamic-host operations consumed by LLVM code generation.

#[path = "hir/types.rs"]
mod hir_types;
pub use hir_types::*;

#[path = "hir/ffi.rs"]
mod ffi;
pub use ffi::*;

#[path = "hir/ir.rs"]
mod ir;
pub use ir::*;

#[path = "hir/program.rs"]
mod program;
pub use program::*;

#[path = "hir/closure_analysis.rs"]
mod closure_analysis;
pub use closure_analysis::*;

mod lower;

pub use lower::{
    lower_module, lower_module_with_source_map, normalize_top_level_destructuring, LowerDiagnostic,
    SourceRange,
};
