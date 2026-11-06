//! `JsStr`: the immutable string payload of `Value::Str`. Step 1 is a plain newtype over
//! `Rc<str>`; the representation may grow later without changing this API.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct JsStr(Rc<str>);

impl JsStr {
    /// The shared `Rc<str>` for this string (used for property keys and names).
    #[inline]
    pub fn into_rc(&self) -> Rc<str> {
        self.0.clone()
    }

    /// True when both handles share one allocation (a fast path only; equality is by content).
    #[inline]
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
}

impl Deref for JsStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for JsStr {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for JsStr {
    #[inline]
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl From<&str> for JsStr {
    #[inline]
    fn from(s: &str) -> Self {
        JsStr(Rc::from(s))
    }
}

impl From<String> for JsStr {
    #[inline]
    fn from(s: String) -> Self {
        JsStr(Rc::from(s))
    }
}

impl From<Rc<str>> for JsStr {
    #[inline]
    fn from(s: Rc<str>) -> Self {
        JsStr(s)
    }
}
