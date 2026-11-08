//! Per-identifier scope hints for reads (plan TJ136 PR2).
//!
//! A read of `x` first tries the hint recorded at its identifier: `(hops, index)`, meaning "walk
//! `hops` parents, then `vars[index]` is `x`". The hint is only a guess. It is used only when
//! every scope on the walk has its `mask` bit for `x` clear, the target has no `with` record, and
//! `vars[index]` really is named `x`. Names are unique within one `Vars`, so that entry is the
//! binding the by-name walk would return. Anything else (stale, foreign, or garbage hint) misses
//! and takes the by-name path, which records a fresh hint.

use std::cell::Cell;

use swc_ecma_visit::{VisitMut, VisitMutWith};
use thaw_parser::ast as a;
use thaw_parser::common::SyntaxContext;

use super::*;

/// Hint slots are allocated per annotated identifier. Past this many, identifiers stay unannotated
/// and always take the by-name path. ponytail: eval-in-a-loop grows the table without bound
/// otherwise; add slot recycling if that shows up in profiles.
const MAX_IDENT_IDS: usize = 1 << 22;
/// Low bits of a packed hint hold the index; the high byte holds `hops + 1` (0 = no hint).
const HINT_IDX_BITS: u32 = 24;
const HINT_IDX_MASK: u32 = (1 << HINT_IDX_BITS) - 1;

/// Gives every read identifier (`Expr::Ident`) of a parsed module a fresh id: its `ctxt` is the
/// index of its hint slot in `Interp::ident_hints`. Ids are never 0, which means "unannotated".
struct Annotate<'a> {
    table: &'a mut Vec<Cell<u32>>,
}

impl VisitMut for Annotate<'_> {
    fn visit_mut_expr(&mut self, e: &mut a::Expr) {
        if let a::Expr::Ident(i) = e {
            if self.table.len() < MAX_IDENT_IDS {
                i.ctxt = SyntaxContext::from_u32(self.table.len() as u32);
                self.table.push(Cell::new(0));
            }
        }
        e.visit_mut_children_with(self);
    }
}

/// Annotates `module` for hinted reads; see [`Annotate`].
pub(crate) fn annotate(module: &mut a::Module, table: &mut Vec<Cell<u32>>) {
    module.visit_mut_with(&mut Annotate { table });
}

impl Scope {
    /// Oracle helper: every name in `vars` has its `mask` bit set (no false negatives).
    pub(crate) fn check_mask_invariant(&self) {
        let mask = self.mask.get();
        for name in self.vars.borrow().keys() {
            assert!(
                mask & name_bit(name) != 0,
                "scope mask lacks the bit of `{name}`"
            );
        }
    }
}

impl Interp {
    /// The hinted binding of `name` from `scope`, if the hint at `ctxt` still holds.
    fn hinted_read(&self, scope: &Rc<Scope>, name: &str, ctxt: SyntaxContext) -> Option<Value> {
        let id = ctxt.as_u32() as usize;
        if id == 0 {
            return None;
        }
        let packed = self.ident_hints.get(id)?.get();
        if packed == 0 {
            return None;
        }
        let hops = (packed >> HINT_IDX_BITS) - 1;
        let idx = (packed & HINT_IDX_MASK) as usize;
        let bit = name_bit(name);
        let mut cur: &Scope = scope;
        for _ in 0..hops {
            // an intermediate scope that may hold `name` decides the lookup, not the hint
            if cur.mask.get() & bit != 0 {
                return None;
            }
            cur = cur.parent.as_deref()?;
        }
        if cur.with.is_some() || cur.mask.get() & bit == 0 {
            return None;
        }
        let vars = cur.vars.borrow();
        let (key, binding) = vars.list.get(idx)?;
        if **key != *name {
            return None;
        }
        let value = binding.value.clone();
        drop(vars);
        if self.check_hints {
            let at = self.lookup_at(scope, name).and_then(|(_, loc)| loc);
            assert_eq!(
                at,
                Some((hops as u32, idx as u32)),
                "hinted read of `{name}` disagrees with the by-name lookup"
            );
        }
        Some(value)
    }

    fn record_hint(&self, ctxt: SyntaxContext, hops: u32, idx: u32) {
        let id = ctxt.as_u32() as usize;
        if id == 0 || hops >= 255 || idx > HINT_IDX_MASK {
            return;
        }
        if let Some(cell) = self.ident_hints.get(id) {
            cell.set(((hops + 1) << HINT_IDX_BITS) | idx);
        }
    }

    /// By-name lookup, as `lookup`, that also reports where a plain binding sits: `(hops, index)`
    /// unless a `with` record was passed on the way (then nothing may be hinted).
    fn lookup_at(&self, scope: &Rc<Scope>, name: &str) -> Option<(Value, Option<(u32, u32)>)> {
        let bit = name_bit(name);
        let mut cur: &Scope = scope;
        let mut hops = 0u32;
        let mut clean = true;
        loop {
            if self.check_hints {
                cur.check_mask_invariant();
            }
            if cur.mask.get() & bit != 0 {
                if let Some(o) = &cur.with {
                    clean = false;
                    if let Some(v) = self.with_get(o, name) {
                        return Some((v, None));
                    }
                } else {
                    let vars = cur.vars.borrow();
                    if let Some(p) = vars.position(name) {
                        let value = vars.list[p].1.value.clone();
                        drop(vars);
                        return Some((value, clean.then_some((hops, p as u32))));
                    }
                }
            }
            cur = cur.parent.as_deref()?;
            hops += 1;
        }
    }

    /// Reads identifier `i` through its hint when the hint holds, else by name (recording the hint
    /// for the next read). `None` when no scope on the chain binds it.
    pub(crate) fn lookup_ident(&self, scope: &Rc<Scope>, i: &a::Ident) -> Option<Value> {
        if let Some(v) = self.hinted_read(scope, &i.sym, i.ctxt) {
            return Some(v);
        }
        let (v, loc) = self.lookup_at(scope, &i.sym)?;
        if let Some((hops, idx)) = loc {
            self.record_hint(i.ctxt, hops, idx);
        }
        Some(v)
    }
}
