/// Thaw's exception channel is a single `i8*` string end to end (thrown,
/// caught, and reported to the Lambda Runtime API all as one C string), with
/// no separate `Error` object, so a name and a message share that one
/// string. `new Error(message)`/`new TypeError(message)`/etc. lower to a
/// string literal tagged with this marker followed by the class name and a
/// second marker before the message (see
/// `thaw_hir::lower::expressions::lowering`); a plain `throw "text"`, or a
/// failure string produced by the QuickJS dynamic-call bridge, carries no
/// tag and is treated as an untagged `Error` with the whole string as its
/// message -- matching real JavaScript's own default `Error.prototype.name`.
///
/// A user class extending `Error`/etc. (see `lower/module/classes.rs`) tags
/// with its *full* identity chain instead of just its own name (e.g.
/// `Sub$MyError$Error` for `class Sub extends MyError extends Error`, see
/// `lower/expressions/coercions.rs`'s `coerce_primitive_to_string`), so
/// `instanceof` on an intermediate ancestor still matches; `.name` only ever
/// reports the first (most-derived) segment, matching how real JavaScript's
/// `Error.prototype.name` names the actual thrown class, not its ancestors.
///
/// `\u{1}` (SOH) was picked because it can never appear in a JSON-encoded
/// Lambda error body or in ordinary program text, and (unlike `\0`) doesn't
/// truncate the C string it's embedded in.
const ERROR_TAG_MARKER: char = '\u{1}';

fn split_error_tag(message: &str) -> (&str, &str) {
    let Some(rest) = message.strip_prefix(ERROR_TAG_MARKER) else {
        return ("Error", message);
    };
    rest.split_once(ERROR_TAG_MARKER).unwrap_or(("Error", message))
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// The returned pointer, if non-null, is arena-allocated and must not be
/// freed by the caller.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_name(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let (chain, _) = split_error_tag(&text);
    let name = chain.split('$').next().unwrap_or(chain);
    arena_c_string(name).map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// The returned pointer, if non-null, is arena-allocated and must not be
/// freed by the caller.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_message(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let (_, body) = split_error_tag(&text);
    arena_c_string(body).map_or(std::ptr::null(), |value| value.cast())
}

/// Whether `message`'s tagged (or defaulted) identity chain includes
/// `class_name`, or `class_name` is `"Error"` -- every tagged/untagged
/// exception this channel can carry is some kind of `Error`, matching real
/// JavaScript's error class hierarchy without needing to represent it. A
/// multi-level chain (`Sub$MyError$Error`) matches any ancestor's name, not
/// just the most-derived one.
///
/// # Safety
/// Both pointers must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_is_instance(
    message: *const c_char,
    class_name: *const c_char,
) -> bool {
    if message.is_null() || class_name.is_null() {
        return false;
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let class_name = unsafe { CStr::from_ptr(class_name) }.to_string_lossy();
    let (chain, _) = split_error_tag(&text);
    class_name == "Error" || chain.split('$').any(|name| name == class_name)
}

#[cfg(test)]
mod error_native_tests {
    use super::*;

    fn call_name(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_name(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_message(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_message(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_is_instance(message: &str, class_name: &str) -> bool {
        let message = CString::new(message).unwrap();
        let class_name = CString::new(class_name).unwrap();
        unsafe { thaw_error_is_instance(message.as_ptr(), class_name.as_ptr()) }
    }

    #[test]
    fn a_plain_thrown_string_defaults_to_the_error_name() {
        assert_eq!(call_name("boom"), "Error");
        assert_eq!(call_message("boom"), "boom");
        assert!(call_is_instance("boom", "Error"));
        assert!(!call_is_instance("boom", "TypeError"));
    }

    #[test]
    fn a_tagged_error_reports_its_own_class_and_message() {
        let tagged = "\u{1}TypeError\u{1}not a function";
        assert_eq!(call_name(tagged), "TypeError");
        assert_eq!(call_message(tagged), "not a function");
        assert!(call_is_instance(tagged, "Error"));
        assert!(call_is_instance(tagged, "TypeError"));
        assert!(!call_is_instance(tagged, "RangeError"));
    }

    #[test]
    fn a_tagged_message_may_itself_contain_the_marker_byte() {
        // Only the first marker after the name is treated as the separator;
        // anything past it, marker bytes included, is part of the message.
        let tagged = "\u{1}Error\u{1}first\u{1}second";
        assert_eq!(call_name(tagged), "Error");
        assert_eq!(call_message(tagged), "first\u{1}second");
    }

    #[test]
    fn a_multi_level_identity_chain_matches_any_ancestor_but_names_only_the_leaf() {
        let tagged = "\u{1}Sub$MyError$Error\u{1}deep failure";
        assert_eq!(call_name(tagged), "Sub");
        assert_eq!(call_message(tagged), "deep failure");
        assert!(call_is_instance(tagged, "Sub"));
        assert!(call_is_instance(tagged, "MyError"));
        assert!(call_is_instance(tagged, "Error"));
        assert!(!call_is_instance(tagged, "TypeError"));
    }

    #[test]
    fn an_empty_message_still_round_trips() {
        assert_eq!(call_name(""), "Error");
        assert_eq!(call_message(""), "");
        let tagged = "\u{1}RangeError\u{1}";
        assert_eq!(call_name(tagged), "RangeError");
        assert_eq!(call_message(tagged), "");
    }
}
