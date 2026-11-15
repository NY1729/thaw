//! Legacy RegExp statics (`RegExp.$1`, `RegExp.lastMatch`, ...). The per-realm `LastMatch` is written
//! by `record_match` on every successful match (exec core, search, split); the statics are accessors
//! on the `RegExp` constructor, and their reads slice the recorded subject lazily.

use crate::builtins::regexp_flags;
use crate::{arg, define_accessor, find_own, Interp, JsStr, Kind, Obj, Slot, Value};
use std::ops::Range;
use std::rc::Rc;

/// The eight flag getters on `RegExp.prototype`, in the order `flags` reads them (the canonical order).
const FLAG_GETTERS: [(&str, char); 8] = [
    ("hasIndices", 'd'),
    ("global", 'g'),
    ("ignoreCase", 'i'),
    ("multiline", 'm'),
    ("dotAll", 's'),
    ("unicode", 'u'),
    ("unicodeSets", 'v'),
    ("sticky", 'y'),
];

/// The last successful match of a realm. `caps[0]` is the whole match; ranges are byte offsets into
/// `subject`. `input` is separate because `RegExp.input` is writable and must not move the subject.
pub(crate) struct LastMatch {
    subject: JsStr,
    input: JsStr,
    caps: Vec<Option<Range<usize>>>,
}

impl Default for LastMatch {
    fn default() -> Self {
        LastMatch {
            subject: "".into(),
            input: "".into(),
            caps: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
enum Legacy {
    Input,
    Last,
    LastParen,
    Left,
    Right,
    Group(usize),
}

impl Interp {
    /// Record a successful match. Clones the subject (a refcount bump) and reuses `caps`'s capacity.
    pub(crate) fn record_match(&mut self, s: &JsStr, m: &regress::Match) {
        let lm = &mut self.last_match;
        lm.subject = s.clone();
        lm.input = s.clone();
        lm.caps.clear();
        lm.caps.extend(m.groups());
    }

    fn legacy_value(&self, which: Legacy) -> Value {
        let lm = &self.last_match;
        let cap = |n: usize| match lm.caps.get(n) {
            Some(Some(r)) => Value::str(&lm.subject[r.clone()]),
            _ => Value::str(""),
        };
        match which {
            Legacy::Input => Value::Str(lm.input.clone()),
            Legacy::Last => cap(0),
            Legacy::LastParen if lm.caps.len() > 1 => cap(lm.caps.len() - 1),
            Legacy::LastParen => Value::str(""),
            Legacy::Left => match lm.caps.first() {
                Some(Some(r)) => Value::str(&lm.subject[..r.start]),
                _ => Value::str(""),
            },
            Legacy::Right => match lm.caps.first() {
                Some(Some(r)) => Value::str(&lm.subject[r.end..]),
                _ => Value::str(""),
            },
            Legacy::Group(n) => cap(n),
        }
    }

    /// Define the 19 legacy accessors on the `RegExp` constructor. Only `input`/`$_` take a value.
    pub(crate) fn install_legacy_statics(&mut self, regexp: &Obj) {
        let fixed = [
            ("input", Legacy::Input),
            ("$_", Legacy::Input),
            ("lastMatch", Legacy::Last),
            ("$&", Legacy::Last),
            ("lastParen", Legacy::LastParen),
            ("$+", Legacy::LastParen),
            ("leftContext", Legacy::Left),
            ("$`", Legacy::Left),
            ("rightContext", Legacy::Right),
            ("$'", Legacy::Right),
        ];
        let numbered = (1..=9).map(|n| (format!("${n}"), Legacy::Group(n)));
        let all = fixed
            .iter()
            .map(|(k, w)| (k.to_string(), *w))
            .chain(numbered);
        for (key, which) in all {
            let get = self.native(move |it, _, _| Ok(it.legacy_value(which)));
            let set = self.native(move |it, _, args| {
                if let Legacy::Input = which {
                    it.last_match.input = it.to_str(&arg(args, 0));
                }
                Ok(Value::Undefined)
            });
            define_accessor(regexp, &key, Some(get), Some(set), false);
        }
    }

    /// `RegExp.prototype`'s flag, `source` and `flags` getters (no setters). A RegExp receiver reads its
    /// own slots; the prototype itself gives `undefined` / `(?:)` / ""; anything else is a TypeError.
    pub(crate) fn install_regexp_proto_accessors(&mut self, rp: &Obj) {
        for (key, flag) in FLAG_GETTERS {
            let proto = rp.clone();
            let get = self.native(move |it, this, _| match this {
                Value::Object(o) if matches!(&*o.kind.borrow(), Kind::RegExp(_)) => {
                    Ok(Value::Bool(regexp_flags(o).contains(flag)))
                }
                Value::Object(o) if Rc::ptr_eq(o, &proto) => Ok(Value::Undefined),
                _ => it.throw_error(
                    "TypeError",
                    &format!("RegExp.prototype.{key} getter called on non-RegExp object"),
                ),
            });
            define_accessor(rp, key, Some(get), None, false);
        }
        let proto = rp.clone();
        let source = self.native(move |it, this, _| match this {
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::RegExp(_)) => {
                Ok(match find_own(o, "source") {
                    Some((Slot::Data(v), _)) => v,
                    _ => Value::str("(?:)"),
                })
            }
            Value::Object(o) if Rc::ptr_eq(o, &proto) => Ok(Value::str("(?:)")),
            _ => it.throw_error(
                "TypeError",
                "RegExp.prototype.source getter called on non-RegExp object",
            ),
        });
        define_accessor(rp, "source", Some(source), None, false);
        let flags = self.native(|it, this, _| {
            if !matches!(this, Value::Object(_)) {
                let shown = it.to_str(this);
                return it.throw_error(
                    "TypeError",
                    &format!("RegExp.prototype.flags getter called on non-object {shown}"),
                );
            }
            let mut out = String::new();
            for (key, flag) in FLAG_GETTERS {
                if it.get(this, key)?.truthy() {
                    out.push(flag);
                }
            }
            Ok(Value::str(&out))
        });
        define_accessor(rp, "flags", Some(flags), None, false);
    }
}
