//! `JsStr`: the immutable string payload of `Value::Str`. Short strings are a plain shared
//! `Rc<str>`; long strings own a `String` (spare capacity for later appends) plus a cached
//! ASCII flag. The representation is private, so the API does not change with it.
//!
//! `push_joined` (`s += t`) appends in place only when the `Rc` is unique, so an aliased string is
//! never mutated: a shared value is copied once, into a fresh buffer with slack.

use std::borrow::Borrow;
use std::cell::Cell;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

/// Strings at or above this many bytes use the `Buf` representation.
const BUF_MIN: usize = 256;

#[derive(Clone)]
enum Repr {
    /// `len < BUF_MIN`, or a long string that arrived as an `Rc<str>` (no way to take its slack).
    Plain(Rc<str>),
    Buf(Rc<BufStr>),
}

struct BufStr {
    /// 0 = not yet computed, 1 = ASCII, 2 = not ASCII.
    ascii: Cell<u8>,
    s: String,
}

#[derive(Clone)]
pub struct JsStr(Repr);

impl JsStr {
    #[inline]
    fn buf(s: String) -> Self {
        JsStr(Repr::Buf(Rc::new(BufStr {
            ascii: Cell::new(0),
            s,
        })))
    }

    /// The shared `Rc<str>` for this string (used for property keys and names).
    /// A `Buf` is copied here: its `String` may have spare capacity and an `Rc<str>` cannot share it.
    #[inline]
    pub fn into_rc(&self) -> Rc<str> {
        match &self.0 {
            Repr::Plain(r) => r.clone(),
            Repr::Buf(b) => Rc::from(b.s.as_str()),
        }
    }

    /// True when both handles share one allocation (a fast path only; equality is by content).
    #[inline]
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        match (&a.0, &b.0) {
            (Repr::Plain(x), Repr::Plain(y)) => Rc::ptr_eq(x, y),
            (Repr::Buf(x), Repr::Buf(y)) => Rc::ptr_eq(x, y),
            _ => false,
        }
    }

    /// `str::is_ascii`, memoised on the string for a long `Buf` (computed once, then O(1)).
    #[inline]
    pub(crate) fn is_ascii_cached(&self) -> bool {
        match &self.0 {
            Repr::Plain(r) => scan_ascii(r),
            Repr::Buf(b) => match b.ascii.get() {
                1 => true,
                2 => false,
                _ => {
                    let a = scan_ascii(&b.s);
                    b.ascii.set(if a { 1 } else { 2 });
                    a
                }
            },
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Full-buffer copies made by `push_joined` on this thread (a test-only work counter).
    static COPIES: Cell<usize> = const { Cell::new(0) };
}

/// Full-buffer copies `push_joined` has made on this thread (test builds only).
#[cfg(test)]
pub(crate) fn copies_made() -> usize {
    COPIES.with(Cell::get)
}

#[cfg(test)]
thread_local! {
    /// Full O(n) `is_ascii` scans of string payloads on this thread (a test-only work counter).
    static ASCII_SCANS: Cell<usize> = const { Cell::new(0) };
    /// Chars visited by UTF-16 index walks on this thread (a test-only work counter).
    static CHAR_WALK_STEPS: Cell<usize> = const { Cell::new(0) };
}

/// Full ASCII scans made on this thread (test builds only).
#[cfg(test)]
pub(crate) fn ascii_scans() -> usize {
    ASCII_SCANS.with(Cell::get)
}

/// Char steps of index walks made on this thread (test builds only).
#[cfg(test)]
pub(crate) fn char_walk_steps() -> usize {
    CHAR_WALK_STEPS.with(Cell::get)
}

/// `s.is_ascii()`, counted in test builds. Every O(n) ASCII scan goes through here.
#[inline]
pub(crate) fn scan_ascii(s: &str) -> bool {
    #[cfg(test)]
    ASCII_SCANS.with(|c| c.set(c.get() + 1));
    s.is_ascii()
}

/// One char visited by a UTF-16 index walk (counted in test builds only).
#[inline]
pub(crate) fn walk_step() {
    #[cfg(test)]
    CHAR_WALK_STEPS.with(|c| c.set(c.get() + 1));
}

impl JsStr {
    /// `self += t`, with the lone-surrogate seam rule. A uniquely owned `Buf` grows in place
    /// (amortised O(t)); any other string is copied once into a buffer with slack and owned here.
    pub(crate) fn push_joined(&mut self, t: &str) {
        let unique = match &mut self.0 {
            Repr::Buf(rc) => Rc::get_mut(rc),
            Repr::Plain(_) => None,
        };
        if let Some(b) = unique {
            crate::builtins::push_joined(&mut b.s, t);
            // appending can only turn an ASCII string into a non-ASCII one, never the reverse
            b.ascii.set(match b.ascii.get() {
                1 if !t.is_ascii() => 2,
                other => other,
            });
            return;
        }
        #[cfg(test)]
        COPIES.with(|c| c.set(c.get() + 1));
        let len = self.len() + t.len();
        let mut s = String::with_capacity(if len < BUF_MIN { len } else { 2 * len });
        s.push_str(self);
        crate::builtins::push_joined(&mut s, t);
        *self = JsStr::from(s);
    }
}

impl Deref for JsStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        match &self.0 {
            Repr::Plain(r) => r,
            Repr::Buf(b) => b.s.as_str(),
        }
    }
}

impl PartialEq for JsStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        JsStr::ptr_eq(self, other) || **self == **other
    }
}

impl Eq for JsStr {}

impl Hash for JsStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Must hash exactly like `str` for the `Borrow<str>` lookups to work.
        Hash::hash(&**self, state)
    }
}

impl AsRef<str> for JsStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self
    }
}

impl Borrow<str> for JsStr {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl From<&str> for JsStr {
    #[inline]
    fn from(s: &str) -> Self {
        if s.len() < BUF_MIN {
            JsStr(Repr::Plain(Rc::from(s)))
        } else {
            JsStr::buf(s.to_owned())
        }
    }
}

impl From<String> for JsStr {
    /// A long string is moved in, not copied.
    #[inline]
    fn from(s: String) -> Self {
        if s.len() < BUF_MIN {
            JsStr(Repr::Plain(Rc::from(s)))
        } else {
            JsStr::buf(s)
        }
    }
}

impl From<Rc<str>> for JsStr {
    #[inline]
    fn from(s: Rc<str>) -> Self {
        JsStr(Repr::Plain(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::collections::HashMap;

    fn is_plain(s: &JsStr) -> bool {
        matches!(s.0, Repr::Plain(_))
    }

    fn hash_of<T: Hash + ?Sized>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn jsstr_thresholds_pick_the_representation() {
        let short = "a".repeat(BUF_MIN - 1);
        let long = "a".repeat(BUF_MIN);
        assert!(is_plain(&JsStr::from(short.as_str())));
        assert!(is_plain(&JsStr::from(short.clone())));
        assert!(!is_plain(&JsStr::from(long.as_str())));
        assert!(!is_plain(&JsStr::from(long.clone())));
        // an Rc<str> is always Plain, even when long
        assert!(is_plain(&JsStr::from(Rc::<str>::from(long.as_str()))));
    }

    #[test]
    fn jsstr_deref_and_views_agree_for_both_reprs() {
        for text in ["", "abc", "é😀", &"xé".repeat(200)] {
            let plain = JsStr::from(Rc::<str>::from(text));
            let owned = JsStr::from(text.to_owned());
            for s in [&plain, &owned] {
                assert_eq!(&**s, text);
                assert_eq!(s.len(), text.len());
                assert_eq!(s.to_string(), text);
                assert_eq!(format!("{s:?}"), format!("{text:?}"));
                assert_eq!(&*s.into_rc(), text);
            }
        }
    }

    #[test]
    fn jsstr_eq_hash_borrow_match_str() {
        let long = "k".repeat(300);
        let a = JsStr::from(long.clone());
        let b = JsStr::from(Rc::<str>::from(long.as_str()));
        let c = JsStr::from(format!("{long}!"));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(!JsStr::ptr_eq(&a, &b));
        assert_eq!(hash_of(&a), hash_of(long.as_str()));
        assert_eq!(hash_of(&b), hash_of(long.as_str()));
        assert_eq!(hash_of(&JsStr::from("x")), hash_of("x"));

        // the cached ASCII flag is the only interior state; Hash/Eq use the contents
        #[allow(clippy::mutable_key_type)]
        let mut map: HashMap<JsStr, u32> = HashMap::new();
        map.insert(a.clone(), 7);
        assert_eq!(map.get(long.as_str()), Some(&7));
        assert_eq!(map.get(&b), Some(&7));
        assert_eq!(map.get(&c), None);
        let short: JsStr = "hi".into();
        map.insert(short, 1);
        assert_eq!(map.get("hi"), Some(&1));
    }

    #[test]
    fn jsstr_ptr_eq_is_identity_within_a_repr() {
        let a = JsStr::from("x".repeat(300));
        let a2 = a.clone();
        assert!(JsStr::ptr_eq(&a, &a2));
        let other = JsStr::from("x".repeat(300));
        assert!(!JsStr::ptr_eq(&a, &other));
        assert_eq!(a, other);
    }

    #[test]
    fn jsstr_ascii_flag_is_correct_for_ascii_bmp_astral() {
        let short_ascii: JsStr = "abc".into();
        let short_bmp: JsStr = "é".into();
        assert!(short_ascii.is_ascii_cached());
        assert!(!short_bmp.is_ascii_cached());

        let ascii = JsStr::from("a".repeat(300));
        let bmp = JsStr::from(format!("{}é", "a".repeat(299)));
        let astral = JsStr::from(format!("{}😀", "a".repeat(299)));
        let lone_at_end = JsStr::from(format!("{}z", "a".repeat(299)));
        // repeated calls answer from the cache and stay correct
        for _ in 0..2 {
            assert!(ascii.is_ascii_cached());
            assert!(!bmp.is_ascii_cached());
            assert!(!astral.is_ascii_cached());
            assert!(lone_at_end.is_ascii_cached());
        }
        assert_eq!(ascii.is_ascii_cached(), ascii.is_ascii());
        assert_eq!(bmp.is_ascii_cached(), bmp.is_ascii());
    }

    #[test]
    fn jsstr_ascii_flag_is_per_string() {
        let ascii = JsStr::from("a".repeat(300));
        let mixed = JsStr::from(format!("{}é", "a".repeat(299)));
        assert!(ascii.is_ascii_cached());
        assert!(!mixed.is_ascii_cached());
        assert!(ascii.clone().is_ascii_cached());
    }

    #[test]
    fn jsstr_from_string_moves_the_buffer() {
        let mut s = String::with_capacity(1024);
        s.push_str(&"m".repeat(300));
        let p = s.as_ptr();
        let j = JsStr::from(s);
        assert_eq!(j.as_ptr(), p);
        assert_eq!(j.len(), 300);
        assert!(!is_plain(&j));
    }

    #[test]
    fn jsstr_push_joined_copies_only_when_shared() {
        let base = copies_made();
        let mut a = JsStr::from("a".repeat(300));
        a.push_joined("b");
        assert_eq!(copies_made(), base, "a unique buffer grows in place");
        let keep = a.clone();
        a.push_joined("c");
        assert_eq!(copies_made(), base + 1, "a shared buffer is copied once");
        assert_eq!(&*keep, format!("{}b", "a".repeat(300)));
        assert_eq!(&*a, format!("{}bc", "a".repeat(300)));
        assert!(!is_plain(&keep) && !is_plain(&a));
    }

    #[test]
    fn jsstr_push_joined_keeps_the_ascii_flag_exact() {
        let mut a = JsStr::from("a".repeat(300));
        assert!(a.is_ascii_cached());
        a.push_joined("bc");
        assert!(a.is_ascii_cached());
        a.push_joined("é");
        assert!(!a.is_ascii_cached());
        a.push_joined("d");
        assert!(!a.is_ascii_cached());
        assert_eq!(&*a, format!("{}bcéd", "a".repeat(300)));

        // a flag that was never computed stays unknown and is answered correctly later
        let mut b = JsStr::from("a".repeat(300));
        b.push_joined("é");
        assert!(!b.is_ascii_cached());
    }

    #[test]
    fn jsstr_push_joined_keeps_a_buffer_across_the_threshold() {
        let mut t = JsStr::from("x".repeat(299));
        t.push_joined("é");
        assert_eq!(t.len(), 301);
        assert!(!is_plain(&t));
    }
}
