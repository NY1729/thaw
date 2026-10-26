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

/// Bit pattern of the quiet NaN a class field that was never assigned reads as through a
/// structural live view (`{ n: number }` over an instance); observed reads turn it into `undefined`.
pub const UNSET_NUMBER_BITS: u64 = 0x7FF8_0000_0000_5E7D;

pub use lower::{
    hir_expr_contains_await, jit_calls_requiring_dynamic_storage, lower_module,
    lower_module_with_source_map, normalize_top_level_destructuring,
    reorder_class_fields_by_definition, set_thaw_js_routed, LowerDiagnostic, SourceRange,
};
