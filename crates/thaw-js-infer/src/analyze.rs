//! The fixpoint inference engine (phase >= 2).

use crate::narrow::{assigns_in_expr, assigns_in_stmt, assigns_in_stmts, exits};
use crate::prescan::*;
use crate::ty::*;
use crate::Config;
use std::collections::{BTreeMap, HashMap, HashSet};
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

pub enum Key {
    Name(String),
    Num,
    Dyn,
    /// index type not known yet (fixpoint): no information, must not kill anything
    Pending,
}

const PROTO_NAMES: &[&str] = &[
    "constructor",
    "toString",
    "valueOf",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "__proto__",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

pub struct An {
    pub st: State,
    pub cfg: Config,
    pub sites: Vec<Site>,
    site_key: HashMap<(u32, u8), usize>,
    pub changed: bool,
    pub cur: Vec<usize>,
    pub any_hist: BTreeMap<Reason, usize>,
    decl_const: bool,
    pub dts_fns: HashMap<usize, Vec<crate::dts::Sig>>,
    /// functions whose `dts_fns` entry comes from a JSDoc block, not a declaration file
    pub jsdoc_fns: HashSet<usize>,
    dts_sites: HashMap<(usize, usize, usize), usize>,
    pub dts_ret: HashMap<usize, Ty>,
    cb_flowed: std::collections::HashSet<usize>,
    pub lits: Vec<String>,
    ev: HashMap<(String, String), Vec<Ty>>,
    pub escape_final: bool,
    /// bundle factory key -> (exports record, module record)
    mod_sites: HashMap<String, (usize, usize)>,
    dyn_require: bool,
    pub(crate) read_key: u32,
    /// (class, field) -> type stored by that class's own constructor
    ctor_fields: HashMap<((usize, String), String), Ty>,
    /// functions whose value reached dynamic code (their parameters are unknown)
    pub esc_fns: std::collections::HashSet<usize>,
    seen_fns: std::collections::HashSet<usize>,
    pub(crate) narrow: Vec<((usize, String), Ty)>,
    class_sites: HashMap<(usize, String), usize>,
    site_class: HashMap<usize, (usize, String)>,
    /// methods whose function value escaped as a value (`this` may be rebound)
    unsafe_dyn: std::collections::HashSet<usize>,
    def_write: bool,
}

fn ends_abruptly(stmts: &[a::Stmt]) -> bool {
    stmts.last().is_some_and(stmt_ends)
}

fn ft_has_fn(t: &Ty) -> bool {
    matches!(t, Ty::Set(s) if s.iter().any(|a| matches!(a, Atom::Fn(_))))
}

fn ft_has_non_fn(t: &Ty) -> bool {
    matches!(t, Ty::Set(s) if s.iter().any(|a| !matches!(a, Atom::Fn(_))))
}

pub(crate) fn stmt_ends(s: &a::Stmt) -> bool {
    match s {
        a::Stmt::Return(_) | a::Stmt::Throw(_) => true,
        a::Stmt::Block(b) => ends_abruptly(&b.stmts),
        a::Stmt::If(i) => i
            .alt
            .as_ref()
            .is_some_and(|alt| stmt_ends(&i.cons) && stmt_ends(alt)),
        a::Stmt::Try(t) => {
            t.finalizer
                .as_ref()
                .is_some_and(|f| ends_abruptly(&f.stmts))
                || (ends_abruptly(&t.block.stmts)
                    && t.handler
                        .as_ref()
                        .is_some_and(|h| ends_abruptly(&h.body.stmts)))
        }
        a::Stmt::While(w) => matches!(strip(&w.test), a::Expr::Lit(a::Lit::Bool(b)) if b.value),
        a::Stmt::For(f) => f.test.is_none(),
        a::Stmt::Labeled(l) => stmt_ends(&l.body),
        // ponytail: switch / do-while treated as completing normally (adds `undefined` to the
        // return type: wider, still sound).
        _ => false,
    }
}

impl An {
    pub fn new(st: State, cfg: Config) -> An {
        An {
            st,
            cfg,
            sites: Vec::new(),
            site_key: HashMap::new(),
            changed: false,
            cur: vec![0],
            any_hist: BTreeMap::new(),
            decl_const: false,
            dts_fns: HashMap::new(),
            jsdoc_fns: HashSet::new(),
            dts_sites: HashMap::new(),
            dts_ret: HashMap::new(),
            cb_flowed: Default::default(),
            lits: Vec::new(),
            ev: HashMap::new(),
            escape_final: false,
            mod_sites: HashMap::new(),
            dyn_require: false,
            read_key: 0,
            ctor_fields: HashMap::new(),
            esc_fns: Default::default(),
            seen_fns: Default::default(),
            narrow: Vec::new(),
            class_sites: HashMap::new(),
            site_class: HashMap::new(),
            unsafe_dyn: Default::default(),
            def_write: false,
        }
    }

    // ---------- sites ----------
    pub fn find(&mut self, s: usize) -> usize {
        let mut r = s;
        while let Site::Link(n) = self.sites[r] {
            r = n;
        }
        let mut c = s;
        while let Site::Link(n) = self.sites[c] {
            self.sites[c] = Site::Link(r);
            c = n;
        }
        r
    }

    fn site(&mut self, key: (u32, u8), mk: Site) -> usize {
        if let Some(&s) = self.site_key.get(&key) {
            return self.find(s);
        }
        let id = self.sites.len();
        self.sites.push(mk);
        self.site_key.insert(key, id);
        id
    }

    pub fn kill(&mut self, s: usize, r: Reason) {
        let s = self.find(s);
        if matches!(self.sites[s], Site::Dead(_)) {
            return;
        }
        let old = std::mem::replace(&mut self.sites[s], Site::Dead(r));
        self.changed = true;
        // containers nested in a dead one are dead for the same reason (root cause, not "Tainted")
        match old {
            Site::Arr(e) => self.taint_as(&e, r),
            Site::Obj(m) => m.values().for_each(|t| self.taint_as(t, r)),
            _ => {}
        }
    }

    pub fn taint_as(&mut self, t: &Ty, r: Reason) {
        if let Ty::Set(atoms) = t {
            for at in atoms.clone() {
                match at {
                    Atom::Arr(s) | Atom::Obj(s) => self.kill(s, r),
                    Atom::Fn(f) => self.escape_fn(f as usize),
                    _ => {}
                }
            }
        }
    }

    pub fn escape_fn(&mut self, f: usize) {
        if self.esc_fns.insert(f) {
            self.changed = true;
        }
    }

    /// Mark every function reachable from `t` (through arrays / objects) as escaped without killing
    /// the containers: used for values handed to external callers.
    fn escape_fns_in(&mut self, t: &Ty, seen: &mut std::collections::HashSet<usize>) {
        let Ty::Set(atoms) = t else { return };
        for at in atoms.clone() {
            match at {
                Atom::Fn(f) => self.escape_fn(f as usize),
                Atom::Arr(s) | Atom::Obj(s) => {
                    let s = self.find(s);
                    if !seen.insert(s) {
                        continue;
                    }
                    let inner: Vec<Ty> = match &self.sites[s] {
                        Site::Arr(e) => vec![e.clone()],
                        Site::Obj(m) => m.values().cloned().collect(),
                        _ => vec![],
                    };
                    for i in inner {
                        self.escape_fns_in(&i, seen);
                    }
                }
                _ => {}
            }
        }
    }

    fn union_sites(&mut self, x: usize, y: usize) -> usize {
        let (rx, ry) = (self.find(x), self.find(y));
        if rx == ry {
            return rx;
        }
        if self.site_class.contains_key(&rx) || self.site_class.contains_key(&ry) {
            // an instance record joined with something else: give up on both
            self.kill(rx, Reason::Class);
            self.kill(ry, Reason::Class);
            self.sites[ry] = Site::Link(rx);
            self.changed = true;
            return rx;
        }
        self.changed = true;
        let ky = std::mem::replace(&mut self.sites[ry], Site::Link(rx));
        let kx = std::mem::replace(&mut self.sites[rx], Site::Dead(Reason::Other));
        let merged = match (kx, ky) {
            (Site::Dead(r), other) | (other, Site::Dead(r)) => {
                match other {
                    Site::Arr(e) => self.taint_as(&e, r),
                    Site::Obj(m) => m.values().for_each(|t| self.taint_as(t, r)),
                    _ => {}
                }
                Site::Dead(r)
            }
            (Site::Arr(e1), Site::Arr(e2)) => Site::Arr(self.join(&e1, &e2)),
            (Site::Obj(m1), Site::Obj(m2)) => {
                let mut out = BTreeMap::new();
                let keys: std::collections::BTreeSet<_> =
                    m1.keys().chain(m2.keys()).cloned().collect();
                for k in keys {
                    let t1 = m1.get(&k).cloned().unwrap_or_else(Ty::undef);
                    let t2 = m2.get(&k).cloned().unwrap_or_else(Ty::undef);
                    out.insert(k, self.join(&t1, &t2));
                }
                Site::Obj(out)
            }
            _ => Site::Dead(Reason::Other),
        };
        let r = self.find(rx);
        self.sites[r] = merged;
        r
    }

    // ---------- lattice ----------
    fn norm(&mut self, atoms: Vec<Atom>) -> Ty {
        let (mut arr, mut obj): (Option<usize>, Option<usize>) = (None, None);
        let mut out: Vec<Atom> = Vec::new();
        for at in atoms {
            match at {
                Atom::Arr(s) => {
                    arr = Some(match arr {
                        Some(p) => self.union_sites(p, s),
                        None => self.find(s),
                    })
                }
                Atom::Obj(s) => {
                    obj = Some(match obj {
                        Some(p) => self.union_sites(p, s),
                        None => self.find(s),
                    })
                }
                o => out.push(o),
            }
        }
        if let Some(s) = arr {
            out.push(Atom::Arr(self.find(s)));
        }
        if let Some(s) = obj {
            out.push(Atom::Obj(self.find(s)));
        }
        out.sort();
        out.dedup();
        // literals collapse into their base type, and a long literal list is just the base type
        if out.contains(&Atom::Num) {
            out.retain(|a| !matches!(a, Atom::NLit(_)));
        } else if out.iter().filter(|a| matches!(a, Atom::NLit(_))).count() > 8 {
            out.retain(|a| !matches!(a, Atom::NLit(_)));
            out.push(Atom::Num);
        }
        if out.contains(&Atom::Str) {
            out.retain(|a| !matches!(a, Atom::SLit(_)));
        } else if out.iter().filter(|a| matches!(a, Atom::SLit(_))).count() > 8 {
            out.retain(|a| !matches!(a, Atom::SLit(_)));
            out.push(Atom::Str);
        }
        out.sort();
        out.dedup();
        if out.is_empty() {
            Ty::Bottom
        } else {
            Ty::Set(out)
        }
    }

    /// TypeScript's widening of fresh literal types when stored in a mutable location.
    pub fn widen(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Set(s) if s.iter().any(|a| matches!(a, Atom::SLit(_) | Atom::NLit(_))) => {
                let v = s
                    .iter()
                    .map(|a| match a {
                        Atom::SLit(_) => Atom::Str,
                        Atom::NLit(_) => Atom::Num,
                        o => *o,
                    })
                    .collect();
                self.norm(v)
            }
            o => o.clone(),
        }
    }

    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(i) = self.lits.iter().position(|x| x == s) {
            return i as u32;
        }
        self.lits.push(s.to_string());
        (self.lits.len() - 1) as u32
    }

    pub fn join(&mut self, x: &Ty, y: &Ty) -> Ty {
        match (x, y) {
            (Ty::Any(a), Ty::Any(b)) => Ty::Any(*a.min(b)),
            (Ty::Any(r), s @ Ty::Set(_)) | (s @ Ty::Set(_), Ty::Any(r)) => {
                self.taint_as(s, *r);
                Ty::Any(*r)
            }
            (Ty::Any(r), Ty::Bottom) | (Ty::Bottom, Ty::Any(r)) => Ty::Any(*r),
            (Ty::Bottom, Ty::Bottom) => Ty::Bottom,
            (Ty::Set(s), Ty::Bottom) | (Ty::Bottom, Ty::Set(s)) => self.norm(s.clone()),
            (Ty::Set(s1), Ty::Set(s2)) => {
                let mut v = s1.clone();
                v.extend(s2.iter().copied());
                self.norm(v)
            }
        }
    }

    fn flow(&mut self, key: &(usize, String), t: &Ty) {
        let Some((old, is_const)) = self.st.binds.get(key).map(|b| (b.ty.clone(), b.is_const))
        else {
            self.taint_as(t, Reason::UnknownGlobal);
            return;
        };
        let w;
        let t = if is_const {
            t
        } else {
            w = self.widen(t);
            &w
        };
        let old_n = self.join(&old, &Ty::Bottom);
        let new = self.join(&old_n, t);
        if new != old_n || new != old {
            if new != old_n {
                self.changed = true;
            }
            self.st.binds.get_mut(key).unwrap().ty = new;
        }
    }

    fn flow_name(&mut self, name: &str, t: &Ty) {
        match self.st.resolve(*self.cur.last().unwrap(), name) {
            Some(k) => self.flow(&k, t),
            None => self.taint_as(t, Reason::UnknownGlobal),
        }
    }

    fn flow_any_pat(&mut self, p: &a::Pat, r: Reason) {
        let mut ids = Vec::new();
        pat_idents(p, &mut ids);
        for i in ids {
            self.flow_name(i, &Ty::Any(r));
        }
    }

    fn site_elem_flow(&mut self, s: usize, t: &Ty) {
        let t = &self.widen(t);
        let s = self.find(s);
        if let Site::Arr(e) = &self.sites[s] {
            let e = e.clone();
            let n = self.join(&e, t);
            if n != e {
                self.changed = true;
            }
            let s = self.find(s);
            if let Site::Arr(_) = self.sites[s] {
                self.sites[s] = Site::Arr(n);
            }
        } else {
            self.taint_as(t, Reason::ShapeChange);
        }
    }

    fn site_field_flow(&mut self, s: usize, name: &str, t: &Ty, late: bool) {
        let t = &self.widen(t);
        let s = self.find(s);
        let Site::Obj(m) = &self.sites[s] else {
            self.taint_as(t, Reason::ShapeChange);
            return;
        };
        let old = m.get(name).cloned();
        let base = match (&old, late) {
            (Some(o), _) => o.clone(),
            (None, true) => Ty::undef(),
            (None, false) => Ty::Bottom,
        };
        let n = self.join(&base, t);
        if old.as_ref() != Some(&n) {
            self.changed = true;
        }
        let s = self.find(s);
        if let Site::Obj(m) = &mut self.sites[s] {
            m.insert(name.to_string(), n);
        }
    }

    fn arr_elem(&mut self, s: usize) -> Ty {
        let s = self.find(s);
        match &self.sites[s] {
            Site::Arr(e) => e.clone(),
            Site::Dead(r) => Ty::Any(*r),
            _ => Ty::Any(Reason::Other),
        }
    }

    // ---------- operators ----------
    fn arith(&self, x: &Ty, y: &Ty) -> Ty {
        if matches!(x, Ty::Bottom) || matches!(y, Ty::Bottom) {
            return Ty::Bottom;
        }
        // a primitive operand cannot be mixed with BigInt (throws), so the result is a number
        if x.prim_only() || y.prim_only() {
            return Ty::num();
        }
        match (x, y) {
            (Ty::Any(r), _) | (_, Ty::Any(r)) => Ty::Any(*r),
            _ => Ty::Any(Reason::Coercion),
        }
    }

    fn add(&mut self, x: &Ty, y: &Ty) -> Ty {
        match (x, y) {
            (Ty::Bottom, _) | (_, Ty::Bottom) => Ty::Bottom,
            _ if x.is_str() || y.is_str() => Ty::str(),
            (Ty::Any(r), _) | (_, Ty::Any(r)) => Ty::Any(*r),
            (Ty::Set(a1), Ty::Set(a2)) => {
                let mut out = Vec::new();
                for p in a1 {
                    for q in a2 {
                        out.push(match (p, q) {
                            (Atom::Str | Atom::SLit(_), _) | (_, Atom::Str | Atom::SLit(_)) => {
                                Ty::str()
                            }
                            (Atom::Arr(_) | Atom::Obj(_) | Atom::Fn(_) | Atom::Ns(_), _)
                            | (_, Atom::Arr(_) | Atom::Obj(_) | Atom::Fn(_) | Atom::Ns(_)) => {
                                Ty::Any(Reason::Coercion)
                            }
                            _ => Ty::num(),
                        });
                    }
                }
                let mut acc = Ty::Bottom;
                for t in out {
                    acc = self.join(&acc, &t);
                }
                acc
            }
        }
    }

    fn binop(&mut self, op: a::BinaryOp, x: &Ty, y: &Ty) -> Ty {
        use a::BinaryOp::*;
        match op {
            Add => self.add(x, y),
            Sub | Mul | Div | Mod | Exp | BitAnd | BitOr | BitXor | LShift | RShift
            | ZeroFillRShift => self.arith(x, y),
            Lt | Gt | LtEq | GtEq | EqEq | NotEq | EqEqEq | NotEqEq | In | InstanceOf => Ty::bool(),
            LogicalAnd | LogicalOr => self.join(x, y),
            NullishCoalescing => {
                let x2 = self.drop_nullish(x);
                self.join(&x2, y)
            }
        }
    }

    fn drop_nullish(&self, t: &Ty) -> Ty {
        match t {
            Ty::Set(s) => {
                let v: Vec<Atom> = s
                    .iter()
                    .copied()
                    .filter(|a| !matches!(a, Atom::Undef | Atom::Null))
                    .collect();
                if v.is_empty() {
                    Ty::Bottom
                } else {
                    Ty::Set(v)
                }
            }
            o => o.clone(),
        }
    }

    // ---------- members ----------
    fn key_of(&mut self, p: &a::MemberProp) -> Key {
        match p {
            a::MemberProp::Ident(i) => Key::Name(i.sym.to_string()),
            a::MemberProp::PrivateName(_) => Key::Dyn,
            a::MemberProp::Computed(c) => match strip(&c.expr) {
                a::Expr::Lit(a::Lit::Str(s)) => {
                    Key::Name(s.value.as_str().unwrap_or("").to_string())
                }
                _ => {
                    let t = self.ty(&c.expr);
                    if t.is_num() {
                        Key::Num
                    } else if t == Ty::Bottom {
                        Key::Pending
                    } else {
                        Key::Dyn
                    }
                }
            },
        }
    }

    fn member_read(&mut self, obj: &Ty, key: &Key) -> Ty {
        if matches!(key, Key::Pending) {
            return Ty::Bottom;
        }
        let atoms = match obj {
            Ty::Bottom => return Ty::Bottom,
            Ty::Any(r) => return Ty::Any(*r),
            Ty::Set(s) => s.clone(),
        };
        let mut acc = Ty::Bottom;
        for at in atoms {
            let t = match at {
                Atom::Undef | Atom::Null => Ty::Bottom, // throws
                Atom::Num | Atom::NLit(_) | Atom::Bool => Ty::Any(Reason::UnmodeledMethod),
                Atom::Fn(_) => Ty::Any(Reason::UnmodeledMethod),
                Atom::Ns(ns) => match key {
                    Key::Name(n) => {
                        let k = self.read_key;
                        self.ns_prop(ns, n, k)
                    }
                    _ => Ty::Any(Reason::Builtin),
                },
                Atom::Str | Atom::SLit(_) => match key {
                    Key::Name(n) if n == "length" => Ty::num(),
                    Key::Num => Ty::Set(vec![Atom::Str, Atom::Undef]),
                    _ => Ty::Any(Reason::UnmodeledMethod),
                },
                Atom::Arr(s) => match key {
                    Key::Name(n) if n == "length" => Ty::num(),
                    Key::Num => {
                        let e = self.arr_elem(s);
                        if self.cfg.in_bounds_reads {
                            e
                        } else {
                            self.join(&e, &Ty::undef())
                        }
                    }
                    Key::Name(_) => Ty::Any(Reason::UnmodeledMethod),
                    Key::Pending => Ty::Bottom,
                    Key::Dyn => {
                        // a read cannot change the elements, but what it returns is unknown code's
                        let e = self.arr_elem(s);
                        self.escape_fns_in(&e, &mut Default::default());
                        Ty::Any(Reason::DynamicKey)
                    }
                },
                Atom::Obj(s) => {
                    let s = self.find(s);
                    match (&self.sites[s], key) {
                        (Site::Dead(r), _) => Ty::Any(*r),
                        (Site::Obj(_), Key::Name(n)) if PROTO_NAMES.contains(&n.as_str()) => {
                            Ty::Any(Reason::Proto)
                        }
                        (Site::Obj(_), Key::Name(n)) if self.proto_extra_of(s, n) => {
                            Ty::Any(Reason::Proto)
                        }
                        (Site::Obj(m), Key::Name(n)) => {
                            let own = m.get(n).cloned();
                            let methods = self.fam_candidates(s, n);
                            match (own, methods.is_empty()) {
                                (None, false) => {
                                    self.unsafe_dyn.extend(methods);
                                    Ty::Any(Reason::Closure)
                                }
                                (o, _) => o.unwrap_or_else(Ty::undef),
                            }
                        }
                        (Site::Obj(m), _) => {
                            // computed key read: the record is unchanged, its functions may be handed out
                            let fields: Vec<Ty> = m.values().cloned().collect();
                            for f in fields {
                                self.escape_fns_in(&f, &mut Default::default());
                            }
                            Ty::Any(Reason::DynamicKey)
                        }
                        _ => Ty::Any(Reason::DynamicKey),
                    }
                }
            };
            acc = self.join(&acc, &t);
        }
        acc
    }

    fn member_write(&mut self, obj: &Ty, key: &Key, v: &Ty) {
        if matches!(key, Key::Pending) {
            return;
        }
        let atoms = match obj {
            Ty::Any(r) => return self.taint_as(v, *r),
            Ty::Bottom => return,
            Ty::Set(s) => s.clone(),
        };
        for at in atoms {
            match at {
                Atom::Arr(s) => match key {
                    Key::Num => self.site_elem_flow(s, v),
                    Key::Name(n) if n == "length" => {}
                    _ => self.kill(s, Reason::DynamicKey),
                },
                Atom::Obj(s) => match key {
                    Key::Name(n)
                        if !PROTO_NAMES.contains(&n.as_str()) && !self.proto_extra_of(s, n) =>
                    {
                        self.site_field_flow(s, n, v, !self.def_write)
                    }
                    Key::Name(_) => self.kill(s, Reason::Proto),
                    _ => self.kill(s, Reason::DynamicKey),
                },
                // a property written onto an opaque built-in object can be read back by unknown code
                Atom::Ns(n) if crate::builtins::is_ext(n) => self.taint_as(v, Reason::Builtin),
                _ => {}
            }
        }
    }

    // ---------- calls ----------
    fn call_known(&mut self, f: usize, args: &[(Ty, bool)]) -> Ty {
        let spread = args.iter().any(|x| x.1);
        let params = self.st.funcs[f].params.clone();
        let defaults = self.st.funcs[f].defaults.clone();
        for (i, p) in params.iter().enumerate() {
            let t = if spread {
                Some(Ty::Any(Reason::Spread))
            } else if let Some((t, _)) = args.get(i) {
                Some(t.clone())
            } else if !defaults[i] {
                Some(Ty::undef())
            } else {
                None
            };
            match (p, t) {
                (Some(n), Some(t)) => self.flow(&(f, n.clone()), &t),
                (None, Some(t)) => self.taint_as(&t, Reason::Destructure),
                _ => {}
            }
        }
        for (t, _) in args.iter().skip(params.len()) {
            self.taint_as(t, Reason::Arguments);
        }
        let fi = &self.st.funcs[f];
        if fi.async_ {
            return self.async_result(f);
        }
        if fi.gen {
            return Ty::Any(Reason::Generator);
        }
        let ret = fi.ret.clone();
        match (&ret, self.dts_ret.get(&f)) {
            (Ty::Any(_), Some(d)) => d.clone(),
            _ => ret,
        }
    }

    /// Call through a value: every function in the set is a possible target.
    fn call_value(&mut self, callee: &Ty, args: &[(Ty, bool)], recv_site: Option<usize>) -> Ty {
        match callee {
            Ty::Bottom => Ty::Bottom,
            Ty::Any(_) => self.unknown_call(args, Reason::UnknownCall),
            Ty::Set(atoms) => {
                let mut acc = Ty::Bottom;
                for at in atoms.clone() {
                    let t = match at {
                        Atom::Fn(f) => {
                            let f = f as usize;
                            if let (Some(s), true) = (
                                recv_site,
                                self.st.funcs[f].uses_this && !self.st.funcs[f].arrow,
                            ) {
                                // `this` is the receiver: writes through it are invisible to us
                                self.kill(s, Reason::This);
                            }
                            self.call_known(f, args)
                        }
                        Atom::Ns(crate::builtins::ASSERT) => Ty::undef(),
                        // calling a non-function throws
                        _ => Ty::Bottom,
                    };
                    acc = self.join(&acc, &t);
                }
                acc
            }
        }
    }

    fn unknown_call(&mut self, args: &[(Ty, bool)], r: Reason) -> Ty {
        for (t, _) in args {
            self.taint_as(t, r);
        }
        Ty::Any(r)
    }

    fn global_call(&mut self, name: &str, args: &[(Ty, bool)]) -> Ty {
        match name {
            "parseInt" | "parseFloat" | "Number" => Ty::num(),
            "isNaN" | "isFinite" | "Boolean" => Ty::bool(),
            "String" | "encodeURIComponent" | "decodeURIComponent" | "encodeURI" | "decodeURI"
            | "escape" | "unescape" => Ty::str(),
            "BigInt" => Ty::Any(Reason::BigInt),
            "require" => Ty::Any(Reason::Require),
            "eval" => Ty::Any(Reason::Eval),
            // the timer runs the callback later with the extra arguments (none for a microtask)
            "queueMicrotask" | "setTimeout" | "setImmediate" | "setInterval" if matches!(args.first(), Some((Ty::Set(a), false)) if !a.is_empty() && a.iter().all(|x| matches!(x, Atom::Fn(_)))) =>
            {
                let skip = if name == "queueMicrotask" || name == "setImmediate" {
                    1
                } else {
                    2
                };
                let cb = args[0].0.clone();
                let rest: Vec<(Ty, bool)> = args.iter().skip(skip).cloned().collect();
                if rest.iter().any(|r| r.1) {
                    return self.unknown_call(args, Reason::UnknownGlobal);
                }
                self.call_value(&cb, &rest, None);
                for (t, _) in args.iter().skip(1) {
                    self.taint_as(t, Reason::UnknownGlobal);
                }
                if name == "queueMicrotask" {
                    Ty::undef()
                } else {
                    Ty::Any(Reason::Builtin)
                }
            }
            _ => self.unknown_call(args, Reason::UnknownGlobal),
        }
    }

    fn static_call(&mut self, obj: &str, name: &str, args: &[(Ty, bool)], key: u32) -> Option<Ty> {
        Some(match (obj, name) {
            ("Math", _) => Ty::num(),
            ("Number", "isInteger" | "isFinite" | "isNaN" | "isSafeInteger") => Ty::bool(),
            ("Number", "parseFloat" | "parseInt") => Ty::num(),
            ("String", "fromCharCode" | "fromCodePoint" | "raw") => Ty::str(),
            ("Array", "isArray") => Ty::bool(),
            ("Date", "now" | "UTC") => Ty::num(),
            ("JSON", "stringify") => Ty::Set(vec![Atom::Str, Atom::Undef]),
            ("Date", "parse") => Ty::num(),
            ("Object", "freeze" | "seal" | "preventExtensions") => match args.first() {
                Some((t, false)) => t.clone(),
                _ => return None,
            },
            ("Object", "create") if matches!(args, [(t, false)] if *t == Ty::one(Atom::Null)) => {
                // a prototype-less dictionary: reads of absent names are `undefined`
                let s = self.site((key, 15), Site::Obj(Default::default()));
                Ty::one(Atom::Obj(s))
            }
            ("JSON", "parse") => Ty::Any(Reason::Builtin),
            ("Object", "isFrozen" | "isSealed" | "isExtensible" | "hasOwn" | "is") => Ty::bool(),
            ("Object", "getOwnPropertyNames") => {
                let s = self.site((key, 14), Site::Arr(Ty::str()));
                Ty::one(Atom::Arr(s))
            }
            ("Object", "keys") => {
                let s = self.site((key, 3), Site::Arr(Ty::str()));
                Ty::one(Atom::Arr(s))
            }
            _ => return None,
        })
    }

    fn method_call(
        &mut self,
        recv: &Ty,
        name: &str,
        args: &[(Ty, bool)],
        cbs: &[Option<usize>],
        key: u32,
    ) -> Ty {
        let atoms = match recv {
            Ty::Bottom => return Ty::Bottom,
            Ty::Any(r) => {
                for (t, _) in args {
                    self.taint_as(t, *r);
                }
                return Ty::Any(*r);
            }
            Ty::Set(s) => s.clone(),
        };
        let all_arr = atoms
            .iter()
            .all(|a| matches!(a, Atom::Arr(_) | Atom::Undef | Atom::Null));
        let cb = cbs.first().copied().flatten().filter(|_| all_arr);
        let cb_res = atoms.iter().find_map(|a| match (a, cb) {
            (Atom::Arr(s), Some(f)) => self.arr_callback(name, *s, args, f, key),
            _ => None,
        });
        let mut acc = Ty::Bottom;
        for at in atoms {
            let t = match at {
                Atom::Undef | Atom::Null => Ty::Bottom,
                Atom::Arr(_) if cb_res.is_some() => cb_res.clone().unwrap(),
                Atom::Ns(ns) => match self.ns_call(ns, name, args, key) {
                    Some(t) => t,
                    None => self.unknown_call(args, Reason::Builtin),
                },
                Atom::Fn(f) => {
                    // `f.call/apply/bind(...)`: invoked with unknown arguments
                    self.escape_fn(f as usize);
                    self.unknown_call(args, Reason::UnmodeledMethod)
                }
                Atom::Num | Atom::NLit(_) => match name {
                    "toFixed" | "toString" | "toPrecision" | "toExponential" | "toLocaleString" => {
                        Ty::str()
                    }
                    "valueOf" => Ty::num(),
                    _ => self.unknown_call(args, Reason::UnmodeledMethod),
                },
                Atom::Bool => match name {
                    "toString" => Ty::str(),
                    "valueOf" => Ty::bool(),
                    _ => self.unknown_call(args, Reason::UnmodeledMethod),
                },
                Atom::Str | Atom::SLit(_) => match name {
                    "charAt" | "toLowerCase" | "toUpperCase" | "toLocaleLowerCase"
                    | "toLocaleUpperCase" | "trim" | "trimStart" | "trimEnd" | "trimLeft"
                    | "trimRight" | "substring" | "substr" | "slice" | "padStart" | "padEnd"
                    | "repeat" | "replace" | "replaceAll" | "concat" | "normalize" | "toString"
                    | "valueOf" => Ty::str(),
                    "at" => Ty::Set(vec![Atom::Str, Atom::Undef]),
                    "charCodeAt" | "indexOf" | "lastIndexOf" | "localeCompare" | "search" => {
                        Ty::num()
                    }
                    "codePointAt" => Ty::Set(vec![Atom::Num, Atom::Undef]),
                    "startsWith" | "endsWith" | "includes" => Ty::bool(),
                    "split" => {
                        let s = self.site((key, 4), Site::Arr(Ty::str()));
                        Ty::one(Atom::Arr(s))
                    }
                    _ => self.unknown_call(args, Reason::UnmodeledMethod),
                },
                Atom::Arr(s) => match name {
                    "push" | "unshift" => {
                        for (t, sp) in args {
                            let t = if *sp {
                                Ty::Any(Reason::Spread)
                            } else {
                                t.clone()
                            };
                            self.site_elem_flow(s, &t);
                        }
                        Ty::num()
                    }
                    "pop" | "shift" | "at" => {
                        let e = self.arr_elem(s);
                        self.join(&e, &Ty::undef())
                    }
                    "join" | "toString" => Ty::str(),
                    "indexOf" | "lastIndexOf" => Ty::num(),
                    "includes" => Ty::bool(),
                    "slice" | "reverse" | "sort" => Ty::one(Atom::Arr(s)),
                    "fill" => {
                        if let Some((t, _)) = args.first() {
                            self.site_elem_flow(s, &t.clone());
                        }
                        Ty::one(Atom::Arr(s))
                    }
                    "splice" => {
                        for (t, _) in args.iter().skip(2) {
                            self.site_elem_flow(s, &t.clone());
                        }
                        Ty::one(Atom::Arr(s))
                    }
                    "concat" => {
                        for (t, _) in args {
                            match t {
                                Ty::Set(sa) => {
                                    for x in sa.clone() {
                                        match x {
                                            Atom::Arr(o) => {
                                                self.union_sites(s, o);
                                            }
                                            other => self.site_elem_flow(s, &Ty::one(other)),
                                        }
                                    }
                                }
                                o => self.site_elem_flow(s, &o.clone()),
                            }
                        }
                        Ty::one(Atom::Arr(s))
                    }
                    _ => {
                        // callbacks may mutate the receiver through their own parameters
                        self.kill(s, Reason::UnmodeledMethod);
                        self.unknown_call(args, Reason::UnmodeledMethod)
                    }
                },
                Atom::Obj(s) => {
                    let r = self.find_ro(s);
                    let own = matches!(&self.sites[r], Site::Obj(m) if m.contains_key(name));
                    let methods = self.fam_candidates(r, name);
                    if !methods.is_empty() && !own {
                        for m in methods {
                            let t = self.call_known(m, args);
                            acc = self.join(&acc, &t);
                        }
                        continue;
                    }
                    let field = match &self.sites[r] {
                        Site::Obj(m) => m.get(name).cloned(),
                        _ => None,
                    };
                    if let Some(ft @ Ty::Set(_)) = &field {
                        if ft_has_fn(ft) && !ft_has_non_fn(ft) {
                            let t = self.call_value(ft, args, Some(r));
                            acc = self.join(&acc, &t);
                            continue;
                        }
                    }
                    let has_own = matches!(&self.sites[r], Site::Obj(m) if m.contains_key(name));
                    if name == "hasOwnProperty" && !has_own {
                        Ty::bool()
                    } else {
                        self.kill(s, Reason::UnknownCall);
                        self.unknown_call(args, Reason::UnknownCall)
                    }
                }
            };
            acc = self.join(&acc, &t);
        }
        acc
    }

    pub(crate) fn norm_pub(&mut self, v: Vec<Atom>) -> Ty {
        self.norm(v)
    }

    pub(crate) fn site_field_pub(&mut self, s: usize, name: &str, t: &Ty) {
        self.site_field_flow(s, name, t, false);
    }

    pub(crate) fn site_pub(&mut self, key: (u32, u8), mk: Site) -> usize {
        self.site(key, mk)
    }

    pub(crate) fn class_site_pub(&mut self, k: &(usize, String)) -> usize {
        let s = self.class_site(k);
        self.find(s)
    }

    /// Instance fields of a modelled class as (name, type text); `any` for untyped ones.
    pub(crate) fn class_fields(&mut self, k: &(usize, String)) -> Option<Vec<(String, String)>> {
        if !self.class_modelled(k) {
            return None;
        }
        let root = self.family_root(k);
        let s = *self.class_sites.get(&root)?;
        let s = self.find(s);
        let Site::Obj(m) = self.sites[s].clone() else {
            return None;
        };
        let mut out = Vec::new();
        for (n, t) in m {
            // the nearest class (self, then ancestors) whose own constructor stores the field
            let mut definer: Option<(usize, String)> = None;
            let mut cur = Some(k.clone());
            for _ in 0..64 {
                let Some(c) = cur else { break };
                if self.ctor_fields.contains_key(&(c.clone(), n.clone())) {
                    definer = Some(c);
                    break;
                }
                cur = self.st.parents.get(&c).cloned();
            }
            let t = match &definer {
                // inherited from an ancestor's declaration (the TS pipeline rejects redeclaration)
                Some(d) if d != k => continue,
                Some(d) => {
                    let own = self.ctor_fields[&(d.clone(), n.clone())].clone();
                    let keep_undef = matches!(&own, Ty::Set(s) if s.contains(&Atom::Undef));
                    match &t {
                        Ty::Set(s) => {
                            let v: Vec<Atom> = s
                                .iter()
                                .copied()
                                .filter(|a| keep_undef || *a != Atom::Undef)
                                .collect();
                            self.norm_pub(v)
                        }
                        o => o.clone(),
                    }
                }
                None => {
                    // not stored by this class's chain: declare it once, in the root, unless some
                    // other class of the hierarchy stores it
                    let others = self.ctor_fields.keys().any(|(c, f)| f == &n && c != k);
                    if others || self.family_root(k) != *k {
                        continue;
                    }
                    t
                }
            };
            let (c, _) = self.classify_pub(&t);
            let s = if c == Class::Any {
                "any".to_string()
            } else {
                self.ts_pub(&t)
            };
            out.push((
                n,
                if s.contains("Function") || s.contains("NodeModule") {
                    "any".to_string()
                } else {
                    s
                },
            ));
        }
        Some(out)
    }

    pub(crate) fn fam_size(&self, k: &(usize, String)) -> usize {
        let root = self.family_root(k);
        self.st.fam_size.get(&root).copied().unwrap_or(1)
    }

    pub(crate) fn site_class_of(&self, root: usize) -> Option<(usize, String)> {
        self.site_class.get(&root).cloned()
    }

    pub(crate) fn class_modelled(&self, k: &(usize, String)) -> bool {
        self.st.classes.get(k).is_some_and(|c| {
            !self.st.complex.contains(k)
                && self.st.family.contains_key(k)
                && (c.ctor.is_some() || c.es)
        })
    }

    fn is_definite(&self, name: &str) -> bool {
        self.st
            .resolve(*self.cur.last().unwrap(), name)
            .and_then(|k| self.st.binds.get(&k))
            .is_some_and(|b| b.definite)
    }

    /// `arr.map/filter/forEach/...(function literal)`: callback parameters from the element type.
    fn arr_callback(
        &mut self,
        name: &str,
        s: usize,
        args: &[(Ty, bool)],
        f: usize,
        key: u32,
    ) -> Option<Ty> {
        let one = args.len() == 1;
        let ok = match name {
            "map" | "filter" | "forEach" | "some" | "every" | "find" | "findIndex" | "findLast"
            | "findLastIndex" | "sort" => one,
            "reduce" | "reduceRight" => args.len() <= 2,
            _ => false,
        };
        if !ok || args.iter().any(|a| a.1) {
            return None;
        }
        self.cb_flowed.insert(f);
        let e = self.arr_elem(s);
        let me = Ty::one(Atom::Arr(s));
        let ret = self.fn_result(f);
        Some(match name {
            "reduce" | "reduceRight" => {
                let base = match args.get(1) {
                    Some((i, _)) => i.clone(),
                    None => e.clone(),
                };
                let acc = self.join(&base, &ret);
                self.flow_param(f, 0, &acc);
                self.flow_param(f, 1, &e);
                self.flow_param(f, 2, &Ty::num());
                self.flow_param(f, 3, &me);
                acc
            }
            "sort" => {
                self.flow_param(f, 0, &e);
                self.flow_param(f, 1, &e);
                me
            }
            _ => {
                self.flow_param(f, 0, &e);
                self.flow_param(f, 1, &Ty::num());
                self.flow_param(f, 2, &me);
                match name {
                    "map" => {
                        let ns = self.site((key, 5), Site::Arr(Ty::Bottom));
                        self.site_elem_flow(ns, &ret);
                        Ty::one(Atom::Arr(self.find(ns)))
                    }
                    "filter" => me,
                    "forEach" => Ty::undef(),
                    "some" | "every" => Ty::bool(),
                    "find" | "findLast" => self.join(&e, &Ty::undef()),
                    _ => Ty::num(),
                }
            }
        })
    }

    /// `n` was added to the prototype of the class that owns `site` by `defineProperty` / a literal
    fn proto_extra_of(&self, site: usize, n: &str) -> bool {
        if self.st.proto_extra.is_empty() {
            return false;
        }
        let Some(root) = self.site_class.get(&site) else {
            return false;
        };
        self.st
            .proto_extra
            .iter()
            .any(|(k, m)| m == n && self.family_root(k) == *root)
    }

    fn find_ro(&mut self, s: usize) -> usize {
        self.find(s)
    }

    fn call(&mut self, c: &a::CallExpr) -> Ty {
        let args: Vec<(Ty, bool)> = c
            .args
            .iter()
            .map(|x| (self.ty(&x.expr), x.spread.is_some()))
            .collect();
        // function literals passed as arguments: parameters come from a modelled array method, else `any`
        let cbs: Vec<Option<usize>> = c
            .args
            .iter()
            .map(|x| {
                if x.spread.is_some() {
                    None
                } else {
                    self.fn_of_expr(&x.expr)
                }
            })
            .collect();
        self.call_inner(c, &args, &cbs)
    }

    fn fn_of_expr(&self, e: &a::Expr) -> Option<usize> {
        match strip(e) {
            a::Expr::Ident(i) => {
                let k = self.st.resolve(*self.cur.last().unwrap(), &i.sym)?;
                let b = self.st.binds.get(&k)?;
                if b.n_assign == 1 {
                    b.fn_value
                } else {
                    None
                }
            }
            a::Expr::Fn(f) => self
                .st
                .by_span
                .get(&(f.function.span.lo.0, f.function.span.hi.0))
                .copied(),
            a::Expr::Arrow(f) => self.st.by_span.get(&(f.span.lo.0, f.span.hi.0)).copied(),
            _ => None,
        }
    }

    fn flow_param(&mut self, f: usize, i: usize, t: &Ty) {
        match self.st.funcs[f].params.get(i) {
            Some(Some(n)) => {
                let n = n.clone();
                self.flow(&(f, n), t)
            }
            Some(None) => self.taint_as(t, Reason::Destructure),
            None => {}
        }
    }

    /// An async function returns a promise; what it resolves to is read by `await` / `.then`, which are
    /// unknown code, so the values it returns are tainted.
    fn async_result(&mut self, f: usize) -> Ty {
        let ret = self.st.funcs[f].ret.clone();
        self.taint_as(&ret, Reason::Async);
        Ty::one(Atom::Ns(crate::builtins::PROMISE))
    }

    fn fn_result(&mut self, f: usize) -> Ty {
        let fi = &self.st.funcs[f];
        if fi.async_ {
            self.async_result(f)
        } else if fi.gen {
            Ty::Any(Reason::Generator)
        } else {
            fi.ret.clone()
        }
    }

    fn call_inner(&mut self, c: &a::CallExpr, args: &[(Ty, bool)], cbs: &[Option<usize>]) -> Ty {
        let args = args.to_vec();
        let fid = *self.cur.last().unwrap();
        let a::Callee::Expr(ce) = &c.callee else {
            // `super(args)`: the parent constructor runs on the same instance
            let cur_class = self.st.fn_class.get(&self.nonarrow(fid)).cloned();
            let parent = cur_class.and_then(|k| self.st.parents.get(&k).cloned());
            if let (a::Callee::Super(_), Some(p)) = (&c.callee, parent) {
                if let Some(ctor) = self.ctor_of(&p) {
                    self.call_known(ctor, &args);
                    return Ty::undef();
                }
            }
            return self.unknown_call(&args, Reason::Class);
        };
        // `A.call(this, args)` / `A.apply(this, args)` in a constructor-function hierarchy
        if let a::Expr::Member(mm) = strip(ce) {
            if let (a::Expr::Ident(x), a::MemberProp::Ident(pn)) = (strip(&mm.obj), &mm.prop) {
                let this_first = matches!(
                    c.args.first().map(|a0| strip(&a0.expr)),
                    Some(a::Expr::This(_))
                );
                if this_first && matches!(&*pn.sym, "call" | "apply") {
                    if let Some(k) = self.st.resolve(fid, &x.sym) {
                        if self.class_modelled(&k) {
                            if let Some(ctor) = self.ctor_of(&k) {
                                if &*pn.sym == "call" {
                                    self.call_known(ctor, &args[1..]);
                                } else {
                                    self.call_known(ctor, &[(Ty::Any(Reason::Spread), true)]);
                                }
                                return Ty::undef();
                            }
                        }
                    }
                }
            }
        }
        // `super.m(args)`
        if let a::Expr::SuperProp(sp) = strip(ce) {
            if let a::SuperProp::Ident(pn) = &sp.prop {
                let cur_class = self.st.fn_class.get(&self.nonarrow(fid)).cloned();
                let parent = cur_class.and_then(|k| self.st.parents.get(&k).cloned());
                if let Some(m) = parent.and_then(|p| self.method_up(&p, &pn.sym)) {
                    return self.call_known(m, &args);
                }
            }
            return self.unknown_call(&args, Reason::Class);
        }
        if matches!(strip(ce), a::Expr::Ident(i) if &*i.sym == "require") {
            if let Some(a::Expr::Lit(a::Lit::Str(s))) = c.args.first().map(|x| &*x.expr) {
                if let Some(ns) = crate::builtins::ns_of_require(s.value.as_str().unwrap_or("")) {
                    return Ty::one(Atom::Ns(ns));
                }
                let target = self.st.bundle.as_ref().and_then(|b| {
                    let fk = self.st.factory_of(fid);
                    b.require_maps.get(&fk)?.get(s.value.as_str()?).cloned()
                });
                if let Some(t) = target {
                    return self.module_exports_value(&t);
                }
            } else if !c.args.is_empty() {
                self.dyn_require = true;
            }
            return Ty::Any(Reason::Require);
        }
        match strip(ce) {
            a::Expr::Ident(i) => match self.st.resolve(fid, &i.sym) {
                Some(k) => {
                    let b = &self.st.binds[&k];
                    if let (1, Some(f)) = (b.n_assign, b.fn_value) {
                        self.call_known(f, &args)
                    } else {
                        let callee = self.read_name(&i.sym);
                        self.call_value(&callee, &args, None)
                    }
                }
                None => self.global_call(&i.sym, &args),
            },
            a::Expr::Member(m) => {
                if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (strip(&m.obj), &m.prop) {
                    if self.st.resolve(fid, &o.sym).is_none() {
                        if let Some(t) = self.static_call(&o.sym, &p.sym, &args, c.span.lo.0) {
                            return t;
                        }
                        if crate::builtins::ns_of_global(&o.sym).is_none() {
                            return self.unknown_call(&args, Reason::UnknownGlobal);
                        }
                    }
                }
                // Object.prototype.hasOwnProperty.call(o, k)
                if let (a::Expr::Member(inner), a::MemberProp::Ident(p)) = (strip(&m.obj), &m.prop)
                {
                    if p.sym == "call"
                        && matches!(&inner.prop, a::MemberProp::Ident(q) if q.sym == "hasOwnProperty")
                    {
                        return Ty::bool();
                    }
                }
                let recv = self.ty(&m.obj);
                // `var u = require('./x'); u.f(args)`: evidence for the exported `f` of that module
                if let (a::Expr::Ident(o), a::MemberProp::Ident(p)) = (strip(&m.obj), &m.prop) {
                    let target = self
                        .st
                        .resolve(fid, &o.sym)
                        .and_then(|k| self.st.binds.get(&k))
                        .and_then(|b| b.req.clone());
                    if let Some(t) = target {
                        self.add_evidence((t, p.sym.to_string()), &args);
                    }
                }
                match &m.prop {
                    a::MemberProp::Ident(p) => {
                        if recv == Ty::Bottom {
                            // receiver not known yet: decide the callbacks' parameters later
                            cbs.iter().flatten().for_each(|f| {
                                self.cb_flowed.insert(*f);
                            });
                        }
                        self.method_call(&recv, &p.sym, &args, cbs, c.span.lo.0)
                    }
                    _ => {
                        self.taint_as(&recv, Reason::DynamicKey);
                        self.unknown_call(&args, Reason::DynamicKey)
                    }
                }
            }
            e @ (a::Expr::Fn(_) | a::Expr::Arrow(_)) => {
                let f = match e {
                    a::Expr::Fn(f) => self
                        .st
                        .by_span
                        .get(&(f.function.span.lo.0, f.function.span.hi.0)),
                    a::Expr::Arrow(f) => self.st.by_span.get(&(f.span.lo.0, f.span.hi.0)),
                    _ => None,
                }
                .copied();
                e.visit_with(self);
                match f {
                    Some(f) => self.call_known(f, &args),
                    None => self.unknown_call(&args, Reason::UnknownCall),
                }
            }
            other => {
                let t = self.ty(other);
                self.call_value(&t, &args, None)
            }
        }
    }

    // ---------- expressions ----------
    pub fn ty(&mut self, e: &a::Expr) -> Ty {
        let t = self.ty_inner(e);
        if let Ty::Set(atoms) = &t {
            for at in atoms {
                if let Atom::Fn(f) = at {
                    self.seen_fns.insert(*f as usize);
                }
            }
        }
        let fid = *self.cur.last().unwrap();
        self.st.funcs[fid].exprs += 1;
        if let Ty::Any(r) = &t {
            self.st.funcs[fid].any_exprs += 1;
            *self.any_hist.entry(*r).or_default() += 1;
        }
        t
    }

    fn read_name(&mut self, name: &str) -> Ty {
        let fid = *self.cur.last().unwrap();
        match self.st.resolve(fid, name) {
            Some(k) => {
                if let Some(t) = self.narrow_lookup(&k) {
                    return t;
                }
                let t = self.st.binds[&k].ty.clone();
                self.join(&t, &Ty::Bottom)
            }
            None => match name {
                "undefined" => Ty::undef(),
                "NaN" | "Infinity" => Ty::num(),
                "arguments" => Ty::Any(Reason::Arguments),
                _ => match crate::builtins::ns_of_global(name) {
                    Some(ns) => Ty::one(Atom::Ns(ns)),
                    None => Ty::Any(Reason::UnknownGlobal),
                },
            },
        }
    }

    fn ty_inner(&mut self, e: &a::Expr) -> Ty {
        use a::Expr as E;
        match e {
            E::Paren(p) => self.ty(&p.expr),
            E::This(_) => {
                let f = *self.cur.last().unwrap();
                self.this_ty(f)
            }
            E::Lit(l) => match l {
                a::Lit::Num(n) => {
                    if n.value.fract() == 0.0 && n.value.abs() < 1e9 {
                        Ty::one(Atom::NLit(n.value as i32))
                    } else {
                        Ty::num()
                    }
                }
                a::Lit::Str(st) => {
                    let id = self.intern(st.value.as_str().unwrap_or("\u{fffd}"));
                    Ty::one(Atom::SLit(id))
                }
                a::Lit::Bool(_) => Ty::bool(),
                a::Lit::Null(_) => Ty::one(Atom::Null),
                a::Lit::BigInt(_) => Ty::Any(Reason::BigInt),
                a::Lit::Regex(_) => Ty::Any(Reason::Regex),
                _ => Ty::Any(Reason::Other),
            },
            E::Tpl(t) => {
                t.exprs.iter().for_each(|x| {
                    let ty = self.ty(x);
                    self.taint_as(&ty, Reason::Coercion);
                });
                Ty::str()
            }
            E::Ident(i) => self.read_name(&i.sym),
            E::Seq(s) => {
                let mut last = Ty::Bottom;
                for x in &s.exprs {
                    last = self.ty(x);
                }
                last
            }
            E::Cond(c) => {
                self.ty(&c.test);
                let (mut ct, mut cf) = (Vec::new(), Vec::new());
                self.conds(&c.test, true, &mut ct);
                self.conds(&c.test, false, &mut cf);
                let x = self.with_narrow(ct, &|n| assigns_in_expr(&c.cons, n), |s| s.ty(&c.cons));
                let y = self.with_narrow(cf, &|n| assigns_in_expr(&c.alt, n), |s| s.ty(&c.alt));
                self.join(&x, &y)
            }
            E::Bin(b) if matches!(b.op, a::BinaryOp::LogicalAnd | a::BinaryOp::LogicalOr) => {
                let x = self.ty(&b.left);
                let mut cs = Vec::new();
                self.conds(&b.left, b.op == a::BinaryOp::LogicalAnd, &mut cs);
                let y = self.with_narrow(cs, &|n| assigns_in_expr(&b.right, n), |s| s.ty(&b.right));
                // `a && b` is the falsy part of `a`, else `b` (dually for `||`)
                let part = self.apply(
                    &crate::narrow::Flt::Truthy,
                    &x,
                    b.op == a::BinaryOp::LogicalOr,
                );
                match &x {
                    Ty::Set(_) => self.join(&part, &y),
                    _ => self.join(&x, &y),
                }
            }
            E::Bin(b) => {
                let (x, y) = (self.ty(&b.left), self.ty(&b.right));
                self.binop(b.op, &x, &y)
            }
            E::Unary(u) => {
                use a::UnaryOp::*;
                match u.op {
                    TypeOf => {
                        if let E::Ident(i) = strip(&u.arg) {
                            if self.st.resolve(*self.cur.last().unwrap(), &i.sym).is_none() {
                                return Ty::str();
                            }
                        }
                        self.ty(&u.arg);
                        Ty::str()
                    }
                    Bang => {
                        self.ty(&u.arg);
                        Ty::bool()
                    }
                    Void => {
                        self.ty(&u.arg);
                        Ty::undef()
                    }
                    Delete => {
                        if let E::Member(m) = strip(&u.arg) {
                            let o = self.ty(&m.obj);
                            if let Ty::Set(s) = &o {
                                for at in s.clone() {
                                    if let Atom::Obj(x) | Atom::Arr(x) = at {
                                        self.kill(x, Reason::ShapeChange);
                                    }
                                }
                            }
                            self.key_of(&m.prop);
                        } else {
                            self.ty(&u.arg);
                        }
                        Ty::bool()
                    }
                    Plus => {
                        self.ty(&u.arg);
                        Ty::num()
                    }
                    Minus | Tilde => {
                        let t = self.ty(&u.arg);
                        self.arith(&t, &t)
                    }
                }
            }
            E::Update(u) => {
                let t = self.ty(&u.arg);
                let r = if t.prim_only() {
                    Ty::num()
                } else if let Ty::Any(r) = &t {
                    Ty::Any(*r)
                } else if t == Ty::Bottom {
                    Ty::Bottom
                } else {
                    Ty::Any(Reason::Coercion)
                };
                self.assign_to_expr(&u.arg, &r);
                r
            }
            E::Assign(asg) => self.assign(asg),
            E::Member(m) => {
                if let (E::Ident(o), a::MemberProp::Ident(p)) = (strip(&m.obj), &m.prop) {
                    if self.st.resolve(*self.cur.last().unwrap(), &o.sym).is_none() {
                        if let Some(t) = self.global_prop(&o.sym, &p.sym) {
                            return t;
                        }
                    }
                }
                let o = self.ty(&m.obj);
                let k = self.key_of(&m.prop);
                self.read_key = m.span.lo.0;
                self.member_read(&o, &k)
            }
            E::Call(c) => self.call(c),
            E::New(n) => {
                let args: Vec<(Ty, bool)> = n
                    .args
                    .iter()
                    .flatten()
                    .map(|x| (self.ty(&x.expr), x.spread.is_some()))
                    .collect();
                let key = match strip(&n.callee) {
                    E::Ident(i) => self.st.resolve(*self.cur.last().unwrap(), &i.sym),
                    _ => None,
                };
                if key.is_none() {
                    n.callee.visit_with(self);
                }
                let ext = match (&key, strip(&n.callee)) {
                    (None, E::Ident(i)) => crate::builtins::ext_of_ctor(&i.sym),
                    _ => None,
                };
                match key.and_then(|k| self.new_class(&k, &args)) {
                    Some(t) => t,
                    None if ext.is_some() => {
                        for (t, _) in &args {
                            self.taint_as(t, Reason::New);
                        }
                        Ty::one(Atom::Ns(ext.unwrap()))
                    }
                    None => {
                        for (t, _) in &args {
                            self.taint_as(t, Reason::New);
                        }
                        Ty::Any(Reason::New)
                    }
                }
            }
            E::Array(arr) => {
                let s = self.site((arr.span.lo.0, 2), Site::Arr(Ty::Bottom));
                for el in &arr.elems {
                    match el {
                        None => self.site_elem_flow(s, &Ty::undef()),
                        Some(x) => {
                            let t = self.ty(&x.expr);
                            let t = if x.spread.is_some() {
                                self.taint_as(&t, Reason::Spread);
                                Ty::Any(Reason::Spread)
                            } else {
                                t
                            };
                            self.site_elem_flow(s, &t);
                        }
                    }
                }
                Ty::one(Atom::Arr(self.find(s)))
            }
            E::Object(o) => self.object_lit(o),
            E::Fn(f) => {
                f.function.visit_with(self);
                match self
                    .st
                    .by_span
                    .get(&(f.function.span.lo.0, f.function.span.hi.0))
                {
                    Some(&id) => Ty::one(Atom::Fn(id as u32)),
                    None => Ty::Any(Reason::Closure),
                }
            }
            E::Arrow(f) => {
                f.visit_with(self);
                match self.st.by_span.get(&(f.span.lo.0, f.span.hi.0)) {
                    Some(&id) => Ty::one(Atom::Fn(id as u32)),
                    None => Ty::Any(Reason::Closure),
                }
            }
            E::Class(c) => {
                c.class.visit_with(self);
                Ty::Any(Reason::Class)
            }
            E::Await(x) => {
                let t = self.ty(&x.arg);
                self.taint_as(&t, Reason::Async);
                Ty::Any(Reason::Async)
            }
            E::Yield(y) => {
                y.arg.visit_with(self);
                Ty::Any(Reason::Generator)
            }
            E::OptChain(_) => {
                e.visit_children_with(self);
                Ty::Any(Reason::OptChain)
            }
            _ => {
                e.visit_children_with(self);
                Ty::Any(Reason::Other)
            }
        }
    }

    fn object_lit(&mut self, o: &a::ObjectLit) -> Ty {
        let s = self.site((o.span.lo.0, 1), Site::Obj(BTreeMap::new()));
        let mut dead: Option<Reason> = None;
        for p in &o.props {
            match p {
                a::PropOrSpread::Spread(sp) => {
                    let t = self.ty(&sp.expr);
                    self.taint_as(&t, Reason::Spread);
                    dead = Some(Reason::Spread);
                }
                a::PropOrSpread::Prop(p) => match &**p {
                    a::Prop::Shorthand(i) => {
                        let t = self.read_name(&i.sym);
                        self.site_field_flow(s, &i.sym, &t, false);
                    }
                    a::Prop::KeyValue(kv) => {
                        let name = match &kv.key {
                            a::PropName::Ident(i) => Some(i.sym.to_string()),
                            a::PropName::Str(s) => s.value.as_str().map(str::to_string),
                            a::PropName::Num(n) => Some(n.value.to_string()),
                            _ => None,
                        };
                        let t = self.ty(&kv.value);
                        match name {
                            Some(n) if n == "__proto__" => dead = Some(Reason::Proto),
                            Some(n) => self.site_field_flow(s, &n, &t, false),
                            None => {
                                if let a::PropName::Computed(c) = &kv.key {
                                    self.ty(&c.expr);
                                }
                                dead = Some(Reason::DynamicKey);
                            }
                        }
                    }
                    other => {
                        other.visit_children_with(self);
                        dead = Some(Reason::Closure);
                    }
                },
            }
        }
        if let Some(r) = dead {
            self.kill(s, r);
        }
        Ty::one(Atom::Obj(self.find(s)))
    }

    fn assign_to_expr(&mut self, target: &a::Expr, v: &Ty) {
        match strip(target) {
            a::Expr::Ident(i) => self.flow_name(&i.sym, v),
            a::Expr::Member(m) => {
                let o = self.ty(&m.obj);
                let k = self.key_of(&m.prop);
                self.member_write(&o, &k, v);
            }
            _ => self.taint_as(v, Reason::DynamicKey),
        }
    }

    fn assign(&mut self, asg: &a::AssignExpr) -> Ty {
        let rhs = self.ty(&asg.right);
        use a::AssignOp as O;
        let binop = match asg.op {
            O::Assign => None,
            O::AddAssign => Some(a::BinaryOp::Add),
            O::SubAssign => Some(a::BinaryOp::Sub),
            O::MulAssign => Some(a::BinaryOp::Mul),
            O::DivAssign => Some(a::BinaryOp::Div),
            O::ModAssign => Some(a::BinaryOp::Mod),
            O::LShiftAssign => Some(a::BinaryOp::LShift),
            O::RShiftAssign => Some(a::BinaryOp::RShift),
            O::ZeroFillRShiftAssign => Some(a::BinaryOp::ZeroFillRShift),
            O::BitOrAssign => Some(a::BinaryOp::BitOr),
            O::BitXorAssign => Some(a::BinaryOp::BitXor),
            O::BitAndAssign => Some(a::BinaryOp::BitAnd),
            O::ExpAssign => Some(a::BinaryOp::Exp),
            O::AndAssign => Some(a::BinaryOp::LogicalAnd),
            O::OrAssign => Some(a::BinaryOp::LogicalOr),
            O::NullishAssign => Some(a::BinaryOp::NullishCoalescing),
        };
        match &asg.left {
            a::AssignTarget::Simple(s) => {
                let target_expr = match s {
                    a::SimpleAssignTarget::Ident(bi) => Some(a::Expr::Ident(bi.id.clone())),
                    a::SimpleAssignTarget::Member(m) => Some(a::Expr::Member(m.clone())),
                    _ => None,
                };
                let Some(te) = target_expr else {
                    self.taint_as(&rhs, Reason::Other);
                    return Ty::Any(Reason::Other);
                };
                // `this.f = e;` as a direct constructor statement: always initialised
                let cur = *self.cur.last().unwrap();
                let ctor_class = self
                    .st
                    .fn_class
                    .get(&cur)
                    .filter(|k| self.st.classes[*k].ctor == Some(cur))
                    .cloned();
                let direct = binop.is_none()
                    && ctor_class.is_some()
                    && self.st.funcs[cur].direct_this.contains_key(&asg.span.lo.0);
                self.def_write = direct
                    && ctor_class
                        .as_ref()
                        .is_some_and(|k| self.family_root(k) == *k);
                let v = match binop {
                    None => rhs.clone(),
                    Some(op) => {
                        let cur = self.ty(&te);
                        self.binop(op, &cur, &rhs)
                    }
                };
                self.assign_to_expr(&te, &v);
                self.def_write = false;
                if let (true, Some(k)) = (direct, ctor_class) {
                    // what this class's own constructor stores (no `undefined` from mere absence)
                    if let Some(name) = self.st.funcs[cur].direct_this.get(&asg.span.lo.0).cloned()
                    {
                        let w = self.widen(&v);
                        let old = self
                            .ctor_fields
                            .remove(&(k.clone(), name.clone()))
                            .unwrap_or(Ty::Bottom);
                        let n = self.join(&old, &w);
                        self.ctor_fields.insert((k, name), n);
                    }
                }
                v
            }
            a::AssignTarget::Pat(p) => {
                self.taint_as(&rhs, Reason::Destructure);
                let pat = match p {
                    a::AssignTargetPat::Array(x) => Some(a::Pat::Array(x.clone())),
                    a::AssignTargetPat::Object(x) => Some(a::Pat::Object(x.clone())),
                    _ => None,
                };
                if let Some(pat) = pat {
                    self.flow_any_pat(&pat, Reason::Destructure);
                }
                Ty::Any(Reason::Destructure)
            }
        }
    }

    // ---------- classes / `this` ----------
    /// Root of the hierarchy a class belongs to (instances of the hierarchy share one record).
    pub(crate) fn family_root(&self, k: &(usize, String)) -> (usize, String) {
        self.st.family.get(k).cloned().unwrap_or_else(|| k.clone())
    }

    fn class_site(&mut self, k: &(usize, String)) -> usize {
        let root = self.family_root(k);
        if let Some(&s) = self.class_sites.get(&root) {
            return self.find(s);
        }
        let id = self.sites.len();
        self.sites.push(Site::Obj(BTreeMap::new()));
        self.class_sites.insert(root.clone(), id);
        self.site_class.insert(id, root);
        id
    }

    /// Nearest constructor at or above `k` in its hierarchy.
    fn ctor_of(&self, k: &(usize, String)) -> Option<usize> {
        let mut cur = k.clone();
        for _ in 0..64 {
            if let Some(c) = self.st.classes.get(&cur).and_then(|c| c.ctor) {
                return Some(c);
            }
            cur = self.st.parents.get(&cur)?.clone();
        }
        None
    }

    /// Nearest method `name` at or above `k` (for `super.m()`).
    fn method_up(&self, k: &(usize, String), name: &str) -> Option<usize> {
        let mut cur = k.clone();
        for _ in 0..64 {
            if let Some(m) = self.st.classes.get(&cur).and_then(|c| c.methods.get(name)) {
                return Some(*m);
            }
            cur = self.st.parents.get(&cur)?.clone();
        }
        None
    }

    fn fam_candidates(&self, site: usize, name: &str) -> Vec<usize> {
        self.site_class
            .get(&site)
            .and_then(|root| self.st.fam_methods.get(root))
            .and_then(|m| m.get(name))
            .cloned()
            .unwrap_or_default()
    }

    fn nonarrow(&self, mut f: usize) -> usize {
        while f != 0 && self.st.funcs[f].arrow {
            f = self.st.funcs[f].parent;
        }
        f
    }

    /// Instance method of a class that is never exported and whose instances never reach dynamic
    /// code: its callers are all visible, so its parameters come from those calls.
    fn method_closed(&mut self, fid: usize) -> bool {
        let Some(k) = self.st.fn_class.get(&fid).cloned() else {
            return false;
        };
        let f = &self.st.funcs[fid];
        let root = self.family_root(&k);
        if f.is_ctor
            || !f.is_proto
            || self.st.fam_escaped.contains(&root)
            || self.unsafe_dyn.contains(&fid)
        {
            return false;
        }
        // never instantiated in the package: the instances come from outside
        if !self.st.fam_instantiated.contains(&root) {
            return false;
        }
        match self.class_sites.get(&root).copied() {
            Some(s) => {
                let s = self.find(s);
                !matches!(self.sites[s], Site::Dead(_))
            }
            None => true,
        }
    }

    pub fn this_ok(&self, fid: usize) -> bool {
        let f = self.nonarrow(fid);
        self.st.fn_class.contains_key(&f) && !self.unsafe_dyn.contains(&f)
    }

    /// `this` inside `fid`: an instance of the modelled class, else `any`.
    pub fn this_ty(&mut self, fid: usize) -> Ty {
        let f = self.nonarrow(fid);
        match self.st.fn_class.get(&f).cloned() {
            Some(k) if !self.unsafe_dyn.contains(&f) => {
                let s = self.class_site(&k);
                Ty::one(Atom::Obj(self.find(s)))
            }
            _ => Ty::Any(Reason::This),
        }
    }

    /// `new C(args)` for a modelled class.
    fn new_class(&mut self, k: &(usize, String), args: &[(Ty, bool)]) -> Option<Ty> {
        if !self.class_modelled(k) {
            return None;
        }
        let ctor = self.ctor_of(k);
        match ctor {
            Some(c) => {
                let ret = self.call_known(c, args);
                // a constructor returning an object replaces `this`
                if !matches!(&ret, Ty::Bottom) && !ret.only(Atom::Undef) {
                    for (t, _) in args {
                        self.taint_as(t, Reason::New);
                    }
                    return None;
                }
            }
            None => args.iter().for_each(|(t, _)| self.taint_as(t, Reason::New)),
        }
        let s = self.class_site(k);
        Some(Ty::one(Atom::Obj(self.find(s))))
    }

    // ---------- bundle modules ----------
    /// The `exports` and `module` records of a bundle factory (created on first use).
    fn mod_site_pair(&mut self, key: &str) -> (usize, usize) {
        if let Some(&(e, m)) = self.mod_sites.get(key) {
            return (self.find(e), self.find(m));
        }
        let e = self.sites.len();
        self.sites.push(Site::Obj(BTreeMap::new()));
        let m = self.sites.len();
        let mut fields = BTreeMap::new();
        fields.insert("exports".to_string(), Ty::one(Atom::Obj(e)));
        self.sites.push(Site::Obj(fields));
        self.mod_sites.insert(key.to_string(), (e, m));
        (e, m)
    }

    /// What `require('./x')` evaluates to: the target module's `module.exports`.
    fn module_exports_value(&mut self, key: &str) -> Ty {
        let (_, m) = self.mod_site_pair(key);
        let v = match &self.sites[m] {
            Site::Obj(f) => f.get("exports").cloned(),
            Site::Dead(r) => return Ty::Any(*r),
            _ => None,
        };
        match v {
            Some(t) => self.join(&t, &Ty::Bottom),
            None => Ty::Any(Reason::Require),
        }
    }

    fn factory_key(&self, fid: usize) -> Option<String> {
        self.st.funcs[fid]
            .exports
            .iter()
            .find(|(_, n)| n == "<factory>")
            .map(|(k, _)| k.clone())
    }

    /// Records that leave the bundle (the entry module) or can be reached by a dynamic `require`
    /// are visible to unknown code.
    fn escape_module_records(&mut self) {
        let keys: Vec<String> = match &self.st.bundle {
            Some(b) if self.dyn_require => b.factories.iter().map(|(k, _)| k.clone()).collect(),
            Some(b) => vec![b.entry.clone()],
            None => return,
        };
        for k in keys {
            let (e, m) = self.mod_site_pair(&k);
            let t = Ty::Set(vec![Atom::Obj(e), Atom::Obj(m)]);
            self.taint_as(&t, Reason::Escaped);
        }
    }

    // ---------- call-site evidence ----------
    fn add_evidence(&mut self, key: (String, String), args: &[(Ty, bool)]) {
        let spread = args.iter().any(|x| x.1);
        let had = self.ev.contains_key(&key);
        let mut v = self.ev.remove(&key).unwrap_or_default();
        while v.len() < args.len() {
            v.push(if had { Ty::undef() } else { Ty::Bottom });
        }
        for (i, slot) in v.iter_mut().enumerate() {
            let t = if spread {
                Ty::Any(Reason::Spread)
            } else {
                args.get(i).map_or_else(Ty::undef, |x| x.0.clone())
            };
            let t = self.widen(&t);
            let n = self.join(slot, &t);
            if n != *slot {
                self.changed = true;
            }
            *slot = n;
        }
        self.ev.insert(key, v);
    }

    /// Parameter `i` of an exported function from observed calls (cross-module calls in the bundle and
    /// the smoke programs / README examples given in `Config::evidence`). Trusted contract.
    fn evidence_param(&mut self, fid: usize, i: usize) -> Option<Ty> {
        let exports = self.st.funcs[fid].exports.clone();
        let mut acc: Option<Ty> = None;
        for (fk, name) in exports {
            if let Some(v) = self.ev.get(&(fk.clone(), name.clone())).cloned() {
                let t = v.get(i).cloned().unwrap_or_else(Ty::undef);
                acc = Some(match acc {
                    None => t,
                    Some(p) => self.join(&p, &t),
                });
            }
            let is_entry = match &self.st.bundle {
                Some(b) => b.entry == fk,
                None => fk.is_empty(),
            };
            let calls = match (&self.cfg.evidence, is_entry) {
                (Some(ev), true) => ev.calls.get(&name).cloned().unwrap_or_default(),
                _ => Vec::new(),
            };
            for (ci, call) in calls.iter().enumerate() {
                let t = match call.get(i) {
                    Some(dt) => {
                        let mut ctr = 0;
                        self.inst(dt, fid, 2_000_000 + ci * 64 + i, &mut ctr)
                    }
                    None => Ty::undef(),
                };
                acc = Some(match acc {
                    None => t,
                    Some(p) => self.join(&p, &t),
                });
            }
        }
        acc
    }

    // ---------- .d.ts hints ----------
    pub fn init_dts(&mut self) {
        let Some(idx) = self.cfg.dts.clone() else {
            return;
        };
        for fid in 1..self.st.funcs.len() {
            let f = &self.st.funcs[fid];
            if f.iife || f.bind.is_some() {
                continue;
            }
            // an anonymous function exported as the module's default value (`module.exports = (a) => ..`)
            // is the declaration that `export =` / `export default` names
            let anon_default = if matches!(f.prop_name.as_deref(), None | Some("exports"))
                && f.owner.is_none()
                && f.exports.iter().any(|(_, n)| n == "<default>")
            {
                idx.default_name.clone()
            } else {
                None
            };
            let Some(name) = anon_default.or_else(|| f.prop_name.clone()) else {
                continue;
            };
            let sigs: Vec<crate::dts::Sig> = idx
                .lookup(&name, f.owner.as_deref())
                .into_iter()
                .cloned()
                .collect();
            let sigs = if sigs.is_empty() && f.owner.is_some() && !f.is_proto && !f.is_ctor {
                // `utils.merge = function` on a plain local object: the declaration is a free function
                idx.lookup(&name, None).into_iter().cloned().collect()
            } else {
                sigs
            };
            if !sigs.is_empty() {
                self.dts_fns.insert(fid, sigs);
            }
        }
    }

    fn inst(&mut self, dt: &crate::dts::DT, fid: usize, slot: usize, ctr: &mut usize) -> Ty {
        use crate::dts::DT;
        match dt {
            DT::Num => Ty::num(),
            DT::Str => Ty::str(),
            DT::Bool => Ty::bool(),
            DT::Undef => Ty::undef(),
            DT::Null => Ty::one(Atom::Null),
            DT::Any => Ty::Any(Reason::Declared),
            DT::Union(ms) => {
                let mut acc = Ty::Bottom;
                for m in ms {
                    let t = self.inst(m, fid, slot, ctr);
                    acc = self.join(&acc, &t);
                }
                acc
            }
            DT::Arr(e) => {
                *ctr += 1;
                let k = (fid, slot, *ctr);
                let s = match self.dts_sites.get(&k) {
                    Some(&s) => self.find(s),
                    None => {
                        let id = self.sites.len();
                        self.sites.push(Site::Arr(Ty::Bottom));
                        self.dts_sites.insert(k, id);
                        id
                    }
                };
                let et = self.inst(e, fid, slot, ctr);
                self.site_elem_flow(s, &et);
                Ty::one(Atom::Arr(self.find(s)))
            }
            DT::Obj(fields) => {
                *ctr += 1;
                let k = (fid, slot, *ctr);
                let s = match self.dts_sites.get(&k) {
                    Some(&s) => self.find(s),
                    None => {
                        let id = self.sites.len();
                        self.sites.push(Site::Obj(BTreeMap::new()));
                        self.dts_sites.insert(k, id);
                        id
                    }
                };
                for (n, ft) in fields {
                    let t = self.inst(ft, fid, slot, ctr);
                    self.site_field_flow(s, n, &t, false);
                }
                Ty::one(Atom::Obj(self.find(s)))
            }
        }
    }

    /// Declared type of parameter `i` of `fid` (join over overloads), `None` if no signature has it.
    pub(crate) fn dts_param(&mut self, fid: usize, i: usize) -> Option<Ty> {
        let sigs = self.dts_fns.get(&fid)?.clone();
        let mut acc: Option<Ty> = None;
        for (si, s) in sigs.iter().enumerate() {
            if let Some(dt) = s.params.get(i) {
                let mut ctr = 0;
                let t = self.inst(dt, fid, i + 1000 * si, &mut ctr);
                acc = Some(match acc {
                    None => t,
                    Some(p) => self.join(&p, &t),
                });
            }
        }
        acc
    }

    fn refresh_dts_ret(&mut self) {
        let ids: Vec<usize> = self.dts_fns.keys().copied().collect();
        for fid in ids {
            let sigs = self.dts_fns[&fid].clone();
            let mut acc = Ty::Bottom;
            for (si, s) in sigs.iter().enumerate() {
                let mut ctr = 0;
                let t = self.inst(&s.ret, fid, 900_000 + si, &mut ctr);
                acc = self.join(&acc, &t);
            }
            self.dts_ret.insert(fid, acc);
        }
    }

    // ---------- driver ----------
    pub fn init_params(&mut self) {
        self.refresh_dts_ret();
        self.escape_module_records();
        for fid in 1..self.st.funcs.len() {
            let closed = {
                let f = &self.st.funcs[fid];
                f.iife || f.bind.is_some()
            } || self.method_closed(fid)
                || {
                    // a function value we can follow: closed as long as it never reaches dynamic code
                    let f = &self.st.funcs[fid];
                    f.atomic()
                        && f.exports.is_empty()
                        && !self.esc_fns.contains(&fid)
                        && (self.seen_fns.contains(&fid) || !self.escape_final)
                };
            if self.st.funcs[fid].async_ {
                // the promise it returns is read by unknown code
                self.async_result(fid);
            }
            if !closed {
                // closures returned from a function that external code calls are called externally
                let ret = self.st.funcs[fid].ret.clone();
                self.escape_fns_in(&ret, &mut Default::default());
            }
            let (uses_args, params, guess) = {
                let f = &self.st.funcs[fid];
                (f.uses_args, f.params.clone(), f.guess.clone())
            };
            let mut declared_params: Vec<String> = self.st.funcs[fid]
                .declared
                .iter()
                .filter(|(_, k)| **k == DK::Param)
                .map(|(n, _)| n.clone())
                .collect();
            declared_params.sort();
            let factory = self.factory_key(fid);
            for n in declared_params {
                let simple = params.iter().any(|p| p.as_deref() == Some(n.as_str()));
                let factory_param = factory.as_ref().and_then(|k| {
                    let i = params
                        .iter()
                        .position(|p| p.as_deref() == Some(n.as_str()))?;
                    (i < 2).then(|| (k.clone(), i))
                });
                let t = if let Some((k, i)) = factory_param {
                    // `module` / `exports` of a bundled module are records we can follow
                    let (e, m) = self.mod_site_pair(&k);
                    Some(Ty::one(Atom::Obj(if i == 0 { m } else { e })))
                } else if !simple {
                    Some(Ty::Any(Reason::Destructure))
                } else if uses_args {
                    Some(Ty::Any(Reason::Arguments))
                } else if !closed {
                    let i = params
                        .iter()
                        .position(|p| p.as_deref() == Some(n.as_str()))
                        .unwrap();
                    let dts = if self.dts_fns.contains_key(&fid) {
                        self.dts_param(fid, i)
                    } else {
                        None
                    };
                    if dts.is_some() {
                        self.st.funcs[fid].trust |= if self.jsdoc_fns.contains(&fid) {
                            TRUST_JSDOC
                        } else {
                            TRUST_DTS
                        };
                        dts
                    } else if let Some(t) = self.evidence_param(fid, i) {
                        self.st.funcs[fid].trust |= TRUST_EVIDENCE;
                        Some(t)
                    } else if self.cfg.hint_params && self.st.funcs[fid].defaults[i] {
                        // the default value (flowed in `enter`) is the evidence
                        self.st.funcs[fid].trust |= TRUST_HINT;
                        None
                    } else if let Some(t) = self.default_contract_ty(fid, i) {
                        self.st.funcs[fid].trust |= TRUST_DEFAULT;
                        Some(t)
                    } else if !self.escape_final && !self.st.funcs[fid].exports.is_empty() {
                        // evidence about an exported function may still arrive: decide `any` at the end
                        None
                    } else {
                        Some(match (self.cfg.hint_params, guess.get(&n)) {
                            (true, Some((true, false))) => {
                                self.st.funcs[fid].trust |= TRUST_HINT;
                                Ty::num()
                            }
                            (true, Some((false, true))) => {
                                self.st.funcs[fid].trust |= TRUST_HINT;
                                Ty::str()
                            }
                            _ => Ty::Any(Reason::Escaped),
                        })
                    }
                } else {
                    None
                };
                if let Some(t) = t {
                    self.flow(&(fid, n), &t);
                }
            }
        }
    }

    /// `function f(n = 10)`: with `default_contract`, callers are taken to pass what the default is
    /// (or nothing); the entry guard sends anything else down the original path.
    fn default_contract_ty(&self, fid: usize, i: usize) -> Option<Ty> {
        if !self.cfg.default_contract {
            return None;
        }
        match self.st.funcs[fid].default_lit.get(i)? {
            1 => Some(Ty::num()),
            2 => Some(Ty::str()),
            3 => Some(Ty::bool()),
            _ => None,
        }
    }

    /// JSDoc `@param` blocks directly in front of a function become its (guarded) contract.
    pub fn init_jsdoc(&mut self, src: &str, off: &dyn Fn(u32) -> usize) {
        if !self.cfg.jsdoc {
            return;
        }
        let blocks = crate::jsdoc::blocks(src);
        if blocks.is_empty() {
            return;
        }
        for fid in 1..self.st.funcs.len() {
            let f = &self.st.funcs[fid];
            if f.iife || f.bind.is_some() || self.dts_fns.contains_key(&fid) {
                continue;
            }
            let lo = off(f.lo);
            // the nearest block that ends before the function
            let Some(b) = blocks.iter().rev().find(|b| b.end <= lo) else {
                continue;
            };
            if !crate::jsdoc::gap_ok(&src[b.end..lo]) {
                continue;
            }
            let params: Vec<crate::dts::DT> = f
                .params
                .iter()
                .map(|p| {
                    p.as_ref()
                        .and_then(|n| b.params.iter().find(|(m, _)| m == n))
                        .map_or(crate::dts::DT::Any, |(_, t)| t.clone())
                })
                .collect();
            if params.iter().all(|p| *p == crate::dts::DT::Any) {
                continue;
            }
            self.dts_fns.insert(
                fid,
                vec![crate::dts::Sig {
                    owner: None,
                    params,
                    ret: crate::dts::DT::Any,
                }],
            );
            self.jsdoc_fns.insert(fid);
        }
    }

    pub fn reset_counts(&mut self) {
        for f in &mut self.st.funcs {
            f.exprs = 0;
            f.any_exprs = 0;
        }
        self.any_hist.clear();
        self.cb_flowed.clear();
    }

    fn enter(&mut self, f: F<'_>) {
        let Some(&fid) = self.st.by_span.get(&f.span()) else {
            return;
        };
        self.cur.push(fid);
        let saved_narrow = std::mem::take(&mut self.narrow);
        for p in f.pats() {
            if let a::Pat::Assign(asg) = p {
                let t = self.ty(&asg.right);
                if let a::Pat::Ident(bi) = &*asg.left {
                    self.flow(&(fid, bi.id.sym.to_string()), &t);
                } else {
                    self.taint_as(&t, Reason::Destructure);
                }
                asg.left.visit_with(self);
            } else {
                p.visit_with(self);
            }
        }
        match f.body() {
            Body::Stmts(s) => {
                self.stmt_seq(s);
                if !ends_abruptly(s) {
                    self.add_ret(fid, &Ty::undef());
                }
            }
            Body::Expr(e) => {
                let t = self.ty(e);
                self.add_ret(fid, &t);
            }
            Body::None => {}
        }
        self.narrow = saved_narrow;
        self.cur.pop();
    }

    /// Statement list with guard-clause narrowing (`if (!ok) return;` narrows the rest).
    fn stmt_seq(&mut self, s: &[a::Stmt]) {
        let mark = self.narrow.len();
        for (i, st) in s.iter().enumerate() {
            st.visit_with(self);
            if let a::Stmt::If(ifs) = st {
                let truth = if ifs.alt.is_none() && exits(&ifs.cons) {
                    Some(false)
                } else if ifs.alt.as_ref().is_some_and(|alt| exits(alt)) && !exits(&ifs.cons) {
                    Some(true)
                } else {
                    None
                };
                if let Some(truth) = truth {
                    let rest = &s[i + 1..];
                    let mut cs = Vec::new();
                    self.conds(&ifs.test, truth, &mut cs);
                    self.push_narrow(cs, &|n| assigns_in_stmts(rest, n));
                }
            }
        }
        self.narrow.truncate(mark);
    }

    fn add_ret(&mut self, fid: usize, t: &Ty) {
        let old = self.st.funcs[fid].ret.clone();
        let old_n = self.join(&old, &Ty::Bottom);
        let n = self.join(&old_n, t);
        if n != old_n {
            self.changed = true;
        }
        self.st.funcs[fid].ret = n;
    }
}

impl Visit for An {
    fn visit_function(&mut self, n: &a::Function) {
        self.enter(F::Func(n));
    }
    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        self.enter(F::Arrow(n));
    }
    fn visit_constructor(&mut self, n: &a::Constructor) {
        self.enter(F::Ctor(n));
    }
    fn visit_expr(&mut self, e: &a::Expr) {
        self.ty(e);
    }
    fn visit_stmts(&mut self, n: &[a::Stmt]) {
        self.stmt_seq(n);
    }
    fn visit_if_stmt(&mut self, n: &a::IfStmt) {
        self.ty(&n.test);
        let (mut ct, mut cf) = (Vec::new(), Vec::new());
        self.conds(&n.test, true, &mut ct);
        self.conds(&n.test, false, &mut cf);
        self.with_narrow(ct, &|nm| assigns_in_stmt(&n.cons, nm), |s| {
            n.cons.visit_with(s)
        });
        if let Some(alt) = &n.alt {
            self.with_narrow(cf, &|nm| assigns_in_stmt(alt, nm), |s| alt.visit_with(s));
        }
    }
    fn visit_while_stmt(&mut self, n: &a::WhileStmt) {
        self.ty(&n.test);
        let mut ct = Vec::new();
        self.conds(&n.test, true, &mut ct);
        self.with_narrow(
            ct,
            &|nm| assigns_in_stmt(&n.body, nm) || assigns_in_expr(&n.test, nm),
            |s| n.body.visit_with(s),
        );
    }
    fn visit_throw_stmt(&mut self, n: &a::ThrowStmt) {
        // `throw new TypeError(msg)`: a fresh built-in error handed straight to the unwinder; it is
        // never a value of the program, only its arguments are
        if let a::Expr::New(ne) = strip(&n.arg) {
            if let a::Expr::Ident(i) = strip(&ne.callee) {
                let fid = *self.cur.last().unwrap();
                if self.st.resolve(fid, &i.sym).is_none()
                    && matches!(
                        &*i.sym,
                        "Error"
                            | "TypeError"
                            | "RangeError"
                            | "SyntaxError"
                            | "ReferenceError"
                            | "EvalError"
                            | "URIError"
                    )
                {
                    for x in ne.args.iter().flatten() {
                        let t = self.ty(&x.expr);
                        self.taint_as(&t, Reason::New);
                    }
                    return;
                }
            }
        }
        let t = self.ty(&n.arg);
        self.taint_as(&t, Reason::Catch);
    }
    fn visit_class_prop(&mut self, n: &a::ClassProp) {
        if let Some(v) = &n.value {
            let t = self.ty(v);
            self.taint_as(&t, Reason::Class);
        }
    }
    fn visit_private_prop(&mut self, n: &a::PrivateProp) {
        if let Some(v) = &n.value {
            let t = self.ty(v);
            self.taint_as(&t, Reason::Class);
        }
    }
    fn visit_return_stmt(&mut self, n: &a::ReturnStmt) {
        let t = match &n.arg {
            Some(e) => self.ty(e),
            None => Ty::undef(),
        };
        let fid = *self.cur.last().unwrap();
        self.add_ret(fid, &t);
    }
    fn visit_var_decl(&mut self, n: &a::VarDecl) {
        let saved = self.decl_const;
        self.decl_const = n.kind == a::VarDeclKind::Const;
        n.visit_children_with(self);
        self.decl_const = saved;
    }
    fn visit_var_declarator(&mut self, n: &a::VarDeclarator) {
        match (&n.name, &n.init) {
            (a::Pat::Ident(bi), init) => {
                let name = bi.id.sym.to_string();
                match init {
                    Some(e) => {
                        let t = self.ty(e);
                        self.flow_name(&name, &t);
                    }
                    None if !self.cfg.optimistic_uninit && !self.is_definite(&name) => {
                        self.flow_name(&name, &Ty::undef())
                    }
                    None => {}
                }
            }
            (p, Some(e)) if destructure_simple(p) => {
                // `const { a, b: c = 1 } = obj` / `const [x, y] = arr`: plain reads of known shapes
                let t = self.ty(e);
                let arr_only = matches!(&t, Ty::Set(s) if !s.is_empty() && s.iter().all(|x| matches!(x, Atom::Arr(_))));
                match p {
                    a::Pat::Object(o) => {
                        for pr in &o.props {
                            let (key, name, dflt) = match pr {
                                a::ObjectPatProp::Assign(ap) => (
                                    ap.key.sym.to_string(),
                                    ap.key.sym.to_string(),
                                    ap.value.as_deref(),
                                ),
                                a::ObjectPatProp::KeyValue(kv) => {
                                    let key = match &kv.key {
                                        a::PropName::Ident(i) => i.sym.to_string(),
                                        a::PropName::Str(s) => {
                                            s.value.as_str().unwrap_or("\u{fffd}").to_string()
                                        }
                                        _ => unreachable!(),
                                    };
                                    let (nm, d) = match &*kv.value {
                                        a::Pat::Ident(b) => (b.id.sym.to_string(), None),
                                        a::Pat::Assign(ap) => match &*ap.left {
                                            a::Pat::Ident(b) => {
                                                (b.id.sym.to_string(), Some(&*ap.right))
                                            }
                                            _ => unreachable!(),
                                        },
                                        _ => unreachable!(),
                                    };
                                    (key, nm, d)
                                }
                                _ => unreachable!(),
                            };
                            let r = self.member_read(&t, &Key::Name(key));
                            let r = self.with_default(r, dflt);
                            self.flow_name(&name, &r);
                        }
                    }
                    a::Pat::Array(ar) if arr_only => {
                        for el in ar.elems.iter().flatten() {
                            let (name, d) = match el {
                                a::Pat::Ident(b) => (b.id.sym.to_string(), None),
                                a::Pat::Assign(ap) => match &*ap.left {
                                    a::Pat::Ident(b) => (b.id.sym.to_string(), Some(&*ap.right)),
                                    _ => unreachable!(),
                                },
                                _ => unreachable!(),
                            };
                            let r = self.member_read(&t, &Key::Num);
                            let r = self.with_default(r, d);
                            self.flow_name(&name, &r);
                        }
                    }
                    _ => {
                        self.taint_as(&t, Reason::Destructure);
                        self.flow_any_pat(p, Reason::Destructure);
                    }
                }
            }
            (p, init) => {
                if let Some(e) = init {
                    let t = self.ty(e);
                    self.taint_as(&t, Reason::Destructure);
                }
                self.flow_any_pat(p, Reason::Destructure);
                p.visit_with(self);
            }
        }
    }
    fn visit_catch_clause(&mut self, n: &a::CatchClause) {
        if let Some(p) = &n.param {
            self.flow_any_pat(p, Reason::Catch);
        }
        n.body.visit_with(self);
    }
    fn visit_for_in_stmt(&mut self, n: &a::ForInStmt) {
        self.ty(&n.right);
        self.head(&n.left, Ty::str());
        n.body.visit_with(self);
    }
    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        let r = self.ty(&n.right);
        let elem = match &r {
            Ty::Set(s) if s.len() == 1 => match s[0] {
                Atom::Arr(site) => self.arr_elem(site),
                Atom::Str | Atom::SLit(_) => Ty::str(),
                _ => Ty::Any(Reason::Iterate),
            },
            Ty::Bottom => Ty::Bottom,
            _ => {
                self.taint_as(&r, Reason::Iterate);
                Ty::Any(Reason::Iterate)
            }
        };
        self.head(&n.left, elem);
        n.body.visit_with(self);
    }
}

impl An {
    /// `r` with `undefined` replaced by the type of the default (`const { a = 5 } = o`).
    fn with_default(&mut self, r: Ty, dflt: Option<&a::Expr>) -> Ty {
        let Some(d) = dflt else { return r };
        let dt = self.ty(d);
        let kept = match r {
            Ty::Set(s) => {
                let v: Vec<Atom> = s.into_iter().filter(|x| *x != Atom::Undef).collect();
                if v.is_empty() {
                    Ty::Bottom
                } else {
                    Ty::Set(v)
                }
            }
            o => o,
        };
        self.join(&kept, &dt)
    }

    fn head(&mut self, h: &a::ForHead, t: Ty) {
        let mut ids: Vec<String> = Vec::new();
        match h {
            a::ForHead::VarDecl(v) => {
                for d in &v.decls {
                    let mut x = Vec::new();
                    pat_idents(&d.name, &mut x);
                    ids.extend(x.into_iter().map(String::from));
                }
            }
            a::ForHead::Pat(p) => {
                if let a::Pat::Expr(e) = &**p {
                    return self.assign_to_expr(e, &t);
                }
                let mut x = Vec::new();
                pat_idents(p, &mut x);
                ids.extend(x.into_iter().map(String::from));
            }
            _ => {}
        }
        for i in ids {
            self.flow_name(&i, &t);
        }
    }
}

/// A destructuring pattern of plain identifiers (with defaults): `{ a, b: c = 1 }`, `[x, y = 2]`.
fn destructure_simple(p: &a::Pat) -> bool {
    fn leaf(p: &a::Pat) -> bool {
        match p {
            a::Pat::Ident(_) => true,
            a::Pat::Assign(ap) => matches!(&*ap.left, a::Pat::Ident(_)),
            _ => false,
        }
    }
    match p {
        a::Pat::Object(o) => o.props.iter().all(|pr| match pr {
            a::ObjectPatProp::Assign(_) => true,
            a::ObjectPatProp::KeyValue(kv) => {
                matches!(&kv.key, a::PropName::Ident(_) | a::PropName::Str(_)) && leaf(&kv.value)
            }
            _ => false,
        }),
        a::Pat::Array(ar) => ar.elems.iter().flatten().all(leaf),
        _ => false,
    }
}
