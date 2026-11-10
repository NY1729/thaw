//! Code generation (plan TJ150 section 3): `FuncDef` -> `CompiledFn`. Names are resolved here, at compile
//! time: a declared name becomes a frame slot, anything else is a free name read through the closure's scope.

use std::collections::HashSet;
use std::rc::Rc;

use thaw_parser::ast as a;

use super::analyze::{DeclKind, FnPlan, NameClass};
use super::{expr, stmt, CompiledFn, Ex, FallbackReason, FrameInit};
use crate::{Body, FuncDef};

/// Codegen result: `Err` names the reason the function stays on the AST interpreter.
pub(crate) type CG<T> = Result<T, FallbackReason>;

/// Every construct outside the supported subset. The analyzer blocks most of them first; this is the second
/// line of defence, so it must never panic.
pub(crate) fn reject<T>() -> CG<T> {
    Err(FallbackReason::Unsupported)
}

/// A declared name visible at some point of the function.
#[derive(Clone)]
pub(crate) struct Local {
    pub(crate) name: Rc<str>,
    pub(crate) slot: u32,
    pub(crate) mutable: bool,
}

/// Compile-time environment. `scopes[0]` is the function level (parameters and hoisted `var`s); each block
/// pushes a level for its own `let`/`const`.
pub(crate) struct Gen {
    scopes: Vec<Vec<Local>>,
    next_slot: u32,
    pub(crate) arrow: bool,
    /// Loops enclosing the statement being compiled: an unlabeled `break`/`continue` needs one.
    pub(crate) loops: u32,
    /// Loops and `switch`es enclosing the statement being compiled: an unlabeled `break` needs one. Separate from
    /// `loops`, because a `break` inside a `switch` (with no loop around it) is legal and a `continue` is not.
    pub(crate) breakables: u32,
    /// Names an inner closure can reach: never declared here, so they resolve by name through the frame scope.
    scoped: HashSet<String>,
    /// Test-only: bitwise operator nodes built so far (copied into `CompiledFn`).
    #[cfg(test)]
    pub(crate) bit_ex_built: u64,
    /// Test-only: element-read nodes (`elem.rs`) built so far.
    #[cfg(test)]
    pub(crate) elem_built: u64,
}

impl Gen {
    fn new(arrow: bool, scoped: HashSet<String>) -> Gen {
        Gen {
            scopes: vec![Vec::new()],
            next_slot: 0,
            arrow,
            loops: 0,
            breakables: 0,
            scoped,
            #[cfg(test)]
            bit_ex_built: 0,
            #[cfg(test)]
            elem_built: 0,
        }
    }

    /// Whether `name` is a scoped name (see `scoped`).
    pub(crate) fn is_scoped(&self, name: &str) -> bool {
        self.scoped.contains(name)
    }

    pub(crate) fn push(&mut self) {
        self.scopes.push(Vec::new());
    }

    pub(crate) fn pop(&mut self) {
        self.scopes.pop();
    }

    fn fresh(&mut self, name: &str, mutable: bool, level: usize) -> u32 {
        let slot = self.next_slot;
        self.next_slot += 1;
        self.scopes[level].push(Local {
            name: Rc::from(name),
            slot,
            mutable,
        });
        slot
    }

    /// A `let`/`const` in the innermost block. Called after its initialiser is compiled (`let x = x` binds
    /// the outer `x`, as the interpreter does).
    pub(crate) fn declare_block(&mut self, name: &str, mutable: bool) -> u32 {
        let level = self.scopes.len() - 1;
        self.fresh(name, mutable, level)
    }

    /// A function-level name. A name already declared there (duplicate parameter, `var` of a parameter)
    /// keeps its slot, exactly like `declare_if_absent`.
    pub(crate) fn declare_fn(&mut self, name: &str, mutable: bool) -> u32 {
        if let Some(l) = self.scopes[0].iter().find(|l| &*l.name == name) {
            return l.slot;
        }
        self.fresh(name, mutable, 0)
    }

    /// Where a `var` statement stores: always the function-level binding.
    pub(crate) fn var_local(&self, name: &str) -> Option<Local> {
        self.scopes[0].iter().find(|l| &*l.name == name).cloned()
    }

    /// The innermost visible declaration of `name`, or `None` for a free name.
    pub(crate) fn resolve(&self, name: &str) -> Option<Local> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev())
            .find(|l| &*l.name == name)
            .cloned()
    }
}

/// Compiles an analyzer-accepted function. `plan` only contributes the decisions the analyzer owns. `intern`
/// gives the shared `Rc<str>` of a name the call's `Scope` declares (see `Interp::intern`).
pub(crate) fn compile(
    def: &FuncDef,
    plan: &FnPlan,
    intern: &mut dyn FnMut(&str) -> Rc<str>,
) -> CG<CompiledFn> {
    if def.class.is_some()
        || def.is_gen
        || def.is_async
        // the analyzer scopes only parameters and `var`s; any other scoped name here is a bug, not a fallback
        || plan
            .names
            .iter()
            .any(|n| n.class == NameClass::Scoped && !matches!(n.kind, DeclKind::Param | DeclKind::Var))
    {
        return reject();
    }
    let scoped = plan
        .names
        .iter()
        .filter(|n| n.class == NameClass::Scoped)
        .map(|n| n.name.clone())
        .collect();
    let mut g = Gen::new(def.arrow, scoped);
    // the self name: the prologue stores the callee here, before the parameters; a `var` of the same name reuses it
    let self_slot = plan
        .names
        .iter()
        .find(|n| n.kind == DeclKind::SelfName)
        .map(|n| g.declare_fn(&n.name, true));
    let mut param_slots = Vec::with_capacity(def.params.len());
    let mut scoped_params = Vec::new();
    for (i, p) in def.params.iter().enumerate() {
        let a::Pat::Ident(b) = p else {
            return reject();
        };
        if g.is_scoped(&b.id.sym) {
            scoped_params.push((i, intern(&b.id.sym)));
            param_slots.push(None);
        } else {
            param_slots.push(Some(g.declare_fn(&b.id.sym, true)));
        }
    }
    let mut scoped_vars = Vec::new();
    for name in &def.hoisted {
        if g.is_scoped(name) {
            scoped_vars.push(intern(name));
        } else {
            g.declare_fn(name, true);
        }
    }
    let (body, expr_body) = match &def.body {
        Body::Block(stmts) => (stmt::body(&mut g, stmts)?, None),
        Body::Expr(e) => (Vec::new(), Some(expr::compile(&mut g, e)?)),
    };
    debug_assert_eq!(
        g.next_slot, plan.nslots,
        "slot count of the analyzer and codegen differ"
    );
    Ok(CompiledFn {
        nslots: g.next_slot,
        self_slot,
        param_slots,
        body,
        expr_body,
        frame: plan.needs_scope.then_some(FrameInit {
            set_this: def.uses_this && !def.arrow,
            params: scoped_params,
            vars: scoped_vars,
        }),
        arrow: def.arrow,
        uses_this: def.uses_this,
        strict: def.strict,
        pure_kernel: plan.pure_kernel,
        #[cfg(test)]
        bit_ex_built: g.bit_ex_built,
        #[cfg(test)]
        elem_built: g.elem_built,
    })
}

/// The `Ex` that is `undefined` (used for `let x;`).
pub(crate) fn undefined() -> Ex {
    expr::lit(crate::Value::Undefined)
}
