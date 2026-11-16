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

/// Node's dotenv reading for `process.loadEnvFile`: `KEY=VALUE` lines with an optional `export ` prefix, `#`
/// comments, quoted values (' " `) that may span lines (`\n` expands inside double quotes), and unquoted values
/// cut at the first `#`. A line without `=` is skipped.
/// ponytail: an unclosed quote falls back to the unquoted rule (what Node does); the `\\n` expansion is the only escape.
pub(crate) fn parse_dotenv(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < src.len() {
        pos += src[pos..].len() - src[pos..].trim_start().len();
        if src[pos..].starts_with('#') {
            pos = src[pos..].find('\n').map_or(src.len(), |i| pos + i + 1);
            continue;
        }
        let line_end = src[pos..].find('\n').map_or(src.len(), |i| pos + i);
        let Some(eq) = src[pos..line_end].find('=') else {
            pos = line_end;
            continue;
        };
        let raw_key = src[pos..pos + eq].trim();
        let key = match raw_key.strip_prefix("export") {
            Some(tail) if tail.starts_with(char::is_whitespace) => tail.trim(),
            _ => raw_key,
        };
        let after_eq = pos + eq + 1;
        let value_start = after_eq
            + (src[after_eq..].len() - src[after_eq..].trim_start_matches([' ', '\t']).len());
        let quote = src[value_start..]
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"' | '`'));
        let closed = quote.and_then(|q| {
            src[value_start + 1..]
                .find(q)
                .map(|i| (q, value_start + 1 + i))
        });
        let value = match closed {
            Some((q, close)) => {
                pos = src[close..].find('\n').map_or(src.len(), |i| close + i);
                let body = &src[value_start + 1..close];
                if q == '"' {
                    body.replace("\\n", "\n")
                } else {
                    body.to_string()
                }
            }
            None => {
                pos = src[value_start..]
                    .find('\n')
                    .map_or(src.len(), |i| value_start + i);
                let line = &src[value_start..pos];
                line.split('#').next().unwrap_or("").trim().to_string()
            }
        };
        if !key.is_empty() {
            out.push((key.to_string(), value));
        }
    }
    out
}

/// `process.loadEnvFile` assignment: each parsed pair whose name is not already in the environment (a preset
/// value, even an empty one, wins over the file).
pub(crate) fn apply_dotenv(src: &str) {
    for (k, v) in parse_dotenv(src) {
        if env_get(&k).is_none() && settable(&k) && !v.contains('\0') {
            std::env::set_var(&k, &v);
        }
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
