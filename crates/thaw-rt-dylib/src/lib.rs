//! `libthaw_rt.so`: the Thaw runtime as one shared object. The only public
//! API is the C ABI in `cabi`; everything else is exported for generated code
//! only (see `thaw_rt.map`).
pub mod cabi;

// Keep the runtime crates in the link even though this crate names none of
// their items directly.
pub use thaw_arena as _arena;
pub use thaw_jit as _jit;
pub use thaw_runtime as _runtime;
pub use thaw_std as _std;
