//! Code generation (plan TJ150 section 3): `FuncDef` -> `CompiledFn`. Names are resolved here, at compile
//! time: a declared name becomes a frame slot, anything else is a free name read through the closure's scope.

use std::rc::Rc;

use thaw_parser::ast as a;

use super::analyze::{DeclKind, FnPlan, NameClass};
use super::{expr, stmt, CompiledFn, Ex, FallbackReason};
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
    /// Test-only: bitwise operator nodes built so far (copied into `CompiledFn`).
    #[cfg(test)]
    pub(crate) bit_ex_built: u64,
    /// Test-only: element-read nodes (`elem.rs`) built so far.
    #[cfg(test)]
    pub(crate) elem_built: u64,
}

impl Gen {
    fn new(arrow: bool) -> Gen {
        Gen {
            scopes: vec![Vec::new()],
            next_slot: 0,
            arrow,
            loops: 0,
            #[cfg(test)]
            bit_ex_built: 0,
            #[cfg(test)]
            elem_built: 0,
        }
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

/// Compiles an analyzer-accepted function. `plan` only contributes the decisions the analyzer owns.
pub(crate) fn compile(def: &FuncDef, plan: &FnPlan) -> CG<CompiledFn> {
    if def.class.is_some()
        || def.is_gen
        || def.is_async
        // no inner function is compiled yet, so no name is scoped: a scoped name here is a bug, not a fallback
        || plan.names.iter().any(|n| n.class == NameClass::Scoped)
    {
        return reject();
    }
    let mut g = Gen::new(def.arrow);
    // the self name: the prologue stores the callee here, before the parameters; a `var` of the same name reuses it
    let self_slot = plan
        .names
        .iter()
        .find(|n| n.kind == DeclKind::SelfName)
        .map(|n| g.declare_fn(&n.name, true));
    let mut param_slots = Vec::with_capacity(def.params.len());
    for p in &def.params {
        let a::Pat::Ident(b) = p else {
            return reject();
        };
        param_slots.push(Some(g.declare_fn(&b.id.sym, true)));
    }
    for name in &def.hoisted {
        g.declare_fn(name, true);
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
