//! Closure-compiling evaluator (TJ143). C0 is plumbing only: `analyze` rejects every function with
//! `NotYet`, so any mode produces the same results as `off`. Later PRs fill in the analyzer and the
//! compiled path behind `try_compiled`.

mod check;
#[cfg(test)]
mod tests;

use super::{FuncDef, Interp, Value, R};

/// `THAW_JS_COMPILE=off|on|check`. Unknown values mean `Off`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileMode {
    Off,
    On,
    Check,
}

impl CompileMode {
    pub(crate) fn parse(v: Option<&str>) -> CompileMode {
        match v.map(str::trim) {
            Some("on") => CompileMode::On,
            Some("check") => CompileMode::Check,
            _ => CompileMode::Off,
        }
    }
}

/// Calls before a function is analysed. Missing, zero or non-numeric values mean the default.
pub(crate) const DEFAULT_AFTER: u32 = 2;

pub(crate) fn parse_after(v: Option<&str>) -> u32 {
    v.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_AFTER)
}

/// `THAW_JS_COMPILE_STATS` set to anything but empty or `0` prints one stderr line per rejected function.
pub(crate) fn stats_enabled(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some(s) if !s.is_empty() && s != "0")
}

/// Why a function stays on the AST interpreter. Only `NotYet` exists until the analyzer lands (C1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackReason {
    NotYet,
}

impl FallbackReason {
    /// Number of variants: the size of `Stats::rejected`.
    pub(crate) const COUNT: usize = 1;

    fn name(self) -> &'static str {
        match self {
            FallbackReason::NotYet => "not-yet",
        }
    }
}

/// Per-function lazy state. `Cold(n)`: n calls seen so far. `Never`: analysis rejected it for good.
pub(crate) enum CState {
    Cold(u32),
    Never(FallbackReason),
}

impl Default for CState {
    fn default() -> CState {
        CState::Cold(0)
    }
}

impl CState {
    /// Counts one call. True on the call that reaches `after`, when the analyzer must run.
    pub(crate) fn on_call(&mut self, after: u32) -> bool {
        match self {
            CState::Cold(n) => {
                *n = n.saturating_add(1);
                *n >= after
            }
            CState::Never(_) => false,
        }
    }

    pub(crate) fn reject(&mut self, reason: FallbackReason) {
        *self = CState::Never(reason);
    }
}

/// Counters for `THAW_JS_COMPILE_STATS`, indexed by `FallbackReason`.
#[derive(Default, Debug)]
pub(crate) struct Stats {
    rejected: [u64; FallbackReason::COUNT],
}

impl Stats {
    pub(crate) fn record_rejected(&mut self, reason: FallbackReason) {
        self.rejected[reason as usize] += 1;
    }

    pub(crate) fn rejected(&self, reason: FallbackReason) -> u64 {
        self.rejected[reason as usize]
    }
}

/// Compiled bodies take their statements, slots and captured scope from this (C2 onwards).
/// Only the size asserts below use it now.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Flow {
    Next,
    Break(u16),
    Continue(u16),
    Return,
}

/// Internal error type of compiled closures: boxing `Ctl` keeps `CR<f64>` in two registers.
#[allow(dead_code)]
pub(crate) type CR<T> = Result<T, Box<crate::Ctl>>;

const _: () = assert!(std::mem::size_of::<CR<f64>>() == 16);
const _: () = assert!(std::mem::size_of::<Flow>() <= 4);

/// The analyzer. C0 accepts nothing; C1 adds the blockers and the slot/capture plan.
pub(crate) fn analyze(_def: &FuncDef) -> Result<(), FallbackReason> {
    Err(FallbackReason::NotYet)
}

impl Interp {
    /// A realm with the given compile mode, independent of `THAW_JS_COMPILE`. For tests and embedders.
    pub fn with_compile_mode(mode: CompileMode) -> Interp {
        let mut it = Interp::new();
        it.compile_mode = mode;
        it
    }

    /// Dispatch from `call_closure_body` (the caller checks `compile_mode != Off` first). Returns `Some`
    /// only when a compiled body ran, which C0 never does.
    pub(crate) fn try_compiled(&mut self, def: &FuncDef) -> Option<R<Value>> {
        let mut st = def.cstate.borrow_mut();
        if st.on_call(self.compile_after) {
            // C2 adds the `Ok` path (compile and store `CState::Compiled`).
            if let Err(reason) = analyze(def) {
                st.reject(reason);
                self.cstats.record_rejected(reason);
                if self.compile_stats {
                    let name = def.name.as_deref().unwrap_or("<anonymous>");
                    eprintln!(
                        "thaw-js compile: {name} at {}:{} rejected: {}",
                        def.src.0,
                        def.src.1,
                        reason.name()
                    );
                }
            }
        }
        None
    }
}
