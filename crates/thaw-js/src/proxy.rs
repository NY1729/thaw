//! `Proxy` / `Proxy.revocable`: get, set, has, deleteProperty, ownKeys, apply, construct,
//! defineProperty and getOwnPropertyDescriptor traps; every other operation reaches the target.

use crate::builtins::{same_value, PropDesc};
use crate::*;

pub(crate) struct ProxyData {
    pub(crate) target: Value,
    pub(crate) handler: RefCell<Value>,
    pub(crate) callable: bool,
    /// `process.env`: inspection renders the live environment (see process_env.rs).
    pub(crate) env_view: bool,
}

/// An object (a Symbol is a heap object here but not a language-level Object).
pub(crate) fn is_obj(v: &Value) -> bool {
    matches!(v, Value::Object(_)) && !is_symbol_object(v)
}

/// A property descriptor with `Option` for absent fields.
pub(crate) struct Pd {
    value: Option<Value>,
    writable: Option<bool>,
    get: Option<Value>,
    set: Option<Value>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
}

impl From<PropDesc> for Pd {
    fn from(d: PropDesc) -> Pd {
        let b = |x: Option<Value>| x.map(|v| v.truthy());
        Pd {
            value: d.value,
            writable: b(d.writable),
            get: d.get,
            set: d.set,
            enumerable: b(d.enumerable),
            configurable: b(d.configurable),
        }
    }
}

impl Pd {
    fn accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }
    fn data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }
    /// CompletePropertyDescriptor.
    fn complete(mut self) -> Pd {
        if self.accessor() {
            self.get.get_or_insert(Value::Undefined);
            self.set.get_or_insert(Value::Undefined);
        } else {
            self.value.get_or_insert(Value::Undefined);
            self.writable.get_or_insert(false);
        }
        self.enumerable.get_or_insert(false);
        self.configurable.get_or_insert(false);
        self
    }
    /// FromPropertyDescriptor.
    fn to_object(&self, it: &Interp) -> Value {
        let d = it.plain();
        let p = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                put(&d, k, v);
            }
        };
        p("value", self.value.clone());
        p("writable", self.writable.map(Value::Bool));
        p("get", self.get.clone());
        p("set", self.set.clone());
        p("enumerable", self.enumerable.map(Value::Bool));
        p("configurable", self.configurable.map(Value::Bool));
        Value::Object(d)
    }
    /// IsCompatiblePropertyDescriptor (ValidateAndApplyPropertyDescriptor without an object).
    fn compatible(&self, extensible: bool, cur: Option<&Pd>) -> bool {
        let Some(cur) = cur else {
            return extensible;
        };
        if cur.configurable == Some(false) {
            if self.configurable == Some(true) {
                return false;
            }
            if self.enumerable.is_some() && self.enumerable != cur.enumerable {
                return false;
            }
            if !self.accessor() && !self.data() {
                return true;
            }
            if self.accessor() != cur.accessor() {
                return false;
            }
            let same = |a: &Option<Value>, b: &Option<Value>| match (a, b) {
                (None, _) => true,
                (Some(x), Some(y)) => same_value(x, y),
                (Some(x), None) => matches!(x, Value::Undefined),
            };
            if cur.accessor() {
                return same(&self.get, &cur.get) && same(&self.set, &cur.set);
            }
            if cur.writable == Some(false) {
                return self.writable != Some(true) && same(&self.value, &cur.value);
            }
        }
        true
    }
}

impl Interp {
    /// The own keys a proxy trap reported, checked against the target's (ownKeys invariants).
    fn check_own_keys(&mut self, target: &Value, reported: &[Rc<str>]) -> R<()> {
        let ext = self.target_extensible(target)?;
        let target_keys = self.target_own_keys(target)?;
        let mut fixed = Vec::new();
        let mut loose = Vec::new();
        for k in &target_keys {
            let key = self.key_of(k);
            match self.target_gopd(target, &key)? {
                Some(d) if d.configurable == Some(false) => fixed.push(key),
                _ => loose.push(key),
            }
        }
        if ext && fixed.is_empty() {
            return Ok(());
        }
        let mut unchecked: Vec<Rc<str>> = reported.to_vec();
        let mut take = |k: &Rc<str>| match unchecked.iter().position(|u| u == k) {
            Some(i) => {
                unchecked.remove(i);
                true
            }
            None => false,
        };
        for k in &fixed {
            if !take(k) {
                return self.proxy_invariant(
                    "ownKeys",
                    "result must include every non-configurable target key",
                );
            }
        }
        if ext {
            return Ok(());
        }
        for k in &loose {
            if !take(k) {
                return self.proxy_invariant(
                    "ownKeys",
                    "result must include every key of a non-extensible target",
                );
            }
        }
        if !unchecked.is_empty() {
            return self.proxy_invariant(
                "ownKeys",
                "result must not add keys to a non-extensible target",
            );
        }
        Ok(())
    }

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
            if let Some(s) = crate::symbol_table::by_id(id) {
                return Value::Object(s);
            }
        }
        Value::Str(key.into())
    }

    /// `(target, handler, trap)` for a proxy; `trap` is `Undefined` when the handler lacks it (or has it as
    /// `null`). A trap that is neither nullish nor callable is a TypeError (GetMethod).
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
        let mut trap = self.get(&handler, name)?;
        if matches!(trap, Value::Null) {
            trap = Value::Undefined;
        } else if !matches!(trap, Value::Undefined) && !trap.is_callable() {
            return self.throw_error(
                "TypeError",
                &format!("proxy handler's {name} trap is not a function"),
            );
        }
        Ok(Some((p.target.clone(), handler, trap)))
    }

    /// ToPropertyKey: a symbol keeps its reserved key, anything else goes through ToPrimitive(string),
    /// whose abrupt completions propagate.
    pub(crate) fn prop_key(&mut self, v: &Value) -> R<Rc<str>> {
        if matches!(v, Value::Object(_)) && !is_symbol_object(v) {
            let p = self.primitive_of(v, extras::Hint::String)?;
            return Ok(self.key_of(&p));
        }
        Ok(self.key_of(v))
    }

    /// An invariant violation of a proxy trap.
    fn proxy_invariant<T>(&mut self, trap: &str, why: &str) -> R<T> {
        self.throw_error("TypeError", &format!("'{trap}' on proxy: {why}"))
    }

    /// The own descriptor of `key` on `target`, read through `Object.getOwnPropertyDescriptor`.
    fn target_gopd(&mut self, target: &Value, key: &str) -> R<Option<Pd>> {
        let object = self
            .lookup(&self.global.clone(), "Object")
            .unwrap_or(Value::Undefined);
        let gopd = self.get(&object, "getOwnPropertyDescriptor")?;
        let k = self.key_value(key);
        let d = self.call(&gopd, &object, &[target.clone(), k])?;
        if matches!(d, Value::Undefined) {
            return Ok(None);
        }
        let pd = self.prop_desc_of(&d)?;
        Ok(Some(Pd::from(pd)))
    }

    fn target_extensible(&mut self, target: &Value) -> R<bool> {
        match target {
            Value::Object(t) => crate::reflect::is_extensible(self, t),
            _ => Ok(false),
        }
    }

    pub(crate) fn proxy_get(&mut self, o: &Obj, key: &str, receiver: &Value) -> R<Option<Value>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "get")? else {
            return Ok(None);
        };
        if trap.is_callable() {
            let k = self.key_value(key);
            let v = self.call(&trap, &handler, &[target.clone(), k, receiver.clone()])?;
            if !Self::proxy_is_view(o) {
                if let Some(td) = self.target_gopd(&target, key)? {
                    if td.configurable == Some(false) {
                        if td.data() && td.writable == Some(false) {
                            let same = td.value.as_ref().is_some_and(|x| same_value(x, &v));
                            if !same {
                                return self.proxy_invariant("get", "value differs from the non-writable, non-configurable target property");
                            }
                        } else if td.accessor()
                            && matches!(td.get, None | Some(Value::Undefined))
                            && !matches!(v, Value::Undefined)
                        {
                            return self.proxy_invariant(
                                "get",
                                "non-configurable accessor without a getter must report undefined",
                            );
                        }
                    }
                }
            }
            return Ok(Some(v));
        }
        let Value::Object(t) = &target else {
            return Ok(Some(Value::Undefined));
        };
        Ok(Some(crate::reflect::get(self, t, key, receiver)?))
    }

    fn proxy_is_view(o: &Obj) -> bool {
        Self::proxy_data(o).is_some_and(|p| p.env_view)
    }

    /// `[[Set]]` of a proxy, answering the trap's (or the target's) verdict.
    pub(crate) fn proxy_set_full(
        &mut self,
        o: &Obj,
        key: &str,
        value: &Value,
        receiver: &Value,
    ) -> R<bool> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "set")? else {
            return Ok(false);
        };
        if !trap.is_callable() {
            let Value::Object(t) = &target else {
                return Ok(false);
            };
            let intr = crate::reflect::intrinsics(self);
            return crate::reflect::set(self, t, key, value.clone(), receiver, &intr);
        }
        let k = self.key_value(key);
        let ok = self
            .call(
                &trap,
                &handler,
                &[target.clone(), k, value.clone(), receiver.clone()],
            )?
            .truthy();
        if ok && !Self::proxy_is_view(o) {
            if let Some(td) = self.target_gopd(&target, key)? {
                if td.configurable == Some(false) {
                    if td.data() && td.writable == Some(false) {
                        let same = td.value.as_ref().is_some_and(|x| same_value(x, value));
                        if !same {
                            return self.proxy_invariant("set", "cannot change the value of a non-writable, non-configurable target property");
                        }
                    } else if td.accessor() && matches!(td.set, None | Some(Value::Undefined)) {
                        return self.proxy_invariant(
                            "set",
                            "cannot set a non-configurable accessor without a setter",
                        );
                    }
                }
            }
        }
        Ok(ok)
    }

    /// The `[[Set]]` hook of the `=` operator: `Ok(false)` when `o` is no proxy.
    pub(crate) fn proxy_set(
        &mut self,
        o: &Obj,
        key: &str,
        value: &Value,
        receiver: &Value,
        strict: bool,
    ) -> R<bool> {
        if Self::proxy_data(o).is_none() {
            return Ok(false);
        }
        self.proxy_set_chain(o, key, value, receiver, strict)?;
        Ok(true)
    }

    /// A write that reached proxy `o` (itself, or on a prototype chain): a refusal throws in strict code.
    pub(crate) fn proxy_set_chain(
        &mut self,
        o: &Obj,
        key: &str,
        value: &Value,
        receiver: &Value,
        strict: bool,
    ) -> R<()> {
        if !self.proxy_set_full(o, key, value, receiver)? && strict {
            return self.proxy_invariant(
                "set",
                &format!("trap returned falsish for property '{key}'"),
            );
        }
        Ok(())
    }

    pub(crate) fn proxy_has(&mut self, o: &Obj, key: &str) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "has")? else {
            return Ok(None);
        };
        if !trap.is_callable() {
            return Ok(Some(self.has_property(&target, key)?));
        }
        let k = self.key_value(key);
        let r = self.call(&trap, &handler, &[target.clone(), k])?.truthy();
        if !r && !Self::proxy_is_view(o) {
            if let Some(td) = self.target_gopd(&target, key)? {
                if td.configurable == Some(false) {
                    return self.proxy_invariant(
                        "has",
                        "a non-configurable target property cannot be reported as absent",
                    );
                }
                if !self.target_extensible(&target)? {
                    return self.proxy_invariant(
                        "has",
                        "a property of a non-extensible target cannot be reported as absent",
                    );
                }
            }
        }
        Ok(Some(r))
    }

    /// `[[GetOwnProperty]]` of a proxy: its trap's descriptor object (checked against the target), or the target's.
    pub(crate) fn proxy_gopd(&mut self, o: &Obj, key: &str) -> R<Value> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "getOwnPropertyDescriptor")? else {
            return Ok(Value::Undefined);
        };
        if !trap.is_callable() {
            let object = self
                .lookup(&self.global.clone(), "Object")
                .unwrap_or(Value::Undefined);
            let gopd = self.get(&object, "getOwnPropertyDescriptor")?;
            let k = self.key_value(key);
            return self.call(&gopd, &object, &[target, k]);
        }
        let k = self.key_value(key);
        let r = self.call(&trap, &handler, &[target.clone(), k])?;
        if Self::proxy_is_view(o) {
            return Ok(r);
        }
        if !is_obj(&r) && !matches!(r, Value::Undefined) {
            return self.proxy_invariant(
                "getOwnPropertyDescriptor",
                "trap result must be an object or undefined",
            );
        }
        let td = self.target_gopd(&target, key)?;
        if matches!(r, Value::Undefined) {
            if let Some(td) = td {
                if td.configurable == Some(false) {
                    return self.proxy_invariant(
                        "getOwnPropertyDescriptor",
                        "a non-configurable target property cannot be reported as absent",
                    );
                }
                if !self.target_extensible(&target)? {
                    return self.proxy_invariant(
                        "getOwnPropertyDescriptor",
                        "a property of a non-extensible target cannot be reported as absent",
                    );
                }
            }
            return Ok(Value::Undefined);
        }
        let ext = self.target_extensible(&target)?;
        let rd = Pd::from(self.prop_desc_of(&r)?).complete();
        if !rd.compatible(ext, td.as_ref()) {
            return self.proxy_invariant(
                "getOwnPropertyDescriptor",
                "trap result is incompatible with the target property",
            );
        }
        if rd.configurable == Some(false) {
            match &td {
                Some(t) if t.configurable == Some(false) => {
                    if rd.writable == Some(false) && t.writable == Some(true) {
                        return self.proxy_invariant(
                            "getOwnPropertyDescriptor",
                            "a writable target property cannot be reported non-writable",
                        );
                    }
                }
                _ => {
                    return self.proxy_invariant("getOwnPropertyDescriptor", "a configurable or missing target property cannot be reported non-configurable");
                }
            }
        }
        Ok(rd.to_object(self))
    }

    /// `[[DefineOwnProperty]]` of a proxy.
    pub(crate) fn proxy_define(&mut self, o: &Obj, key: &str, desc: &Value) -> R<bool> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "defineProperty")? else {
            return Ok(false);
        };
        let d = Pd::from(self.prop_desc_of(desc)?);
        if !trap.is_callable() {
            let Value::Object(t) = &target else {
                return Ok(false);
            };
            return crate::reflect::define(self, t, key, desc);
        }
        let k = self.key_value(key);
        let dobj = d.to_object(self);
        let r = self
            .call(&trap, &handler, &[target.clone(), k, dobj])?
            .truthy();
        if !r || Self::proxy_is_view(o) {
            return Ok(r);
        }
        let td = self.target_gopd(&target, key)?;
        let ext = self.target_extensible(&target)?;
        let setting_nc = d.configurable == Some(false);
        match td {
            None => {
                if !ext {
                    return self.proxy_invariant(
                        "defineProperty",
                        "cannot add a property to a non-extensible target",
                    );
                }
                if setting_nc {
                    return self.proxy_invariant(
                        "defineProperty",
                        "cannot define a non-configurable property that is missing on the target",
                    );
                }
            }
            Some(td) => {
                if !d.compatible(ext, Some(&td)) {
                    return self.proxy_invariant(
                        "defineProperty",
                        "descriptor is incompatible with the target property",
                    );
                }
                if setting_nc && td.configurable == Some(true) {
                    return self.proxy_invariant("defineProperty", "cannot define a non-configurable property that is configurable on the target");
                }
                if td.data()
                    && td.configurable == Some(false)
                    && td.writable == Some(true)
                    && d.writable == Some(false)
                {
                    return self.proxy_invariant(
                        "defineProperty",
                        "cannot make a writable target property non-writable",
                    );
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn has_property(&mut self, v: &Value, key: &str) -> R<bool> {
        let Value::Object(o) = v else {
            return self.throw_error("TypeError", "Cannot use 'in' operator on a non-object");
        };
        if let Some(r) = self.proxy_has(o, key)? {
            return Ok(r);
        }
        // a proxy on the prototype chain answers for the rest of the walk
        {
            let mut cur = o.proto.borrow().clone();
            let mut depth = 0;
            while let Some(p) = cur {
                if matches!(&*p.kind.borrow(), Kind::Proxy(_)) {
                    if self.has_own_property(v, &self.key_value(key))? {
                        return Ok(true);
                    }
                    let mut link = o.proto.borrow().clone();
                    while let Some(l) = link {
                        if Rc::ptr_eq(&l, &p) {
                            break;
                        }
                        if self.has_own_property(&Value::Object(l.clone()), &self.key_value(key))? {
                            return Ok(true);
                        }
                        link = l.proto.borrow().clone();
                    }
                    return self.has_property(&Value::Object(p), key);
                }
                depth += 1;
                if depth > 10_000 {
                    break;
                }
                cur = p.proto.borrow().clone();
            }
        }
        // a typed array's numeric key answers for the element alone, never the prototype chain
        if let Some(valid) = crate::typed::typed_own_key(o, key) {
            return Ok(valid);
        }
        let is_index = matches!(&*o.kind.borrow(), Kind::Array(items) if key == "length" || array_index(key).is_some_and(|i| array_has_index(o, items, i)));
        if matches!(&*o.kind.borrow(), Kind::Global) {
            // a context's global: the sandbox's chain answers first (its vars, its names, Object.prototype)
            if let Some(sb) = crate::vm::sandbox_of(o) {
                if self.has_property(&Value::Object(sb), key)? {
                    return Ok(true);
                }
            }
            if crate::hidden_global_name(key) {
                return Ok(false);
            }
            if self.global_prop(key).is_some() {
                return Ok(true);
            }
        }
        Ok(is_index || find_in_chain(o, key).is_some())
    }

    /// `[[GetPrototypeOf]]` of a proxy: its trap's answer, else the target's prototype (`None`: not a proxy).
    pub(crate) fn proxy_get_prototype_of(&mut self, o: &Obj) -> R<Option<Value>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "getPrototypeOf")? else {
            return Ok(None);
        };
        let Value::Object(t) = &target else {
            return Ok(Some(Value::Null));
        };
        if !trap.is_callable() {
            return Ok(Some(crate::reflect::get_prototype_of(self, t)?));
        }
        let r = self.call(&trap, &handler, std::slice::from_ref(&target))?;
        if !is_obj(&r) && !matches!(r, Value::Null) {
            return self.proxy_invariant("getPrototypeOf", "trap result must be an object or null");
        }
        if !Self::proxy_is_view(o) && !crate::reflect::is_extensible(self, t)? {
            let tp = crate::reflect::get_prototype_of(self, t)?;
            if !same_value(&tp, &r) {
                return self.proxy_invariant(
                    "getPrototypeOf",
                    "a non-extensible target's prototype must be reported unchanged",
                );
            }
        }
        Ok(Some(r))
    }

    /// `[[SetPrototypeOf]]` of a proxy: its trap's answer, else set the target's prototype (`None`: not a proxy).
    pub(crate) fn proxy_set_prototype_of(&mut self, o: &Obj, proto: &Value) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "setPrototypeOf")? else {
            return Ok(None);
        };
        let Value::Object(t) = &target else {
            return Ok(Some(false));
        };
        if !trap.is_callable() {
            let p = match proto {
                Value::Object(p) => Some(p.clone()),
                _ => None,
            };
            return Ok(Some(crate::reflect::set_prototype_of(self, t, p)?));
        }
        let r = self
            .call(&trap, &handler, &[target.clone(), proto.clone()])?
            .truthy();
        if !r || Self::proxy_is_view(o) {
            return Ok(Some(r));
        }
        if !crate::reflect::is_extensible(self, t)? {
            let tp = crate::reflect::get_prototype_of(self, t)?;
            if !same_value(&tp, proto) {
                return self.proxy_invariant(
                    "setPrototypeOf",
                    "a non-extensible target's prototype cannot change",
                );
            }
        }
        Ok(Some(true))
    }

    /// `[[IsExtensible]]` of a proxy: its trap's answer, else the target's (`None`: not a proxy).
    pub(crate) fn proxy_is_extensible(&mut self, o: &Obj) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "isExtensible")? else {
            return Ok(None);
        };
        if !trap.is_callable() {
            return Ok(Some(self.target_extensible(&target)?));
        }
        let r = self
            .call(&trap, &handler, std::slice::from_ref(&target))?
            .truthy();
        if !Self::proxy_is_view(o) && r != self.target_extensible(&target)? {
            return self.proxy_invariant(
                "isExtensible",
                "result must match the target's extensibility",
            );
        }
        Ok(Some(r))
    }

    /// `[[PreventExtensions]]` of a proxy: its trap's answer, else the target's (`None`: not a proxy).
    pub(crate) fn proxy_prevent_extensions(&mut self, o: &Obj) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "preventExtensions")? else {
            return Ok(None);
        };
        if !trap.is_callable() {
            return Ok(Some(match &target {
                Value::Object(t) => crate::reflect::prevent_extensions(self, t)?,
                _ => false,
            }));
        }
        let r = self
            .call(&trap, &handler, std::slice::from_ref(&target))?
            .truthy();
        if r && !Self::proxy_is_view(o) && self.target_extensible(&target)? {
            return self.proxy_invariant(
                "preventExtensions",
                "returned true but the target is still extensible",
            );
        }
        Ok(Some(r))
    }

    /// The object an integrity or array check should inspect: `v` with every proxy layer
    /// unwrapped to its target (throws on a revoked proxy); non-proxies come back unchanged.
    pub(crate) fn proxy_unwrap(&mut self, v: &Value, name: &str) -> R<Value> {
        let mut cur = v.clone();
        loop {
            let next = match &cur {
                Value::Object(o) => match Self::proxy_data(o) {
                    Some(p) => {
                        // ponytail: revocation is read off the handler slot, no trap lookup
                        if matches!(&*p.handler.borrow(), Value::Null) {
                            return self.throw_error(
                                "TypeError",
                                &format!(
                                    "Cannot perform '{name}' on a proxy that has been revoked"
                                ),
                            );
                        }
                        Some(p.target.clone())
                    }
                    None => None,
                },
                _ => None,
            };
            match next {
                Some(t) => cur = t,
                None => return Ok(cur),
            }
        }
    }

    pub(crate) fn proxy_delete(&mut self, o: &Obj, key: &str) -> R<Option<bool>> {
        let Some((target, handler, trap)) = self.proxy_trap(o, "deleteProperty")? else {
            return Ok(None);
        };
        if !trap.is_callable() {
            if let Value::Object(t) = &target {
                return Ok(Some(crate::reflect::delete_own(self, t, key)?));
            }
            return Ok(Some(true));
        }
        let k = self.key_value(key);
        let r = self.call(&trap, &handler, &[target.clone(), k])?.truthy();
        if !r || Self::proxy_is_view(o) {
            return Ok(Some(r));
        }
        if let Some(td) = self.target_gopd(&target, key)? {
            if td.configurable == Some(false) {
                return self.proxy_invariant(
                    "deleteProperty",
                    "a non-configurable target property cannot be reported deleted",
                );
            }
            if !self.target_extensible(&target)? {
                return self.proxy_invariant(
                    "deleteProperty",
                    "a property of a non-extensible target cannot be reported deleted",
                );
            }
        }
        Ok(Some(true))
    }

    /// All own keys of a target: string keys, then symbols.
    fn target_own_keys(&mut self, target: &Value) -> R<Vec<Value>> {
        if let Value::Object(t) = target {
            if let Some(keys) = self.proxy_own_keys(t, false)? {
                return Ok(keys);
            }
        }
        let object = self
            .lookup(&self.global.clone(), "Object")
            .unwrap_or(Value::Undefined);
        let names = self.get(&object, "getOwnPropertyNames")?;
        let r = self.call(&names, &object, std::slice::from_ref(target))?;
        let mut keys = self.array_items(&r);
        let symbols = self.get(&object, "getOwnPropertySymbols")?;
        let r = self.call(&symbols, &object, std::slice::from_ref(target))?;
        keys.extend(self.array_items(&r));
        Ok(keys)
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
            if !is_obj(&r) {
                return self.proxy_invariant("ownKeys", "trap result must be an array-like object");
            }
            let items = crate::reflect::list_from(self, &r)?;
            // CreateListFromArrayLike(.., string | symbol), then no duplicates
            let mut seen: Vec<Rc<str>> = Vec::new();
            for item in &items {
                let is_symbol = is_symbol_object(item);
                if !matches!(item, Value::Str(_)) && !is_symbol {
                    let text = self.to_str(item);
                    return self
                        .throw_error("TypeError", &format!("{text} is not a valid property name"));
                }
                let key = self.key_of(item);
                if seen.contains(&key) {
                    return self.throw_error(
                        "TypeError",
                        "'ownKeys' on proxy: trap returned duplicate entries",
                    );
                }
                seen.push(key);
            }
            if !Self::proxy_is_view(o) {
                self.check_own_keys(&target, &seen)?;
            }
            items
        } else if enumerable_only {
            self.own_keys(&target)
                .into_iter()
                .map(|k| Value::Str(k.into()))
                .collect()
        } else {
            self.target_own_keys(&target)?
        };
        if !enumerable_only {
            return Ok(Some(all));
        }
        // a key is listed when its descriptor is enumerable: the getOwnPropertyDescriptor trap's, else the target's
        let gopd = self.proxy_trap(o, "getOwnPropertyDescriptor")?;
        let allowed = self.own_keys(&target);
        let mut keep = Vec::new();
        for k in all {
            let Value::Str(s) = &k else { continue };
            let enumerable = match &gopd {
                Some((t, h, trap)) if trap.is_callable() => {
                    let d = self.call(trap, h, &[t.clone(), k.clone()])?;
                    matches!(d, Value::Object(_)) && self.get(&d, "enumerable")?.truthy()
                }
                _ => allowed.contains(&s.into_rc()),
            };
            if enumerable {
                keep.push(k);
            }
        }
        Ok(Some(keep))
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

    pub(crate) fn proxy_construct(
        &mut self,
        o: &Obj,
        args: &[Value],
        new_target: &Value,
    ) -> R<Value> {
        let (target, handler, trap) = self
            .proxy_trap(o, "construct")?
            .expect("proxy_construct on a non-proxy");
        if trap.is_callable() {
            let arr = self.array(args.to_vec());
            let r = self.call(&trap, &handler, &[target, arr, new_target.clone()])?;
            if !is_obj(&r) {
                return self.throw_error("TypeError", "proxy [[Construct]] must return an object");
            }
            return Ok(r);
        }
        self.construct_with(&target, args, new_target)
    }

    pub(crate) fn install_proxy(&mut self) {
        let ctor = self.native(|it, _, args| {
            if !it.rt.constructing {
                return it.throw_error("TypeError", "Constructor Proxy requires 'new'");
            }
            it.make_proxy(&arg(args, 0), &arg(args, 1))
        });
        let Value::Object(c) = &ctor else { return };
        put_hidden(c, "@@ctor", Value::Bool(true));
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
            if let Value::Object(f) = &revoke {
                crate::builtin_table::set_fn_head(f, "", 0.0);
                f.no_ctor.set(true);
            }
            let out = it.plain();
            put(&out, "proxy", proxy);
            put(&out, "revoke", revoke);
            Ok(Value::Object(out))
        });
        self.global.declare("Proxy", ctor, true);
        // Brand check for structuredClone and util.types.isProxy (no JS-visible way to tell a proxy apart).
        let is_proxy = self.native(|_, _, args| {
            Ok(Value::Bool(
                matches!(arg(args, 0), Value::Object(o) if Interp::proxy_data(&o).is_some()),
            ))
        });
        self.global.declare("__thaw_is_proxy", is_proxy, true);
        // a sparse tail (length past the dense slots): the copying prelude methods refuse it, as stage 1 does
        let has_tail = self.native(|_, _, args| {
            Ok(Value::Bool(
                matches!(arg(args, 0), Value::Object(o) if crate::array_has_sparse_tail(&o)),
            ))
        });
        self.global.declare("__thaw_array_has_tail", has_tail, true);
    }

    fn make_proxy(&mut self, target: &Value, handler: &Value) -> R<Value> {
        let (Value::Object(t), true) = (target, is_obj(target) && is_obj(handler)) else {
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
                env_view: false,
            })),
        )))
    }
}
