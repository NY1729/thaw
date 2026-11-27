//! `process` is an EventEmitter instance, as in Node 22: `process` -> `process.prototype` ->
//! `EventEmitter.prototype` -> `Object.prototype`. The listener state is Node's, on the object itself:
//! `_events[name]` is one listener or an array of them, `_eventsCount` the number of names, and
//! `_maxListeners` the limit. The methods are natives on the emitter prototype: a JS prelude would be
//! parsed in every realm. The Rust hooks in eventloop.rs read the state with `listener_count` and
//! dispatch through the `emit` property, so a patched `process.emit` is seen, as in Node.
//! ponytail: the Node default `newListener`/`removeListener`/`warning` entries are no-ops and there are no `EventEmitter`
//! statics (`defaultMaxListeners`, `once`); add them when a package needs them.

use crate::eventloop::node_error;
use crate::{
    arg, find_own, new_obj, number_to_string, put, put_hidden, put_with, strict_equals, Ctl,
    Interp, Kind, Obj, Slot, Value, R,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Node's `EventEmitter.defaultMaxListeners`.
const DEFAULT_MAX_LISTENERS: f64 = 10.0;

impl Interp {
    /// Make `p` an EventEmitter: its prototype chain, and its own `_events`, `_eventsCount` and `_maxListeners`.
    pub(crate) fn install_process_events(&mut self, p: &Obj) {
        let base = self.plain();
        let ctor = self.native(|_, this, _| {
            if let Value::Object(o) = this {
                events_of(o);
            }
            Ok(Value::Undefined)
        });
        name_and_length(&ctor, "EventEmitter", 1.0);
        put_hidden(&base, "constructor", ctor.clone());
        if let Value::Object(c) = &ctor {
            put_hidden(c, "prototype", Value::Object(base.clone()));
        }
        put(&base, "_events", Value::Undefined);
        put(&base, "_eventsCount", Value::Number(0.0));
        put(&base, "_maxListeners", Value::Undefined);

        self.emitter_method(&base, &["addListener", "on"], 2.0, |it, t, a| {
            add_listener(it, t, a, false, false)
        });
        self.emitter_method(&base, &["prependListener"], 2.0, |it, t, a| {
            add_listener(it, t, a, true, false)
        });
        self.emitter_method(&base, &["once"], 2.0, |it, t, a| {
            add_listener(it, t, a, false, true)
        });
        self.emitter_method(&base, &["prependOnceListener"], 2.0, |it, t, a| {
            add_listener(it, t, a, true, true)
        });
        self.emitter_method(&base, &["removeListener", "off"], 2.0, remove_listener);
        self.emitter_method(&base, &["removeAllListeners"], 1.0, remove_all_listeners);
        self.emitter_method(&base, &["emit"], 1.0, emit_event);
        self.emitter_method(&base, &["listeners"], 1.0, |it, t, a| {
            listeners_of(it, t, a, true)
        });
        self.emitter_method(&base, &["rawListeners"], 1.0, |it, t, a| {
            listeners_of(it, t, a, false)
        });
        self.emitter_method(&base, &["listenerCount"], 2.0, listener_count_method);
        self.emitter_method(&base, &["eventNames"], 0.0, event_names);
        self.emitter_method(&base, &["setMaxListeners"], 1.0, set_max_listeners);
        self.emitter_method(&base, &["getMaxListeners"], 0.0, |it, t, _| {
            Ok(Value::Number(max_listeners(it, t)?))
        });

        // `process.prototype`, whose `constructor` is the `process` function
        let process_proto = new_obj(Some(base), Kind::Plain);
        let process_ctor = self.native(|_, _, _| Ok(Value::Undefined));
        name_and_length(&process_ctor, "process", 0.0);
        put_hidden(&process_proto, "constructor", process_ctor.clone());
        if let Value::Object(c) = &process_ctor {
            put_hidden(c, "prototype", Value::Object(process_proto.clone()));
        }
        *p.proto.borrow_mut() = Some(process_proto);
        // Node's own start-up listeners (signal wiring and the warning printer), as no-ops here.
        let events = new_obj(None, Kind::Plain);
        for (name, count) in [("newListener", 2), ("removeListener", 2), ("warning", 1)] {
            let hooks: Vec<Value> = (0..count)
                .map(|_| self.native(|_, _, _| Ok(Value::Undefined)))
                .collect();
            let value = if count == 1 {
                hooks[0].clone()
            } else {
                self.array(hooks)
            };
            if count == 1 {
                name_and_length(&value, "onWarning", 1.0);
            }
            put(&events, name, value);
        }
        put(p, "_events", Value::Object(events));
        put(p, "_eventsCount", Value::Number(3.0));
        put(p, "_maxListeners", Value::Undefined);
    }

    /// A native method on the emitter prototype under each of `names` (the first one is its `name`).
    fn emitter_method(
        &self,
        base: &Obj,
        names: &[&str],
        length: f64,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> R<Value> + 'static,
    ) {
        let v = self.native(f);
        name_and_length(&v, names[0], length);
        // enumerable, as Node's `EventEmitter.prototype.x = ...` assignments are
        for name in names {
            put(base, name, v.clone());
        }
    }
}

/// Node's own `name` and `length` on a native function (both read-only, non-enumerable).
fn name_and_length(f: &Value, name: &str, length: f64) {
    if let Value::Object(o) = f {
        put_with(o, "length", Value::Number(length), false, false, true);
        put_with(o, "name", Value::str(name), false, false, true);
    }
}

/// `target`'s own `_events` object, when it has one.
pub(crate) fn own_events(target: &Obj) -> Option<Obj> {
    match find_own(target, "_events") {
        Some((Slot::Data(Value::Object(e)), _)) => Some(e),
        _ => None,
    }
}

/// An own data property's value (`None` for an absent or accessor property).
pub(crate) fn own_value(o: &Obj, key: &str) -> Option<Value> {
    match find_own(o, key) {
        Some((Slot::Data(v), _)) => Some(v),
        _ => None,
    }
}

/// `target._events`, created with `_eventsCount` 0 when there is none (Node's `_addListener` does the same).
fn events_of(target: &Obj) -> Obj {
    if let Some(e) = own_events(target) {
        return e;
    }
    let e = new_obj(None, Kind::Plain);
    put(target, "_events", Value::Object(e.clone()));
    put(target, "_eventsCount", Value::Number(0.0));
    e
}

fn events_count(target: &Obj) -> f64 {
    match own_value(target, "_eventsCount") {
        Some(Value::Number(n)) => n,
        _ => 0.0,
    }
}

/// Node's `_events = Object.create(null); _eventsCount = 0`.
fn reset_events(target: &Obj) {
    put(target, "_events", Value::Object(new_obj(None, Kind::Plain)));
    put(target, "_eventsCount", Value::Number(0.0));
}

/// Node drops a name with no listener left (the last one resets `_events` entirely).
fn remove_key(target: &Obj, key: &str) {
    put(
        target,
        "_eventsCount",
        Value::Number(events_count(target) - 1.0),
    );
    match own_events(target) {
        Some(events) if events_count(target) > 0.0 => events.remove_prop(key),
        _ => reset_events(target),
    }
}

/// The listeners `key` has, as stored: one function, or the array of them.
fn listeners_list(it: &mut Interp, target: &Obj, key: &str) -> R<Vec<Value>> {
    match own_events(target).and_then(|e| own_value(&e, key)) {
        Some(list) if list.is_callable() => Ok(vec![list]),
        Some(list) => {
            it.sparse_unsupported(&list)?;
            Ok(it.array_items(&list))
        }
        None => Ok(vec![]),
    }
}

/// A `once` wrapper's `listener`, or the function itself (Node's `fn.listener || fn`).
fn unwrap_listener(f: &Value) -> Value {
    if let Value::Object(o) = f {
        if let Some(l) = own_value(o, "listener") {
            if l.is_callable() {
                return l;
            }
        }
    }
    f.clone()
}

/// Node's `removeListener` match: the listener itself, or a `once` wrapper of it.
fn is_listener(f: &Value, listener: &Value) -> bool {
    strict_equals(f, listener) || strict_equals(&unwrap_listener(f), listener)
}

/// Whether `target` has a `name` listener, as Node's `events.name !== undefined` checks.
fn has_hook(target: &Obj, name: &str) -> bool {
    own_events(target)
        .and_then(|e| own_value(&e, name))
        .is_some_and(|v| !matches!(v, Value::Undefined))
}

/// `this[name](...args)` through the property chain, as Node's internals call methods.
pub(crate) fn invoke(it: &mut Interp, this: &Value, name: &str, args: &[Value]) -> R<Value> {
    let f = it.get(this, name)?;
    it.call(&f, this, args)
}

/// The emitter method's receiver.
fn receiver(it: &mut Interp, this: &Value) -> R<Obj> {
    match this {
        Value::Object(o) => Ok(o.clone()),
        _ => it.throw_error("TypeError", "Illegal invocation"),
    }
}

/// Node's "Received ..." phrase for a bad argument.
fn describe(it: &mut Interp, v: &Value) -> String {
    match v {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(_) => format!("type boolean ({})", it.inspect(v, false)),
        Value::Number(_) => format!("type number ({})", it.inspect(v, false)),
        Value::Str(_) => format!("type string ({})", it.inspect(v, false)),
        _ => "an instance of Object".into(),
    }
}

/// Node's `checkListener`.
fn check_listener(it: &mut Interp, v: &Value) -> R<()> {
    if v.is_callable() {
        return Ok(());
    }
    let received = describe(it, v);
    Err(node_error(
        it,
        "TypeError",
        "ERR_INVALID_ARG_TYPE",
        &format!("The \"listener\" argument must be of type function. Received {received}"),
    ))
}

/// Node's `_addListener`: `prepend` puts the listener first, `once` wraps it to fire once.
fn add_listener(it: &mut Interp, this: &Value, a: &[Value], prepend: bool, once: bool) -> R<Value> {
    let target = receiver(it, this)?;
    let kind = arg(a, 0);
    let key = it.key_of(&kind);
    let listener = arg(a, 1);
    check_listener(it, &listener)?;
    // `newListener` fires before the listener is added, and sees the listener as given (not a once wrapper)
    if has_hook(&target, "newListener") {
        invoke(
            it,
            this,
            "emit",
            &[Value::str("newListener"), kind.clone(), listener.clone()],
        )?;
    }
    let events = events_of(&target);
    let stored = if once {
        once_wrapper(it, this, &kind, &listener)
    } else {
        listener
    };
    match own_value(&events, &key) {
        None => {
            put(&events, &key, stored);
            put(
                &target,
                "_eventsCount",
                Value::Number(events_count(&target) + 1.0),
            );
        }
        Some(existing) if existing.is_callable() => {
            let items = if prepend {
                vec![stored, existing]
            } else {
                vec![existing, stored]
            };
            put(&events, &key, it.array(items));
        }
        Some(Value::Object(arr)) => {
            // Node mutates the array in place, so its `warned` flag outlives the add
            if let Kind::Array(items) = &mut *arr.kind.borrow_mut() {
                if prepend {
                    items.insert(0, stored);
                } else {
                    items.push(stored);
                }
            }
        }
        Some(_) => {}
    }
    if let Some(Value::Object(arr)) = own_value(&events, &key) {
        let len = match &*arr.kind.borrow() {
            Kind::Array(items) => items.len(),
            _ => 0,
        };
        let warned = matches!(own_value(&arr, "warned"), Some(Value::Bool(true)));
        let max = max_listeners(it, this)?;
        if max > 0.0 && len as f64 > max && !warned {
            put(&arr, "warned", Value::Bool(true));
            warn_max_listeners(it, this, &kind, len, max)?;
        }
    }
    Ok(this.clone())
}

/// Node's `MaxListenersExceededWarning`, sent through the global `process.emitWarning`.
fn warn_max_listeners(
    it: &mut Interp,
    target: &Value,
    kind: &Value,
    count: usize,
    max: f64,
) -> R<()> {
    let ctor = it.get(target, "constructor")?;
    let class = if ctor.is_callable() {
        it.get(&ctor, "name")?
    } else {
        Value::str("Object")
    };
    let class = it.to_str(&class);
    let kind = it.to_str(kind);
    let msg = format!(
        "Possible EventEmitter memory leak detected. {count} {kind} listeners added to [{class}]. MaxListeners is {max}. Use emitter.setMaxListeners() to increase limit"
    );
    let warning = it.make_error("Error", &msg)?;
    if let Value::Object(w) = &warning {
        put(w, "name", Value::str("MaxListenersExceededWarning"));
        put(w, "emitter", target.clone());
        put(w, "type", Value::Str(kind.clone()));
        put(w, "count", Value::Number(count as f64));
    }
    if let Some(process) = it.lookup(&it.global.clone(), "process") {
        invoke(it, &process, "emitWarning", &[warning])?;
    }
    Ok(())
}

/// Node's `once` wrapper: the first call removes the wrapper, then calls the listener with the emitter as `this`.
fn once_wrapper(it: &Interp, target: &Value, kind: &Value, listener: &Value) -> Value {
    let fired = Cell::new(false);
    // ponytail: the wrapper holds itself through this cell, a cycle per `once` registration; weak-ref it if once-heavy code shows up
    let me: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Undefined));
    let (emitter, kind_arg, inner, me_slot) =
        (target.clone(), kind.clone(), listener.clone(), me.clone());
    let wrapper = it.native(move |it, _, args| {
        if fired.replace(true) {
            return Ok(Value::Undefined);
        }
        let this_wrapper = me_slot.borrow().clone();
        invoke(
            it,
            &emitter,
            "removeListener",
            &[kind_arg.clone(), this_wrapper],
        )?;
        it.call(&inner, &emitter, args)
    });
    *me.borrow_mut() = wrapper.clone();
    if let Value::Object(w) = &wrapper {
        put(w, "listener", listener.clone());
    }
    wrapper
}

/// Node's `removeListener`: the last matching listener goes; `removeListener` fires after it.
fn remove_listener(it: &mut Interp, this: &Value, a: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let kind = arg(a, 0);
    let key = it.key_of(&kind);
    let listener = arg(a, 1);
    check_listener(it, &listener)?;
    let Some(events) = own_events(&target) else {
        return Ok(this.clone());
    };
    let Some(list) = own_value(&events, &key) else {
        return Ok(this.clone());
    };
    if list.is_callable() {
        if !is_listener(&list, &listener) {
            return Ok(this.clone());
        }
        remove_key(&target, &key);
        if has_hook(&target, "removeListener") {
            let fired = unwrap_listener(&list);
            invoke(
                it,
                this,
                "emit",
                &[Value::str("removeListener"), kind, fired],
            )?;
        }
    } else {
        it.sparse_unsupported(&list)?;
        let mut items = it.array_items(&list);
        let Some(pos) = items.iter().rposition(|f| is_listener(f, &listener)) else {
            return Ok(this.clone());
        };
        items.remove(pos);
        if let Value::Object(arr) = &list {
            if let Kind::Array(v) = &mut *arr.kind.borrow_mut() {
                v.remove(pos);
            }
        }
        if items.len() == 1 {
            put(&events, &key, items[0].clone());
        }
        if has_hook(&target, "removeListener") {
            invoke(
                it,
                this,
                "emit",
                &[Value::str("removeListener"), kind, listener],
            )?;
        }
    }
    Ok(this.clone())
}

/// Node's `removeAllListeners`: for one name, remove its listeners (last first); with none given, every name.
fn remove_all_listeners(it: &mut Interp, this: &Value, a: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let Some(events) = own_events(&target) else {
        return Ok(this.clone());
    };
    if a.is_empty() {
        for k in it.own_keys(&Value::Object(events)) {
            if &*k != "removeListener" {
                let name = it.key_value(&k);
                invoke(it, this, "removeAllListeners", &[name])?;
            }
        }
        invoke(
            it,
            this,
            "removeAllListeners",
            &[Value::str("removeListener")],
        )?;
        reset_events(&target);
    } else {
        let key = it.key_of(&a[0]);
        for f in listeners_list(it, &target, &key)?.iter().rev() {
            invoke(it, this, "removeListener", &[a[0].clone(), f.clone()])?;
        }
    }
    Ok(this.clone())
}

/// Node's `emit`: calls the listeners of `name` with `this` as the emitter; `error` with none throws.
fn emit_event(it: &mut Interp, this: &Value, a: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let key = it.key_of(&arg(a, 0));
    let list = listeners_list(it, &target, &key)?;
    if list.is_empty() {
        if &*key == "error" {
            let er = arg(a, 1);
            if let Value::Object(o) = &er {
                if crate::builtins::is_error_instance(o) {
                    return Err(Ctl::Throw(er));
                }
            }
            let shown = it.inspect(&er, false);
            return Err(node_error(
                it,
                "Error",
                "ERR_UNHANDLED_ERROR",
                &format!("Unhandled error. ({shown})"),
            ));
        }
        return Ok(Value::Bool(false));
    }
    let args = a.get(1..).unwrap_or(&[]);
    for f in &list {
        it.call(f, this, args)?;
    }
    Ok(Value::Bool(true))
}

/// Node's `listeners` (unwrapped) and `rawListeners` (as stored).
fn listeners_of(it: &mut Interp, this: &Value, a: &[Value], unwrap: bool) -> R<Value> {
    let target = receiver(it, this)?;
    let key = it.key_of(&arg(a, 0));
    let list = listeners_list(it, &target, &key)?;
    let out = if unwrap {
        list.iter().map(unwrap_listener).collect()
    } else {
        list
    };
    Ok(it.array(out))
}

/// Node's `listenerCount(type[, listener])`.
fn listener_count_method(it: &mut Interp, this: &Value, a: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let key = it.key_of(&arg(a, 0));
    let list = listeners_list(it, &target, &key)?;
    let wanted = arg(a, 1);
    let n = if matches!(wanted, Value::Undefined | Value::Null) {
        list.len()
    } else {
        list.iter().filter(|f| is_listener(f, &wanted)).count()
    };
    Ok(Value::Number(n as f64))
}

/// Node's `eventNames`: the keys of `_events`, as property keys or symbols.
fn event_names(it: &mut Interp, this: &Value, _: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let keys = match own_events(&target) {
        Some(events) => it.own_keys(&Value::Object(events)),
        None => vec![],
    };
    let names = keys.iter().map(|k| it.key_value(k)).collect();
    Ok(it.array(names))
}

/// The limit on listeners per name: `_maxListeners`, or the default when it is undefined.
fn max_listeners(it: &mut Interp, target: &Value) -> R<f64> {
    Ok(match it.get(target, "_maxListeners")? {
        Value::Number(n) => n,
        _ => DEFAULT_MAX_LISTENERS,
    })
}

/// Node's `setMaxListeners(n)`, validated as `validateNumber(n, 'setMaxListeners', 0)`.
fn set_max_listeners(it: &mut Interp, this: &Value, a: &[Value]) -> R<Value> {
    let target = receiver(it, this)?;
    let n = arg(a, 0);
    let Value::Number(x) = n else {
        let received = describe(it, &n);
        return Err(node_error(
            it,
            "TypeError",
            "ERR_INVALID_ARG_TYPE",
            &format!(
                "The \"setMaxListeners\" argument must be of type number. Received {received}"
            ),
        ));
    };
    if x.is_nan() || x < 0.0 {
        return Err(node_error(
            it,
            "RangeError",
            "ERR_OUT_OF_RANGE",
            &format!(
                "The value of \"setMaxListeners\" is out of range. It must be >= 0. Received {}",
                number_to_string(x)
            ),
        ));
    }
    put(&target, "_maxListeners", Value::Number(x));
    Ok(this.clone())
}
