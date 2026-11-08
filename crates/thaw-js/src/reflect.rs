//! `Reflect`: the thirteen static methods as natives over the object model. Proxy operations go to
//! the handler trap, or to the target when the trap is absent, the same routing the operators use.

use crate::*;

/// The `Object` functions captured at install time, so later user changes to `Object` do not reach
/// `Reflect`. They are proxy-aware themselves.
struct Intrinsics {
    gopd: Value,
    define: Value,
    gopn: Value,
    gops: Value,
}

fn is_proxy(o: &Obj) -> bool {
    matches!(&*o.kind.borrow(), Kind::Proxy(_))
}

fn obj(it: &mut Interp, name: &str, v: &Value) -> R<Obj> {
    match v {
        Value::Object(o) => Ok(o.clone()),
        _ => it.throw_error("TypeError", &format!("Reflect.{name} called on non-object")),
    }
}

/// `CreateListFromArrayLike`. ponytail: array-likes are read by `array_items` (plain objects with a
/// numeric `length` only).
fn list_from(it: &mut Interp, v: &Value) -> R<Vec<Value>> {
    match v {
        Value::Object(_) => Ok(it.array_items(v)),
        _ => it.throw_error("TypeError", "CreateListFromArrayLike called on non-object"),
    }
}

/// Own property `key` as (slot, writable). Array elements and `length` count as writable data; a
/// frozen object has nothing writable.
fn own_prop(o: &Obj, key: &str) -> Option<(Slot, bool)> {
    let writable = !o.frozen.get();
    if let Kind::Array(items) = &*o.kind.borrow() {
        if key == "length" {
            return Some((
                Slot::Data(Value::Number(items.len() as f64)),
                o.len_writable(),
            ));
        }
        if let Some(v) = array_index(key).and_then(|i| items.get(i)) {
            if !matches!(v, Value::Hole) {
                return Some((Slot::Data(v.clone()), writable));
            }
        }
    }
    o.force_key(key);
    let i = o.position(key)?;
    let props = o.raw_props.borrow();
    Some((props[i].slot.clone(), props[i].writable && writable))
}

fn is_extensible(it: &mut Interp, o: &Obj) -> R<bool> {
    if let Some((target, handler, trap)) = it.proxy_trap(o, "isExtensible")? {
        if trap.is_callable() {
            return Ok(it.call(&trap, &handler, &[target])?.truthy());
        }
        return match &target {
            Value::Object(t) => is_extensible(it, t),
            _ => Ok(false),
        };
    }
    Ok(o.ext.get() == 0 && !o.frozen.get())
}

fn prevent_extensions(it: &mut Interp, o: &Obj) -> R<bool> {
    if let Some((target, handler, trap)) = it.proxy_trap(o, "preventExtensions")? {
        if trap.is_callable() {
            return Ok(it.call(&trap, &handler, &[target])?.truthy());
        }
        return match &target {
            Value::Object(t) => prevent_extensions(it, t),
            _ => Ok(false),
        };
    }
    o.ext.set(o.ext.get().max(1));
    Ok(true)
}

fn get(it: &mut Interp, o: &Obj, key: &str, recv: &Value) -> R<Value> {
    if matches!(recv, Value::Object(r) if Rc::ptr_eq(r, o)) {
        return it.get(&Value::Object(o.clone()), key);
    }
    // a different receiver: the first holder decides, a getter runs with the receiver as `this`
    // ponytail: array elements and typed-array indices are only seen when the receiver is the object itself
    let mut cur = o.clone();
    loop {
        if is_proxy(&cur) {
            return Ok(it.proxy_get(&cur, key, recv)?.unwrap_or(Value::Undefined));
        }
        match own_prop(&cur, key) {
            Some((Slot::Data(v), _)) => return Ok(v),
            Some((Slot::LazyProto { .. }, _)) => unreachable!("own_prop forces the key"),
            Some((Slot::Accessor { get, .. }, _)) => {
                return match get {
                    Some(getter) => it.call(&getter, recv, &[]),
                    None => Ok(Value::Undefined),
                }
            }
            None => {}
        }
        let next = cur.proto.borrow().clone();
        match next {
            Some(p) => cur = p,
            None => return Ok(Value::Undefined),
        }
    }
}

fn set(it: &mut Interp, o: &Obj, key: &str, v: Value, recv: &Value, intr: &Intrinsics) -> R<bool> {
    // OrdinarySet: the first holder of `key` along the chain decides, then the receiver takes the value
    let mut cur = o.clone();
    let holder_writable = loop {
        if is_proxy(&cur) {
            return set_via_proxy(it, &cur, key, v, recv, intr);
        }
        match own_prop(&cur, key) {
            Some((Slot::Accessor { set, .. }, _)) => {
                return match set {
                    Some(setter) => {
                        it.call(&setter, recv, &[v])?;
                        Ok(true)
                    }
                    None => Ok(false),
                };
            }
            Some((Slot::Data(_), writable)) => break writable,
            Some((Slot::LazyProto { .. }, _)) => unreachable!("own_prop forces the key"),
            None => {}
        }
        let next = cur.proto.borrow().clone();
        match next {
            Some(p) => cur = p,
            None => break true,
        }
    };
    if !holder_writable {
        return Ok(false);
    }
    let Value::Object(r) = recv else {
        return Ok(false);
    };
    let k = Value::Str(key.into());
    let existing = it.call(&intr.gopd, &Value::Undefined, &[recv.clone(), k.clone()])?;
    if let Value::Object(_) = &existing {
        let accessor = !matches!(it.get(&existing, "get")?, Value::Undefined)
            || !matches!(it.get(&existing, "set")?, Value::Undefined);
        if accessor || !it.get(&existing, "writable")?.truthy() {
            return Ok(false);
        }
        let d = it.plain();
        put(&d, "value", v);
        it.call(
            &intr.define,
            &Value::Undefined,
            &[recv.clone(), k, Value::Object(d)],
        )?;
        return Ok(true);
    }
    if !is_extensible(it, r)? {
        return Ok(false);
    }
    let d = it.plain();
    put(&d, "value", v);
    put(&d, "writable", Value::Bool(true));
    put(&d, "enumerable", Value::Bool(true));
    put(&d, "configurable", Value::Bool(true));
    it.call(
        &intr.define,
        &Value::Undefined,
        &[recv.clone(), k, Value::Object(d)],
    )?;
    Ok(true)
}

fn set_via_proxy(
    it: &mut Interp,
    p: &Obj,
    key: &str,
    v: Value,
    recv: &Value,
    intr: &Intrinsics,
) -> R<bool> {
    let Some((target, handler, trap)) = it.proxy_trap(p, "set")? else {
        return Ok(false);
    };
    if trap.is_callable() {
        let k = it.key_value(key);
        return Ok(it
            .call(&trap, &handler, &[target, k, v, recv.clone()])?
            .truthy());
    }
    match &target {
        Value::Object(t) => set(it, t, key, v, recv, intr),
        _ => Ok(false),
    }
}

fn define(it: &mut Interp, o: &Obj, key: &str, desc: &Value) -> R<bool> {
    if let Some((target, handler, trap)) = it.proxy_trap(o, "defineProperty")? {
        if trap.is_callable() {
            let k = it.key_value(key);
            return Ok(it
                .call(&trap, &handler, &[target, k, desc.clone()])?
                .truthy());
        }
        return match &target {
            Value::Object(t) => define(it, t, key, desc),
            _ => Ok(false),
        };
    }
    it.try_define(o, key, desc)
}

/// `[[Delete]]` of own property `key`, shared by the `delete` operator, `Reflect.deleteProperty`,
/// the proxy fallthrough and the native property predicates. A refused delete is `false`.
// ponytail: sloppy semantics only; strict-mode code should throw a TypeError on a refused delete, but
// strictness is not tracked, so it never does.
pub(crate) fn delete_own(it: &mut Interp, o: &Obj, key: &str) -> R<bool> {
    if is_proxy(o) {
        return Ok(it.proxy_delete(o, key)?.unwrap_or(true));
    }
    if matches!(&*o.kind.borrow(), Kind::Global) {
        if let Some(deleted) = it.delete_global_binding(key) {
            return Ok(deleted);
        }
    }
    if own_prop(o, key).is_none() {
        return Ok(true);
    }
    // array elements have no Prop: they are configurable unless the object is frozen or sealed
    if o.frozen.get() || o.ext.get() == 2 {
        return Ok(false);
    }
    if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
        if key == "length" {
            return Ok(false);
        }
        if let Some(slot) = array_index(key).and_then(|i| items.get_mut(i)) {
            *slot = Value::Hole;
        }
    }
    if let Some(i) = o.position(key) {
        if !o.raw_props.borrow()[i].configurable {
            return Ok(false);
        }
    }
    o.remove_prop(key);
    Ok(true)
}

fn get_prototype_of(it: &mut Interp, o: &Obj) -> R<Value> {
    if is_proxy(o) {
        return Ok(it.proxy_get_prototype_of(o)?.unwrap_or(Value::Null));
    }
    Ok(o.proto.borrow().clone().map_or(Value::Null, Value::Object))
}

fn set_prototype_of(it: &mut Interp, o: &Obj, p: Option<Obj>) -> R<bool> {
    if let Some((target, handler, trap)) = it.proxy_trap(o, "setPrototypeOf")? {
        if trap.is_callable() {
            let pv = p.map_or(Value::Null, Value::Object);
            return Ok(it.call(&trap, &handler, &[target, pv])?.truthy());
        }
        return match &target {
            Value::Object(t) => set_prototype_of(it, t, p),
            _ => Ok(false),
        };
    }
    let same = match (&*o.proto.borrow(), &p) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return Ok(true);
    }
    if !is_extensible(it, o)? {
        return Ok(false);
    }
    // a cycle through `o` is refused; a proxy on the new chain ends the walk (ponytail: not followed)
    let mut q = p.clone();
    while let Some(x) = q.take() {
        if Rc::ptr_eq(&x, o) {
            return Ok(false);
        }
        if is_proxy(&x) {
            break;
        }
        q = x.proto.borrow().clone();
    }
    *o.proto.borrow_mut() = p;
    Ok(true)
}

impl Interp {
    pub(crate) fn install_reflect(&mut self) {
        let object = self
            .lookup(&self.global.clone(), "Object")
            .unwrap_or(Value::Undefined);
        let grab = |it: &mut Interp, name: &str| it.get(&object, name).unwrap_or(Value::Undefined);
        let intr = Rc::new(Intrinsics {
            gopd: grab(self, "getOwnPropertyDescriptor"),
            define: grab(self, "defineProperty"),
            gopn: grab(self, "getOwnPropertyNames"),
            gops: grab(self, "getOwnPropertySymbols"),
        });
        let r = self.plain();
        self.set_native(&r, "get", |it, _, args| {
            let o = obj(it, "get", &arg(args, 0))?;
            let key = it.key_of(&arg(args, 1));
            let recv = args
                .get(2)
                .cloned()
                .unwrap_or_else(|| Value::Object(o.clone()));
            get(it, &o, &key, &recv)
        });
        {
            let i = intr.clone();
            self.set_native(&r, "set", move |it, _, args| {
                let o = obj(it, "set", &arg(args, 0))?;
                let key = it.key_of(&arg(args, 1));
                let recv = args
                    .get(3)
                    .cloned()
                    .unwrap_or_else(|| Value::Object(o.clone()));
                Ok(Value::Bool(set(it, &o, &key, arg(args, 2), &recv, &i)?))
            });
        }
        self.set_native(&r, "has", |it, _, args| {
            let o = obj(it, "has", &arg(args, 0))?;
            let key = it.key_of(&arg(args, 1));
            Ok(Value::Bool(it.has_property(&Value::Object(o), &key)?))
        });
        {
            let i = intr.clone();
            self.set_native(&r, "ownKeys", move |it, _, args| {
                let o = obj(it, "ownKeys", &arg(args, 0))?;
                if is_proxy(&o) {
                    let keys = it.proxy_own_keys(&o, false)?.unwrap_or_default();
                    return Ok(it.array(keys));
                }
                let names = it.call(&i.gopn, &Value::Undefined, &[Value::Object(o.clone())])?;
                let syms = it.call(&i.gops, &Value::Undefined, &[Value::Object(o.clone())])?;
                let mut keys = it.array_items(&names);
                keys.extend(it.array_items(&syms));
                Ok(it.array(keys))
            });
        }
        self.set_native(&r, "defineProperty", |it, _, args| {
            let o = obj(it, "defineProperty", &arg(args, 0))?;
            let key = it.key_of(&arg(args, 1));
            let desc = arg(args, 2);
            if !matches!(desc, Value::Object(_)) {
                return it.throw_error("TypeError", "Property description must be an object");
            }
            Ok(Value::Bool(define(it, &o, &key, &desc)?))
        });
        {
            let i = intr.clone();
            self.set_native(&r, "getOwnPropertyDescriptor", move |it, _, args| {
                let o = obj(it, "getOwnPropertyDescriptor", &arg(args, 0))?;
                let key = it.key_of(&arg(args, 1));
                let k = Value::Str((&*key).into());
                it.call(&i.gopd, &Value::Undefined, &[Value::Object(o), k])
            });
        }
        self.set_native(&r, "deleteProperty", |it, _, args| {
            let o = obj(it, "deleteProperty", &arg(args, 0))?;
            let key = it.key_of(&arg(args, 1));
            Ok(Value::Bool(delete_own(it, &o, &key)?))
        });
        self.set_native(&r, "getPrototypeOf", |it, _, args| {
            let o = obj(it, "getPrototypeOf", &arg(args, 0))?;
            get_prototype_of(it, &o)
        });
        self.set_native(&r, "setPrototypeOf", |it, _, args| {
            let o = obj(it, "setPrototypeOf", &arg(args, 0))?;
            let p = match arg(args, 1) {
                Value::Object(p) => Some(p),
                Value::Null => None,
                _ => {
                    return it.throw_error(
                        "TypeError",
                        "Object prototype may only be an Object or null",
                    )
                }
            };
            Ok(Value::Bool(set_prototype_of(it, &o, p)?))
        });
        self.set_native(&r, "isExtensible", |it, _, args| {
            let o = obj(it, "isExtensible", &arg(args, 0))?;
            Ok(Value::Bool(is_extensible(it, &o)?))
        });
        self.set_native(&r, "preventExtensions", |it, _, args| {
            let o = obj(it, "preventExtensions", &arg(args, 0))?;
            Ok(Value::Bool(prevent_extensions(it, &o)?))
        });
        self.set_native(&r, "apply", |it, _, args| {
            let f = arg(args, 0);
            if !f.is_callable() {
                return it.throw_error("TypeError", "Reflect.apply target is not a function");
            }
            let list = list_from(it, &arg(args, 2))?;
            it.call(&f, &arg(args, 1), &list)
        });
        self.set_native(&r, "construct", |it, _, args| {
            let target = arg(args, 0);
            if !target.is_callable() {
                return it
                    .throw_error("TypeError", "Reflect.construct target is not a constructor");
            }
            let list = list_from(it, &arg(args, 1))?;
            let nt = args.get(2).cloned().unwrap_or_else(|| target.clone());
            if !nt.is_callable() {
                return it.throw_error(
                    "TypeError",
                    "Reflect.construct newTarget is not a constructor",
                );
            }
            it.construct_with(&target, &list, &nt)
        });
        self.global.declare("Reflect", Value::Object(r), true);
    }
}
