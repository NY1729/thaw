//! `process.env`: a Proxy whose traps read and write the real process environment (std::env, which
//! is the libc environ with getenv/setenv/unsetenv). JS reads and writes, `in`, enumeration, `delete`
//! and the compiled `process.env.NAME` read (libc getenv) therefore share one table.
//! Inspection (`console.log(process.env)`) renders the live environment through `env_view`.

use crate::host::cstr;
use crate::proxy::ProxyData;
use crate::*;
use std::ffi::c_char;

/// Names setenv accepts: non-empty and without `=`.
fn settable(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.contains('\0')
}

fn env_get(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

fn key_of(it: &mut Interp, key: &Value) -> Option<String> {
    match key {
        Value::Str(_) => Some(it.to_str(key).to_string()),
        _ => None,
    }
}

impl Interp {
    /// A plain object holding the live environment in environ order: what `process.env` shows when inspected.
    pub(crate) fn process_env_view(&self) -> Obj {
        let view = self.plain();
        for (k, v) in std::env::vars_os() {
            put(
                &view,
                &k.to_string_lossy(),
                Value::str(&v.to_string_lossy()),
            );
        }
        view
    }

    /// The `process.env` object: a Proxy over an empty target whose traps answer from the environment.
    pub(crate) fn make_process_env(&self) -> Value {
        let target = self.plain();
        let proto = target.proto.borrow().clone();
        let handler = self.plain();
        self.set_native(&handler, "get", |it, _, a| {
            let Some(k) = key_of(it, &arg(a, 1)) else {
                return Ok(Value::Undefined);
            };
            match env_get(&k) {
                Some(v) => Ok(Value::str(&v)),
                // not in the environment: the prototype chain (hasOwnProperty, toString, ...)
                None => it.get(&arg(a, 0), &k),
            }
        });
        self.set_native(&handler, "set", |it, _, a| {
            if let Some(k) = key_of(it, &arg(a, 1)) {
                // Node stores every value as its string form: 5 -> '5', undefined -> 'undefined'
                let text = it.to_str(&arg(a, 2)).to_string();
                if settable(&k) && !text.contains('\0') {
                    std::env::set_var(&k, &text);
                }
            }
            Ok(Value::Bool(true))
        });
        self.set_native(&handler, "has", |it, _, a| {
            let Some(k) = key_of(it, &arg(a, 1)) else {
                return Ok(Value::Bool(false));
            };
            if env_get(&k).is_some() {
                return Ok(Value::Bool(true));
            }
            Ok(Value::Bool(it.has_property(&arg(a, 0), &k)?))
        });
        self.set_native(&handler, "deleteProperty", |it, _, a| {
            if let Some(k) = key_of(it, &arg(a, 1)) {
                if settable(&k) {
                    std::env::remove_var(&k);
                }
            }
            Ok(Value::Bool(true))
        });
        self.set_native(&handler, "ownKeys", |it, _, _| {
            let names = std::env::vars_os()
                .map(|(k, _)| Value::str(&k.to_string_lossy()))
                .collect();
            Ok(it.array(names))
        });
        self.set_native(&handler, "getOwnPropertyDescriptor", |it, _, a| {
            let Some(v) = key_of(it, &arg(a, 1)).and_then(|k| env_get(&k)) else {
                return Ok(Value::Undefined);
            };
            let d = it.plain();
            put(&d, "value", Value::str(&v));
            put(&d, "writable", Value::Bool(true));
            put(&d, "enumerable", Value::Bool(true));
            put(&d, "configurable", Value::Bool(true));
            Ok(Value::Object(d))
        });
        Value::Object(new_obj(
            proto,
            Kind::Proxy(Rc::new(ProxyData {
                target: Value::Object(target),
                handler: RefCell::new(Value::Object(handler)),
                callable: false,
                env_view: true,
            })),
        ))
    }
}

/// The routed build's `setProcessEnv(name, value)` (the compiled `ESBUILD_BINARY_PATH` shim, aliased
/// from `thaw_js_set_process_env`): writes the real environment, so it reads back through
/// `process.env` and the compiled getenv alike. 1 on success.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_process_env(name: *const c_char, value: *const c_char) -> u8 {
    let (Some(name), Some(value)) = (cstr(name), cstr(value)) else {
        return 0;
    };
    if !settable(&name) {
        return 0;
    }
    std::env::set_var(&name, &value);
    1
}
