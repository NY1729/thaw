//! TypeScript-style type inference over plain JavaScript.
//!
//! Pipeline: JS source -> SWC AST -> this crate infers a type per variable / parameter / return
//! (flow-insensitive, interprocedural inside the file, iterated to a fixpoint, with unions and
//! abstract-allocation-site arrays/objects) -> `annotate` writes the result back as TypeScript
//! annotations (`any` where inference gives up) so the *existing* TS lowering consumes it.
//! Soundness rule: a type may only be narrower than `any` if every runtime value is covered.

mod analyze;
pub mod dts;
mod narrow;
mod prescan;
mod report;
pub mod ty;

pub use report::{analyze_source, annotate, FnReport, Summary};
pub use ty::{Class, Reason};

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// declared types for exported / escaped functions (trusted contract, see `dts`)
    pub dts: Option<std::sync::Arc<dts::DtsIndex>>,
    /// Parameters of functions that escape (exports, callbacks, methods) are guessed from how the
    /// body uses them. Stand-in for `.d.ts` hints: an upper bound, NOT sound on its own.
    pub hint_params: bool,
    /// `annotate` also writes return types (needed by the TS pipeline for method chains)
    pub returns: bool,
    /// `var x;` does not contribute `undefined` (assumes definite assignment). Upper bound.
    pub optimistic_uninit: bool,
}

#[cfg(test)]
mod tests;
