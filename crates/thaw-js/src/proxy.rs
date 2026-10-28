//! `Proxy` / `Proxy.revocable`: get, set, has, deleteProperty, ownKeys, apply, construct,
//! defineProperty and getOwnPropertyDescriptor traps; every other operation reaches the target.

use crate::*;

pub(crate) struct ProxyData {
    pub(crate) target: Value,
    pub(crate) handler: RefCell<Value>,
    pub(crate) callable: bool,
}

impl Interp {
    fn proxy_data(o: &Obj) -> Option<Rc<ProxyData>> {
        match &*o.kind.borrow() {
            Kind::Proxy(p) => Some(p.clone()),
            _ => None,
        }
    }

    pub(crate) fn key_value(&self, key: &str) -> Value {
        if let Some(id) = key
            .strip_prefix("@@sym:")
            .and_then(|i| i.parse::<u64>().ok())
        {
            if let Some(s) = self.rt.symbols.get(&id) {
                return Value::Object(s.clone());
            }
        }
        Value::Str(key.into())
    }

    /// `(target, handler, trap)` for a proxy; `trap` is `Undefined` when the handler lacks it.
    pub(crate) fn proxy_trap(&mut self, o: &Obj, name: &str) -> R<Option<(Value, Value, Value)>> {
        let Some(p) = Self::proxy_data(o) else {
            return Ok(None);
        };
        let handler = p.handler.borrow().clone();
        if matches!(handler, Value::Null) {
            return self.throw_error(
                "TypeError",
                &format!("Cannot perform '{name}' on a proxy that has been revoked"),
            );
        }
        let trap = self.get(&handler, name)?;
        Ok(Some((p.target.clone(), handler, trap)))
    }

    pub(crate) fn proxy_get(&mut self, o: &Obj, key: &str, receiver: &Value) -> R<Option<Value>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "get")? else {
            return Ok(None);
        };
        if trap.is_callable() {
            let k = self.key_value(key);
            return Ok(Some(self.call(
                &trap,
                &handler,
                &[target, k, receiver.clone()],
            )?));
        }
        Ok(Some(self.get(&target, key)?))
    }

    pub(crate) fn proxy_set(
        &mut self,
        o: &Obj,
        key: &str,
        value: &Value,
        receiver: &Value,
    ) -> R<bool> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "set")? else {
            return Ok(false);
        };
        if trap.is_callable() {
            let k = self.key_value(key);
            self.call(
                &trap,
                &handler,
                &[target, k, value.clone(), receiver.clone()],
            )?;
        } else {
            self.set(&target, key, value.clone())?;
        }
        Ok(true)
    }

    pub(crate) fn proxy_has(&mut self, o: &Obj, key: &str) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "has")? else {
            return Ok(None);
        };
        if trap.is_callable() {
            let k = self.key_value(key);
            return Ok(Some(self.call(&trap, &handler, &[target, k])?.truthy()));
        }
        Ok(Some(self.has_property(&target, key)?))
    }

    pub(crate) fn has_property(&mut self, v: &Value, key: &str) -> R<bool> {
        let Value::Object(o) = v else {
            return self.throw_error("TypeError", "Cannot use 'in' operator on a non-object");
        };
        if let Some(r) = self.proxy_has(o, key)? {
            return Ok(r);
        }
        let is_index = matches!(&*o.kind.borrow(), Kind::Array(items) if key.parse::<usize>().is_ok_and(|i| i < items.len()));
        Ok(is_index || find_in_chain(o, key).is_some())
    }

    pub(crate) fn proxy_delete(&mut self, o: &Obj, key: &str) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "deleteProperty")? else {
            return Ok(None);
        };
        if trap.is_callable() {
            let k = self.key_value(key);
            return Ok(Some(self.call(&trap, &handler, &[target, k])?.truthy()));
        }
        if let Value::Object(t) = &target {
            if let Some(r) = self.proxy_delete(t, key)? {
                return Ok(Some(r));
            }
            if !t.frozen.get() {
                t.remove_prop(key);
            }
        }
        Ok(Some(true))
    }

    /// All own string keys of a proxy (`enumerable_only` filters through the target).
    pub(crate) fn proxy_own_keys(
        &mut self,
        o: &Obj,
        enumerable_only: bool,
    ) -> R<Option<Vec<Value>>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "ownKeys")? else {
            return Ok(None);
        };
        let all: Vec<Value> = if trap.is_callable() {
            let r = self.call(&trap, &handler, std::slice::from_ref(&target))?;
            self.array_items(&r)
        } else if enumerable_only {
            self.own_keys(&target).into_iter().map(Value::Str).collect()
        } else {
            let object = self
                .lookup(&self.global.clone(), "Object")
                .unwrap_or(Value::Undefined);
            let names = self.get(&object, "getOwnPropertyNames")?;
            let r = self.call(&names, &object, std::slice::from_ref(&target))?;
            self.array_items(&r)
        };
        if !enumerable_only {
            return Ok(Some(all));
        }
        let allowed = self.own_keys(&target);
        Ok(Some(
            all.into_iter()
                .filter(|k| matches!(k, Value::Str(s) if allowed.contains(s)))
                .collect(),
        ))
    }

    pub(crate) fn proxy_call(&mut self, o: &Obj, this: &Value, args: &[Value]) -> R<Value> {
        let (target, handler, trap) = self
            .proxy_trap(o, "apply")?
            .expect("proxy_call on a non-proxy");
        if trap.is_callable() {
            let arr = self.array(args.to_vec());
            return self.call(&trap, &handler, &[target, this.clone(), arr]);
        }
        self.call(&target, this, args)
    }

    pub(crate) fn proxy_construct(&mut self, o: &Obj, f: &Value, args: &[Value]) -> R<Value> {
        let (target, handler, trap) = self
            .proxy_trap(o, "construct")?
            .expect("proxy_construct on a non-proxy");
        if trap.is_callable() {
            let arr = self.array(args.to_vec());
            let r = self.call(&trap, &handler, &[target, arr, f.clone()])?;
            if !matches!(r, Value::Object(_)) {
                return self.throw_error("TypeError", "proxy [[Construct]] must return an object");
            }
            return Ok(r);
        }
        self.construct(&target, args)
    }

    pub(crate) fn install_proxy(&mut self) {
        let ctor = self.native(|it, _, args| {
            if !it.rt.constructing {
                return it.throw_error("TypeError", "Constructor Proxy requires 'new'");
            }
            it.make_proxy(&arg(args, 0), &arg(args, 1))
        });
        let Value::Object(c) = &ctor else { return };
        self.set_native(c, "revocable", |it, _, args| {
            let proxy = it.make_proxy(&arg(args, 0), &arg(args, 1))?;
            let p = proxy.clone();
            let revoke = it.native(move |_, _, _| {
                if let Value::Object(o) = &p {
                    if let Some(d) = Interp::proxy_data(o) {
                        *d.handler.borrow_mut() = Value::Null;
                    }
                }
                Ok(Value::Undefined)
            });
            let out = it.plain();
            put(&out, "proxy", proxy);
            put(&out, "revoke", revoke);
            Ok(Value::Object(out))
        });
        self.global.declare("Proxy", ctor, true);
    }

    fn make_proxy(&mut self, target: &Value, handler: &Value) -> R<Value> {
        let (Value::Object(t), Value::Object(_)) = (target, handler) else {
            return self.throw_error(
                "TypeError",
                "Cannot create proxy with a non-object as target or handler",
            );
        };
        let callable = target.is_callable();
        let proto = t.proto.borrow().clone();
        Ok(Value::Object(new_obj(
            proto,
            Kind::Proxy(Rc::new(ProxyData {
                target: target.clone(),
                handler: RefCell::new(handler.clone()),
                callable,
            })),
        )))
    }
}
