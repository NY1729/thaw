//! Strict-mode write and delete failures: the V8 texts for a write or `delete` that sloppy code silently drops.
//!
//! `StrictMode` is the rollout switch, read from `THAW_JS_STRICT` (`off` | `audit` | `on`, default `on`).
//! Only code whose lexical strictness is on (a `'use strict'` directive, a class body, an ES module) can
//! fail. The public `Interp::set` is always sloppy, so natives, the host and the preludes keep their
//! behaviour; prelude runs (`run_builtin`) never throw even when the switch is on.

use crate::{array_index, class_instance_name, number_to_string, Interp, Kind, Obj, Value, R};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrictMode {
    /// Strict code keeps the sloppy silent failure (the rollback switch, `THAW_JS_STRICT=off`).
    Off,
    /// Behaves sloppy, logs every would-throw to stderr as `thaw-strict-audit: ...`.
    Audit,
    #[default]
    On,
}

impl StrictMode {
    /// `THAW_JS_STRICT`: `off` or `audit` (trimmed, lower case, like the compile switch); anything else,
    /// including unset, is `On`.
    pub(crate) fn parse(v: Option<&str>) -> StrictMode {
        match v.map(str::trim) {
            Some("off") => StrictMode::Off,
            Some("audit") => StrictMode::Audit,
            _ => StrictMode::On,
        }
    }
}

/// Why a write to an object was refused.
#[derive(Clone, Copy)]
pub(crate) enum Refusal {
    /// A non-writable data property, or a non-writable array `length`.
    ReadOnly,
    /// A new property on an object that is not extensible (or frozen).
    NotExtensible,
    /// An accessor without a setter.
    GetterOnly,
    /// A `length` truncation stopped at this non-configurable element (named by the key).
    NotDeletable,
}

impl Interp {
    /// Sets the rollout switch of this realm (the env var is read once, in `Interp::new`). For tests.
    #[cfg(test)]
    pub(crate) fn set_strict_mode(&mut self, mode: StrictMode) {
        self.strict_mode = mode;
    }

    /// Whether the running code is builtin (a prelude or a native's JS helper): its failures never throw.
    pub(crate) fn in_builtin(&self) -> bool {
        self.builtin_runs.contains(&self.run_id)
    }

    /// A refused write or delete in code of lexical strictness `strict`. Sloppy code ignores it (`Ok`).
    /// Strict code throws `class` with `msg()` when the switch is `On` and the code is user code; in `Audit`
    /// mode it logs the would-be throw and returns `Ok`.
    pub(crate) fn strict_fail(
        &mut self,
        strict: bool,
        class: &str,
        msg: impl FnOnce(&mut Interp) -> String,
    ) -> R<()> {
        if !strict {
            return Ok(());
        }
        match self.strict_mode {
            StrictMode::Off => Ok(()),
            StrictMode::Audit => {
                let text = msg(self);
                eprintln!(
                    "thaw-strict-audit: {class}: {text} builtin={}",
                    self.in_builtin()
                );
                Ok(())
            }
            StrictMode::On if self.in_builtin() => Ok(()),
            StrictMode::On => {
                let text = msg(self);
                self.throw_error(class, &text)
            }
        }
    }

    /// The source text of a script function, as `Function.prototype.toString` prints it (`None` otherwise).
    fn fn_source_text(&self, o: &Obj) -> Option<String> {
        let Kind::Closure { def, .. } = &*o.kind.borrow() else {
            return None;
        };
        let (run, lo, hi) = def.src;
        let src = self.sources.get(&run)?;
        src.get(lo.saturating_sub(1) as usize..hi.saturating_sub(1) as usize)
            .map(|t| t.trim_start().to_string())
    }

    /// V8's name for an object in a message: `[object Array]` for arrays, `#<Ctor>` for an instance of a named
    /// class, `#<Object>` as the fallback, and `[object Object]` for a null-prototype object.
    pub(crate) fn obj_name(&mut self, o: &Obj) -> String {
        if matches!(&*o.kind.borrow(), Kind::Array(_)) {
            return "[object Array]".into();
        }
        if o.proto.borrow().is_none() {
            return "[object Object]".into();
        }
        match class_instance_name(self, o) {
            Some(name) => format!("#<{name}>"),
            None => "#<Object>".into(),
        }
    }

    /// The text of a refused write to `key` on `o`, V8's wording for `why`.
    pub(crate) fn refusal_message(&mut self, o: &Obj, key: &str, why: Refusal) -> String {
        match why {
            Refusal::ReadOnly => match self.fn_source_text(o) {
                Some(text) => {
                    format!("Cannot assign to read only property '{key}' of function '{text}'")
                }
                None => format!(
                    "Cannot assign to read only property '{key}' of object '{}'",
                    self.obj_name(o)
                ),
            },
            Refusal::NotExtensible => {
                format!("Cannot add property {key}, object is not extensible")
            }
            Refusal::GetterOnly => format!(
                "Cannot set property {key} of {} which has only a getter",
                self.obj_name(o)
            ),
            Refusal::NotDeletable => {
                format!("Cannot delete property '{key}' of {}", self.obj_name(o))
            }
        }
    }

    /// A write to `key` on the object `o` that was refused, in code of lexical strictness `strict`.
    pub(crate) fn refuse_write(&mut self, strict: bool, o: &Obj, key: &str, why: Refusal) -> R<()> {
        self.strict_fail(strict, "TypeError", |it| it.refusal_message(o, key, why))
    }

    /// A write to `key` on a primitive `target` (a string index or `length`, or any other property, which
    /// would create one on the wrapper).
    // ponytail: primitive-prototype setters are not called (the write is refused as if there were none).
    pub(crate) fn refuse_primitive_write(
        &mut self,
        strict: bool,
        target: &Value,
        key: &str,
    ) -> R<()> {
        self.strict_fail(strict, "TypeError", |_| {
            let text = match target {
                Value::Str(s) => s.to_string(),
                Value::Number(n) => number_to_string(*n),
                Value::Bool(b) => b.to_string(),
                Value::BigInt(b) => b.to_string(),
                _ => String::new(),
            };
            let kind = match target {
                Value::Str(_) => "string",
                Value::Number(_) => "number",
                Value::Bool(_) => "boolean",
                _ => "bigint",
            };
            match target {
                Value::Str(s)
                    if key == "length" || array_index(key).is_some_and(|i| i < s.len16()) =>
                {
                    format!("Cannot assign to read only property '{key}' of string '{text}'")
                }
                _ => format!("Cannot create property '{key}' on {kind} '{text}'"),
            }
        })
    }

    /// A refused `delete` of `key` on `o` (a sealed or non-configurable property).
    pub(crate) fn refuse_delete(&mut self, strict: bool, o: &Obj, key: &str) -> R<()> {
        self.strict_fail(strict, "TypeError", |it| match it.fn_source_text(o) {
            Some(text) => format!("Cannot delete property '{key}' of {text}"),
            None => format!("Cannot delete property '{key}' of {}", it.obj_name(o)),
        })
    }
}
