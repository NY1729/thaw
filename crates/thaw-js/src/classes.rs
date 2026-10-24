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
}

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
        let proto_parent = match &parent {
            None => Some(self.object_proto.clone()),
            Some(Value::Null) => None,
            Some(p) if p.is_callable() => {
                if matches!(p, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Native(_)) && !self.is_plain_ctor(p))
                {
                    return unsupported("extends-native");
                }
                match self.get(p, "prototype")? {
                    Value::Object(pp) => Some(pp),
                    _ => None,
                }
            }
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
            if let a::ClassMember::ClassProp(p) = m {
                if !p.is_static {
                    let k = self.prop_name(&p.key, &proto_scope)?;
                    fields.push((k, p.value.as_ref().map(|v| (**v).clone())));
                }
            }
        }
        let info = ClassInfo {
            derived: parent.is_some(),
            parent: parent.clone(),
            explicit_ctor: ctor_member.is_some(),
            fields,
            field_scope: proto_scope.clone(),
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
        let def = Rc::new(FuncDef {
            name: name.clone(),
            params,
            body: Body::Block(body),
            arrow: false,
            uses_arguments: false,
            strict: true,
            class: Some(info),
        });
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
        put_hidden(&ctor_obj, "prototype", Value::Object(proto.clone()));
        put_hidden(&proto, "constructor", ctor.clone());
        if let Some(n) = &name {
            cscope.declare(n, ctor.clone(), false);
        }
        cscope.declare("%class", ctor.clone(), false);
        let static_scope = Scope::new(Some(cscope.clone()));
        static_scope.declare("%home", ctor.clone(), false);
        static_scope.declare("this", ctor.clone(), false);
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
                    let f = self.function_value(Some(key.clone()), &cm.function, &mscope)?;
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
                        Some(e) => self.eval(e, &static_scope)?,
                        None => Value::Undefined,
                    };
                    put(&ctor_obj, &key, v);
                }
                a::ClassMember::PrivateMethod(_) | a::ClassMember::PrivateProp(_) => {
                    return unsupported("private-name")
                }
                a::ClassMember::StaticBlock(_) => return unsupported("static-block"),
                a::ClassMember::AutoAccessor(_) => return unsupported("auto-accessor"),
                _ => {}
            }
        }
        Ok(ctor)
    }

    /// `Object`-like natives that behave as plain base constructors (safe to extend).
    fn is_plain_ctor(&self, v: &Value) -> bool {
        matches!(self.lookup(&self.global, "Object"), Some(o) if strict_equals(&o, v))
    }

    pub(crate) fn run_fields(&mut self, class: &Value, this: &Value) -> R<()> {
        let Value::Object(co) = class else {
            return Ok(());
        };
        let (fields, scope) = match &*co.kind.borrow() {
            Kind::Closure { def, .. } => match &def.class {
                Some(info) if !info.fields.is_empty() => (
                    info.fields
                        .iter()
                        .map(|(k, e)| (k.clone(), e.clone()))
                        .collect::<Vec<_>>(),
                    info.field_scope.clone(),
                ),
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        let frame = Scope::new(Some(scope));
        frame.declare("this", this.clone(), false);
        for (key, init) in fields {
            let v = match init {
                Some(e) => self.eval(&e, &frame)?,
                None => Value::Undefined,
            };
            put(
                match this {
                    Value::Object(o) => o,
                    _ => return Ok(()),
                },
                &key,
                v,
            );
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
        let Some((def, scope)) = class_parts else {
            return self.call(ctor, this, args);
        };
        let info = def.class.as_ref().unwrap();
        if self.depth >= MAX_DEPTH {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.depth += 1;
        let result = (|| {
            if !info.derived {
                self.run_fields(ctor, this)?;
                self.call_closure(ctor, &def, &scope, this, args)
            } else if info.explicit_ctor {
                self.call_closure(ctor, &def, &scope, this, args)
            } else {
                let parent = info.parent.clone().unwrap_or(Value::Undefined);
                if !matches!(parent, Value::Null) {
                    self.init_instance(&parent, this, args)?;
                }
                self.run_fields(ctor, this)?;
                Ok(Value::Undefined)
            }
        })();
        self.depth -= 1;
        result
    }

    /// `super(...args)` inside a derived constructor.
    pub(crate) fn super_call(&mut self, args: &[Value], scope: &Rc<Scope>) -> R<Value> {
        let class = self.lookup(scope, "%class").unwrap_or(Value::Undefined);
        let this = self.lookup(scope, "this").unwrap_or(Value::Undefined);
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
        self.init_instance(&parent, &this, args)?;
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
