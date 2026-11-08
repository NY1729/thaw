//! Closure-compiling evaluator (TJ143). C1 adds the analyzer (`analyze.rs`) and the coverage stats. Nothing
//! is compiled yet: `try_compiled` always returns `None`, so every mode runs the AST interpreter.

mod analyze;
#[cfg(test)]
mod analyze_tests;
mod check;
#[cfg(test)]
mod tests;

use super::{FuncDef, Interp, Value, R};

pub(crate) use analyze::analyze;

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

/// `THAW_JS_COMPILE_STATS` set to anything but empty or `0` enables the stats.
pub(crate) fn stats_enabled(v: Option<&str>) -> bool {
    stats_level(v) > 0
}

/// `THAW_JS_COMPILE_STATS`: 0 off; 1 the per-reason report at exit and one line per rejected function;
/// 2 also one line per function the analyzer accepts (the functions that WOULD compile).
pub(crate) fn stats_level(v: Option<&str>) -> u8 {
    match v.map(str::trim) {
        None | Some("") | Some("0") => 0,
        Some("2") => 2,
        Some(_) => 1,
    }
}

/// Why a function stays on the AST interpreter. The blocker of highest `priority` is the verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FallbackReason {
    /// Calls and `new`: the C3 call path is not compiled yet.
    NotYet,
    // Permanent (plan 1.3, 1.7): the function stays on the AST interpreter.
    With,
    DirectEval,
    FunctionCtor,
    Super,
    PrivateName,
    ClassBody,
    ClassConstructor,
    AnnexBBlockFunction,
    LabeledFunction,
    ImportMeta,
    DynamicImport,
    ProtoLiteralKey,
    Unsupported,
    SwitchLexical,
    /// Tier A (plan 2.1): a lexical slot read before its declaration can run.
    ReadBeforeDecl,
    // Staged: a later PR lifts each one (plan section 5).
    InnerFunction,
    SelfBinding,
    Arguments,
    NewTarget,
    Generator,
    Async,
    Destructuring,
    DefaultParam,
    RestParam,
    Spread,
    OptionalChain,
    TaggedTemplate,
    RegexLiteral,
    ObjectMethod,
    Try,
    Throw,
    Switch,
    Labels,
    ForIn,
    ForOf,
    ForAwait,
    Delete,
    InstanceOfIn,
}

impl FallbackReason {
    /// Every variant in declaration order: `Stats` indexes its counters by `reason as usize`.
    pub(crate) const ALL: &'static [FallbackReason] = &[
        FallbackReason::NotYet,
        FallbackReason::With,
        FallbackReason::DirectEval,
        FallbackReason::FunctionCtor,
        FallbackReason::Super,
        FallbackReason::PrivateName,
        FallbackReason::ClassBody,
        FallbackReason::ClassConstructor,
        FallbackReason::AnnexBBlockFunction,
        FallbackReason::LabeledFunction,
        FallbackReason::ImportMeta,
        FallbackReason::DynamicImport,
        FallbackReason::ProtoLiteralKey,
        FallbackReason::Unsupported,
        FallbackReason::SwitchLexical,
        FallbackReason::ReadBeforeDecl,
        FallbackReason::InnerFunction,
        FallbackReason::SelfBinding,
        FallbackReason::Arguments,
        FallbackReason::NewTarget,
        FallbackReason::Generator,
        FallbackReason::Async,
        FallbackReason::Destructuring,
        FallbackReason::DefaultParam,
        FallbackReason::RestParam,
        FallbackReason::Spread,
        FallbackReason::OptionalChain,
        FallbackReason::TaggedTemplate,
        FallbackReason::RegexLiteral,
        FallbackReason::ObjectMethod,
        FallbackReason::Try,
        FallbackReason::Throw,
        FallbackReason::Switch,
        FallbackReason::Labels,
        FallbackReason::ForIn,
        FallbackReason::ForOf,
        FallbackReason::ForAwait,
        FallbackReason::Delete,
        FallbackReason::InstanceOfIn,
    ];
    pub(crate) const COUNT: usize = Self::ALL.len();

    /// Higher wins; of equal priority the first one found wins. `ReadBeforeDecl` outranks the other
    /// permanent blockers, so the tier A programs of plan 2.1 report it even when they also contain a
    /// switch-case declaration.
    pub(crate) fn priority(self) -> u8 {
        match self {
            FallbackReason::ReadBeforeDecl => 3,
            FallbackReason::With
            | FallbackReason::DirectEval
            | FallbackReason::FunctionCtor
            | FallbackReason::Super
            | FallbackReason::PrivateName
            | FallbackReason::ClassBody
            | FallbackReason::ClassConstructor
            | FallbackReason::AnnexBBlockFunction
            | FallbackReason::LabeledFunction
            | FallbackReason::ImportMeta
            | FallbackReason::DynamicImport
            | FallbackReason::ProtoLiteralKey
            | FallbackReason::Unsupported
            | FallbackReason::SwitchLexical => 2,
            _ => 1,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            FallbackReason::NotYet => "calls-and-new",
            FallbackReason::With => "with",
            FallbackReason::DirectEval => "direct-eval",
            FallbackReason::FunctionCtor => "function-ctor",
            FallbackReason::Super => "super",
            FallbackReason::PrivateName => "private-name",
            FallbackReason::ClassBody => "class-body",
            FallbackReason::ClassConstructor => "class-constructor",
            FallbackReason::AnnexBBlockFunction => "annex-b-block-function",
            FallbackReason::LabeledFunction => "labeled-function",
            FallbackReason::ImportMeta => "import-meta",
            FallbackReason::DynamicImport => "dynamic-import",
            FallbackReason::ProtoLiteralKey => "proto-literal-key",
            FallbackReason::Unsupported => "unsupported",
            FallbackReason::SwitchLexical => "switch-lexical",
            FallbackReason::ReadBeforeDecl => "read-before-decl",
            FallbackReason::InnerFunction => "inner-function",
            FallbackReason::SelfBinding => "self-binding",
            FallbackReason::Arguments => "arguments",
            FallbackReason::NewTarget => "new-target",
            FallbackReason::Generator => "generator",
            FallbackReason::Async => "async",
            FallbackReason::Destructuring => "destructuring",
            FallbackReason::DefaultParam => "default-param",
            FallbackReason::RestParam => "rest-param",
            FallbackReason::Spread => "spread",
            FallbackReason::OptionalChain => "optional-chain",
            FallbackReason::TaggedTemplate => "tagged-template",
            FallbackReason::RegexLiteral => "regex-literal",
            FallbackReason::ObjectMethod => "object-method",
            FallbackReason::Try => "try",
            FallbackReason::Throw => "throw",
            FallbackReason::Switch => "switch",
            FallbackReason::Labels => "labels",
            FallbackReason::ForIn => "for-in",
            FallbackReason::ForOf => "for-of",
            FallbackReason::ForAwait => "for-await",
            FallbackReason::Delete => "delete",
            FallbackReason::InstanceOfIn => "in-instanceof",
        }
    }
}

/// Per-function lazy state: `Cold(n)` counts calls; the analysis then accepts the function (`Eligible`)
/// or rejects it for good (`Never`). Compiled bodies arrive in C2.
pub(crate) enum CState {
    Cold(u32),
    Eligible,
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
            CState::Eligible | CState::Never(_) => false,
        }
    }

    pub(crate) fn reject(&mut self, reason: FallbackReason) {
        *self = CState::Never(reason);
    }
}

/// Counters for `THAW_JS_COMPILE_STATS`. Functions are counted once, when analysed; calls are counted on
/// every call while the mode is not `Off`.
#[derive(Debug)]
pub(crate) struct Stats {
    level: u8,
    /// Functions analysed and accepted.
    eligible: u64,
    /// Functions analysed and rejected, by their verdict.
    rejected: [u64; FallbackReason::COUNT],
    calls: u64,
    calls_eligible: u64,
    calls_rejected: [u64; FallbackReason::COUNT],
}

impl Default for Stats {
    fn default() -> Stats {
        Stats::with_level(0)
    }
}

impl Stats {
    pub(crate) fn with_level(level: u8) -> Stats {
        Stats {
            level,
            eligible: 0,
            rejected: [0; FallbackReason::COUNT],
            calls: 0,
            calls_eligible: 0,
            calls_rejected: [0; FallbackReason::COUNT],
        }
    }

    pub(crate) fn level(&self) -> u8 {
        self.level
    }

    pub(crate) fn record_rejected(&mut self, reason: FallbackReason) {
        self.rejected[reason as usize] += 1;
    }

    pub(crate) fn record_eligible(&mut self) {
        self.eligible += 1;
    }

    #[cfg(test)]
    pub(crate) fn rejected(&self, reason: FallbackReason) -> u64 {
        self.rejected[reason as usize]
    }

    #[cfg(test)]
    pub(crate) fn eligible(&self) -> u64 {
        self.eligible
    }

    #[cfg(test)]
    pub(crate) fn calls(&self) -> u64 {
        self.calls
    }

    #[cfg(test)]
    pub(crate) fn calls_eligible(&self) -> u64 {
        self.calls_eligible
    }

    #[cfg(test)]
    pub(crate) fn calls_rejected(&self, reason: FallbackReason) -> u64 {
        self.calls_rejected[reason as usize]
    }

    /// The per-reason report; empty when no function was called or analysed.
    pub(crate) fn report(&self) -> String {
        let analysed = self.eligible + self.rejected.iter().sum::<u64>();
        if self.calls == 0 && analysed == 0 {
            return String::new();
        }
        let pct = if self.calls == 0 {
            0.0
        } else {
            100.0 * self.calls_eligible as f64 / self.calls as f64
        };
        let mut out = format!(
            "thaw-js compile: calls={} eligible-calls={} ({pct:.1}%) functions analysed={analysed} eligible={}\n",
            self.calls, self.calls_eligible, self.eligible
        );
        for (i, reason) in FallbackReason::ALL.iter().enumerate() {
            if self.rejected[i] > 0 || self.calls_rejected[i] > 0 {
                out += &format!(
                    "  {:<24} functions={:<6} calls={}\n",
                    reason.name(),
                    self.rejected[i],
                    self.calls_rejected[i]
                );
            }
        }
        out
    }
}

impl Drop for Stats {
    /// The exit-time report. `Interp` owns the stats, so dropping the realm prints them (when enabled).
    fn drop(&mut self) {
        if self.level > 0 {
            eprint!("{}", self.report());
        }
    }
}

/// Compiled bodies take their statements, slots and captured scope from this (C2 onwards).
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

impl Interp {
    /// A realm with the given compile mode, independent of `THAW_JS_COMPILE`. For tests and embedders.
    pub fn with_compile_mode(mode: CompileMode) -> Interp {
        let mut it = Interp::new();
        it.compile_mode = mode;
        it
    }

    /// Dispatch from `call_closure_body` (the caller checks `compile_mode != Off` first). Always `None` in
    /// C1: the analysis only feeds the stats.
    pub(crate) fn try_compiled(&mut self, def: &FuncDef) -> Option<R<Value>> {
        let mut st = def.cstate.borrow_mut();
        self.cstats.calls += 1;
        if st.on_call(self.compile_after) {
            let plan = analyze(def);
            let name = def.name.as_deref().unwrap_or("<anonymous>");
            match plan.verdict() {
                Ok(()) => {
                    *st = CState::Eligible;
                    self.cstats.record_eligible();
                    if self.compile_stats && self.cstats.level() >= 2 {
                        eprintln!(
                            "thaw-js compile: {name} at {}:{}-{} would compile",
                            def.src.0, def.src.1, def.src.2
                        );
                    }
                }
                Err(reason) => {
                    st.reject(reason);
                    self.cstats.record_rejected(reason);
                    if self.compile_stats && self.cstats.level() >= 1 {
                        eprintln!(
                            "thaw-js compile: {name} at {}:{} rejected: {}",
                            def.src.0,
                            def.src.1,
                            reason.name()
                        );
                    }
                }
            }
        }
        match &*st {
            CState::Eligible => self.cstats.calls_eligible += 1,
            CState::Never(r) => self.cstats.calls_rejected[*r as usize] += 1,
            CState::Cold(_) => {}
        }
        None
    }
}
