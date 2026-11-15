//! Legacy RegExp statics (`RegExp.$1`, `RegExp.lastMatch`, ...). The per-realm `LastMatch` is written
//! by `record_match` on every successful match (exec core, search, split); the statics are accessors
//! on the `RegExp` constructor, and their reads slice the recorded subject lazily.

use crate::{arg, define_accessor, Interp, JsStr, Obj, Value};
use std::ops::Range;

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
}
