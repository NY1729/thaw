//! Resolver and escape analysis (TJ143 plan 1.3). `analyze` is a pure function of the function's AST:
//! it never mutates the `FuncDef` and never touches a `Scope`. It classifies every name the function
//! declares (Slot or Scoped), lists its free names, collects its labels and purity, and records the
//! blockers. The verdict is the highest-priority blocker (see `FallbackReason::priority`).

use std::collections::BTreeSet;

use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;
use thaw_parser::common::Spanned;

use super::FallbackReason;
use crate::{Body, FuncDef};

/// What kind of declaration a name is. Only the lexical kinds take part in the tier A check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclKind {
    Param,
    Var,
    FnDecl,
    BlockFn,
    Let,
    Const,
    Class,
    Catch,
    SelfName,
}

impl DeclKind {
    fn lexical(self) -> bool {
        matches!(self, DeclKind::Let | DeclKind::Const | DeclKind::Class)
    }
}

fn lex_kind(k: a::VarDeclKind) -> DeclKind {
    if matches!(k, a::VarDeclKind::Const) {
        DeclKind::Const
    } else {
        DeclKind::Let
    }
}

/// Where a declared name lives at run time (plan 1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameClass {
    /// Not referenced from any nested function: a frame slot.
    Slot,
    /// Referenced from a nested function: resolved by name in the capture scope.
    Scoped,
}

impl NameClass {
    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NameClass::Slot => "slot",
            NameClass::Scoped => "scoped",
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // read by the tests and by C2+ only
pub(crate) struct NamePlan {
    pub(crate) name: String,
    pub(crate) kind: DeclKind,
    pub(crate) class: NameClass,
    /// Frame slot index (Slot names only), in declaration order.
    pub(crate) slot: Option<u32>,
    /// Block level of the declaration: 0 function level, 1 body top, 2 any nested block.
    pub(crate) level: u8,
}

/// The analysis result for one function. `verdict()` is `Ok` only when the function could be compiled.
#[derive(Debug)]
#[allow(dead_code)] // some fields are read by the tests and by C2+ only
pub(crate) struct FnPlan {
    /// Every declared name, function-level names first (parameters first), then block-level names.
    pub(crate) names: Vec<NamePlan>,
    /// Names referenced here but declared nowhere in the function (sorted, unique).
    pub(crate) free: Vec<String>,
    pub(crate) nslots: u32,
    /// Label table: label id `i + 1` is `labels[i]` (id 0 is the unlabeled break/continue).
    pub(crate) labels: Vec<String>,
    /// No calls, `new`, closures, object literals, free-variable writes, or property names other than
    /// `length` (the check-mode kernels of plan 4.1).
    pub(crate) pure_kernel: bool,
    /// Every blocker found, as `FallbackReason::bit` bits. `reason` is the first one, by priority.
    pub(crate) blockers: u64,
    /// Inner functions need a per-call `Scope`: a captured name, strict mode, or a `this` seen by an arrow.
    /// Nothing reads it yet (no behaviour change in this step).
    pub(crate) needs_scope: bool,
    reason: Option<FallbackReason>,
}

impl FnPlan {
    pub(crate) fn verdict(&self) -> Result<(), FallbackReason> {
        match self.reason {
            None => Ok(()),
            Some(r) => Err(r),
        }
    }

    /// Every blocker of the function, in `FallbackReason::ALL` order.
    #[cfg(test)]
    pub(crate) fn all_reasons(&self) -> Vec<FallbackReason> {
        FallbackReason::ALL
            .iter()
            .copied()
            .filter(|r| self.blockers & r.bit() != 0)
            .collect()
    }
}

/// One declaration record. Block-level records exist only for the blocks that declare them.
struct Decl {
    name: String,
    /// The scope frame that declares it (0 function level, 1 body top, deeper for blocks).
    frame: usize,
    kind: DeclKind,
    /// Byte position where a lexical declaration takes effect (the end of its declarator, or the start
    /// of the loop body for `for-in`/`for-of` heads). Zero for every other kind.
    decl_end: u32,
    /// Declared directly in a `switch` case body.
    in_switch: bool,
}

/// Walk state. Declarations and reads are recorded only at function depth zero; nested functions only
/// contribute to the capture set and to the blockers.
struct Walk {
    reason: Option<FallbackReason>,
    /// Every blocker seen (`block` and the tier A check), as `FallbackReason::bit` bits.
    blockers: u64,
    decls: Vec<Decl>,
    /// Scope stack of declaration indices. `frames[0]` is the function-level frame.
    frames: Vec<Vec<usize>>,
    /// Nested functions and classes entered so far.
    fn_depth: u32,
    /// Nested non-arrow functions entered so far (`arguments` and `new.target` belong to them).
    nonarrow_depth: u32,
    captured: BTreeSet<String>,
    free: BTreeSet<String>,
    /// Nested functions and arrows entered directly in this function (not inside another nested one).
    inner_fns: u32,
    /// Set by `visit_fn_expr` for the function expression `visit_function` is about to enter.
    ok_fn: bool,
    /// References to lexical declarations: (declaration index, byte position).
    reads: Vec<(usize, u32)>,
    labels: Vec<String>,
    impure: bool,
    self_name: Option<String>,
}

impl Walk {
    fn new() -> Walk {
        Walk {
            reason: None,
            blockers: 0,
            decls: Vec::new(),
            frames: vec![Vec::new()],
            fn_depth: 0,
            nonarrow_depth: 0,
            captured: BTreeSet::new(),
            free: BTreeSet::new(),
            inner_fns: 0,
            ok_fn: false,
            reads: Vec::new(),
            labels: Vec::new(),
            impure: false,
            self_name: None,
        }
    }

    /// Keeps the verdict at the highest priority; the first of equal priority wins.
    fn block(&mut self, r: FallbackReason) {
        self.blockers |= r.bit();
        if self.reason.is_none_or(|cur| r.priority() > cur.priority()) {
            self.reason = Some(r);
        }
    }

    fn declare(
        &mut self,
        frame: usize,
        name: &str,
        kind: DeclKind,
        decl_end: u32,
        in_switch: bool,
    ) {
        if self.fn_depth > 0 {
            return;
        }
        // a body-level `let`/`const`/`class` of the self name replaces the self binding in the interpreter's
        // frame, so it stays on the AST (a nested block shadow is an ordinary slot)
        if frame == 1 && kind.lexical() && self.self_name.as_deref() == Some(name) {
            self.block(FallbackReason::SelfBinding);
        }
        let idx = self.decls.len();
        self.decls.push(Decl {
            name: name.to_string(),
            frame,
            kind,
            decl_end,
            in_switch,
        });
        self.frames[frame].push(idx);
    }

    /// The self name of a named function, unless a parameter or function declaration already holds the name.
    fn declare_self(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            if !self.frames[0].iter().any(|&i| self.decls[i].name == name) {
                self.declare_fn_level(name, DeclKind::SelfName);
                self.self_name = Some(name.to_string());
            }
        }
    }

    /// A function-level name; a name declared earlier (parameter, `var`, function) keeps its record.
    fn declare_fn_level(&mut self, name: &str, kind: DeclKind) {
        if self.frames[0].iter().any(|&i| self.decls[i].name == name) {
            return;
        }
        self.declare(0, name, kind, 0, false);
    }

    /// A name bound inside a nested function (for the capture set).
    fn bound(&mut self, name: &str) {
        if self.fn_depth > 0 {
            self.captured.insert(name.to_string());
        }
    }

    fn resolve(&self, name: &str) -> Option<usize> {
        self.frames
            .iter()
            .rev()
            .flat_map(|f| f.iter().rev())
            .copied()
            .find(|&i| self.decls[i].name == name)
    }

    /// Declares the lexical names of one statement list in the innermost frame (which the caller has
    /// just pushed). `fn_top`: the list is a function body, whose function declarations are function-level.
    fn prepopulate(&mut self, stmts: &[a::Stmt], fn_top: bool, in_switch: bool) {
        let top = self.frames.len() - 1;
        let mut lexical = false;
        for s in stmts {
            match s {
                a::Stmt::Decl(a::Decl::Var(v)) if !matches!(v.kind, a::VarDeclKind::Var) => {
                    let kind = lex_kind(v.kind);
                    for d in &v.decls {
                        let mut names = Vec::new();
                        pat_names(&d.name, &mut names);
                        for n in names {
                            self.declare(top, &n, kind, d.span.hi.0, in_switch);
                        }
                    }
                    lexical = true;
                }
                a::Stmt::Decl(a::Decl::Class(c)) => {
                    let end = c.class.span.hi.0;
                    self.declare(top, &c.ident.sym, DeclKind::Class, end, in_switch);
                    lexical = true;
                }
                a::Stmt::Decl(a::Decl::Fn(f)) if !fn_top => {
                    self.declare(top, &f.ident.sym, DeclKind::BlockFn, 0, in_switch);
                    lexical = true;
                }
                _ => {}
            }
        }
        if in_switch && lexical {
            self.block(FallbackReason::SwitchLexical);
        }
    }

    /// A function body (or arrow block body): its own frame, then the statements. Top-level function
    /// declarations are plain function-level bindings, not Annex B block functions.
    fn body_stmts(&mut self, stmts: &[a::Stmt]) {
        self.frames.push(Vec::new());
        self.prepopulate(stmts, true, false);
        for s in stmts {
            match s {
                a::Stmt::Decl(a::Decl::Fn(f)) => {
                    self.bound(&f.ident.sym);
                    f.function.visit_with(self);
                }
                _ => s.visit_with(self),
            }
        }
        self.frames.pop();
    }

    /// Parameter, catch and declaration patterns: the bound names are not references, but default
    /// values and computed keys are.
    fn pat_refs(&mut self, p: &a::Pat) {
        match p {
            a::Pat::Ident(b) => self.bound(&b.id.sym),
            a::Pat::Array(ap) => {
                for e in ap.elems.iter().flatten() {
                    self.pat_refs(e);
                }
            }
            a::Pat::Object(op) => {
                for prop in &op.props {
                    match prop {
                        a::ObjectPatProp::KeyValue(kv) => {
                            kv.key.visit_with(self);
                            self.pat_refs(&kv.value);
                        }
                        a::ObjectPatProp::Assign(ap) => {
                            self.bound(&ap.key.id.sym);
                            if let Some(v) = &ap.value {
                                v.visit_with(self);
                            }
                        }
                        a::ObjectPatProp::Rest(r) => self.pat_refs(&r.arg),
                    }
                }
            }
            a::Pat::Assign(ap) => {
                ap.right.visit_with(self);
                self.pat_refs(&ap.left);
            }
            a::Pat::Rest(r) => self.pat_refs(&r.arg),
            a::Pat::Expr(e) => e.visit_with(self),
            a::Pat::Invalid(_) => self.block(FallbackReason::Unsupported),
        }
    }

    /// `for (let x of e) body` / `for (x in e) body`: the head scope covers the right-hand side, so a
    /// head binding is in effect only from the body on.
    fn for_loop(&mut self, left: &a::ForHead, right: &a::Expr, body: &a::Stmt) {
        self.frames.push(Vec::new());
        let top = self.frames.len() - 1;
        let body_lo = body.span().lo.0;
        if let a::ForHead::VarDecl(v) = left {
            if !matches!(v.kind, a::VarDeclKind::Var) {
                for d in &v.decls {
                    let mut names = Vec::new();
                    pat_names(&d.name, &mut names);
                    for n in names {
                        self.declare(top, &n, lex_kind(v.kind), body_lo, false);
                    }
                }
            }
        }
        right.visit_with(self);
        match left {
            a::ForHead::VarDecl(v) => {
                for d in &v.decls {
                    self.pat_refs(&d.name);
                }
            }
            a::ForHead::Pat(p) => p.visit_with(self),
            a::ForHead::UsingDecl(_) => self.block(FallbackReason::Unsupported),
        }
        body.visit_with(self);
        self.frames.pop();
    }

    fn is_resolved(&self, name: &str) -> bool {
        self.resolve(name).is_some()
    }
}

/// Bound names of a pattern, in source order.
fn pat_names(p: &a::Pat, out: &mut Vec<String>) {
    match p {
        a::Pat::Ident(b) => out.push(b.id.sym.to_string()),
        a::Pat::Array(ap) => {
            for e in ap.elems.iter().flatten() {
                pat_names(e, out);
            }
        }
        a::Pat::Object(op) => {
            for prop in &op.props {
                match prop {
                    a::ObjectPatProp::KeyValue(kv) => pat_names(&kv.value, out),
                    a::ObjectPatProp::Assign(ap) => out.push(ap.key.id.sym.to_string()),
                    a::ObjectPatProp::Rest(r) => pat_names(&r.arg, out),
                }
            }
        }
        a::Pat::Assign(ap) => pat_names(&ap.left, out),
        a::Pat::Rest(r) => pat_names(&r.arg, out),
        a::Pat::Expr(_) | a::Pat::Invalid(_) => {}
    }
}

fn is_proto_key(k: &a::PropName) -> bool {
    match k {
        a::PropName::Ident(i) => &*i.sym == "__proto__",
        a::PropName::Str(s) => s.value.as_str() == Some("__proto__"),
        _ => false,
    }
}

/// Every identifier that appears anywhere in a function's parameters or body (nested code included).
struct IdentNames(std::collections::BTreeSet<String>);

impl Visit for IdentNames {
    fn visit_ident(&mut self, n: &a::Ident) {
        self.0.insert(n.sym.to_string());
    }
}

/// `var` names and Annex B block function names of a function body (not crossing function boundaries).
struct VarNames(Vec<String>);

impl Visit for VarNames {
    fn visit_function(&mut self, _: &a::Function) {}
    fn visit_arrow_expr(&mut self, _: &a::ArrowExpr) {}
    fn visit_class(&mut self, _: &a::Class) {}
    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        self.0.push(n.ident.sym.to_string());
    }
    fn visit_var_decl(&mut self, n: &a::VarDecl) {
        if matches!(n.kind, a::VarDeclKind::Var) {
            for d in &n.decls {
                pat_names(&d.name, &mut self.0);
            }
        }
        n.visit_children_with(self);
    }
}

impl Visit for Walk {
    fn visit_ident(&mut self, n: &a::Ident) {
        let name = &*n.sym;
        if name == "arguments" && self.nonarrow_depth == 0 {
            self.block(FallbackReason::Arguments);
        }
        if self.fn_depth > 0 {
            if self.self_name.as_deref() == Some(name) {
                self.block(FallbackReason::SelfBinding);
            }
            self.captured.insert(name.to_string());
            return;
        }
        match self.resolve(name) {
            Some(i) => {
                let kind = self.decls[i].kind;
                if kind.lexical() {
                    self.reads.push((i, n.span.lo.0));
                }
            }
            None => {
                self.free.insert(name.to_string());
            }
        }
    }

    fn visit_var_declarator(&mut self, d: &a::VarDeclarator) {
        if let Some(init) = &d.init {
            init.visit_with(self);
        }
        if !matches!(d.name, a::Pat::Ident(_)) {
            self.block(FallbackReason::Destructuring);
        }
        self.pat_refs(&d.name);
    }

    fn visit_fn_decl(&mut self, n: &a::FnDecl) {
        // Only reached for declarations that are not direct children of a function body.
        self.bound(&n.ident.sym);
        self.block(FallbackReason::AnnexBBlockFunction);
        n.function.visit_with(self);
    }

    fn visit_fn_expr(&mut self, n: &a::FnExpr) {
        if let Some(id) = &n.ident {
            self.bound(&id.sym);
        }
        self.ok_fn = true;
        n.function.visit_with(self);
    }

    fn visit_function(&mut self, f: &a::Function) {
        if self.fn_depth == 0 {
            self.inner_fns += 1;
        }
        // only a plain function expression is accepted; declarations, methods, accessors (`ok_fn` unset),
        // async and generator functions stay refused
        if !std::mem::take(&mut self.ok_fn) || f.is_async || f.is_generator {
            self.block(FallbackReason::InnerFunction);
        }
        self.impure = true;
        self.fn_depth += 1;
        self.nonarrow_depth += 1;
        for p in &f.params {
            p.visit_with(self);
        }
        if let Some(b) = &f.body {
            self.body_stmts(&b.stmts);
        }
        self.fn_depth -= 1;
        self.nonarrow_depth -= 1;
    }

    fn visit_arrow_expr(&mut self, n: &a::ArrowExpr) {
        if self.fn_depth == 0 {
            self.inner_fns += 1;
        }
        if n.is_async || n.is_generator {
            self.block(FallbackReason::InnerFunction);
        }
        self.impure = true;
        self.fn_depth += 1;
        for p in &n.params {
            p.visit_with(self);
        }
        match &*n.body {
            a::ArrowFunctionBody::FunctionBody(b) => self.body_stmts(&b.stmts),
            a::ArrowFunctionBody::Expr(e) => e.visit_with(self),
        }
        self.fn_depth -= 1;
    }

    fn visit_class_decl(&mut self, n: &a::ClassDecl) {
        self.bound(&n.ident.sym);
        self.block(FallbackReason::ClassBody);
        self.impure = true;
        n.class.visit_with(self);
    }

    fn visit_class_expr(&mut self, n: &a::ClassExpr) {
        if let Some(id) = &n.ident {
            self.bound(&id.sym);
        }
        self.block(FallbackReason::ClassBody);
        self.impure = true;
        n.class.visit_with(self);
    }

    fn visit_class(&mut self, n: &a::Class) {
        // Members, `extends` and computed keys all count as nested code.
        self.fn_depth += 1;
        self.nonarrow_depth += 1;
        n.visit_children_with(self);
        self.fn_depth -= 1;
        self.nonarrow_depth -= 1;
    }

    fn visit_super(&mut self, _: &a::Super) {
        self.block(FallbackReason::Super);
    }

    fn visit_import(&mut self, _: &a::Import) {
        self.block(FallbackReason::DynamicImport);
    }

    fn visit_meta_prop_expr(&mut self, n: &a::MetaPropExpr) {
        match n.kind {
            a::MetaPropKind::NewTarget => {
                if self.nonarrow_depth == 0 {
                    self.block(FallbackReason::NewTarget);
                }
            }
            a::MetaPropKind::ImportMeta => self.block(FallbackReason::ImportMeta),
        }
    }

    fn visit_private_name(&mut self, _: &a::PrivateName) {
        self.block(FallbackReason::PrivateName);
    }

    fn visit_with_stmt(&mut self, n: &a::WithStmt) {
        self.block(FallbackReason::With);
        n.visit_children_with(self);
    }

    fn visit_yield_expr(&mut self, n: &a::YieldExpr) {
        self.block(FallbackReason::Generator);
        n.visit_children_with(self);
    }

    fn visit_await_expr(&mut self, n: &a::AwaitExpr) {
        self.block(FallbackReason::Async);
        n.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, n: &a::CallExpr) {
        if let a::Callee::Expr(e) = &n.callee {
            if let a::Expr::Ident(id) = &**e {
                match &*id.sym {
                    "eval" => self.block(FallbackReason::DirectEval),
                    "Function" => self.block(FallbackReason::FunctionCtor),
                    _ => {}
                }
            }
        }
        self.impure = true;
        n.visit_children_with(self);
    }

    fn visit_new_expr(&mut self, n: &a::NewExpr) {
        if let a::Expr::Ident(id) = &*n.callee {
            if &*id.sym == "Function" {
                self.block(FallbackReason::FunctionCtor);
            }
        }
        self.impure = true;
        n.visit_children_with(self);
    }

    fn visit_unary_expr(&mut self, n: &a::UnaryExpr) {
        if matches!(n.op, a::UnaryOp::Delete) {
            self.block(FallbackReason::Delete);
            self.impure = true;
        }
        n.visit_children_with(self);
    }

    fn visit_bin_expr(&mut self, n: &a::BinExpr) {
        // `in` (Proxy `has`) and `instanceof` (`Symbol.hasInstance`) can run user code
        if matches!(n.op, a::BinaryOp::In | a::BinaryOp::InstanceOf) {
            self.impure = true;
        }
        n.visit_children_with(self);
    }

    fn visit_assign_expr(&mut self, n: &a::AssignExpr) {
        match &n.left {
            a::AssignTarget::Pat(_) => self.block(FallbackReason::Destructuring),
            a::AssignTarget::Simple(a::SimpleAssignTarget::Ident(b))
                if self.fn_depth == 0 && !self.is_resolved(&b.id.sym) =>
            {
                self.impure = true;
            }
            _ => {}
        }
        n.visit_children_with(self);
    }

    fn visit_update_expr(&mut self, n: &a::UpdateExpr) {
        if let a::Expr::Ident(b) = &*n.arg {
            if self.fn_depth == 0 && !self.is_resolved(&b.sym) {
                self.impure = true;
            }
        }
        n.visit_children_with(self);
    }

    fn visit_member_expr(&mut self, n: &a::MemberExpr) {
        if let a::MemberProp::Ident(i) = &n.prop {
            if &*i.sym != "length" {
                self.impure = true;
            }
        }
        n.visit_children_with(self);
    }

    fn visit_object_lit(&mut self, n: &a::ObjectLit) {
        self.impure = true;
        n.visit_children_with(self);
    }

    fn visit_prop(&mut self, p: &a::Prop) {
        match p {
            a::Prop::KeyValue(kv) => {
                if is_proto_key(&kv.key) {
                    self.block(FallbackReason::ProtoLiteralKey);
                }
            }
            a::Prop::Assign(_) => self.block(FallbackReason::Unsupported),
            a::Prop::Method(_) | a::Prop::Getter(_) | a::Prop::Setter(_) => {
                self.block(FallbackReason::ObjectMethod)
            }
            a::Prop::Shorthand(_) => {}
        }
        p.visit_children_with(self);
    }

    fn visit_spread_element(&mut self, n: &a::SpreadElement) {
        self.block(FallbackReason::Spread);
        n.visit_children_with(self);
    }

    fn visit_expr_or_spread(&mut self, n: &a::ExprOrSpread) {
        if n.spread.is_some() {
            self.block(FallbackReason::Spread);
        }
        n.visit_children_with(self);
    }

    fn visit_tagged_tpl(&mut self, n: &a::TaggedTpl) {
        self.block(FallbackReason::TaggedTemplate);
        n.visit_children_with(self);
    }

    fn visit_opt_chain_expr(&mut self, n: &a::OptChainExpr) {
        self.block(FallbackReason::OptionalChain);
        n.visit_children_with(self);
    }

    fn visit_regex(&mut self, _: &a::Regex) {
        // allocates a RegExp object per evaluation
        self.impure = true;
    }

    fn visit_labeled_stmt(&mut self, n: &a::LabeledStmt) {
        if matches!(&*n.body, a::Stmt::Decl(a::Decl::Fn(_))) {
            self.block(FallbackReason::LabeledFunction);
        } else {
            self.block(FallbackReason::Labels);
            let name = n.label.sym.to_string();
            if self.fn_depth == 0 && !self.labels.contains(&name) {
                self.labels.push(name);
            }
        }
        n.body.visit_with(self);
    }

    // Label names are not references.
    fn visit_break_stmt(&mut self, _: &a::BreakStmt) {}
    fn visit_continue_stmt(&mut self, _: &a::ContinueStmt) {}

    fn visit_switch_stmt(&mut self, n: &a::SwitchStmt) {
        // compiled since TJ156; `SwitchLexical` (in `prepopulate`) still refuses case-scoped declarations
        n.discriminant.visit_with(self);
        self.frames.push(Vec::new());
        for case in &n.cases {
            self.prepopulate(&case.cons, false, true);
        }
        for case in &n.cases {
            if let Some(t) = &case.test {
                t.visit_with(self);
            }
            for s in &case.cons {
                s.visit_with(self);
            }
        }
        self.frames.pop();
    }

    fn visit_throw_stmt(&mut self, n: &a::ThrowStmt) {
        // compiled since TJ156; the argument's own blockers and impurity (calls, `new`) still apply
        n.visit_children_with(self);
    }

    fn visit_try_stmt(&mut self, n: &a::TryStmt) {
        self.block(FallbackReason::Try);
        n.visit_children_with(self);
    }

    fn visit_catch_clause(&mut self, n: &a::CatchClause) {
        self.frames.push(Vec::new());
        let top = self.frames.len() - 1;
        if let Some(p) = &n.param {
            let mut names = Vec::new();
            pat_names(p, &mut names);
            for name in names {
                self.declare(top, &name, DeclKind::Catch, 0, false);
            }
            self.pat_refs(p);
        }
        n.body.visit_with(self);
        self.frames.pop();
    }

    fn visit_block_stmt(&mut self, n: &a::BlockStmt) {
        self.frames.push(Vec::new());
        self.prepopulate(&n.stmts, false, false);
        for s in &n.stmts {
            s.visit_with(self);
        }
        self.frames.pop();
    }

    fn visit_for_stmt(&mut self, n: &a::ForStmt) {
        self.frames.push(Vec::new());
        let top = self.frames.len() - 1;
        if let Some(a::VarDeclOrExpr::VarDecl(v)) = &n.init {
            if !matches!(v.kind, a::VarDeclKind::Var) {
                for d in &v.decls {
                    let mut names = Vec::new();
                    pat_names(&d.name, &mut names);
                    for name in names {
                        self.declare(top, &name, lex_kind(v.kind), d.span.hi.0, false);
                    }
                }
            }
        }
        n.visit_children_with(self);
        self.frames.pop();
    }

    fn visit_for_in_stmt(&mut self, n: &a::ForInStmt) {
        self.block(FallbackReason::ForIn);
        self.for_loop(&n.left, &n.right, &n.body);
    }

    fn visit_for_of_stmt(&mut self, n: &a::ForOfStmt) {
        self.block(if n.is_await {
            FallbackReason::ForAwait
        } else {
            FallbackReason::ForOf
        });
        self.for_loop(&n.left, &n.right, &n.body);
    }

    fn visit_using_decl(&mut self, _: &a::UsingDecl) {
        self.block(FallbackReason::Unsupported);
    }
}

/// Analyses one function. Never fails: a blocked function still gets its classification, and the
/// verdict says why it cannot be compiled.
pub(crate) fn analyze(def: &FuncDef) -> FnPlan {
    let mut w = Walk::new();
    if def.class.is_some() {
        w.block(FallbackReason::ClassConstructor);
    }
    if def.is_gen {
        w.block(FallbackReason::Generator);
    }
    if def.is_async {
        w.block(FallbackReason::Async);
    }
    for p in &def.params {
        let mut names = Vec::new();
        pat_names(p, &mut names);
        for name in names {
            w.declare_fn_level(&name, DeclKind::Param);
        }
        match p {
            a::Pat::Ident(_) => {}
            a::Pat::Assign(_) => w.block(FallbackReason::DefaultParam),
            a::Pat::Rest(_) => w.block(FallbackReason::RestParam),
            _ => w.block(FallbackReason::Destructuring),
        }
    }
    // The self name is a slot only when the body mentions it (plan 1.3). It is declared after the parameters
    // and function declarations (which win, as the interpreter's bindings do) and before the `var`s, so a
    // `var` of the same name reuses it: the interpreter's `declare_if_absent` keeps the callee.
    let mut idents = IdentNames(Default::default());
    def.params.visit_with(&mut idents);
    match &def.body {
        Body::Block(stmts) => stmts.visit_with(&mut idents),
        Body::Expr(e) => e.visit_with(&mut idents),
    }
    let self_name = def
        .name
        .as_deref()
        .filter(|n| def.bind_self && idents.0.contains(*n));
    if let Body::Block(stmts) = &def.body {
        let mut vars = VarNames(Vec::new());
        for s in stmts {
            match s {
                a::Stmt::Decl(a::Decl::Fn(f)) => w.declare_fn_level(&f.ident.sym, DeclKind::FnDecl),
                _ => s.visit_with(&mut vars),
            }
        }
        w.declare_self(self_name);
        for name in vars.0 {
            w.declare_fn_level(&name, DeclKind::Var);
        }
    } else {
        w.declare_self(self_name);
    }
    for p in &def.params {
        w.pat_refs(p);
    }
    match &def.body {
        Body::Block(stmts) => w.body_stmts(stmts),
        Body::Expr(e) => e.visit_with(&mut w),
    }

    let Walk {
        mut reason,
        mut blockers,
        decls,
        captured,
        free,
        reads,
        labels,
        impure,
        inner_fns,
        ..
    } = w;
    let class_of = |name: &str| {
        if captured.contains(name) {
            NameClass::Scoped
        } else {
            NameClass::Slot
        }
    };
    // Tier A (plan 2.1): a lexical Slot may only be read textually after its declarator, and never in
    // a switch case. Anything else would observe the missing initialisation.
    for &(i, pos) in &reads {
        let d = &decls[i];
        if d.kind.lexical()
            && class_of(&d.name) == NameClass::Slot
            && (d.in_switch || pos < d.decl_end)
        {
            blockers |= FallbackReason::ReadBeforeDecl.bit();
            if reason.is_none_or(|cur| FallbackReason::ReadBeforeDecl.priority() > cur.priority()) {
                reason = Some(FallbackReason::ReadBeforeDecl);
            }
        }
    }

    let mut nslots = 0u32;
    let names: Vec<NamePlan> = decls
        .iter()
        .map(|d| {
            let class = class_of(&d.name);
            let slot = (class == NameClass::Slot).then(|| {
                nslots += 1;
                nslots - 1
            });
            NamePlan {
                name: d.name.clone(),
                kind: d.kind,
                class,
                slot,
                level: d.frame.min(2) as u8,
            }
        })
        .collect();
    // A captured name declared in a nested block needs per-iteration or per-block scopes (not modelled yet).
    // It is a blocker bit only: it never becomes the verdict, so no function changes compiled/not-compiled.
    if names
        .iter()
        .any(|n| n.level >= 2 && n.class == NameClass::Scoped)
    {
        blockers |= FallbackReason::ScopedBlock.bit();
    }
    // A captured name is compiled only as a parameter or a `var` (function level). Any other captured name
    // (lexical, block-level, catch, self, class) keeps the function on the AST.
    if names
        .iter()
        .any(|n| n.class == NameClass::Scoped && !matches!(n.kind, DeclKind::Param | DeclKind::Var))
    {
        blockers |= FallbackReason::InnerFunction.bit();
        if reason.is_none_or(|cur| FallbackReason::InnerFunction.priority() > cur.priority()) {
            reason = Some(FallbackReason::InnerFunction);
        }
    }
    let needs_scope = inner_fns > 0
        && (names.iter().any(|n| n.class == NameClass::Scoped)
            || def.strict
            || (def.uses_this && !def.arrow));
    FnPlan {
        names,
        free: free.into_iter().collect(),
        nslots,
        labels,
        pure_kernel: !impure && !def.is_gen && !def.is_async,
        blockers,
        needs_scope,
        reason,
    }
}
