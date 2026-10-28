//! TypeScript-style type inference over plain JavaScript.
//!
//! Pipeline: JS source -> SWC AST -> this crate infers a type per variable / parameter / return
//! (flow-insensitive, interprocedural inside the file, iterated to a fixpoint, with unions and
//! abstract-allocation-site arrays/objects) -> `annotate` writes the result back as TypeScript
//! annotations (`any` where inference gives up) so the *existing* TS lowering consumes it.
//! Soundness rule: a type may only be narrower than `any` if every runtime value is covered.

mod analyze;
mod builtins;
pub mod bundle;
pub mod clone;
pub mod dts;
pub mod evidence;
mod guard;
mod jsdoc;
mod narrow;
mod prescan;
mod report;
pub mod ty;

pub use prescan::{TRUST_DEFAULT, TRUST_DTS, TRUST_EVIDENCE, TRUST_HINT, TRUST_JSDOC};
pub use report::{analyze_source, annotate, report_json, summary_json, FnReport, Summary};
pub use ty::{Class, Reason};

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// declared types for exported / escaped functions (trusted contract, see `dts`)
    pub dts: Option<std::sync::Arc<dts::DtsIndex>>,
    /// observed calls into the package's API (smoke programs, README examples): trusted contract
    pub evidence: Option<std::sync::Arc<evidence::Evidence>>,
    /// Parameters of functions that escape (exports, callbacks, methods) are guessed from how the
    /// body uses them. Stand-in for `.d.ts` hints: an upper bound, NOT sound on its own.
    pub hint_params: bool,
    /// `annotate` also writes return types (needed by the TS pipeline for method chains)
    pub returns: bool,
    /// `annotate` turns functions whose parameter types rest on a trusted contract into guarded
    /// dispatchers (typed clone behind a runtime check, original body as the fallback)
    pub guards: bool,
    /// `var x;` does not contribute `undefined` (assumes definite assignment). Upper bound.
    pub optimistic_uninit: bool,
    /// `a[i]` is typed as the element type, without `undefined`. Only for code whose element reads
    /// are proven in bounds (AOT clone bodies, see `clone`).
    pub in_bounds_reads: bool,
    /// JSDoc `@param {T}` tags are a trusted contract for the function that follows them (`TRUST_JSDOC`)
    pub jsdoc: bool,
    /// a parameter with a primitive literal default (`n = 10`) is that primitive, behind an entry guard
    /// (`TRUST_DEFAULT`)
    pub default_contract: bool,
    /// top-level names of the program an AOT clone is compiled into (helpers with these names are renamed)
    pub reserved_names: Vec<String>,
}

#[cfg(test)]
mod tests;
