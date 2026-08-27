//! Thaw's typed intermediate representation. Definitions here mirror the
//! design doc (section 6) plus the small amount of scaffolding (`HirStmt`,
//! `HirParam`, `BinOp`, `FfiSignature`) that section only sketched.
//!
//! Phase 0 only *populates* a thin slice of this shape (see `lower.rs`):
//! top-level functions with number/string/boolean/void params and returns,
//! literals, identifiers, binary ops, and calls. The rest of the enum
//! (`Promise`, `Union`, `FfiCall`, `DynamicCall`, ...) exists so the shape
//! matches the target design, but nothing constructs those variants yet.

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

mod lower;

pub use lower::{
    lower_module, lower_module_with_source_map, normalize_top_level_destructuring, LowerDiagnostic,
    SourceRange,
};
