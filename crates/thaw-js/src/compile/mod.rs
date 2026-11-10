//! Closure-compiling evaluator (TJ143). C1 added the analyzer (`analyze.rs`) and the coverage stats. C2 PR1
//! (plan TJ150) adds the plumbing and compiles the straight-line leaf subset (`gen.rs`, `expr.rs`,
//! `stmt.rs`, `run.rs`). Any function outside the subset stays on the AST interpreter.

mod analyze;
#[cfg(test)]
mod analyze_scope_tests;
#[cfg(test)]
mod analyze_tests;
#[cfg(test)]
mod blocker_stats_tests;
mod call;
mod check;
#[cfg(test)]
mod check_mode_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod diff_tests_args;
#[cfg(test)]
mod diff_tests_calls;
#[cfg(test)]
mod diff_tests_ctl;
#[cfg(test)]
mod diff_tests_direct;
#[cfg(test)]
mod diff_tests_elem;
#[cfg(test)]
mod diff_tests_elemw;
#[cfg(test)]
mod diff_tests_inner;
#[cfg(test)]
mod diff_tests_inre;
#[cfg(test)]
mod diff_tests_mem;
#[cfg(test)]
mod diff_tests_num2;
#[cfg(test)]
mod diff_tests_self;
#[cfg(test)]
mod diff_tests_switch;
#[cfg(test)]
mod diff_tests_throw;
mod elem;
mod expr;
#[cfg(test)]
mod fuzz_tests;
mod gen;
mod member;
mod num;
#[cfg(test)]
mod num_tests;
#[cfg(test)]
mod pool_tests;
mod run;
mod stmt;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use super::{Ctl, FuncDef, Interp, Scope, Value, R};

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
    /// Calls and `new`. No longer produced (C3 compiles them through `call.rs`); kept for its name in the stats.
    #[allow(dead_code)]
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
    /// A captured name declared inside a nested block (`for (let ...)`, `{ let }`, `catch`). Blocker bit only
    /// (lowest priority): it is never the verdict.
    ScopedBlock,
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
    /// No longer produced (TJ156 compiles `throw`); kept for its name in the stats.
    #[allow(dead_code)]
    Throw,
    /// No longer produced (TJ156 compiles `switch`); kept for its name in the stats.
    #[allow(dead_code)]
    Switch,
    Labels,
    ForIn,
    ForOf,
    ForAwait,
    Delete,
    InstanceOfIn,
}

const _: () = assert!(FallbackReason::COUNT <= 64);

impl FallbackReason {
    /// This reason's bit in a blocker mask (`FnPlan::blockers`).
    pub(crate) fn bit(self) -> u64 {
        1u64 << (self as u32)
    }

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
        FallbackReason::ScopedBlock,
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
            FallbackReason::ScopedBlock => 0,
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
            FallbackReason::ScopedBlock => "scoped-block",
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
/// or rejects it for good (`Never`). An accepted function is `Compiled` when code generation supports it;
/// `Eligible` with no code means the analyzer accepted it but codegen did not (it runs on the AST).
pub(crate) enum CState {
    Cold(u32),
    Eligible,
    Compiled(Rc<CompiledFn>),
    /// Rejected with the first blocker only (no blocker mask).
    #[cfg_attr(not(test), allow(dead_code))] // the analyzer path uses `Blocked`
    Never(FallbackReason),
    /// Rejected: the first blocker, and the mask of every blocker (`FnPlan::blockers`).
    Blocked(FallbackReason, u64),
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
            CState::Eligible | CState::Compiled(_) | CState::Never(_) | CState::Blocked(..) => {
                false
            }
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // the analyzer path uses `reject_with_blockers`
    pub(crate) fn reject(&mut self, reason: FallbackReason) {
        *self = CState::Never(reason);
    }

    pub(crate) fn reject_with_blockers(&mut self, reason: FallbackReason, blockers: u64) {
        *self = CState::Blocked(reason, blockers);
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
    /// Functions whose code was generated.
    compiled: u64,
    /// Calls that ran compiled code (not the AST).
    compiled_calls: u64,
    /// Analyzer-accepted functions that codegen refused, by the reason (always `Unsupported` in C2 PR1).
    codegen_rejected: [u64; FallbackReason::COUNT],
    /// Test-only: calls that took the `Interp::call_direct` fast path.
    #[cfg(test)]
    direct_calls: u64,
    /// Test-only: compiled call sites that took a recycled argument buffer (`call.rs` `eval_args`).
    #[cfg(test)]
    arg_pool_hits: u64,
    /// Test-only: bitwise operator nodes built (`num::bit_ex`), summed over compiled functions.
    #[cfg(test)]
    bit_ex_built: u64,
    /// Test-only: run-time hits of the Number-Number fast path of a bitwise or fused numeric node.
    #[cfg(test)]
    nm_fast_hits: u64,
    /// Test-only: element-read nodes built (`elem.rs`), summed over compiled functions.
    #[cfg(test)]
    elem_built: u64,
    /// Test-only: run-time hits of the element fast path (a Number element of an Array or typed array).
    #[cfg(test)]
    elem_fast: u64,
    /// Test-only: run-time element reads that took the interpreter helper.
    #[cfg(test)]
    elem_slow: u64,
    /// Test-only: run-time element stores that took the plain-store fast path (`elem.rs` `fast_set`).
    #[cfg(test)]
    elem_store_fast: u64,
    /// Test-only: run-time element stores that took `set_index` / `set`.
    #[cfg(test)]
    elem_store_slow: u64,
    /// Rejected functions that have the reason among their blockers (any), and those whose only
    /// blocker it is (sole). The first-blocker count is `rejected`.
    any_functions: [u64; FallbackReason::COUNT],
    sole_functions: [u64; FallbackReason::COUNT],
    /// Calls of rejected functions, attributed to every blocker (any) and to the sole blocker (sole).
    calls_any: [u64; FallbackReason::COUNT],
    calls_sole: [u64; FallbackReason::COUNT],
    /// Per-call `Scope`s built for compiled inner functions (always 0 until the closure step lands).
    scope_frames: u64,
    /// Compiled closure creations (always 0 until the closure step lands).
    closures_created: u64,
    /// Per-call `Scope`s still unshared when their call returned (always 0 until the closure step lands).
    scope_unique_at_exit: u64,
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
            compiled: 0,
            compiled_calls: 0,
            codegen_rejected: [0; FallbackReason::COUNT],
            #[cfg(test)]
            direct_calls: 0,
            #[cfg(test)]
            arg_pool_hits: 0,
            #[cfg(test)]
            bit_ex_built: 0,
            #[cfg(test)]
            nm_fast_hits: 0,
            #[cfg(test)]
            elem_built: 0,
            #[cfg(test)]
            elem_fast: 0,
            #[cfg(test)]
            elem_slow: 0,
            #[cfg(test)]
            elem_store_fast: 0,
            #[cfg(test)]
            elem_store_slow: 0,
            any_functions: [0; FallbackReason::COUNT],
            sole_functions: [0; FallbackReason::COUNT],
            calls_any: [0; FallbackReason::COUNT],
            calls_sole: [0; FallbackReason::COUNT],
            scope_frames: 0,
            closures_created: 0,
            scope_unique_at_exit: 0,
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

    /// Counts a rejected function's blocker mask: each blocker is an "any", and the only one is a "sole".
    pub(crate) fn record_blockers(&mut self, mask: u64) {
        Self::add_mask(&mut self.any_functions, &mut self.sole_functions, mask);
    }

    /// Counts one call of a rejected function, with its blocker mask.
    pub(crate) fn note_blocked_call(&mut self, mask: u64) {
        Self::add_mask(&mut self.calls_any, &mut self.calls_sole, mask);
    }

    fn add_mask(any: &mut [u64], sole: &mut [u64], mask: u64) {
        for (i, r) in FallbackReason::ALL.iter().enumerate() {
            if mask & r.bit() != 0 {
                any[i] += 1;
            }
        }
        if mask.count_ones() == 1 {
            sole[mask.trailing_zeros() as usize] += 1;
        }
    }

    #[cfg(test)]
    pub(crate) fn any_functions(&self, reason: FallbackReason) -> u64 {
        self.any_functions[reason as usize]
    }

    #[cfg(test)]
    pub(crate) fn sole_functions(&self, reason: FallbackReason) -> u64 {
        self.sole_functions[reason as usize]
    }

    #[cfg(test)]
    pub(crate) fn calls_any(&self, reason: FallbackReason) -> u64 {
        self.calls_any[reason as usize]
    }

    #[cfg(test)]
    pub(crate) fn calls_sole(&self, reason: FallbackReason) -> u64 {
        self.calls_sole[reason as usize]
    }

    pub(crate) fn record_codegen_rejected(&mut self, reason: FallbackReason) {
        self.codegen_rejected[reason as usize] += 1;
    }

    #[cfg(test)]
    pub(crate) fn rejected(&self, reason: FallbackReason) -> u64 {
        self.rejected[reason as usize]
    }

    #[cfg(test)]
    pub(crate) fn compiled(&self) -> u64 {
        self.compiled
    }

    #[cfg(test)]
    pub(crate) fn compiled_calls(&self) -> u64 {
        self.compiled_calls
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
    pub(crate) fn direct_calls(&self) -> u64 {
        self.direct_calls
    }

    #[cfg(test)]
    pub(crate) fn note_direct_call(&mut self) {
        self.direct_calls += 1;
    }

    #[cfg(test)]
    pub(crate) fn arg_pool_hits(&self) -> u64 {
        self.arg_pool_hits
    }

    #[cfg(test)]
    pub(crate) fn note_arg_pool_hit(&mut self) {
        self.arg_pool_hits += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_direct_call(&mut self) {}

    #[cfg(test)]
    pub(crate) fn bit_ex_built(&self) -> u64 {
        self.bit_ex_built
    }

    #[cfg(test)]
    pub(crate) fn nm_fast_hits(&self) -> u64 {
        self.nm_fast_hits
    }

    #[cfg(test)]
    pub(crate) fn note_nm_fast(&mut self) {
        self.nm_fast_hits += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_nm_fast(&mut self) {}

    #[cfg(test)]
    pub(crate) fn note_bit_ex_built(&mut self, n: u64) {
        self.bit_ex_built += n;
    }

    #[cfg(test)]
    pub(crate) fn note_elem_built(&mut self, n: u64) {
        self.elem_built += n;
    }

    #[cfg(test)]
    pub(crate) fn elem_built(&self) -> u64 {
        self.elem_built
    }

    #[cfg(test)]
    pub(crate) fn elem_fast(&self) -> u64 {
        self.elem_fast
    }

    #[cfg(test)]
    pub(crate) fn elem_slow(&self) -> u64 {
        self.elem_slow
    }

    #[cfg(test)]
    pub(crate) fn note_elem_fast(&mut self) {
        self.elem_fast += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_elem_fast(&mut self) {}

    #[cfg(test)]
    pub(crate) fn note_elem_slow(&mut self) {
        self.elem_slow += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_elem_slow(&mut self) {}

    #[cfg(test)]
    pub(crate) fn elem_store_fast(&self) -> u64 {
        self.elem_store_fast
    }

    #[cfg(test)]
    pub(crate) fn elem_store_slow(&self) -> u64 {
        self.elem_store_slow
    }

    #[cfg(test)]
    pub(crate) fn note_elem_store_fast(&mut self) {
        self.elem_store_fast += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_elem_store_fast(&mut self) {}

    #[cfg(test)]
    pub(crate) fn note_elem_store_slow(&mut self) {
        self.elem_store_slow += 1;
    }

    #[cfg(not(test))]
    #[inline]
    pub(crate) fn note_elem_store_slow(&mut self) {}

    #[cfg(test)]
    pub(crate) fn scope_frames(&self) -> u64 {
        self.scope_frames
    }

    #[cfg(test)]
    pub(crate) fn closures_created(&self) -> u64 {
        self.closures_created
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
        if self.compiled > 0 || self.codegen_rejected.iter().any(|&n| n > 0) {
            out += &format!(
                "thaw-js compile: compiled={} compiled-calls={}\n",
                self.compiled, self.compiled_calls
            );
            for (i, reason) in FallbackReason::ALL.iter().enumerate() {
                if self.codegen_rejected[i] > 0 {
                    out += &format!(
                        "  codegen {:<17} functions={}\n",
                        reason.name(),
                        self.codegen_rejected[i]
                    );
                }
            }
        }
        // Appended after the lines above, and only when a rejected function has blockers to show.
        let blocker_rows: Vec<usize> = (0..FallbackReason::COUNT)
            .filter(|&i| self.any_functions[i] > 0 || self.calls_any[i] > 0)
            .collect();
        if !blocker_rows.is_empty() {
            out += "thaw-js compile blockers: first = first blocker (functions, calls), any = a blocker, sole = the only blocker\n";
            for i in blocker_rows {
                out += &format!(
                    "  {:<24} functions first={:<6} any={:<6} sole={:<6} calls first={:<8} any={:<8} sole={}\n",
                    FallbackReason::ALL[i].name(),
                    self.rejected[i],
                    self.any_functions[i],
                    self.sole_functions[i],
                    self.calls_rejected[i],
                    self.calls_any[i],
                    self.calls_sole[i]
                );
            }
        }
        if self.scope_frames + self.closures_created + self.scope_unique_at_exit > 0 {
            out += &format!(
                "thaw-js compile scopes: frames={} closures={} unique-at-exit={}\n",
                self.scope_frames, self.closures_created, self.scope_unique_at_exit
            );
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

/// Completion of a compiled statement. Only unlabeled `break`/`continue` inside a loop are produced (`0`).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Flow {
    Next,
    Break(u16),
    Continue(u16),
    Return,
}

/// Internal error type of compiled closures: boxing `Ctl` keeps `CR<f64>` in two registers.
pub(crate) type CR<T> = Result<T, Box<Ctl>>;

const _: () = assert!(std::mem::size_of::<CR<f64>>() == 16);
const _: () = assert!(std::mem::size_of::<Flow>() <= 4);

/// A compiled expression: one closure per AST node, evaluated against the frame of one call.
pub(crate) type Ex = Box<dyn Fn(&mut Interp, &mut Frame) -> CR<Value>>;
/// A compiled statement.
pub(crate) type St = Box<dyn Fn(&mut Interp, &mut Frame) -> CR<Flow>>;

/// The locals of one call. Declared names live in `slots`; everything else resolves through `scope`.
pub(crate) struct Frame {
    /// Parameters first, then `var`s and block-level `let`/`const` (see `gen.rs`).
    pub(crate) slots: Vec<Value>,
    /// The scope free names resolve from: the closure's scope, or the call's own `Scope` when `frame` is set.
    pub(crate) scope: Rc<Scope>,
    /// The call's `this`, already sloppy-normalised (non-arrow functions only).
    pub(crate) this: Value,
    /// Set by `return`, read by `CompiledFn::run`.
    pub(crate) ret: Value,
}

/// The per-call `Scope` of a function with inner closures (`FnPlan::needs_scope`). It holds the names the
/// closures can reach (the captured parameters and `var`s, which are not slots), and the `this` of arrows.
pub(crate) struct FrameInit {
    /// The non-arrow `this` is bound in the `Scope` (an inner arrow reads it through the chain).
    pub(crate) set_this: bool,
    /// Captured parameters: (argument index, interned name), in argument order.
    pub(crate) params: Vec<(usize, Rc<str>)>,
    /// Captured `var`s: declared with `declare_if_absent` after the parameters, as `call_closure_ast` does.
    pub(crate) vars: Vec<Rc<str>>,
}

/// A compiled function body. `run` is in `run.rs`.
pub(crate) struct CompiledFn {
    pub(crate) nslots: u32,
    /// The slot of the function's own name (a named function), filled with the callee on every call.
    pub(crate) self_slot: Option<u32>,
    /// Per parameter position, the slot of that parameter name (a duplicate name shares its slot).
    pub(crate) param_slots: Vec<Option<u32>>,
    /// Block body statements (empty for an expression body).
    pub(crate) body: Vec<St>,
    /// Arrow functions with an expression body.
    pub(crate) expr_body: Option<Ex>,
    /// Set when the function creates inner closures that need the call's own `Scope`.
    pub(crate) frame: Option<FrameInit>,
    pub(crate) arrow: bool,
    pub(crate) uses_this: bool,
    pub(crate) strict: bool,
    /// The analyzer's `pure_kernel` verdict: the functions `check` mode compares (plan 6).
    pub(crate) pure_kernel: bool,
    /// Test-only: bitwise operator nodes this function's codegen built.
    #[cfg(test)]
    pub(crate) bit_ex_built: u64,
    /// Test-only: element-read nodes this function's codegen built.
    #[cfg(test)]
    pub(crate) elem_built: u64,
}

/// Compiled and interpreted `Ctl` are the same error: this unboxes a compiled error back to `R`.
pub(crate) fn unbox<T>(r: CR<T>) -> R<T> {
    r.map_err(|b| *b)
}

/// The inverse of `unbox`, for every call into an interpreter helper.
pub(crate) fn bx<T>(r: R<T>) -> CR<T> {
    r.map_err(Box::new)
}

impl Interp {
    /// A realm for running programs: the closure compiler is on unless `THAW_JS_COMPILE` says otherwise
    /// (`off`, `check`). `Interp::new()` keeps `Off` as its default so embedders and tests are unchanged.
    pub fn runtime() -> Interp {
        let mut it = Interp::new();
        // unset or empty means the shipped default (a workflow cannot unset a variable, only blank it)
        if std::env::var("THAW_JS_COMPILE").map_or(true, |v| v.trim().is_empty()) {
            it.compile_mode = CompileMode::On;
        }
        it
    }

    /// A realm with the given compile mode, independent of `THAW_JS_COMPILE`. For tests and embedders.
    pub fn with_compile_mode(mode: CompileMode) -> Interp {
        let mut it = Interp::new();
        it.compile_mode = mode;
        it
    }

    /// Dispatch from `call_closure_body` (the caller checks `compile_mode != Off` first). Returns `None` when
    /// the call must run on the AST: the function is not compiled, it is a constructor call, or it was
    /// created inside a `with` (its scope chain has a dynamic object record).
    pub(crate) fn try_compiled(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
        new_target: &Option<Value>,
    ) -> Option<R<Value>> {
        let mut st = def.cstate.borrow_mut();
        self.cstats.calls += 1;
        if st.on_call(self.compile_after) {
            let plan = analyze(def);
            let name = def.name.as_deref().unwrap_or("<anonymous>");
            match plan.verdict() {
                Ok(()) => {
                    self.cstats.record_eligible();
                    // A function created inside `with` never compiles: its scope chain is dynamic.
                    // Codegen refusals stay `Eligible` (no code, AST path) and are never retried.
                    *st = CState::Eligible;
                    if !scope.in_with {
                        let compiled = gen::compile(def, &plan, &mut |n| self.intern(n));
                        match compiled {
                            Ok(c) => {
                                #[cfg(test)]
                                self.cstats.note_bit_ex_built(c.bit_ex_built);
                                #[cfg(test)]
                                self.cstats.note_elem_built(c.elem_built);
                                *st = CState::Compiled(Rc::new(c));
                                self.cstats.compiled += 1;
                                if self.compile_stats && self.cstats.level() >= 2 {
                                    eprintln!(
                                        "thaw-js compile: {name} at {}:{}-{} compiled",
                                        def.src.0, def.src.1, def.src.2
                                    );
                                }
                            }
                            Err(r) => self.cstats.record_codegen_rejected(r),
                        }
                    }
                    if self.compile_stats && self.cstats.level() >= 2 {
                        eprintln!(
                            "thaw-js compile: {name} at {}:{}-{} would compile",
                            def.src.0, def.src.1, def.src.2
                        );
                    }
                }
                Err(reason) => {
                    st.reject_with_blockers(reason, plan.blockers);
                    self.cstats.record_rejected(reason);
                    self.cstats.record_blockers(plan.blockers);
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
            CState::Eligible | CState::Compiled(_) => self.cstats.calls_eligible += 1,
            CState::Never(r) => self.cstats.calls_rejected[*r as usize] += 1,
            CState::Blocked(r, mask) => {
                self.cstats.calls_rejected[*r as usize] += 1;
                self.cstats.note_blocked_call(*mask);
            }
            CState::Cold(_) => {}
        }
        // a constructor call runs compiled unless the body reads `new.target` (the analyzer blocks that anyway);
        // `class` bodies stay on the AST
        if (new_target.is_some() && def.uses_new_target) || def.class.is_some() {
            return None;
        }
        let CState::Compiled(c) = &*st else {
            return None;
        };
        let c = c.clone();
        // the borrow ends before the body runs: a recursive call re-enters `try_compiled` for this function
        drop(st);
        self.cstats.compiled_calls += 1;
        if check::applies(self.compile_mode, &c) {
            return Some(self.run_checked(f, def, scope, this, args, &c));
        }
        Some(c.run_shared(self, f, scope, this, args))
    }
}
