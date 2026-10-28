thread_local! {
    /// Whether the program deletes a `stack` property anywhere (`delete e.stack`). Only then does a
    /// typed Error's `.stack` read become `string | undefined`; every other program keeps `string`.
    static PROGRAM_DELETES_STACK: Cell<bool> = const { Cell::new(false) };
    /// Maps a decorated class's own identity marker field name (the
    /// `__thaw_class_identity_<chain>` field every native class carries
    /// as `fields[0]`, see `module/classes.rs`'s `resolve_layout`) to the
    /// name of that class's decorator "class token" global -- a real,
    /// live `JsValue` (`new Function()`, see `lower_class_decorator_
    /// tokens` in `module/globals.rs`) created once per decorated class
    /// so a decorator's `target` argument (and, symmetrically, an
    /// instance of that class crossing a dynamic-call boundary, see
    /// `coerce_to_declared`'s `HirType::Json` branch) can share one real,
    /// stable identity -- the same thing real `object.constructor`-keyed
    /// metadata storage (class-validator, TypeORM, ...) needs to find its
    /// own registered metadata again at `validate()`/query time.
    ///
    /// Read from `coerce_to_declared` (`inference/coercions.rs`), which
    /// runs inside an arbitrary function's own `FnLowerer` -- reachable
    /// from anywhere in the program, not just the module-init lowering
    /// this gets populated from -- so a thread-local (matching thaw-
    /// bridge's `ALLOW_NATIVE_BYTES_TYPE` precedent) is far less invasive
    /// than threading a new parameter through every lowering entry
    /// point. Single-threaded, sequential-per-program compilation makes
    /// this safe; cleared at the top of `lower_module` (the one, non-
    /// recursive entry point for a whole compilation) so no stale entry
    /// from an earlier, unrelated compilation on the same thread can
    /// leak into this one.
    static DECORATOR_CLASS_TOKENS: RefCell<HashMap<String, Symbol>> =
        RefCell::new(HashMap::new());
    // Original private names map to physical slots; ordinary public fields
    // with similar spelling must retain their public property semantics.
    static PRIVATE_CLASS_SLOTS: RefCell<HashMap<Symbol, HashMap<Symbol, Symbol>>> =
        RefCell::new(HashMap::new());
    // Names of classes that carry decorators: their instances keep crossing
    // dynamic boundaries as snapshots with a `constructor` token.
    static DECORATED_CLASS_NAMES: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn class_has_decorators(class: &swc_ecma_ast::Class) -> bool {
    !class.decorators.is_empty()
        || class.body.iter().any(|member| match member {
            ClassMember::ClassProp(property) => !property.decorators.is_empty(),
            ClassMember::Method(method) => !method.function.decorators.is_empty(),
            _ => false,
        })
}

fn class_decorator_token_symbol(class_name: &str) -> Symbol {
    class_static_field_symbol(class_name, "__thaw_decorator_target__")
}

/// Class names that appear as a bare *value* reference somewhere in the
/// module (`Expr::Ident`), as opposed to only ever being constructed
/// (`new Class`) or used in a type position. Such a class is handed to real
/// JavaScript as a value -- `plainToInstance(User, ...)`, `@Type(() =>
/// Address)`, `value instanceof Address` -- and so needs the same stable,
/// live "class token" a *decorated* class already gets (see
/// `lower_class_decorator_tokens`). Without one, every crossing would
/// create a fresh native-callback wrapper with no shared identity, so
/// `instanceof`/metadata lookups against it could never match.
///
/// Also includes a class that is ever constructed (`new Class`), because
/// its instances can reach `Object.getPrototypeOf`, which resolves the
/// class's real prototype through the same token. Still excludes a
/// `class X extends Base` super reference (Thaw consumes the base's
/// *layout*, not its value identity).
fn class_names_referenced_as_values(
    module: &swc_ecma_ast::Module,
    classes: &[&ClassDecl],
) -> HashSet<Symbol> {
    use swc_ecma_visit::{Visit, VisitWith};

    struct Collector<'a> {
        classes: &'a HashSet<Symbol>,
        found: HashSet<Symbol>,
        /// Classes ever constructed (`new C`). Only merged into `found` when
        /// the module actually calls `Object.getPrototypeOf`/
        /// `Reflect.getPrototypeOf`, so a program that never asks for a
        /// prototype doesn't pay to materialize a class token (and pull in
        /// QuickJS) for every class it instantiates.
        constructed: HashSet<Symbol>,
        reads_prototype: bool,
    }
    impl Visit for Collector<'_> {
        fn visit_expr(&mut self, expr: &swc_ecma_ast::Expr) {
            if let swc_ecma_ast::Expr::Ident(ident) = expr {
                if self.classes.contains(ident.sym.as_ref()) {
                    self.found.insert(ident.sym.to_string());
                }
            }
            expr.visit_children_with(self);
        }

        fn visit_call_expr(&mut self, call: &swc_ecma_ast::CallExpr) {
            if let swc_ecma_ast::Callee::Expr(callee) = &call.callee {
                if let swc_ecma_ast::Expr::Member(member) = callee.as_ref() {
                    if let swc_ecma_ast::Expr::Ident(namespace) = member.obj.as_ref() {
                        let is_prototype_read = matches!(
                            namespace.sym.as_ref(),
                            "Object" | "Reflect"
                        ) && matches!(
                            &member.prop,
                            swc_ecma_ast::MemberProp::Ident(property)
                                if property.sym == *"getPrototypeOf"
                        );
                        if is_prototype_read {
                            self.reads_prototype = true;
                        }
                    }
                }
            }
            call.visit_children_with(self);
        }

        fn visit_new_expr(&mut self, expression: &swc_ecma_ast::NewExpr) {
            // A constructed class may need a stable class token: its
            // instances can be passed to `Object.getPrototypeOf` (and, in
            // later prototype work, have their methods/prototype read),
            // which resolves through that token. Recorded separately and
            // merged only if the module reads a prototype (see
            // `reads_prototype`). Still visit the arguments, which may
            // reference a class as a value (`new Thing(User)`).
            if let swc_ecma_ast::Expr::Ident(ident) = expression.callee.as_ref() {
                if self.classes.contains(ident.sym.as_ref()) {
                    self.constructed.insert(ident.sym.to_string());
                }
            } else {
                expression.callee.visit_with(self);
            }
            if let Some(arguments) = &expression.args {
                for argument in arguments {
                    argument.visit_with(self);
                }
            }
        }

        fn visit_class(&mut self, class: &swc_ecma_ast::Class) {
            // `extends Base` consumes Base's layout, not its value identity.
            for member in &class.body {
                member.visit_with(self);
            }
        }
    }

    let names: HashSet<Symbol> = classes
        .iter()
        .map(|declaration| declaration.ident.sym.to_string())
        .collect();
    let mut collector = Collector {
        classes: &names,
        found: HashSet::new(),
        constructed: HashSet::new(),
        reads_prototype: false,
    };
    module.visit_with(&mut collector);
    if collector.reads_prototype {
        collector.found.extend(collector.constructed);
    }
    collector.found.extend(classes.iter()
        .filter(|declaration| class_has_decorators(&declaration.class))
        .map(|declaration| declaration.ident.sym.to_string()));
    loop {
        let mut added = false;
        for declaration in classes {
            if !collector.found.contains(declaration.ident.sym.as_ref()) { continue; }
            if let Some(Expr::Ident(base)) = declaration.class.super_class.as_deref() {
                if names.contains(base.sym.as_ref()) {
                    added |= collector.found.insert(base.sym.to_string());
                }
            }
        }
        if !added { break; }
    }
    collector.found
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    // The first attempt also keeps `undefined` through returns, initialisers, container
    // writes and object literals; if that analysis changed anything and the module no
    // longer lowers, the plain analysis decides.
    EXTENDED_STORAGE.with(|extended| extended.set(true));
    EXTENDED_STORAGE_USED.with(|used| used.set(false));
    let first = lower_module_once(module);
    EXTENDED_STORAGE.with(|extended| extended.set(false));
    if first.is_err() && EXTENDED_STORAGE_USED.with(Cell::get) {
        return lower_module_once(module);
    }
    first
}

fn lower_module_once(module: &Module) -> Result<HirProgram, String> {
    DECORATOR_CLASS_TOKENS.with(|tokens| tokens.borrow_mut().clear());
    DECORATED_CLASS_NAMES.with(|names| names.borrow_mut().clear());
    PRIVATE_CLASS_SLOTS.with(|slots| slots.borrow_mut().clear());
    PROGRAM_DELETES_STACK.with(|flag| flag.set(module_deletes_stack(module)));
    let normalized = normalize_module_for_lowering(module)?;
    let mut program = lower_normalized_module(&normalized)?;
    // `HirType::Bytes` is a lowering-time distinction only; nothing past
    // here knows it, so collapse it to its physical `Array(F64)`.
    erase_bytes(&mut program);
    Ok(program)
}

/// TypeScript rejects a default parameter that reads itself or a later parameter outside a
/// deferred closure (TS2373/TS2372); at run time that read is a TDZ `ReferenceError`, which a
/// compiled call cannot reproduce, so it is a compile error here too.
fn reject_forward_parameter_references(module: &Module) -> Result<(), String> {
    struct Refs<'a> {
        forbidden: &'a [String],
        hit: Option<String>,
    }
    impl Visit for Refs<'_> {
        fn visit_expr(&mut self, expression: &Expr) {
            match expression {
                Expr::Ident(name) if self.hit.is_none() && self.forbidden.iter().any(|f| f == name.sym.as_ref()) => {
                    self.hit = Some(name.sym.to_string());
                }
                // A nested function body runs later, after every parameter is bound.
                Expr::Fn(_) | Expr::Arrow(_) | Expr::Class(_) => {}
                _ => expression.visit_children_with(self),
            }
        }
    }
    fn check(patterns: &[&Pat]) -> Result<(), String> {
        let names: Vec<Option<String>> = patterns.iter().map(|pattern| match pattern {
            Pat::Assign(assign) => match assign.left.as_ref() {
                Pat::Ident(binding) => Some(binding.id.sym.to_string()),
                _ => None,
            },
            Pat::Ident(binding) => Some(binding.id.sym.to_string()),
            _ => None,
        }).collect();
        for (index, pattern) in patterns.iter().enumerate() {
            let Pat::Assign(assign) = pattern else { continue };
            let forbidden: Vec<String> = names[index..].iter().flatten().cloned().collect();
            let mut refs = Refs { forbidden: &forbidden, hit: None };
            assign.right.visit_with(&mut refs);
            if let Some(hit) = refs.hit {
                let owner = names[index].clone().unwrap_or_default();
                return Err(if hit == owner {
                    format!("Parameter `{owner}` cannot reference itself")
                } else {
                    format!("Parameter `{owner}` cannot reference identifier `{hit}` declared after it")
                });
            }
        }
        Ok(())
    }
    struct Walk { error: Option<String> }
    impl Visit for Walk {
        fn visit_function(&mut self, function: &swc_ecma_ast::Function) {
            if self.error.is_none() {
                let patterns: Vec<&Pat> = function.params.iter().map(|param| &param.pat).collect();
                self.error = check(&patterns).err();
            }
            function.visit_children_with(self);
        }
        fn visit_constructor(&mut self, constructor: &swc_ecma_ast::Constructor) {
            if self.error.is_none() {
                let patterns: Vec<Option<&Pat>> = constructor.params.iter().map(|param| match param {
                    swc_ecma_ast::ParamOrTsParamProp::Param(param) => Some(&param.pat),
                    _ => None,
                }).collect();
                if patterns.iter().all(Option::is_some) {
                    let patterns: Vec<&Pat> = patterns.into_iter().flatten().collect();
                    self.error = check(&patterns).err();
                }
            }
            constructor.visit_children_with(self);
        }
        fn visit_arrow_expr(&mut self, arrow: &swc_ecma_ast::ArrowExpr) {
            if self.error.is_none() {
                let patterns: Vec<&Pat> = arrow.params.iter().collect();
                self.error = check(&patterns).err();
            }
            arrow.visit_children_with(self);
        }
    }
    let mut walk = Walk { error: None };
    module.visit_with(&mut walk);
    walk.error.map_or(Ok(()), Err)
}

fn normalize_module_for_lowering(module: &Module) -> Result<Module, String> {
    reject_forward_parameter_references(module)?;
    let merged = normalize_interface_merges(module)?;
    let merged = flatten_namespaces(&merged);
    let merged = enums_as_values(&merged);
    let normalized = normalize_top_level_destructuring(&merged)?;
    let normalized = lift_local_classes(&normalized);
    let normalized = normalize_top_level_class_expressions(&normalized)?;
    let normalized = normalize_static_computed_class_members(&normalized);
    let normalized = normalize_private_class_members(&normalized);
    let normalized = normalize_constructor_functions(&normalized)?;
    let normalized = specialize_new_target_functions(&normalized);
    let normalized = specialize_new_target_constructors(&normalized);
    let normalized = annotate_untyped_default_parameters(&normalized);
    let normalized = normalize_default_parameter_closures(&normalized);
    let normalized = normalize_implicit_returns(&normalized);
    let normalized = infer_collection_type_args(&normalized);
    let normalized = normalize_any_async_call_aliases(&normalized);
    let normalized = normalize_uninitialized_globals(&normalized);
    specialize_named_promise_rejection_callbacks(&normalized)
}

const NEW_TARGET_CONSTRUCT_SUFFIX: &str = "__thaw_construct";

/// `new.target` in a top-level function declaration. Function bodies are native functions with
/// fixed parameters and no hidden construct flag, so such a function gets a second declaration
/// (`F__thaw_construct`) in which `new.target` is `F`; the original body sees `undefined`, and
/// every direct `new F(...)` is routed to the construct body. Functions that never mention
/// `new.target` are untouched. A construct reaching `F` from QuickJS still runs the call body.
fn specialize_new_target_functions(module: &Module) -> Module {
    use swc_ecma_ast::{Ident, MetaPropKind};
    struct Replace {
        with: Ident,
        hit: bool,
    }
    impl VisitMut for Replace {
        fn visit_mut_expr(&mut self, expression: &mut Expr) {
            if matches!(expression, Expr::MetaProp(meta) if meta.kind == MetaPropKind::NewTarget) {
                *expression = Expr::Ident(self.with.clone());
                self.hit = true;
            } else {
                expression.visit_mut_children_with(self);
            }
        }
        // A nested function or class has its own `new.target`; arrows inherit ours.
        fn visit_mut_function(&mut self, _: &mut swc_ecma_ast::Function) {}
        fn visit_mut_class(&mut self, _: &mut swc_ecma_ast::Class) {}
    }
    fn rewrite(declaration: &mut FnDecl, with: Ident) -> bool {
        let mut replace = Replace { with, hit: false };
        let function = &mut declaration.function;
        function.params.visit_mut_with(&mut replace);
        function.body.visit_mut_with(&mut replace);
        replace.hit
    }
    struct BindingCounts(HashMap<Symbol, usize>);
    impl Visit for BindingCounts {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            *self.0.entry(declaration.ident.sym.to_string()).or_default() += 1;
            declaration.function.visit_with(self);
        }
        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            *self.0.entry(binding.id.sym.to_string()).or_default() += 1;
        }
    }
    let mut counts = BindingCounts(HashMap::new());
    module.visit_with(&mut counts);
    let undefined = |span| Ident::new_no_ctxt("undefined".into(), span);
    let mut names = HashSet::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) = item else {
            continue;
        };
        let function = &declaration.function;
        let mut probe = declaration.clone();
        // ponytail: unshadowed top-level, non-async, non-generator declarations only.
        if function.body.is_some()
            && !function.is_async
            && !function.is_generator
            && counts.0.get(declaration.ident.sym.as_ref()) == Some(&1)
            && rewrite(&mut probe, undefined(declaration.ident.span))
        {
            names.insert(declaration.ident.sym.to_string());
        }
    }
    if names.is_empty() {
        return module.clone();
    }
    struct Route<'a>(&'a HashSet<Symbol>);
    impl VisitMut for Route<'_> {
        fn visit_mut_expr(&mut self, expression: &mut Expr) {
            expression.visit_mut_children_with(self);
            let Expr::New(construction) = expression else {
                return;
            };
            let Expr::Ident(callee) = construction.callee.as_ref() else {
                return;
            };
            if !self.0.contains(callee.sym.as_ref()) {
                return;
            }
            let mut callee = callee.clone();
            callee.sym = format!("{}{NEW_TARGET_CONSTRUCT_SUFFIX}", callee.sym).into();
            *expression = Expr::Call(CallExpr {
                span: construction.span,
                ctxt: construction.ctxt,
                callee: Callee::Expr(Box::new(Expr::Ident(callee))),
                args: construction.args.take().unwrap_or_default(),
                type_args: construction.type_args.take(),
            });
        }
    }
    let mut routed = module.clone();
    routed.visit_mut_with(&mut Route(&names));
    let mut body = Vec::with_capacity(routed.body.len() + names.len());
    for item in routed.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(mut declaration))) = item else {
            body.push(item);
            continue;
        };
        if !names.contains(declaration.ident.sym.as_ref()) {
            body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))));
            continue;
        }
        let function_value = declaration.ident.clone();
        let mut construct = declaration.clone();
        construct.ident.sym = format!("{}{NEW_TARGET_CONSTRUCT_SUFFIX}", function_value.sym).into();
        rewrite(&mut declaration, undefined(function_value.span));
        rewrite(&mut construct, function_value);
        body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))));
        body.push(ModuleItem::Stmt(Stmt::Decl(Decl::Fn(construct))));
    }
    Module { body, ..routed }
}

/// `new.target` inside a class constructor. The instance's nominal identity is registered before
/// any constructor code runs, so `new.target` is the most-derived class of `this`; with the whole
/// hierarchy known at compile time that is expressible with `instanceof` alone:
/// `new.target === C` -> `this instanceof C` and not an instance of any subclass of `C`;
/// `new.target.name` -> a conditional chain over `C` and its descendants (most derived first);
/// `new.target === undefined` -> `false`; and the `Object.setPrototypeOf(this, new.target.prototype)`
/// idiom -> nothing (the native layout already has the right identity). Other uses of
/// `new.target` (passing it on, `.prototype` elsewhere) keep the lowering error.
fn specialize_new_target_constructors(module: &Module) -> Module {
    use swc_ecma_ast::{
        BinExpr, Bool, CondExpr, EmptyStmt, Ident, MetaPropKind, Str, ThisExpr, UnaryExpr,
        UnaryOp,
    };
    fn class_of(item: &ModuleItem) -> Option<&ClassDecl> {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => Some(class),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(swc_ecma_ast::ExportDecl {
                decl: Decl::Class(class),
                ..
            })) => Some(class),
            _ => None,
        }
    }
    fn class_of_mut(item: &mut ModuleItem) -> Option<&mut ClassDecl> {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class))) => Some(class),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(swc_ecma_ast::ExportDecl {
                decl: Decl::Class(class),
                ..
            })) => Some(class),
            _ => None,
        }
    }
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    for item in &module.body {
        if let Some(class) = class_of(item) {
            let parent = match class.class.super_class.as_deref() {
                Some(Expr::Ident(parent)) => Some(parent.sym.to_string()),
                _ => None,
            };
            parents.insert(class.ident.sym.to_string(), parent);
        }
    }
    let depth = |name: &str| {
        let mut depth = 0;
        let mut at = parents.get(name).cloned().flatten();
        while let Some(parent) = at {
            depth += 1;
            at = parents.get(&parent).cloned().flatten();
            if depth > parents.len() {
                break;
            }
        }
        depth
    };
    let descends_from = |name: &str, ancestor: &str| {
        let mut at = parents.get(name).cloned().flatten();
        let mut steps = 0;
        while let Some(parent) = at {
            if parent == ancestor {
                return true;
            }
            at = parents.get(&parent).cloned().flatten();
            steps += 1;
            if steps > parents.len() {
                break;
            }
        }
        false
    };
    struct Rewrite<'a> {
        owner: String,
        parents: &'a HashMap<String, Option<String>>,
        descendants: Vec<String>,
        direct: &'a dyn Fn(&str) -> Vec<String>,
    }
    fn ident(name: &str, span: swc_common::Span) -> Box<Expr> {
        Box::new(Expr::Ident(Ident::new_no_ctxt(name.into(), span)))
    }
    impl Rewrite<'_> {
        fn instance_of(&self, name: &str, span: swc_common::Span) -> Box<Expr> {
            Box::new(Expr::Bin(BinExpr {
                span,
                op: BinaryOp::InstanceOf,
                left: Box::new(Expr::This(ThisExpr { span })),
                right: ident(name, span),
            }))
        }
        fn exact(&self, name: &str, span: swc_common::Span) -> Box<Expr> {
            let mut test = self.instance_of(name, span);
            for sub in (self.direct)(name) {
                test = Box::new(Expr::Bin(BinExpr {
                    span,
                    op: BinaryOp::LogicalAnd,
                    left: test,
                    right: Box::new(Expr::Unary(UnaryExpr {
                        span,
                        op: UnaryOp::Bang,
                        arg: self.instance_of(&sub, span),
                    })),
                }));
            }
            test
        }
        fn head_name(&self, span: swc_common::Span) -> Box<Expr> {
            let text = |name: &str| {
                Box::new(Expr::Lit(Lit::Str(Str {
                    span,
                    value: display_class_name(name).into(),
                    raw: None,
                })))
            };
            let mut chain = text(&self.owner);
            for name in self.descendants.iter().rev() {
                chain = Box::new(Expr::Cond(CondExpr {
                    span,
                    test: self.instance_of(name, span),
                    cons: text(name),
                    alt: chain,
                }));
            }
            // Parenthesize so `new.target.name + "x"` keeps its grouping.
            Box::new(Expr::Paren(swc_ecma_ast::ParenExpr { span, expr: chain }))
        }
        fn is_nt(e: &Expr) -> bool {
            matches!(e, Expr::MetaProp(m) if m.kind == MetaPropKind::NewTarget)
        }
        fn known(&self, e: &Expr) -> Option<String> {
            match e {
                Expr::Ident(i) if self.parents.contains_key(i.sym.as_ref()) => {
                    Some(i.sym.to_string())
                }
                _ => None,
            }
        }
        fn is_undefined(e: &Expr) -> bool {
            matches!(e, Expr::Ident(i) if i.sym.as_ref() == "undefined")
        }
    }
    impl VisitMut for Rewrite<'_> {
        fn visit_mut_stmt(&mut self, statement: &mut Stmt) {
            if let Stmt::Expr(expression) = statement {
                if let Expr::Call(call) = expression.expr.as_ref() {
                    let is_set = matches!(&call.callee, Callee::Expr(c) if matches!(c.as_ref(),
                        Expr::Member(m) if matches!(m.obj.as_ref(), Expr::Ident(o) if o.sym.as_ref() == "Object")
                            && matches!(&m.prop, MemberProp::Ident(p) if p.sym.as_ref() == "setPrototypeOf")));
                    if let (true, [target, prototype]) = (is_set, call.args.as_slice()) {
                        let prototype_of = match prototype.expr.as_ref() {
                            Expr::Member(m)
                                if prototype.spread.is_none()
                                    && matches!(&m.prop, MemberProp::Ident(p) if p.sym.as_ref() == "prototype") =>
                            {
                                Some(m.obj.as_ref())
                            }
                            _ => None,
                        };
                        if matches!(target.expr.as_ref(), Expr::This(_))
                            && prototype_of
                                .is_some_and(|o| Self::is_nt(o) || self.known(o).as_deref() == Some(&self.owner))
                        {
                            *statement = Stmt::Empty(EmptyStmt { span: expression.span });
                            return;
                        }
                    }
                }
            }
            statement.visit_mut_children_with(self);
        }
        fn visit_mut_expr(&mut self, expression: &mut Expr) {
            expression.visit_mut_children_with(self);
            match expression {
                Expr::Member(m)
                    if Self::is_nt(&m.obj)
                        && matches!(&m.prop, MemberProp::Ident(p) if p.sym.as_ref() == "name") =>
                {
                    *expression = *self.head_name(m.span);
                }
                Expr::Bin(b)
                    if matches!(
                        b.op,
                        BinaryOp::EqEq | BinaryOp::EqEqEq | BinaryOp::NotEq | BinaryOp::NotEqEq
                    ) =>
                {
                    let negate = matches!(b.op, BinaryOp::NotEq | BinaryOp::NotEqEq);
                    let other = if Self::is_nt(&b.left) {
                        &b.right
                    } else if Self::is_nt(&b.right) {
                        &b.left
                    } else {
                        return;
                    };
                    let span = b.span;
                    let test = if Self::is_undefined(other) {
                        Box::new(Expr::Lit(Lit::Bool(Bool { span, value: false })))
                    } else if let Some(name) = self.known(other) {
                        self.exact(&name, span)
                    } else {
                        return;
                    };
                    *expression = if negate {
                        Expr::Unary(UnaryExpr { span, op: UnaryOp::Bang, arg: Box::new(Expr::Paren(swc_ecma_ast::ParenExpr { span, expr: test })) })
                    } else {
                        Expr::Paren(swc_ecma_ast::ParenExpr { span, expr: test })
                    };
                }
                _ => {}
            }
        }
        // A nested function or class has its own `new.target`; arrows inherit ours.
        fn visit_mut_function(&mut self, _: &mut swc_ecma_ast::Function) {}
        fn visit_mut_class(&mut self, _: &mut swc_ecma_ast::Class) {}
    }
    let direct = |name: &str| -> Vec<String> {
        let mut subs: Vec<String> = parents
            .iter()
            .filter(|(_, parent)| parent.as_deref() == Some(name))
            .map(|(child, _)| child.clone())
            .collect();
        subs.sort();
        subs
    };
    let mut module = module.clone();
    for item in &mut module.body {
        let Some(class) = class_of_mut(item) else {
            continue;
        };
        let owner = class.ident.sym.to_string();
        let mut descendants: Vec<String> = parents
            .keys()
            .filter(|name| descends_from(name, &owner))
            .cloned()
            .collect();
        descendants.sort_by_key(|name| (std::cmp::Reverse(depth(name)), name.clone()));
        let mut rewrite = Rewrite { owner, parents: &parents, descendants, direct: &direct };
        for member in &mut class.class.body {
            if let ClassMember::Constructor(constructor) = member {
                constructor.params.visit_mut_with(&mut rewrite);
                constructor.body.visit_mut_with(&mut rewrite);
            }
        }
    }
    module
}

const PROMISE_REJECTION_CALLBACK_SUFFIX: &str = "__thaw_promise_rejection";

fn specialize_named_promise_rejection_callbacks(module: &Module) -> Result<Module, String> {
    struct Collector {
        names: HashSet<Symbol>,
    }
    impl Visit for Collector {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            let Callee::Expr(callee) = &call.callee else {
                call.visit_children_with(self);
                return;
            };
            let Expr::Member(member) = callee.as_ref() else {
                call.visit_children_with(self);
                return;
            };
            let MemberProp::Ident(property) = &member.prop else {
                call.visit_children_with(self);
                return;
            };
            let argument = match property.sym.as_ref() {
                "catch" => call.args.first(),
                "then" => call.args.get(1),
                _ => None,
            };
            if let Some(Expr::Ident(ident)) = argument.map(|argument| argument.expr.as_ref()) {
                self.names.insert(ident.sym.to_string());
            }
            call.visit_children_with(self);
        }
    }

    let mut collector = Collector {
        names: HashSet::new(),
    };
    module.visit_with(&mut collector);
    if collector.names.is_empty() {
        return Ok(module.clone());
    }

    let declared = module
        .body
        .iter()
        .filter_map(|item| match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) => {
                Some(function.ident.sym.to_string())
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    collector.names.retain(|name| declared.contains(name));
    struct BindingCounts {
        counts: HashMap<Symbol, usize>,
    }
    impl Visit for BindingCounts {
        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            *self
                .counts
                .entry(declaration.ident.sym.to_string())
                .or_default() += 1;
            declaration.function.visit_with(self);
        }

        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            *self.counts.entry(binding.id.sym.to_string()).or_default() += 1;
            binding.type_ann.visit_with(self);
        }
    }
    let mut binding_counts = BindingCounts {
        counts: HashMap::new(),
    };
    module.visit_with(&mut binding_counts);
    // ponytail: specialize only unshadowed top-level declarations; extend the
    // lexical resolver when stored/local function values need the same path.
    collector
        .names
        .retain(|name| binding_counts.counts.get(name) == Some(&1));
    if collector.names.is_empty() {
        return Ok(module.clone());
    }

    struct Rewriter<'a> {
        names: &'a HashSet<Symbol>,
    }
    impl VisitMut for Rewriter<'_> {
        fn visit_mut_call_expr(&mut self, call: &mut CallExpr) {
            call.visit_mut_children_with(self);
            let Callee::Expr(callee) = &call.callee else {
                return;
            };
            let Expr::Member(member) = callee.as_ref() else {
                return;
            };
            let MemberProp::Ident(property) = &member.prop else {
                return;
            };
            let argument = match property.sym.as_ref() {
                "catch" => call.args.first_mut(),
                "then" => call.args.get_mut(1),
                _ => None,
            };
            let Some(Expr::Ident(ident)) = argument.map(|argument| argument.expr.as_mut()) else {
                return;
            };
            if self.names.contains(ident.sym.as_ref()) {
                ident.sym = format!("{}{PROMISE_REJECTION_CALLBACK_SUFFIX}", ident.sym).into();
            }
        }
    }

    let mut specialized = module.clone();
    specialized.visit_mut_with(&mut Rewriter {
        names: &collector.names,
    });
    struct References {
        names: HashSet<Symbol>,
    }
    impl Visit for References {
        fn visit_ident(&mut self, ident: &swc_ecma_ast::Ident) {
            self.names.insert(ident.sym.to_string());
        }

        fn visit_fn_decl(&mut self, declaration: &FnDecl) {
            declaration.function.visit_with(self);
        }
    }
    let mut references = References {
        names: HashSet::new(),
    };
    specialized.visit_with(&mut references);
    specialized.body.retain(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
            return true;
        };
        !collector.names.contains(function.ident.sym.as_ref())
            || references.names.contains(function.ident.sym.as_ref())
    });
    let clones = module.body.iter().filter_map(|item| {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function))) = item else {
            return None;
        };
        collector
            .names
            .contains(function.ident.sym.as_ref())
            .then(|| {
                let mut function = function.clone();
                function.ident.sym =
                    format!("{}{PROMISE_REJECTION_CALLBACK_SUFFIX}", function.ident.sym).into();
                ModuleItem::Stmt(Stmt::Decl(Decl::Fn(function)))
            })
    });
    specialized.body.extend(clones);
    Ok(specialized)
}

fn normalize_interface_merges(module: &Module) -> Result<Module, String> {
    fn interface(item: &ModuleItem) -> Option<&TsInterfaceDecl> {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(value))) => Some(value),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(value)) => match &value.decl {
                Decl::TsInterface(value) => Some(value),
                _ => None,
            },
            _ => None,
        }
    }
    fn interface_mut(item: &mut ModuleItem) -> Option<&mut TsInterfaceDecl> {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(value))) => Some(value),
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(value)) => match &mut value.decl {
                Decl::TsInterface(value) => Some(value),
                _ => None,
            },
            _ => None,
        }
    }

    let mut body = Vec::with_capacity(module.body.len());
    let mut positions = HashMap::new();
    for item in &module.body {
        let Some(next) = interface(item) else {
            body.push(item.clone());
            continue;
        };
        let name = next.id.sym.to_string();
        let Some(&index) = positions.get(&name) else {
            positions.insert(name, body.len());
            body.push(item.clone());
            continue;
        };
        let current = interface_mut(&mut body[index]).unwrap();
        let current_params = current.type_params.as_ref().map(|value| &value.params);
        let next_params = next.type_params.as_ref().map(|value| &value.params);
        if current_params.map(Vec::len) != next_params.map(Vec::len) {
            return Err(format!(
                "merged interface `{name}` must use the same type parameters"
            ));
        }
        current.body.body.extend(next.body.body.iter().cloned());
        current.extends.extend(next.extends.iter().cloned());
    }
    let mut merged = module.clone();
    merged.body = body;
    Ok(merged)
}

/// `Function.prototype.length` of a declared function: the leading parameters
/// before the first default-valued or rest parameter. TypeScript `?` markers
/// are erased by SWC's pattern, so optional parameters still count.
fn source_parameter_count(params: &[swc_ecma_ast::Param]) -> usize {
    params
        .iter()
        .take_while(|parameter| {
            !matches!(
                parameter.pat,
                swc_ecma_ast::Pat::Assign(_) | swc_ecma_ast::Pat::Rest(_)
            )
        })
        .count()
}

fn generic_receiver_pattern_from_annotation(
    ty: &TsType,
    substitutions: &HashMap<Symbol, GenericTypePattern>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces<'_>,
) -> GenericReceiverPattern {
    generic_type_pattern(
        ty,
        substitutions,
        interfaces,
        generic_interfaces,
        &mut Vec::new(),
    )
    .map(GenericReceiverPattern::Pattern)
    .unwrap_or(GenericReceiverPattern::LegacyUnmatched)
}

fn lower_normalized_module(module: &Module) -> Result<HirProgram, String> {
    let _storage_scope = StorageScope::enter(module)?;
    let (mut interfaces, generic_interfaces) = resolve_interfaces(module)?;
    let specialized = specialize_generic_classes(module, &interfaces, &generic_interfaces)
        .map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        })?;
    if let Some(specialized) = specialized {
        return lower_normalized_module(&specialized).map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        });
    }
    let (enum_values, enum_reverse_values, enum_types) = collect_enums(module)?;
    for (name, ty) in enum_types {
        if interfaces.insert(name.clone(), ty).is_some() {
            return Err(format!(
                "enum `{name}` conflicts with an interface declaration"
            ));
        }
    }
    let native_interfaces = if module.body.iter().any(|item| matches!(item,
        ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration)))
            if declaration.function.body.is_none() && dynamic_symbol(declaration.ident.sym.as_ref()).is_none())) {
        let _native_layout = StorageScope::suspend();
        let mut native = interfaces.clone();
        collect_native_classes(module, &mut native, &generic_interfaces)?;
        Some(native)
    } else { None };
    let class_decls = collect_native_classes(module, &mut interfaces, &generic_interfaces)?;
    if let Some(specialized) =
        specialize_generic_class_methods(module, &interfaces, &generic_interfaces)?
    {
        return lower_normalized_module(&specialized);
    }
    let value_referenced_classes = class_names_referenced_as_values(module, &class_decls);

    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut global_decls = Vec::new();
    let mut generic_instantiations: HashMap<Symbol, Vec<Vec<HirType>>> = HashMap::new();
    let mut implementation_names = std::collections::HashSet::new();
    let mut implementation_counts: HashMap<Symbol, usize> = HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(declaration))) = item {
            if declaration.function.body.is_some() {
                let name = declaration.ident.sym.to_string();
                implementation_names.insert(name.clone());
                *implementation_counts.entry(name).or_default() += 1;
            }
        }
    }
    if let Some((name, _)) = implementation_counts.iter().find(|(_, count)| **count > 1) {
        return Err(format!("function `{name}` has multiple implementations"));
    }
    let mut public_overloads: HashMap<Symbol, Vec<HirType>> = HashMap::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                let name = fn_decl.ident.sym.to_string();
                let func = &fn_decl.function;
                let is_overload_declaration = func.body.is_none() && implementation_names.contains(&name);
                let is_extern = func.body.is_none() && !is_overload_declaration;
                let annotation_interfaces = if is_extern && dynamic_symbol(&name).is_none() {
                    native_interfaces.as_ref().unwrap_or(&interfaces)
                } else { &interfaces };
                if is_extern && func.is_async {
                    return Err(format!("ambient function `{name}` cannot be async"));
                }
                let type_substitution = function_type_substitution(func);
                let generic_type_params = validate_generic_function(fn_decl)?;
                let generic_type_constraints = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.constraint.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let generic_type_defaults = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.default.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let generic_substitutions = if generic_type_params.is_empty() {
                    None
                } else {
                    Some(
                        generic_type_params
                            .iter()
                            .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                            .collect::<HashMap<_, _>>(),
                    )
                };
                let generic_param_patterns = match &generic_substitutions {
                    None => Vec::new(),
                    Some(substitutions) => func
                        .params
                        .iter()
                        .map(|param| match &param.pat {
                            Pat::Ident(binding) => binding
                                .type_ann
                                .as_ref()
                                .ok_or_else(|| format!("generic function `{name}` needs parameter type annotations"))
                                .and_then(|ann| generic_type_pattern(
                                    &ann.type_ann,
                                    substitutions,
                                    annotation_interfaces,
                                    &generic_interfaces,
                                    &mut Vec::new(),
                                )),
                            Pat::Rest(rest) => rest
                                .type_ann
                                .as_ref()
                                .ok_or_else(|| {
                                    format!(
                                        "generic function `{name}` needs a rest parameter type annotation"
                                    )
                                })
                                .and_then(|ann| {
                                    generic_type_pattern(
                                        &ann.type_ann,
                                        substitutions,
                                        annotation_interfaces,
                                        &generic_interfaces,
                                        &mut Vec::new(),
                                    )
                                }),
                            _ => Err(format!("generic function `{name}` requires identifier parameters")),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                };
                // Best-effort only (never aborts the signature over it,
                // unlike the parameter patterns above): a fallback source
                // for inferring a type parameter that appears in no
                // parameter at all, e.g. `nanoid<Type extends string>
                // (size?: number): Type`. See
                // `infer_generic_type_tuple`/`lower_expr_with_expected_type`.
                let generic_return_pattern = generic_substitutions.as_ref().and_then(|substitutions| {
                    func.return_type.as_ref().and_then(|ann| {
                        generic_type_pattern(
                            &ann.type_ann,
                            substitutions,
                            annotation_interfaces,
                            &generic_interfaces,
                            &mut Vec::new(),
                        )
                        .ok()
                    })
                });
                let variadic = if is_extern {
                    func.params.last().and_then(|param| match &param.pat {
                        Pat::Rest(rest) => Some(rest),
                        _ => None,
                    }).map(|rest| {
                        let annotation = rest.type_ann.as_ref().ok_or_else(|| {
                            format!("ambient variadic function `{name}` needs a rest parameter type annotation")
                        })?;
                        let ty = resolve_ts_type_with_substitution(
                            &annotation.type_ann,
                            &type_substitution,
                            annotation_interfaces,
                            &generic_interfaces,
                            &mut Vec::new(),
                        )?;
                        match ty {
                            HirType::Array(element)
                                if supports_ffi_variadic_element(&element) => Ok(*element),
                            other => Err(format!(
                                "ambient variadic function `{name}` has unsupported rest element layout {other:?}"
                            )),
                        }
                    }).transpose()?
                } else {
                    None
                };
                let fixed_param_count = func.params.len() - usize::from(variadic.is_some());
                let mut params = func
                    .params
                    .iter()
                    .take(fixed_param_count)
                    .map(|p| {
                        lower_param(
                            &p.pat,
                            annotation_interfaces,
                            &generic_interfaces,
                            !is_extern,
                            &type_substitution,
                        )
                        .map(|p| p.ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let quickjs_function_value = is_extern
                    && ordinary_quickjs_typed_symbol(&name).is_some()
                    && func.type_params.as_ref().is_none_or(|params| params.params.is_empty())
                    && func.this_param.is_none()
                    && func.params.iter().all(|parameter| {
                        !matches!(parameter.pat, Pat::Rest(_))
                            && !matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                    });
                if quickjs_function_value {
                    for (parameter, ty) in func.params.iter().take(fixed_param_count).zip(&params) {
                        remember_storage_source(storage_key(parameter.pat.span()), ty);
                    }
                }
                if name.ends_with(PROMISE_REJECTION_CALLBACK_SUFFIX) && !params.is_empty() {
                    // The rejection reason is a string; a leading rest parameter
                    // receives it as a one-element array.
                    params[0] = if matches!(func.params.first().map(|p| &p.pat), Some(Pat::Rest(_))) {
                        HirType::Array(Box::new(HirType::Str))
                    } else {
                        HirType::Str
                    };
                }
                let mut generic_this_pattern = None;
                if let Some(this) = &func.this_param {
                    let annotation = this.type_ann.as_ref().ok_or_else(|| {
                        format!("function `{name}` needs a type annotation for `this`")
                    })?;
                    let receiver_type = resolve_ts_type_with_substitution(
                        &annotation.type_ann,
                        &type_substitution,
                        annotation_interfaces,
                        &generic_interfaces,
                        &mut Vec::new(),
                    )?;
                    params.insert(0, receiver_type);
                    if let Some(substitutions) = &generic_substitutions {
                        generic_this_pattern = Some(generic_receiver_pattern_from_annotation(
                            &annotation.type_ann,
                            substitutions,
                            annotation_interfaces,
                            &generic_interfaces,
                        ));
                    }
                }
                if !is_extern && generic_type_params.is_empty() {
                    for (index, parameter) in func.params.iter().enumerate() {
                        let physical_index = index + usize::from(func.this_param.is_some());
                        if pattern_is_omittable(&parameter.pat)
                            && !matches!(params[physical_index], HirType::Dynamic | HirType::Json)
                        {
                            params[physical_index] = optional_parameter_type(params[physical_index].clone());
                        }
                    }
                }
                let assertion_function = matches!(
                    func.return_type.as_deref().map(|ann| ann.type_ann.as_ref()),
                    Some(TsType::TsTypePredicate(predicate)) if predicate.asserts
                );
                let ret = if func.is_generator {
                    lower_generator_return_type(
                        func.is_async,
                        &func.return_type,
                        &name,
                        annotation_interfaces,
                        &generic_interfaces,
                        &type_substitution,
                    )?
                } else if assertion_function {
                    HirType::Void
                } else if func.return_type.is_none() && !is_extern {
                    HirType::Dynamic
                } else {
                    lower_fn_return_type(
                        func.is_async,
                        &func.return_type,
                        &name,
                        annotation_interfaces,
                        &generic_interfaces,
                        &type_substitution,
                        None,
                    )?
                };
                if quickjs_function_value {
                    remember_storage_source((func.span.lo.0, 0), &ret);
                }
                let ret = if is_extern { ret } else { storage_return_type(func.span, ret) };
                let type_predicate = match func.return_type.as_deref().map(|ann| ann.type_ann.as_ref()) {
                    Some(TsType::TsTypePredicate(predicate)) => {
                        let swc_ecma_ast::TsThisTypeOrIdent::Ident(parameter) = &predicate.param_name else {
                            return Err(format!("function `{name}` cannot predicate `this` yet"));
                        };
                        let index = func
                            .params
                            .iter()
                            .position(|candidate| matches!(&candidate.pat, Pat::Ident(binding) if binding.id.sym == parameter.sym))
                            .ok_or_else(|| format!("type predicate of `{name}` names an unknown parameter `{}`", parameter.sym))?;
                        let target = predicate
                            .type_ann
                            .as_ref()
                            .ok_or_else(|| format!("type predicate of `{name}` needs a target type"))?;
                        let target = if let Some(substitutions) = &generic_substitutions {
                            generic_type_pattern(
                                &target.type_ann,
                                substitutions,
                                annotation_interfaces,
                                &generic_interfaces,
                                &mut Vec::new(),
                            )?
                        } else {
                            GenericTypePattern::Concrete(resolve_ts_type_with_substitution(
                                &target.type_ann,
                                &type_substitution,
                                annotation_interfaces,
                                &generic_interfaces,
                                &mut Vec::new(),
                            )?)
                        };
                        Some((index, target, predicate.asserts))
                    }
                    _ => None,
                };
                // A plain non-ambient function's trailing
                // `...rest: T[]` parameter reuses the exact same
                // `native_rest` mechanism a native class constructor's
                // rest parameter already does -- `params` here still holds
                // it as a regular `Array(T)`-typed last entry (unlike the
                // ambient/FFI `variadic` mechanism above, which excludes it
                // from `params` for a real C variadic ABI), so every call
                // site that already packs trailing arguments into that
                // slot for a constructor needs no changes to also do it
                // here. Ambient functions use their FFI-specific
                // `variadic` handling; a generic rest element is initially
                // `Dynamic` and reconstructed for each monomorphization.
                let native_rest = if is_extern {
                    None
                } else {
                    let patterns = func
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    native_rest_element(&patterns, &params)?
                };
                let signature = FnSignature {
                        public_overloads: Vec::new(),
                        params: params.clone(),
                        variadic,
                        native_rest: native_rest.clone(),
                        abstract_class_constructor: false,
                        ret: ret.clone(),
                        is_async: func.is_async && !func.is_generator,
                        returns_sparse_array: function_returns_sparse_array(func),
                        uses_this: func.this_param.is_some(),
                        is_extern,
                        source_range: (func.span.lo.0, func.span.hi.0),
                        source_length: Some(source_parameter_count(&func.params)),
                        accessor_owner: None,
                        generic_type_params: generic_type_params.clone(),
                        generic_type_constraints,
                        generic_type_defaults,
                        generic_param_patterns,
                        generic_param_optional: func
                            .params
                            .iter()
                            .map(|parameter| {
                                matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                            })
                            .collect(),
                        source_omittable: if is_extern {
                            HirOptionalMask::default()
                        } else {
                            source_omission_mask(
                                &func.params.iter().map(|parameter| parameter.pat.clone()).collect::<Vec<_>>(),
                                usize::from(func.this_param.is_some()),
                            )
                        },
                        generic_this_pattern,
                        generic_return_type: func.return_type.as_ref().map(|ann| ann.type_ann.clone()),
                        generic_return_pattern,
                        type_predicate,
                    };
                if is_overload_declaration {
                    if !generic_type_params.is_empty() {
                        return Err(format!("generic overload declarations for `{name}` are not supported yet"));
                    }
                    let receiver_count = usize::from(func.this_param.is_some());
                    let visible_params = if receiver_count == 1 { &params[1..] } else { &params[..] };
                    let fixed_count = visible_params.len() - usize::from(native_rest.is_some());
                    let public_return = if func.is_async && !func.is_generator {
                        HirType::Promise(Box::new(ret.clone()))
                    } else {
                        ret.clone()
                    };
                    let optional = mask_with_offset(
                        &source_omission_mask(
                            &func.params.iter().map(|parameter| parameter.pat.clone()).collect::<Vec<_>>(),
                            0,
                        ),
                        0,
                        fixed_count,
                    );
                    let visible = HirType::CallableFunction(
                        visible_params[..fixed_count].to_vec(),
                        optional,
                        native_rest.clone().map(Box::new),
                        Box::new(public_return),
                    );
                    let public = if receiver_count == 1 {
                        HirType::FunctionWithThis(Box::new(params[0].clone()), Box::new(visible))
                    } else {
                        visible
                    };
                    public_overloads.entry(name).or_default().push(public);
                } else {
                    signatures.insert(name.clone(), signature);
                }
                if !is_extern && !is_overload_declaration {
                    fn_decls.push(fn_decl);
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class_decl))) => {
                let name = class_decl.ident.sym.to_string();
                let instance_type = interfaces[&name].clone();
                let constructors = class_decl
                    .class
                    .body
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Constructor(constructor) if constructor.body.is_some() => {
                            Some(constructor)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if constructors.len() > 1 {
                    return Err(format!(
                        "class `{name}` has multiple constructor implementations"
                    ));
                }
                let params = constructors
                    .first()
                    .map(|constructor| {
                        constructor
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &class_constructor_param_pattern(parameter),
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|lowered| {
                                    let pattern = class_constructor_param_pattern(parameter);
                                    let ty = storage_parameter_type(&pattern, lowered.ty);
                                    if pattern_is_omittable(&pattern) {
                                        optional_parameter_type(ty)
                                    } else {
                                        ty
                                    }
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let mut params = params;
                // `Symbol.species` builds `new this.constructor(length)`: on an `extends Array`
                // class every required parameter besides a numeric first one (and an optional
                // non-numeric first one) is `any`, because the species call hands it a number /
                // `undefined`.
                if matches!(native_base_type(&instance_type), Some(HirType::Array(_))) {
                    let patterns = constructors
                        .first()
                        .map(|constructor| {
                            constructor.params.iter().map(class_constructor_param_pattern).collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    for (index, (param, pattern)) in params.iter_mut().zip(&patterns).enumerate() {
                        if !matches!(pattern, Pat::Ident(_)) {
                            continue;
                        }
                        match param {
                            HirType::Optional(payload)
                                if index == 0 && !matches!(payload.as_ref(), HirType::F64 | HirType::Json) =>
                            {
                                **payload = HirType::Json;
                            }
                            HirType::Optional(_) | HirType::Json => {}
                            HirType::F64 if index == 0 => {}
                            _ => *param = HirType::Json,
                        }
                    }
                }
                signatures.insert(
                    class_constructor_symbol(&name),
                    FnSignature {
                        public_overloads: Vec::new(),
                        params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: class_decl.class.is_abstract,
                        ret: instance_type.clone(),
                        is_async: false,
                        returns_sparse_array: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        source_length: None,
                        accessor_owner: None,
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_this_pattern: None,
                        generic_return_type: None,
                        generic_return_pattern: None,
                        source_omittable: constructors.first().map_or_else(
                            HirOptionalMask::default,
                            |constructor| source_omission_mask(
                                &constructor.params.iter().map(class_constructor_param_pattern).collect::<Vec<_>>(),
                                0,
                            ),
                        ),
                        type_predicate: None,
                    },
                );
                let mut initializer_params = vec![instance_type.clone()];
                initializer_params.extend(
                    signatures[&class_constructor_symbol(&name)]
                        .params
                        .iter()
                        .cloned(),
                );
                signatures.insert(
                    class_initializer_symbol(&name),
                    FnSignature {
                        public_overloads: Vec::new(),
                        params: initializer_params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: instance_type.clone(),
                        is_async: false,
                        returns_sparse_array: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        source_length: None,
                        accessor_owner: None,
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_this_pattern: None,
                        generic_return_type: None,
                        generic_return_pattern: None,
                        source_omittable: mask_with_offset(
                            &signatures[&class_constructor_symbol(&name)].source_omittable,
                            1,
                            signatures[&class_constructor_symbol(&name)].params.len() + 1,
                        ),
                        type_predicate: None,
                    },
                );
                if let Some(constructor) = constructors.first() {
                    let patterns = constructor
                        .params
                        .iter()
                        .map(class_constructor_param_pattern)
                        .collect::<Vec<_>>();
                    let constructor_symbol = class_constructor_symbol(&name);
                    let initializer_symbol = class_initializer_symbol(&name);
                    let native_rest =
                        native_rest_element(&patterns, &signatures[&constructor_symbol].params)?;
                    signatures
                        .get_mut(&constructor_symbol)
                        .expect("class constructor signature")
                        .native_rest = native_rest.clone();
                    signatures
                        .get_mut(&initializer_symbol)
                        .expect("class initializer signature")
                        .native_rest = native_rest;
                }
                for member in &class_decl.class.body {
                    let ClassMember::Method(method) = member else {
                        continue;
                    };
                    if !matches!(
                        method.kind,
                        MethodKind::Method | MethodKind::Getter | MethodKind::Setter
                    ) {
                        continue;
                    }
                    if method.function.type_params.is_some() {
                        return Err(format!(
                            "class `{name}` method `{}` cannot be generic yet",
                            class_property_name(&method.key)?
                        ));
                    }
                    let method_name = class_property_name(&method.key)?;
                    if method.kind == MethodKind::Getter && !method.function.params.is_empty() {
                        return Err(format!(
                            "class `{name}` getter `{method_name}` cannot accept parameters"
                        ));
                    }
                    if method.kind == MethodKind::Setter && method.function.params.len() != 1 {
                        return Err(format!(
                            "class `{name}` setter `{method_name}` requires exactly one parameter"
                        ));
                    }
                    let mut params = if method.is_static {
                        Vec::new()
                    } else {
                        vec![instance_type.clone()]
                    };
                    params.extend(
                        method
                            .function
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &parameter.pat,
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|lowered| storage_parameter_type(&parameter.pat, lowered.ty))
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                    if method.kind == MethodKind::Method {
                        for (index, parameter) in method.function.params.iter().enumerate() {
                            let physical_index = index + usize::from(!method.is_static);
                            if pattern_is_omittable(&parameter.pat)
                                && !matches!(params[physical_index], HirType::Dynamic | HirType::Json)
                            {
                                params[physical_index] =
                                    optional_parameter_type(params[physical_index].clone());
                            }
                        }
                    }
                    let ret = if method.kind == MethodKind::Setter {
                        HirType::Void
                    } else if method.function.is_generator {
                        lower_generator_return_type(
                            method.function.is_async,
                            &method.function.return_type,
                            &format!("{name}.{method_name}"),
                            &interfaces,
                            &generic_interfaces,
                            &HashMap::new(),
                        )?
                    } else if method.function.return_type.is_none()
                        && !method.is_abstract
                        && (!class_decl.class.is_abstract || method.is_static)
                    {
                        // Mirrors the identical free-function case just
                        // above (`func.return_type.is_none() && !is_extern
                        // => HirType::Dynamic`) -- an unannotated method
                        // previously always got `lower_fn_return_type`'s
                        // own `None => HirType::Void` default *permanently
                        // baked into its signature*, with no path to ever
                        // correct it once the body is actually lowered
                        // (unlike a free function, whose signature-
                        // building pass already deferred to `Dynamic` for
                        // exactly this reason). Confirmed via a real
                        // probe: an unannotated method returning a plain
                        // object literal hard-errored ("unsupported
                        // property access on a value of type Void") and
                        // one returning a number/string/array *silently*
                        // produced a wrong value at runtime (`0` in every
                        // case) instead of either -- the same underlying
                        // bug surfacing two different ways depending on
                        // what the caller does with the bogus `Void`.
                        // Excludes an abstract method (no body of its own
                        // exists to later infer *from* -- an abstract
                        // method genuinely does need an explicit
                        // annotation, matching real TS convention).
                        HirType::Dynamic
                    } else {
                        lower_fn_return_type(
                            method.function.is_async,
                            &method.function.return_type,
                            &format!("{name}.{method_name}"),
                            &interfaces,
                            &generic_interfaces,
                            &HashMap::new(),
                            interfaces.get(name.as_str()),
                        )?
                    };
                    if method.kind == MethodKind::Getter && !method.is_static {
                        remember_class_source_field(&name, &method_name, method.function.span, false);
                        remember_storage_source(storage_key(method.function.span), &ret);
                    }
                    let ret = storage_return_type(method.function.span, ret);
                    let symbol = match method.kind {
                        MethodKind::Getter => {
                            class_getter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Setter => {
                            class_setter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Method if method.is_static => {
                            class_static_method_symbol(&name, &method_name)
                        }
                        MethodKind::Method => class_method_symbol(&name, &method_name),
                    };
                    if signatures.contains_key(&symbol) {
                        return Err(format!(
                            "class `{name}` has duplicate method `{method_name}`"
                        ));
                    }
                    signatures.insert(
                        symbol,
                        FnSignature {
                        public_overloads: Vec::new(),
                            params,
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret,
                            is_async: method.function.is_async && !method.function.is_generator,
                            returns_sparse_array: function_returns_sparse_array(&method.function),
                            uses_this: function_uses_this(&method.function),
                            is_extern: false,
                            source_range: (method.span.lo.0, method.span.hi.0),
                            source_length: None,
                            accessor_owner: Some(name.clone()),
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_this_pattern: None,
                            generic_return_type: None,
                            generic_return_pattern: None,
                            source_omittable: source_omission_mask(
                                &method.function.params.iter().map(|parameter| parameter.pat.clone()).collect::<Vec<_>>(),
                                usize::from(!method.is_static),
                            ),
                            type_predicate: None,
                        },
                    );
                    let patterns = method
                        .function
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    let symbol = class_member_symbol(&name, method)?;
                    let native_rest = native_rest_element(&patterns, &signatures[&symbol].params)?;
                    signatures
                        .get_mut(&symbol)
                        .expect("class method signature")
                        .native_rest = native_rest;
                    let unbound_symbol = (method.kind == MethodKind::Method
                        && signatures[&symbol].uses_this
                        // A `return this` method's detached form can't bind
                        // `this` (see `declarations.rs`'s matching gate).
                        && signatures[&symbol].ret != instance_type)
                        .then(|| unbound_class_method_symbol(&symbol));
                    if let Some(unbound_symbol) = &unbound_symbol {
                        let mut unbound = signatures[&symbol].clone();
                        if !method.is_static {
                            unbound.params.remove(0);
                            unbound.source_omittable = unbound.source_omittable.shifted(1);
                        }
                        unbound.uses_this = false;
                        unbound.generic_this_pattern = None;
                        signatures.insert(unbound_symbol.clone(), unbound);
                    }
                }
            }
            // Already consumed by `resolve_interfaces` above.
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) => {
                for declaration in &var_decl.decls {
                    let Pat::Ident(binding) = &declaration.name else {
                        return Err(
                            "top-level destructuring declarations are not supported yet".into()
                        );
                    };
                    if declaration.init.is_none() {
                        return Err(format!(
                            "top-level binding `{}` needs an initializer",
                            binding.id.sym
                        ));
                    }
                }
                global_decls.push(var_decl.as_ref());
            }
            ModuleItem::Stmt(_) => {}
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }

    // JavaScript synthesizes `constructor(...args) { super(...args); }` for a
    // derived class without an explicit constructor. Propagate the nearest
    // base signature to a fixed point so forward declarations and multi-level
    // implicit constructor chains receive the same concrete argument tuple.
    for _ in 0..class_decls.len() {
        let mut changed = false;
        for derived in &class_decls {
            if derived
                .class
                .body
                .iter()
                .any(|member| {
                    matches!(member, ClassMember::Constructor(constructor) if constructor.body.is_some())
                })
            {
                continue;
            }
            let Some(Expr::Ident(base)) = derived.class.super_class.as_deref() else {
                continue;
            };
            let derived_name = derived.ident.sym.as_ref();
            let base_symbol = class_constructor_symbol(base.sym.as_ref());
            let (base_params, base_native_rest) = match signatures.get(&base_symbol) {
                Some(signature) => (signature.params.clone(), signature.native_rest.clone()),
                None if is_error_family_name(base.sym.as_ref()) => (vec![HirType::Str], None),
                // `extends Map`/`Set` without a constructor takes the optional
                // entries/values array, like `new Map(entries)` / `new Set(values)`.
                // `extends Array<T>` without one takes `...items: T[]`.
                None if base.sym.as_ref() == "Array" => {
                    match interfaces.get(derived_name).and_then(native_base_type) {
                        Some(array @ HirType::Array(element)) => {
                            (vec![array.clone()], Some(element.as_ref().clone()))
                        }
                        _ => {
                            return Err(format!(
                                "derived class `{derived_name}` has no native array layout"
                            ))
                        }
                    }
                }
                None if matches!(base.sym.as_ref(), "Map" | "Set") => {
                    let iterable = match interfaces.get(derived_name).and_then(native_base_type) {
                        Some(HirType::Map(key, value)) => HirType::Array(Box::new(
                            HirType::Tuple(vec![key.as_ref().clone(), value.as_ref().clone()]),
                        )),
                        Some(HirType::Set(element)) => HirType::Array(element.clone()),
                        _ => {
                            return Err(format!(
                                "derived class `{derived_name}` has no native collection layout"
                            ))
                        }
                    };
                    (vec![HirType::Optional(Box::new(iterable))], None)
                }
                None => {
                    return Err(format!(
                        "derived class `{derived_name}` has no constructor signature for `{}`",
                        base.sym
                    ))
                }
            };
            let base_omittable = signatures.get(&base_symbol).map_or_else(
                || {
                    // The native collection's optional entries argument.
                    let native = matches!(base.sym.as_ref(), "Map" | "Set");
                    HirOptionalMask::from_bools(&vec![native; base_params.len()])
                },
                |signature| signature.source_omittable.clone(),
            );
            let constructor = signatures
                .get_mut(&class_constructor_symbol(derived_name))
                .expect("derived constructor signature");
            if constructor.params != base_params {
                constructor.params = base_params.clone();
                changed = true;
            }
            constructor.native_rest = base_native_rest.clone();
            if constructor.source_omittable != base_omittable {
                constructor.source_omittable = base_omittable.clone();
                changed = true;
            }
            let mut initializer_params = vec![interfaces[derived_name].clone()];
            initializer_params.extend(base_params.iter().cloned());
            let initializer = signatures
                .get_mut(&class_initializer_symbol(derived_name))
                .expect("derived initializer signature");
            initializer.params = initializer_params;
            initializer.native_rest = base_native_rest;
            let initializer_omittable = mask_with_offset(
                &base_omittable,
                1,
                base_params.len() + 1,
            );
            if initializer.source_omittable != initializer_omittable {
                initializer.source_omittable = initializer_omittable;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let class_by_name = class_decls
        .iter()
        .map(|declaration| (declaration.ident.sym.to_string(), *declaration))
        .collect::<HashMap<_, _>>();
    let member_key = |member: &swc_ecma_ast::ClassMethod| -> Result<(u8, bool, Symbol), String> {
        Ok((
            match member.kind {
                MethodKind::Method => 0,
                MethodKind::Getter => 1,
                MethodKind::Setter => 2,
            },
            member.is_static,
            class_property_name(&member.key)?,
        ))
    };
    let mut inherited_class_functions = Vec::new();
    let mut inherited_static_globals = Vec::new();
    let mut inherited_virtual_class_decls = Vec::new();
    for derived in &class_decls {
        let derived_name = derived.ident.sym.to_string();
        let derived_type = interfaces[&derived_name].clone();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::Method(method) => member_key(method).ok(),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        while let Some(current_name) = base_name {
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`'s synthetic base-layout branch) and
            // so have no members of their own to inherit here.
            let Some(&base) = class_by_name.get(&current_name) else {
                break;
            };
            for member in &base.class.body {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                let key = member_key(method)?;
                let newly_seen = seen.insert(key);
                if method.is_abstract {
                    if !newly_seen {
                        let base_symbol = class_member_symbol(&current_name, method)?;
                        let derived_symbol = class_member_symbol(&derived_name, method)?;
                        let base_signature = &signatures[&base_symbol];
                        let derived_signature = &signatures[&derived_symbol];
                        let receiver_count = usize::from(!method.is_static);
                        if base_signature.params[receiver_count..]
                            != derived_signature.params[receiver_count..]
                            || base_signature.ret != derived_signature.ret
                            || base_signature.is_async != derived_signature.is_async
                            || base_signature.native_rest != derived_signature.native_rest
                        {
                            return Err(format!(
                                "class `{derived_name}` implements abstract member `{}` from `{current_name}` with an incompatible signature",
                                class_property_name(&method.key)?
                            ));
                        }
                        continue;
                    }
                    if !derived.class.is_abstract {
                        return Err(format!(
                            "concrete class `{derived_name}` must implement abstract member `{}` from `{current_name}`",
                            class_property_name(&method.key)?
                        ));
                    }
                    continue;
                }
                if !newly_seen {
                    continue;
                }
                let base_symbol = class_member_symbol(&current_name, method)?;
                let derived_symbol = class_member_symbol(&derived_name, method)?;
                let mut signature = signatures[&base_symbol].clone();
                if !method.is_static {
                    signature.params[0] = derived_type.clone();
                }
                signatures.insert(derived_symbol.clone(), signature.clone());
                if method.kind == MethodKind::Method && signature.uses_this {
                    let base_unbound = unbound_class_method_symbol(&base_symbol);
                    let derived_unbound = unbound_class_method_symbol(&derived_symbol);
                    let mut unbound_signature =
                        signatures.get(&base_unbound).cloned().unwrap_or_else(|| {
                            let mut unbound = signature.clone();
                            if !method.is_static {
                                unbound.params.remove(0);
                                unbound.source_omittable = unbound.source_omittable.shifted(1);
                            }
                            unbound
                        });
                    unbound_signature.uses_this = false;
                    unbound_signature.generic_this_pattern = None;
                    signatures.insert(derived_unbound, unbound_signature);
                }
                if !derived.class.is_abstract {
                    let mut inherited = (*base).clone();
                    inherited.ident = derived.ident.clone();
                    inherited.class.body = vec![ClassMember::Method(method.clone())];
                    inherited.class.is_abstract = false;
                    inherited.class.implements.clear();
                    inherited_virtual_class_decls.push(inherited);
                }

            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    let mut global_types = HashMap::new();
    let mut immutable_globals = HashSet::new();
    for declaration in &global_decls {
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                unreachable!("top-level patterns were validated above")
            };
            let name = binding.id.sym.to_string();
            if global_types.contains_key(&name) || signatures.contains_key(&name) {
                return Err(format!("duplicate top-level binding `{name}`"));
            }
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)
                })
                .transpose()?
                .unwrap_or(HirType::Dynamic);
            let ty = storage_type(binding.id.span, ty);
            global_types.insert(name, ty);
            if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                immutable_globals.insert(binding.id.sym.to_string());
            }
        }
    }
    for declaration in &class_decls {
        let class_name = declaration.ident.sym.as_ref();
        let mut fields = HashSet::new();
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if !property.is_static {
                continue;
            }
            let field = class_property_name(&property.key)?;
            if property.is_abstract || property.declare {
                return Err(format!(
                    "class `{class_name}` static field `{field}` cannot be abstract or ambient"
                ));
            }
            if !fields.insert(field.clone()) {
                return Err(format!(
                    "class `{class_name}` has duplicate static field `{field}`"
                ));
            }
            for member_symbol in [
                class_static_method_symbol(class_name, &field),
                class_getter_symbol(class_name, &field, true),
                class_setter_symbol(class_name, &field, true),
            ] {
                if signatures.contains_key(&member_symbol) {
                    return Err(format!(
                        "class `{class_name}` static field `{field}` collides with a static method or accessor"
                    ));
                }
            }
            let symbol = class_static_field_symbol(class_name, &field);
            let mut ty = match &property.type_ann {
                Some(annotation) => {
                    lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)?
                }
                None => property
                    .value
                    .as_deref()
                    .and_then(infer_class_field_literal_type)
                    .ok_or_else(|| {
                        format!(
                            "class `{class_name}` static field `{field}` needs a type annotation"
                        )
                    })?,
            };
            if property.is_optional {
                ty = optional_parameter_type(ty);
            }
            if property.value.is_none() && !matches!(ty, HirType::Optional(_) | HirType::Nullish(_))
            {
                return Err(format!(
                    "class `{class_name}` static field `{field}` without an initializer needs an optional or undefined-capable type"
                ));
            }
            if global_types.insert(symbol.clone(), ty).is_some() {
                return Err(format!(
                    "class `{class_name}` static field `{field}` conflicts with an existing generated binding"
                ));
            }
            if property.readonly {
                immutable_globals.insert(symbol);
            }
        }
        if class_has_decorators(&declaration.class)
            || value_referenced_classes.contains(class_name)
        {
            let token_symbol = class_decorator_token_symbol(class_name);
            global_types.insert(token_symbol.clone(), HirType::JsValue);
            immutable_globals.insert(token_symbol.clone());
            let HirType::Object(class_fields) = &interfaces[class_name] else {
                unreachable!("a native class always resolves to an Object layout")
            };
            let identity_marker = class_fields[0].0.clone();
            DECORATOR_CLASS_TOKENS.with(|tokens| {
                tokens.borrow_mut().insert(identity_marker, token_symbol);
            });
            if class_has_decorators(&declaration.class) {
                DECORATED_CLASS_NAMES.with(|names| {
                    names.borrow_mut().insert(class_name.to_string());
                });
            }
        }
    }

    // Inherited static fields share their declaring class's single storage.
    // Derived getters/setters provide the same dispatch surface as inherited
    // static accessors without copying the field into a second global.
    for derived in &class_decls {
        let derived_name = derived.ident.sym.as_ref();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::ClassProp(property) if property.is_static => {
                    class_property_name(&property.key).ok()
                }
                ClassMember::Method(method) if method.is_static => {
                    class_property_name(&method.key).ok()
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        let first_base = base_name.clone();
        while let Some(current_name) = base_name {
            // `Error`/`TypeError`/etc. are not real declared classes (see
            // `lower/module/classes.rs`'s synthetic base-layout branch) and
            // so have no members of their own to inherit here.
            let Some(&base) = class_by_name.get(&current_name) else {
                break;
            };
            for member in &base.class.body {
                let field = match member {
                    ClassMember::ClassProp(property) if property.is_static => {
                        class_property_name(&property.key)?
                    }
                    ClassMember::Method(method) if method.is_static => {
                        class_property_name(&method.key)?
                    }
                    _ => continue,
                };
                if !seen.insert(field.clone()) {
                    continue;
                }
                let ClassMember::ClassProp(property) = member else {
                    continue;
                };
                let storage = class_static_field_symbol(&current_name, &field);
                let ty = global_types[&storage].clone();
                let getter = class_getter_symbol(derived_name, &field, true);
                let source_range = (property.span.lo.0, property.span.hi.0);
                signatures.insert(
                    getter.clone(),
                    FnSignature {
                        public_overloads: Vec::new(),
                        params: Vec::new(),
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: ty.clone(),
                        is_async: false,
                        returns_sparse_array: false,
                        uses_this: false,
                        is_extern: false,
                        source_range,
                        source_length: None,
                        accessor_owner: None,
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_this_pattern: None,
                        generic_return_type: None,
                        generic_return_pattern: None,
                        source_omittable: HirOptionalMask::default(),
                        type_predicate: None,
                    },
                );
                // A write through a derived class creates that class's own
                // property (shadowing the base's); reads fall back to the
                // parent until then. Readonly fields can never be written.
                let own_storage = format!("__thaw_inherited_static_{derived_name}_{field}");
                let own_flag = format!("__thaw_inherited_static_own_{derived_name}_{field}");
                let parent_value = if first_base.as_deref() == Some(current_name.as_str()) {
                    HirExpr::Var(storage.clone())
                } else {
                    let parent = first_base.as_deref().expect("inherited field has a parent");
                    HirExpr::Call(
                        Box::new(HirExpr::Var(class_getter_symbol(parent, &field, true))),
                        Vec::new(),
                    )
                };
                let getter_body = if property.readonly {
                    vec![HirStmt::Return(Some(HirExpr::Var(storage.clone())))]
                } else {
                    inherited_static_globals.push(crate::HirGlobal {
                        name: own_storage.clone(),
                        ty: ty.clone(),
                        init: HirExpr::Var(storage.clone()),
                        mutable: true,
                    });
                    inherited_static_globals.push(crate::HirGlobal {
                        name: own_flag.clone(),
                        ty: HirType::Bool,
                        init: HirExpr::Lit(HirLit::Bool(false)),
                        mutable: true,
                    });
                    vec![
                        HirStmt::If(
                            HirExpr::Var(own_flag.clone()),
                            vec![HirStmt::Return(Some(HirExpr::Var(own_storage.clone())))],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(parent_value)),
                    ]
                };
                let getter_signature = signatures[&getter].clone();
                inherited_class_functions.push(HirFunction {
                    name: getter,
                    params: Vec::new(),
                    ret: ty.clone(),
                    is_async: false,
                    body: getter_body,
                });
                if !property.readonly {
                    let setter = class_setter_symbol(derived_name, &field, true);
                    signatures.insert(
                        setter.clone(),
                        FnSignature {
                        public_overloads: Vec::new(),
                            params: vec![ty.clone()],
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret: ty.clone(),
                            is_async: false,
                            returns_sparse_array: false,
                            uses_this: false,
                            is_extern: false,
                            source_range,
                            source_length: None,
                            accessor_owner: None,
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_this_pattern: None,
                            generic_return_type: None,
                            generic_return_pattern: None,
                            source_omittable: HirOptionalMask::default(),
                            type_predicate: None,
                        },
                    );
                    // `Object.hasOwn(Derived, field)` reads the shadow flag through this accessor.
                    let has_own = format!("__thaw_inherited_static_has_own_{derived_name}_{field}");
                    let mut has_own_signature = getter_signature;
                    has_own_signature.ret = HirType::Bool;
                    signatures.insert(has_own.clone(), has_own_signature);
                    inherited_class_functions.push(HirFunction {
                        name: has_own,
                        params: Vec::new(),
                        ret: HirType::Bool,
                        is_async: false,
                        body: vec![HirStmt::Return(Some(HirExpr::Var(
                            format!("__thaw_inherited_static_own_{derived_name}_{field}"),
                        )))],
                    });
                    let parameter = "__thaw_inherited_static_value".to_string();
                    inherited_class_functions.push(HirFunction {
                        name: setter,
                        params: vec![HirParam {
                            name: parameter.clone(),
                            ty: ty.clone(),
                        }],
                        ret: ty,
                        is_async: false,
                        body: vec![
                            HirStmt::Expr(HirExpr::Assign(
                                own_storage,
                                Box::new(HirExpr::Var(parameter.clone())),
                            )),
                            HirStmt::Expr(HirExpr::Assign(
                                own_flag,
                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                            )),
                            HirStmt::Return(Some(HirExpr::Var(parameter))),
                        ],
                    });
                }
            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    for (name, overloads) in public_overloads {
        let Some(signature) = signatures.get_mut(&name) else {
            return Err(format!("overload declarations for `{name}` have no implementation"));
        };
        signature.public_overloads = overloads;
    }

    // Missing parameter/return annotations and global initializer types are type
    // variables. Re-lower them together until forward references reach a fixed point.
    // signatures discovered in the previous round until forward calls and
    // mutually recursive functions reach a fixed point.
    //
    // The bound has to cover class methods too, not just free functions: this
    // loop's own class-method branch below only threads a converged return
    // type back into `signatures` once per *outer* iteration (`lower_class_
    // methods`'s own per-call lowering doesn't see a sibling method's return
    // type update until the next pass), so a chain of N unannotated-return
    // methods within a single class -- even with zero free functions or
    // globals at all -- genuinely needs N iterations to resolve, one hop at a
    // time. Confirmed empirically: a bare 2-method chain (`stepA(x) { return
    // this.stepB(x); } stepB(x) { return x + 1; }`, no free functions) failed
    // with a confusing internal error ("unbound `this` cannot synthesize
    // unreachable value of type Dynamic") before this fix, since the old
    // bound (`(0 + 0) * 2 + 1 == 1`) allowed only one iteration.
    let class_method_count: usize = class_decls
        .iter()
        .copied()
        .chain(inherited_virtual_class_decls.iter())
        .map(|declaration| {
            declaration
                .class
                .body
                .iter()
                .filter(|member| matches!(member, ClassMember::Method(_)))
                .count()
        })
        .sum();
    for _ in 0..=((fn_decls.len() + global_types.len() + class_method_count) * 2 + 1) {
        let mut changed = false;
        let call_constraints = RefCell::new(Vec::new());
        let globals = lower_global_decls(
            &global_decls,
            &global_types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
        )?;
        for global in globals {
            let ty = global_types.get_mut(&global.name).unwrap();
            if hir_type_contains_dynamic(ty) && *ty != global.ty {
                *ty = global.ty;
                changed = true;
            }
        }
        let _ = lower_top_level_initializers(
            module,
            &global_types,
            &immutable_globals,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
            &value_referenced_classes,
        )?;
        for fn_decl in &fn_decls {
            let name = fn_decl.ident.sym.to_string();
            // This whole loop is a *trial* pass whose only purpose is
            // converging `signatures` -- its own lowered output is
            // otherwise discarded (the real, final lowering happens
            // later, once every signature that can converge has). A
            // hard error here doesn't necessarily mean this function is
            // broken: it can just as easily mean one of *its own*
            // callees hasn't had its `Dynamic`-deferred return type
            // resolved yet (a strict, up-front check like `JSON.
            // stringify`'s own argument validation hard-errors on an
            // unresolved `Dynamic` instead of tolerating it -- see
            // `project_ecmascript_gaps_round14.md`). Previously this `?`
            // aborted the *entire* compile on the very first such
            // ordering mismatch, even though a later iteration -- after
            // the callee's own entry in `fn_decls`/the class loop below
            // runs -- would have resolved it. Skipping a failed attempt
            // for this iteration and retrying it next time is safe: the
            // final pass (below, outside this loop) re-lowers everything
            // for real once this loop reaches a fixed point, so a
            // genuinely unresolvable case (not just "hasn't converged
            // yet") still surfaces its real, honest error there instead
            // of a misleading one from a too-early attempt here.
            let Ok(function) = lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                Some(&call_constraints),
            ) else {
                continue;
            };
            if hir_type_contains_dynamic(&signatures[&name].ret)
                && function.ret != signatures[&name].ret
            {
                signatures.get_mut(&name).unwrap().ret = function.ret;
                changed = true;
            }
        }
        for declaration in class_decls
            .iter()
            .copied()
            .chain(inherited_virtual_class_decls.iter())
        {
            // Same tolerance as the free-function loop just above, and
            // for the identical reason -- a method's own body can
            // reference another not-yet-converged signature (its own
            // class's sibling method, a free function, ...) and hard-
            // error on a strict check instead of just seeing `Dynamic`.
            let Ok(methods) = lower_class_methods(
                declaration,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                true,
            ) else {
                continue;
            };
            for function in methods {
                let Some(signature) = signatures.get(&function.name) else {
                    continue;
                };
                if hir_type_contains_dynamic(&signature.ret) && function.ret != signature.ret {
                    signatures.get_mut(&function.name).unwrap().ret = function.ret;
                    changed = true;
                }
            }
        }
        for constraint in call_constraints.into_inner() {
            match constraint {
                CallConstraint::Generic(callee, types) => {
                    if types.contains(&HirType::Dynamic) {
                        continue;
                    }
                    if signatures
                        .get(&callee)
                        .is_some_and(|signature| signature.is_extern)
                    {
                        continue;
                    }
                    let instances = generic_instantiations.entry(callee).or_default();
                    if !instances.contains(&types) {
                        instances.push(types);
                    }
                }
                CallConstraint::Parameter(callee, index, actual, call_range) => {
                    if actual == HirType::Dynamic {
                        continue;
                    }
                    let is_extern = signatures[&callee].is_extern;
                    let preserves_json_boundary = is_extern
                        || callee.starts_with("__thaw_typed_wrapper_");
                    let param = &mut signatures.get_mut(&callee).unwrap().params[index];
                    if *param == HirType::Dynamic {
                        *param = actual;
                        changed = true;
                    } else if *param == HirType::Json
                        && actual == HirType::JsValue
                        && preserves_json_boundary
                    {
                        // `Json` is an explicit host-marshalling boundary, not
                        // an inference placeholder. The call lowering wraps a
                        // live value (including native callbacks) for it.
                    } else if *param == HirType::Json && actual == HirType::JsValue {
                        *param = HirType::JsValue;
                        changed = true;
                    } else if *param != actual {
                        return Err(format!(
                            "conflicting inferred types for parameter {} of `{callee}`: {param:?} and {actual:?} at bytes {}..{}",
                            index + 1, call_range.0, call_range.1,
                        ));
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // An explicit `any` parameter participates in ordinary call-site
    // inference so a live JS object keeps its handle. If it has no callers,
    // retain the old JSON fallback instead of reporting it as unannotated.
    for declaration in &fn_decls {
        let Some(signature) = signatures.get_mut(declaration.ident.sym.as_ref()) else {
            continue;
        };
        for (parameter, ty) in declaration.function.params.iter().zip(&mut signature.params) {
            if *ty == HirType::Dynamic
                && matches!(
                    &parameter.pat,
                    Pat::Ident(binding)
                        if binding.type_ann.as_ref().is_some_and(|annotation| matches!(
                            annotation.type_ann.as_ref(),
                            TsType::TsKeywordType(keyword)
                                if keyword.kind == TsKeywordTypeKind::TsAnyKeyword
                        ))
                )
            {
                *ty = HirType::Json;
            }
        }
    }
    if let Some((name, signature)) = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && signature.params.contains(&HirType::Dynamic)
    }) {
        let index = signature
            .params
            .iter()
            .position(|ty| *ty == HirType::Dynamic)
            .unwrap();
        // Same re-attribution as the return-type check just below --
        // a parameter can stay `Dynamic` here either because no call
        // site was ever *reachable* enough to constrain it, or because
        // some caller that would have constrained it never itself
        // finished lowering during the trial loop above (tolerated as
        // a possible ordering issue, same as any other failure there).
        // Retry the actual declaration for real before falling back to
        // this generic message.
        let owning_class = class_decls
            .iter()
            .copied()
            .chain(inherited_virtual_class_decls.iter())
            .find(|declaration| {
                name.starts_with(&format!("__thaw_class_{}_", declaration.ident.sym))
            });
        if let Some(declaration) = owning_class {
            lower_class_methods(
                declaration,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                false,
            )?;
        } else if let Some(fn_decl) = fn_decls
            .iter()
            .find(|fn_decl| fn_decl.ident.sym.as_ref() == name.as_str())
        {
            lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                None,
            )?;
        }
        return Err(format!(
            "cannot infer parameter {} of function `{name}` from its call sites; add an explicit type annotation at bytes {}..{}",
            index + 1,
            signature.source_range.0,
            signature.source_range.1,
        ));
    }
    let unresolved = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && hir_type_contains_dynamic(&signature.ret)
    });
    if let Some((name, signature)) = unresolved {
        // Reaching here means this signature's return type never
        // converged away from `Dynamic` during the trial loop above --
        // but that's equally true whether the real cause is "this
        // program never actually calls it in a way that reveals a
        // concrete type" (this generic message) or "its own body is
        // simply broken" (e.g. `function f() { if (x) return 1; return
        // "no"; }` -- `infer_return_type`'s own "incompatible types"
        // error). The trial loop tolerates *any* failure while
        // converging (an ordinary ordering issue looks identical to a
        // genuine bug from the loop's point of view), so before
        // reporting the generic fallback, retry the actual offending
        // declaration one more time without tolerating failure this
        // time -- if it has a real, specific problem, surface that
        // instead of this far vaguer message.
        let owning_class = class_decls
            .iter()
            .copied()
            .chain(inherited_virtual_class_decls.iter())
            .find(|declaration| {
                name.starts_with(&format!("__thaw_class_{}_", declaration.ident.sym))
            });
        if let Some(declaration) = owning_class {
            lower_class_methods(
                declaration,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                false,
            )?;
        } else if let Some(fn_decl) = fn_decls
            .iter()
            .find(|fn_decl| fn_decl.ident.sym.as_ref() == name.as_str())
        {
            lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                None,
            )?;
        }
        return Err(format!(
            "cannot infer the return type of function `{name}`; add an explicit return annotation at bytes {}..{}",
            signature.source_range.0,
            signature.source_range.1,
        ));
    }
    if let Some((name, _)) = global_types
        .iter()
        .find(|(_, ty)| hir_type_contains_dynamic(ty))
    {
        return Err(format!(
            "cannot infer the type of top-level binding `{name}`; add an explicit type annotation"
        ));
    }

    let mut globals = lower_global_decls(
        &global_decls,
        &global_types,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
    )?;
    globals.extend(lower_static_class_globals(
        &class_decls,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
    )?);
    globals.extend(inherited_static_globals);
    globals.extend(lower_class_decorator_tokens(
        &class_decls,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        &value_referenced_classes,
    )?);
    let initializers = lower_top_level_initializers(
        module,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
        &value_referenced_classes,
    )?;

    let extern_functions = signatures
        .iter()
        .filter(|(name, sig)| sig.is_extern && dynamic_symbol(name).is_none())
        .map(|(name, sig)| FfiSignature {
            symbol: name.clone(),
            params: sig.params.clone(),
            variadic: sig.variadic.clone(),
            variadic_abi: crate::FfiVariadicAbi::Native,
            ret: sig.ret.clone(),
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; sig.params.len()],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        })
        .collect();

    let mut specialized = Vec::new();
    for fn_decl in fn_decls {
        let function = lower_fn_decl(
            fn_decl,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            None,
        )?;
        if !signatures[&function.name].generic_type_params.is_empty() {
            continue;
        }
        specialized.push(function);
    }
    for declaration in &class_decls {
        specialized.extend(lower_class_constructor(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    for declaration in class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            false,
        )?);
    }
    for declaration in &inherited_virtual_class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            false,
        )?);
    }
    specialized.extend(inherited_class_functions);
    let mut pending = generic_instantiations
        .iter()
        .flat_map(|(name, instances)| instances.iter().cloned().map(|types| (name.clone(), types)))
        .collect::<Vec<_>>();
    let mut completed: Vec<(Symbol, Vec<HirType>)> = Vec::new();
    while let Some((name, types)) = pending.pop() {
        if completed
            .iter()
            .any(|done| done == &(name.clone(), types.clone()))
        {
            continue;
        }
        let decl = module
            .body
            .iter()
            .find_map(|item| match item {
                ModuleItem::Stmt(Stmt::Decl(Decl::Fn(decl))) if decl.ident.sym.as_ref() == name => {
                    Some(decl)
                }
                _ => None,
            })
            .ok_or_else(|| format!("missing declaration for `{name}`"))?;
        let nested_constraints = RefCell::new(Vec::new());
        let instance = lower_generic_instance(
            decl,
            &types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            Some(&nested_constraints),
        )?;
        completed.push((name, types));
        specialized.push(instance);
        for constraint in nested_constraints.into_inner() {
            if let CallConstraint::Generic(nested_name, nested_types) = constraint {
                if !nested_types.contains(&HirType::Dynamic) {
                    pending.push((nested_name, nested_types));
                }
            }
        }
    }

    Ok(HirProgram {
        globals,
        initializers,
        functions: specialized,
        extern_functions,
    })
}

/// Whether `module` contains a `delete <expr>.stack`.
fn module_deletes_stack(module: &Module) -> bool {
    struct Finder(bool);
    impl Visit for Finder {
        fn visit_unary_expr(&mut self, unary: &swc_ecma_ast::UnaryExpr) {
            if unary.op == swc_ecma_ast::UnaryOp::Delete {
                let mut target = unary.arg.as_ref();
                while let Expr::Paren(inner) = target {
                    target = inner.expr.as_ref();
                }
                if let Expr::Member(member) = target {
                    self.0 |= matches!(&member.prop, MemberProp::Ident(name) if name.sym == *"stack")
                        || matches!(&member.prop, MemberProp::Computed(computed)
                            if matches!(computed.expr.as_ref(), Expr::Lit(swc_ecma_ast::Lit::Str(text)) if text.value == *"stack"));
                }
            }
            unary.visit_children_with(self);
        }
    }
    let mut finder = Finder(false);
    module.visit_with(&mut finder);
    finder.0
}
