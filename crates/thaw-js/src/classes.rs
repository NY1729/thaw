//! Classes (ES2015 semantics on top of the ES5 object model), `super`, and optional chaining.

use crate::*;

/// Per-class data kept on the constructor's `FuncDef`.
pub(crate) struct ClassInfo {
    pub derived: bool,
    pub parent: Option<Value>,
    pub explicit_ctor: bool,
    /// instance field initializers, evaluated per construction with `this` bound
    pub fields: Vec<(Rc<str>, Option<a::Expr>)>,
    pub field_scope: Rc<Scope>,
    /// instance private methods/accessors, installed on each instance before its fields (filled while the class evaluates)
    pub private_methods: Box<RefCell<Vec<PrivMethod>>>,
}

/// One instance private method (`value`) or accessor pair (`get`/`set`) under its per-evaluation key.
#[derive(Clone)]
pub(crate) struct PrivMethod {
    pub key: Rc<str>,
    pub value: Option<Value>,
    pub get: Option<Value>,
    pub set: Option<Value>,
}

static NEXT_CLASS_EVAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Interp {
    pub(crate) fn eval_class(
        &mut self,
        ident: Option<&a::Ident>,
        class: &a::Class,
        scope: &Rc<Scope>,
    ) -> R<Value> {
        let parent = match &class.super_class {
            Some(e) => Some(self.eval(e, scope)?),
            None => None,
        };
        let cscope = Scope::new(Some(scope.clone()));
        cscope.strict.set(true);
        // every evaluation of a class gets its own private names; references resolve through `%pk:<name>`
        let eval_id = NEXT_CLASS_EVAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut private_keys: HashMap<String, Rc<str>> = HashMap::new();
        for m in &class.body {
            let name = match m {
                a::ClassMember::PrivateProp(p) => &p.key.name,
                a::ClassMember::PrivateMethod(pm) => &pm.key.name,
                _ => continue,
            };
            private_keys.entry(name.to_string()).or_insert_with(|| {
                let key: Rc<str> = format!(
                    "@@priv:{}{}e{}",
                    name,
                    crate::private_names::SUFFIX_MARK,
                    eval_id
                )
                .into();
                cscope.declare(&format!("%pk:{name}"), Value::str(&key), false);
                key
            });
        }
        let proto_parent = match &parent {
            None => Some(self.object_proto.clone()),
            Some(Value::Null) => None,
            Some(p) if p.is_callable() => match self.get(p, "prototype")? {
                Value::Object(pp) => Some(pp),
                _ => None,
            },
            Some(_) => {
                return self.throw_error(
                    "TypeError",
                    "Class extends value is not a constructor or null",
                )
            }
        };
        let proto = new_obj(proto_parent, Kind::Plain);
        let name: Option<Rc<str>> = ident.map(|i| i.sym.as_ref().into());

        // constructor + member collection
        let mut ctor_member: Option<&a::Constructor> = None;
        for m in &class.body {
            if let a::ClassMember::Constructor(c) = m {
                ctor_member = Some(c);
            }
        }
        let proto_scope = Scope::new(Some(cscope.clone()));
        proto_scope.declare("%home", Value::Object(proto.clone()), false);
        let mut fields: Vec<(Rc<str>, Option<a::Expr>)> = Vec::new();
        for m in &class.body {
            match m {
                a::ClassMember::ClassProp(p) if !p.is_static => {
                    let k = self.prop_name(&p.key, &proto_scope)?;
                    fields.push((k, p.value.as_ref().map(|v| (**v).clone())));
                }
                a::ClassMember::PrivateProp(p) if !p.is_static => {
                    let k: Rc<str> = private_keys[&*p.key.name].clone();
                    fields.push((k, p.value.as_ref().map(|v| (**v).clone())));
                }
                _ => {}
            }
        }
        let info = ClassInfo {
            derived: parent.is_some(),
            parent: parent.clone(),
            explicit_ctor: ctor_member.is_some(),
            fields,
            field_scope: proto_scope.clone(),
            private_methods: Box::default(),
        };
        let (params, body) = match ctor_member {
            Some(c) => (
                c.params
                    .iter()
                    .map(|p| match p {
                        a::ParamOrTsParamProp::Param(p) => Ok(p.pat.clone()),
                        _ => unsupported("typescript parameter property"),
                    })
                    .collect::<R<Vec<_>>>()?,
                c.body.as_ref().map(|b| b.stmts.clone()).unwrap_or_default(),
            ),
            None => (Vec::new(), Vec::new()),
        };
        // the constructor's own `arguments` (body or parameter defaults), not the enclosing function's
        let (ctor_uses_arguments, ctor_uses_new_target) = {
            let mut finder = ArgumentsFinder::default();
            body.visit_with(&mut finder);
            params.visit_with(&mut finder);
            (finder.found, finder.new_target)
        };
        let hoisted = hoisted_names(&body);
        let param_names = simple_param_names(&params);
        let param_mask = param_mask(&param_names);
        let def = Rc::new(FuncDef {
            name: name.clone(),
            params,
            body: Body::Block(body),
            arrow: false,
            is_async: false,
            is_gen: false,
            bind_self: false,
            no_prototype: false,
            uses_arguments: ctor_uses_arguments,
            uses_new_target: ctor_uses_new_target || parent.is_some(),
            uses_this: true,
            no_compile: self.no_compile_define,
            src: (self.run_id, class.span.lo.0, class.span.hi.0),
            hoisted,
            param_names,
            param_mask,
            strict: true,
            class: Some(info),
            cstate: Default::default(),
            ctor_props: std::cell::Cell::new(0),
        });
        crate::gc_registry::capture_scope(&proto_scope);
        let class_def = def.clone();
        let ctor_obj = new_obj(
            Some(self.function_proto.clone()),
            Kind::Closure {
                def,
                scope: proto_scope.clone(),
            },
        );
        let ctor = Value::Object(ctor_obj.clone());
        if let Some(Value::Object(p)) = &parent {
            if p.proto.borrow().is_some() {
                *ctor_obj.proto.borrow_mut() = Some(p.clone()); // static inheritance
            }
        }
        // Node: a class's `prototype` is non-writable for user and built-in classes alike.
        put_with(
            &ctor_obj,
            "prototype",
            Value::Object(proto.clone()),
            false,
            false,
            false,
        );
        put_hidden(&proto, "constructor", ctor.clone());
        if let Some(n) = &name {
            cscope.declare(n, ctor.clone(), false);
        }
        cscope.declare("%class", ctor.clone(), false);
        let static_scope = Scope::new(Some(cscope.clone()));
        static_scope.declare("%home", ctor.clone(), false);
        static_scope.set_this(ctor.clone());
        static_scope.declare("%class", ctor.clone(), false);

        for m in &class.body {
            match m {
                a::ClassMember::Method(cm) => {
                    let target = if cm.is_static { &ctor_obj } else { &proto };
                    let mscope = if cm.is_static {
                        static_scope.clone()
                    } else {
                        proto_scope.clone()
                    };
                    let key = self.prop_name(&cm.key, &mscope)?;
                    let fname = self.fn_name_of_key(&key);
                    let shown: Rc<str> = match cm.kind {
                        a::MethodKind::Getter => format!("get {fname}").into(),
                        a::MethodKind::Setter => format!("set {fname}").into(),
                        a::MethodKind::Method => fname,
                    };
                    let f = self.method_value(Some(shown), &cm.function, &mscope)?;
                    match cm.kind {
                        a::MethodKind::Method => put_hidden(target, &key, f),
                        a::MethodKind::Getter => {
                            define_accessor(target, &key, Some(f), None, false)
                        }
                        a::MethodKind::Setter => {
                            define_accessor(target, &key, None, Some(f), false)
                        }
                    }
                }
                a::ClassMember::ClassProp(p) if p.is_static => {
                    let key = self.prop_name(&p.key, &static_scope)?;
                    let v = match &p.value {
                        Some(e) => {
                            let v = self.eval(e, &static_scope)?;
                            if crate::is_anonymous_fn_def(e) {
                                crate::name_anonymous_fn(&v, &self.fn_name_of_key(&key));
                            }
                            v
                        }
                        None => Value::Undefined,
                    };
                    put(&ctor_obj, &key, v);
                }
                a::ClassMember::PrivateMethod(pm) => {
                    let mscope = if pm.is_static {
                        static_scope.clone()
                    } else {
                        proto_scope.clone()
                    };
                    let key: Rc<str> = private_keys[&*pm.key.name].clone();
                    let shown: Rc<str> = match pm.kind {
                        a::MethodKind::Getter => {
                            format!("get #{}", crate::private_names::source_name(&pm.key.name))
                                .into()
                        }
                        a::MethodKind::Setter => {
                            format!("set #{}", crate::private_names::source_name(&pm.key.name))
                                .into()
                        }
                        a::MethodKind::Method => {
                            format!("#{}", crate::private_names::source_name(&pm.key.name)).into()
                        }
                    };
                    let f = self.method_value(Some(shown), &pm.function, &mscope)?;
                    let (value, get, set) = match pm.kind {
                        a::MethodKind::Method => (Some(f), None, None),
                        a::MethodKind::Getter => (None, Some(f), None),
                        a::MethodKind::Setter => (None, None, Some(f)),
                    };
                    if pm.is_static {
                        install_private_method(&ctor_obj, &key, value, get, set);
                    } else if let Some(info) = &class_def.class {
                        let mut list = info.private_methods.borrow_mut();
                        match list.iter_mut().find(|m| m.key == key) {
                            Some(m) => {
                                m.get = m.get.take().or(get);
                                m.set = m.set.take().or(set);
                            }
                            None => list.push(PrivMethod {
                                key,
                                value,
                                get,
                                set,
                            }),
                        }
                    }
                }
                a::ClassMember::PrivateProp(p) if p.is_static => {
                    let key: Rc<str> = private_keys[&*p.key.name].clone();
                    let v = match &p.value {
                        Some(e) => {
                            let v = self.eval(e, &static_scope)?;
                            if crate::is_anonymous_fn_def(e) {
                                let name =
                                    format!("#{}", crate::private_names::source_name(&p.key.name));
                                crate::name_anonymous_fn(&v, &name);
                            }
                            v
                        }
                        None => Value::Undefined,
                    };
                    self.private_add(&ctor_obj, &key, v)?;
                }
                a::ClassMember::StaticBlock(block) => {
                    let inner = Scope::new(Some(static_scope.clone()));
                    hoist_vars(&block.body.stmts, &inner);
                    self.exec_stmts(&block.body.stmts, &inner)?;
                }
                a::ClassMember::AutoAccessor(_) => return unsupported("auto-accessor"),
                _ => {}
            }
        }
        Ok(ctor)
    }

    /// `Object`-like natives that behave as plain base constructors (safe to extend).
    pub(crate) fn run_fields(&mut self, class: &Value, this: &Value) -> R<()> {
        let Value::Object(co) = class else {
            return Ok(());
        };
        let (fields, scope, methods) = match &*co.kind.borrow() {
            Kind::Closure { def, .. } => match &def.class {
                Some(info)
                    if !info.fields.is_empty() || !info.private_methods.borrow().is_empty() =>
                {
                    (
                        info.fields
                            .iter()
                            .map(|(k, e)| (k.clone(), e.clone()))
                            .collect::<Vec<_>>(),
                        info.field_scope.clone(),
                        info.private_methods.borrow().clone(),
                    )
                }
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        if let Value::Object(target) = this {
            for m in &methods {
                if target.position(&m.key).is_some() {
                    return self.private_twice(&m.key);
                }
                install_private_method(
                    target,
                    &m.key,
                    m.value.clone(),
                    m.get.clone(),
                    m.set.clone(),
                );
            }
        }
        let frame = Scope::new(Some(scope));
        frame.kind.set(SK_VAR | SK_FIELD);
        frame.set_this(this.clone());
        for (key, init) in fields {
            let v = match init {
                Some(e) => {
                    let v = self.eval(&e, &frame)?;
                    if crate::is_anonymous_fn_def(&e) {
                        let name = match key.strip_prefix("@@priv:") {
                            Some(n) => format!("#{}", crate::private_names::source_name(n)).into(),
                            None => self.fn_name_of_key(&key),
                        };
                        crate::name_anonymous_fn(&v, &name);
                    }
                    v
                }
                None => Value::Undefined,
            };
            let Value::Object(target) = this else {
                return Ok(());
            };
            if key.starts_with("@@priv:") {
                self.private_add(target, &key, v)?;
            } else {
                put(target, &key, v);
            }
        }
        Ok(())
    }

    /// Run `ctor` as the constructor of the already-allocated `this` (class or plain function).
    pub(crate) fn init_instance(&mut self, ctor: &Value, this: &Value, args: &[Value]) -> R<Value> {
        let Value::Object(co) = ctor else {
            return self.throw_error("TypeError", "Super constructor is not a constructor");
        };
        let class_parts = match &*co.kind.borrow() {
            Kind::Closure { def, scope } if def.class.is_some() => {
                Some((def.clone(), scope.clone()))
            }
            _ => None,
        };
        let nt = self.rt.pending_nt.take();
        // a Proxy parent is constructed with the derived newTarget, its result landing on the pre-allocated `this`
        if let Some(po) = match ctor {
            Value::Object(po) if matches!(&*po.kind.borrow(), Kind::Proxy(p) if p.callable) => {
                Some(po.clone())
            }
            _ => None,
        } {
            let nt = match nt {
                Some(v @ Value::Object(_)) => v,
                _ => ctor.clone(),
            };
            let (target, handler, trap) = self
                .proxy_trap(&po, "construct")?
                .expect("init_instance on a non-proxy");
            if !trap.is_callable() {
                self.rt.pending_nt = Some(nt);
                return self.init_instance(&target, this, args);
            }
            let arr = self.array(args.to_vec());
            let r = self.call(&trap, &handler, &[target, arr, nt])?;
            let (Value::Object(t), Value::Object(n)) = (this, &r) else {
                return self.throw_error("TypeError", "proxy [[Construct]] must return an object");
            };
            if !Rc::ptr_eq(t, n) {
                // a lazy `prototype` on `n` must be materialised before its table moves to `t`
                n.force_lazy();
                // the trap built a different object: `this` takes over its contents (the derived
                // constructor keeps the pre-allocated identity). A proxy result's prototype comes from
                // its getPrototypeOf trap, which must be asked before `n` loses its kind.
                let proto = match self.proxy_get_prototype_of(n)? {
                    Some(Value::Object(p)) => Some(p),
                    Some(_) => None,
                    None => n.proto.borrow_mut().take(),
                };
                *t.kind.borrow_mut() = std::mem::replace(&mut *n.kind.borrow_mut(), Kind::Plain);
                *t.proto.borrow_mut() = proto;
                *t.raw_props.borrow_mut() = std::mem::take(&mut *n.raw_props.borrow_mut());
                *t.raw_props.index.borrow_mut() = n.raw_props.index.borrow_mut().take();
                t.ext.set(n.ext.get());
                t.frozen.set(n.frozen.get());
                t.len_ro.set(n.len_ro.get());
            }
            return Ok(this.clone());
        }
        let Some((def, scope)) = class_parts else {
            self.rt.pending_nt = nt;
            let r = self.call(ctor, this, args);
            self.rt.pending_nt = None;
            let r = r?;
            // a native base constructor (Set, Map, Array, Promise, ...) builds its own object:
            // move its internal state into the pre-allocated `this`, keeping `this` identity and prototype
            if let (Value::Object(t), Value::Object(n)) = (this, &r) {
                // a Date base keeps its time in a hidden slot on a plain object (builtins.rs): move that too
                if let (false, Some((Slot::Data(time), false))) =
                    (Rc::ptr_eq(t, n), find_own(n, "__time"))
                {
                    put_hidden(t, "__time", time);
                    return Ok(this.clone());
                }
                if !Rc::ptr_eq(t, n)
                    && matches!(&*co.kind.borrow(), Kind::Native(_))
                    && matches!(&*t.kind.borrow(), Kind::Plain)
                    && !matches!(&*n.kind.borrow(), Kind::Plain | Kind::Global)
                {
                    let k = std::mem::replace(&mut *n.kind.borrow_mut(), Kind::Plain);
                    *t.kind.borrow_mut() = k;
                    return Ok(this.clone());
                }
            }
            return Ok(r);
        };
        let info = def.class.as_ref().unwrap();
        if self.too_deep() {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.depth += 1;
        let mark = self.frame_mark();
        self.call_frames.push((def.clone(), true));
        let result = (|| {
            if !info.derived {
                self.run_fields(ctor, this)?;
                self.rt.pending_nt = nt.clone();
                self.call_closure(ctor, &def, &scope, this, args)
            } else if info.explicit_ctor {
                self.rt.pending_nt = nt.clone();
                self.call_closure(ctor, &def, &scope, this, args)
            } else {
                let parent = info.parent.clone().unwrap_or(Value::Undefined);
                let mut over = None;
                if !matches!(parent, Value::Null) {
                    self.rt.pending_nt = nt.clone();
                    // a base constructor that returns another object makes that object `this`
                    let r = self.init_instance(&parent, this, args)?;
                    if !is_native_fn(&parent)
                        && matches!((&r, this), (Value::Object(o), Value::Object(t)) if !Rc::ptr_eq(o, t))
                    {
                        over = Some(r);
                    }
                }
                self.run_fields(ctor, over.as_ref().unwrap_or(this))?;
                Ok(over.unwrap_or(Value::Undefined))
            }
        })();
        self.frame_truncate(mark);
        self.depth -= 1;
        result
    }

    /// `super(...args)` inside a derived constructor.
    pub(crate) fn super_call(&mut self, args: &[Value], scope: &Rc<Scope>) -> R<Value> {
        let class = self.lookup(scope, "%class").unwrap_or(Value::Undefined);
        let this = self.this_of(scope);
        let parent = match &class {
            Value::Object(co) => match &*co.kind.borrow() {
                Kind::Closure { def, .. } => def.class.as_ref().and_then(|i| i.parent.clone()),
                _ => None,
            },
            _ => None,
        };
        let Some(parent) = parent else {
            return self.throw_error("SyntaxError", "'super' keyword unexpected here");
        };
        self.rt.pending_nt = self.lookup(scope, "%newtarget");
        let r = self.init_instance(&parent, &this, args);
        self.rt.pending_nt = None;
        // a base constructor that returns another object makes that object `this` (and gets the fields)
        let this = match r? {
            over @ Value::Object(_)
                if !is_native_fn(&parent)
                    && !matches!((&over, &this), (Value::Object(o), Value::Object(t)) if Rc::ptr_eq(o, t)) =>
            {
                scope.rebind_this(over.clone());
                over
            }
            _ => this,
        };
        self.run_fields(&class, &this)?;
        Ok(Value::Undefined)
    }

    /// The object `super.x` looks up on: the home object's prototype.
    pub(crate) fn super_base(&mut self, scope: &Rc<Scope>) -> R<Value> {
        match self.lookup(scope, "%home") {
            Some(Value::Object(h)) => {
                Ok(h.proto.borrow().clone().map_or(Value::Null, Value::Object))
            }
            _ => self.throw_error("SyntaxError", "'super' keyword unexpected here"),
        }
    }

    /// `super[key]` once the base and the key are known: accessors on the base run with the current `this`.
    pub(crate) fn super_read(&mut self, base: &Value, key: &str, scope: &Rc<Scope>) -> R<Value> {
        let this = self.this_of(scope);
        match base {
            Value::Object(o) => match find_in_chain(o, key) {
                Some(Slot::Data(v)) => Ok(v),
                Some(Slot::Accessor { get: Some(g), .. }) => self.call(&g, &this, &[]),
                _ => Ok(Value::Undefined),
            },
            _ => Ok(Value::Undefined),
        }
    }

    /// Evaluate a chain containing `?.`; `None` means the chain short-circuited.
    pub(crate) fn eval_opt(&mut self, oc: &a::OptChainExpr, scope: &Rc<Scope>) -> R<Option<Value>> {
        match &*oc.base {
            a::OptChainBase::Member(m) => {
                let Some(obj) = self.eval_chain_part(&m.obj, scope)? else {
                    return Ok(None);
                };
                if oc.optional && matches!(obj, Value::Undefined | Value::Null) {
                    return Ok(None);
                }
                let key = self.member_key(&m.prop, scope)?;
                Ok(Some(self.get(&obj, &key)?))
            }
            a::OptChainBase::Call(c) => {
                let (f, this) = match &*c.callee {
                    a::Expr::Member(m) => {
                        let Some(obj) = self.eval_chain_part(&m.obj, scope)? else {
                            return Ok(None);
                        };
                        let key = self.member_key(&m.prop, scope)?;
                        let f = if matches!(obj, Value::Undefined | Value::Null) && oc.optional {
                            Value::Undefined
                        } else {
                            self.get(&obj, &key)?
                        };
                        (f, obj)
                    }
                    // `a.b?.c(...)`: the callee is itself an optional member access; keep `a.b` as `this`
                    a::Expr::OptChain(inner)
                        if matches!(&*inner.base, a::OptChainBase::Member(_)) =>
                    {
                        let a::OptChainBase::Member(m) = &*inner.base else {
                            unreachable!()
                        };
                        let Some(obj) = self.eval_chain_part(&m.obj, scope)? else {
                            return Ok(None);
                        };
                        if inner.optional && matches!(obj, Value::Undefined | Value::Null) {
                            return Ok(None);
                        }
                        let key = self.member_key(&m.prop, scope)?;
                        (self.get(&obj, &key)?, obj)
                    }
                    other => {
                        let Some(f) = self.eval_chain_part(other, scope)? else {
                            return Ok(None);
                        };
                        (f, Value::Undefined)
                    }
                };
                if oc.optional && matches!(f, Value::Undefined | Value::Null) {
                    return Ok(None);
                }
                let args = self.eval_args(&c.args, scope)?;
                if !f.is_callable() {
                    return self.throw_error(
                        "TypeError",
                        &format!("{} is not a function", expr_text(&c.callee)),
                    );
                }
                Ok(Some(self.call(&f, &this, &args)?))
            }
        }
    }

    fn eval_chain_part(&mut self, e: &a::Expr, scope: &Rc<Scope>) -> R<Option<Value>> {
        match e {
            a::Expr::OptChain(oc) => self.eval_opt(oc, scope),
            other => Ok(Some(self.eval(other, scope)?)),
        }
    }
}

/// A private method is a non-writable own slot; an accessor pair is an own accessor.
fn install_private_method(
    o: &Obj,
    key: &str,
    value: Option<Value>,
    get: Option<Value>,
    set: Option<Value>,
) {
    match value {
        Some(f) => put_with(o, key, f, false, false, true),
        None => define_accessor(o, key, get, set, false),
    }
}

impl Interp {
    fn private_display(key: &str) -> String {
        let name = key.strip_prefix("@@priv:").unwrap_or(key);
        format!("#{}", crate::private_names::source_name(name))
    }

    pub(crate) fn private_twice<T>(&mut self, key: &str) -> R<T> {
        let msg = format!(
            "Cannot initialize {} twice on the same object",
            Self::private_display(key)
        );
        self.throw_error("TypeError", &msg)
    }

    /// Adds private field `key` to `o`; an object that already has it (a constructor returned it twice) throws.
    pub(crate) fn private_add(&mut self, o: &Obj, key: &str, v: Value) -> R<()> {
        if o.position(key).is_some() {
            return self.private_twice(key);
        }
        put_with(o, key, v, true, false, true);
        Ok(())
    }

    /// `o.#x` read: an own slot only (a prototype's or a foreign object's does not count).
    pub(crate) fn private_get(&mut self, target: &Value, key: &str) -> R<Value> {
        let own = match target {
            Value::Object(o) => find_own(o, key),
            _ => None,
        };
        match own {
            Some((Slot::Data(v), _)) => Ok(v),
            Some((Slot::Accessor { get: Some(g), .. }, _)) => self.call(&g, target, &[]),
            Some(_) => {
                let msg = format!(
                    "'{}' was defined without a getter",
                    Self::private_display(key)
                );
                self.throw_error("TypeError", &msg)
            }
            None => {
                let msg = format!(
                    "Cannot read private member {} from an object whose class did not declare it",
                    Self::private_display(key)
                );
                self.throw_error("TypeError", &msg)
            }
        }
    }

    /// `o.#x = v`: writes an own writable slot or calls the setter; anything else is a TypeError.
    pub(crate) fn private_set(&mut self, target: &Value, key: &str, value: Value) -> R<()> {
        let found = match target {
            Value::Object(o) => o.position(key).map(|i| {
                let props = o.raw_props.borrow();
                (o.clone(), i, props[i].slot.clone(), props[i].writable)
            }),
            _ => None,
        };
        match found {
            Some((o, i, Slot::Data(_), true)) => {
                o.raw_props.borrow_mut()[i].slot = Slot::Data(value);
                Ok(())
            }
            Some((_, _, Slot::Data(_), false)) => {
                let msg = format!(
                    "Private method '{}' is not writable",
                    Self::private_display(key)
                );
                self.throw_error("TypeError", &msg)
            }
            Some((_, _, Slot::Accessor { set: Some(s), .. }, _)) => {
                self.call(&s, target, &[value])?;
                Ok(())
            }
            Some(_) => {
                let msg = format!(
                    "'{}' was defined without a setter",
                    Self::private_display(key)
                );
                self.throw_error("TypeError", &msg)
            }
            None => {
                let msg = format!(
                    "Cannot write private member {} to an object whose class did not declare it",
                    Self::private_display(key)
                );
                self.throw_error("TypeError", &msg)
            }
        }
    }

    /// The per-evaluation key of private name `name`, resolved through the lexical class scopes.
    pub(crate) fn private_key(&self, name: &str, scope: &Rc<Scope>) -> Rc<str> {
        match self.lookup(scope, &format!("%pk:{name}")) {
            Some(Value::Str(k)) => k.to_string().into(),
            _ => format!("@@priv:{name}").into(),
        }
    }
}

/// A native constructor builds its own object, which `init_instance` already moved into `this`.
fn is_native_fn(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Native(_)))
}
