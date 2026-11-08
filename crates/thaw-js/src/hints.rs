//! Per-identifier scope hints for reads (plan TJ136 PR2).
//!
//! A read of `x` first tries the hint recorded at its identifier: `(hops, index)`, meaning "walk
//! `hops` parents, then `vars[index]` is `x`". The hint is only a guess. It is used only when
//! every scope on the walk has its `mask` bit for `x` clear, the target has no `with` record, and
//! `vars[index]` really is named `x`. Names are unique within one `Vars`, so that entry is the
//! binding the by-name walk would return. Anything else (stale, foreign, or garbage hint) misses
//! and takes the by-name path, which records a fresh hint.

use std::cell::{Cell, RefCell};

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

/// A by-name lookup result: the value and, unless a `with` record was passed, `(hops, index, key)`.
type HintedLookup = (Value, Option<(u32, u32, Rc<str>)>);

/// The hint of one annotated identifier. `bit` is `name_bit` of its name, computed once by
/// [`Annotate`]; `name` is the vars key the hint was recorded for (kept alive by this clone), so a
/// read validates by pointer first and by string only when the pointers differ.
#[derive(Default)]
pub(crate) struct HintSlot {
    packed: Cell<u32>,
    bit: u64,
    name: RefCell<Option<Rc<str>>>,
}

impl HintSlot {
    fn new(bit: u64) -> Self {
        HintSlot {
            packed: Cell::new(0),
            bit,
            name: RefCell::new(None),
        }
    }

    /// The packed hint (0 = none); the unit tests read it.
    #[cfg(test)]
    pub(crate) fn get(&self) -> u32 {
        self.packed.get()
    }
}

/// Hint counters, enabled by `THAW_JS_HINT_STATS=1` (or by a test setting `on`). Each count is a
/// single bool test when off. The summary is printed on drop when `on`.
#[derive(Default)]
pub(crate) struct HintStats {
    pub(crate) on: bool,
    pub(crate) reads: Cell<u64>,
    pub(crate) hit: Cell<u64>,
    pub(crate) miss_nohint: Cell<u64>,
    pub(crate) miss_mid_mask: Cell<u64>,
    pub(crate) miss_target: Cell<u64>,
    pub(crate) miss_key: Cell<u64>,
    /// Hints stored (every record that reaches the slot).
    pub(crate) record: Cell<u64>,
    /// Stored hint equal to the one already there.
    pub(crate) record_same: Cell<u64>,
    /// Stored hint replacing a different non-zero one (layout drift or thrash).
    pub(crate) record_changed: Cell<u64>,
    /// By-name reads of a binding behind `with`: nothing is recorded.
    pub(crate) unrecorded_with: Cell<u64>,
    /// Reads of identifiers with no slot (id 0 or table full).
    pub(crate) unannotated: Cell<u64>,
}

impl HintStats {
    pub(crate) fn from_env() -> Self {
        let mut stats = HintStats::default();
        stats.on = std::env::var_os("THAW_JS_HINT_STATS").is_some_and(|v| v == "1");
        stats
    }

    #[inline]
    pub(crate) fn bump(&self, c: &Cell<u64>) {
        if self.on {
            c.set(c.get() + 1);
        }
    }

    pub(crate) fn summary(&self) -> String {
        let reads = self.reads.get();
        let hit = self.hit.get();
        let pct = if reads == 0 {
            0.0
        } else {
            100.0 * hit as f64 / reads as f64
        };
        format!(
            "hint stats: reads={reads} hit={hit} ({pct:.1}%) miss_nohint={} miss_mid_mask={} \
             miss_target={} miss_key={} record={} record_same={} record_changed={} \
             unrecorded_with={} unannotated={}",
            self.miss_nohint.get(),
            self.miss_mid_mask.get(),
            self.miss_target.get(),
            self.miss_key.get(),
            self.record.get(),
            self.record_same.get(),
            self.record_changed.get(),
            self.unrecorded_with.get(),
            self.unannotated.get(),
        )
    }
}

impl Drop for HintStats {
    fn drop(&mut self) {
        if self.on {
            eprintln!("{}", self.summary());
        }
    }
}

/// Gives every read identifier (`Expr::Ident`) of a parsed module a fresh id: its `ctxt` is the
/// index of its hint slot in `Interp::ident_hints`. Ids are never 0, which means "unannotated".
struct Annotate<'a> {
    table: &'a mut Vec<HintSlot>,
}

impl VisitMut for Annotate<'_> {
    fn visit_mut_expr(&mut self, e: &mut a::Expr) {
        if let a::Expr::Ident(i) = e {
            if self.table.len() < MAX_IDENT_IDS {
                i.ctxt = SyntaxContext::from_u32(self.table.len() as u32);
                self.table.push(HintSlot::new(name_bit(&i.sym)));
            }
        }
        e.visit_mut_children_with(self);
    }

    /// `x op= v` reads `x` through `eval_ident(&target.id)`, so a plain identifier target needs a hint
    /// id too. Only simple identifier targets are annotated (not every pattern binding), to save slots.
    fn visit_mut_simple_assign_target(&mut self, t: &mut a::SimpleAssignTarget) {
        if let a::SimpleAssignTarget::Ident(b) = t {
            if self.table.len() < MAX_IDENT_IDS {
                b.id.ctxt = SyntaxContext::from_u32(self.table.len() as u32);
                self.table.push(HintSlot::new(name_bit(&b.id.sym)));
            }
        }
        t.visit_mut_children_with(self);
    }
}

/// Annotates `module` for hinted reads; see [`Annotate`].
pub(crate) fn annotate(module: &mut a::Module, table: &mut Vec<HintSlot>) {
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
    /// The hinted binding of the identifier at `ctxt` from `scope`, if the hint still holds.
    fn hinted_read(&self, scope: &Rc<Scope>, ctxt: SyntaxContext) -> Option<Value> {
        let st = &self.hint_stats;
        let id = ctxt.as_u32() as usize;
        let slot = if id == 0 {
            None
        } else {
            self.ident_hints.get(id)
        };
        let Some(slot) = slot else {
            st.bump(&st.unannotated);
            return None;
        };
        let packed = slot.packed.get();
        if packed == 0 {
            st.bump(&st.miss_nohint);
            return None;
        }
        let hops = (packed >> HINT_IDX_BITS) - 1;
        let idx = (packed & HINT_IDX_MASK) as usize;
        let bit = slot.bit;
        let mut cur: &Scope = scope;
        for _ in 0..hops {
            // an intermediate scope that may hold `name` decides the lookup, not the hint
            if cur.mask.get() & bit != 0 {
                st.bump(&st.miss_mid_mask);
                return None;
            }
            let Some(parent) = cur.parent.as_deref() else {
                st.bump(&st.miss_target);
                return None;
            };
            cur = parent;
        }
        if cur.with.is_some() || cur.mask.get() & bit == 0 {
            st.bump(&st.miss_target);
            return None;
        }
        let vars = cur.vars.borrow();
        let stored = slot.name.borrow();
        let Some((key, binding)) = vars.list.get(idx) else {
            st.bump(&st.miss_key);
            return None;
        };
        let Some(name) = stored.as_ref() else {
            st.bump(&st.miss_key);
            return None;
        };
        let same_ptr = Rc::ptr_eq(key, name);
        if !same_ptr && **key != **name {
            st.bump(&st.miss_key);
            return None;
        }
        if self.check_hints && same_ptr {
            assert_eq!(**key, **name, "pointer-equal hint names differ");
        }
        let value = binding.value.clone();
        let text = self.check_hints.then(|| name.clone());
        drop(stored);
        drop(vars);
        st.bump(&st.hit);
        if let Some(text) = text {
            let at = self
                .lookup_at(scope, &text)
                .and_then(|(_, loc)| loc)
                .map(|(h, i, _)| (h, i));
            assert_eq!(
                at,
                Some((hops, idx as u32)),
                "hinted read of `{text}` disagrees with the by-name lookup"
            );
        }
        Some(value)
    }

    /// Stores a hint for the identifier at `ctxt`: `key` is the vars key it was found under.
    fn record_hint(&self, ctxt: SyntaxContext, hops: u32, idx: u32, key: &Rc<str>) {
        let id = ctxt.as_u32() as usize;
        if id == 0 || hops >= 255 || idx > HINT_IDX_MASK {
            return;
        }
        let Some(slot) = self.ident_hints.get(id) else {
            return;
        };
        let new = ((hops + 1) << HINT_IDX_BITS) | idx;
        let old = slot.packed.get();
        let st = &self.hint_stats;
        st.bump(&st.record);
        if old == new {
            st.bump(&st.record_same);
        } else if old != 0 {
            st.bump(&st.record_changed);
        }
        slot.packed.set(new);
        let mut stored = slot.name.borrow_mut();
        if !matches!(stored.as_ref(), Some(n) if Rc::ptr_eq(n, key)) {
            *stored = Some(key.clone());
        }
    }

    /// By-name lookup, as `lookup`, that also reports where a plain binding sits: `(hops, index,
    /// key)` unless a `with` record was passed on the way (then nothing may be hinted).
    fn lookup_at(&self, scope: &Rc<Scope>, name: &str) -> Option<HintedLookup> {
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
                        let loc = clean.then(|| (hops, p as u32, vars.list[p].0.clone()));
                        drop(vars);
                        return Some((value, loc));
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
        let st = &self.hint_stats;
        st.bump(&st.reads);
        if let Some(v) = self.hinted_read(scope, i.ctxt) {
            return Some(v);
        }
        let (v, loc) = self.lookup_at(scope, &i.sym)?;
        match loc {
            Some((hops, idx, key)) => self.record_hint(i.ctxt, hops, idx, &key),
            None => st.bump(&st.unrecorded_with),
        }
        Some(v)
    }
}
