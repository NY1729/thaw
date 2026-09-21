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
const ERROR_CAUSE_MARKER: char = '\u{2}';
const ERROR_CODE_MARKER: char = '\u{3}';
/// Marks a `.name` override -- the *runtime* value of a thrown Error-
/// family object's own `name` field (see `lower/expressions/
/// coercions.rs`'s `HirType::Object` tagging), which takes priority
/// over the static, compile-time-derived identity chain whenever
/// present. Always produced by that one path, and always the last
/// trailing segment (canonical order: name override, then cause, then
/// code) -- a class-instance throw never also carries a cause/code, so
/// this never needs to coexist with them in practice, but the split
/// functions still handle the combination correctly regardless.
const ERROR_NAME_OVERRIDE_MARKER: char = '\u{4}';
/// Trailing bag of an exception's own extra properties (a JSON object),
/// appended by thaw-quickjs's `describe_tagged_exception` for a JS error
/// crossing native code and back (e.g. an `http-errors` error's `status`).
/// Native `.message`/`.name` reads must ignore it.
const ERROR_PROPS_MARKER: char = '\u{5}';

fn split_error_tag(message: &str) -> (&str, &str) {
    let Some(rest) = message.strip_prefix(ERROR_TAG_MARKER) else {
        // No leading tag: a plain message, possibly with a *trailing*
        // `\u{1}name\u{1}message` segment a labeled rejection appends (see
        // thaw-quickjs's `describe_promise_exception`), plus an optional
        // `\u{5}` property bag. Keep only the text before either.
        let body = message.split_once(ERROR_PROPS_MARKER).map_or(message, |value| value.0);
        let body = body.split_once(ERROR_TAG_MARKER).map_or(body, |value| value.0);
        return ("Error", body);
    };
    let (name, body) = rest
        .split_once(ERROR_TAG_MARKER)
        .unwrap_or(("Error", message));
    let body = body
        .split_once(ERROR_PROPS_MARKER)
        .map_or(body, |value| value.0);
    let body = body
        .split_once(ERROR_NAME_OVERRIDE_MARKER)
        .map_or(body, |value| value.0);
    let body = body.split_once(ERROR_CAUSE_MARKER).map_or(body, |value| value.0);
    (name, body.split_once(ERROR_CODE_MARKER).map_or(body, |value| value.0))
}

/// The runtime `.name` override embedded after the message, if any --
/// see `ERROR_NAME_OVERRIDE_MARKER`.
fn split_error_name_override(message: &str) -> Option<&str> {
    let (_, after) = message.split_once(ERROR_NAME_OVERRIDE_MARKER)?;
    let after = after.split_once(ERROR_PROPS_MARKER).map_or(after, |value| value.0);
    let after = after.split_once(ERROR_CAUSE_MARKER).map_or(after, |value| value.0);
    Some(after.split_once(ERROR_CODE_MARKER).map_or(after, |value| value.0))
}

/// The reported `.name` -- the runtime override when present, otherwise
/// the static identity chain's most-derived segment (a plain
/// `new Error(...)`-style tag, which has no override to embed).
fn resolved_error_name(message: &str) -> String {
    if let Some(name) = split_error_name_override(message) {
        return name.to_string();
    }
    let (chain, _) = split_error_tag(message);
    chain.split('$').next().unwrap_or(chain).to_string()
}

fn split_error_cause(message: &str) -> Option<&str> {
    let after = message.split_once(ERROR_CAUSE_MARKER)?.1;
    let after = after.split_once(ERROR_CODE_MARKER).map_or(after, |value| value.0);
    Some(after.split_once(ERROR_PROPS_MARKER).map_or(after, |value| value.0))
}

fn split_error_code(message: &str) -> Option<&str> {
    message
        .split_once(ERROR_CODE_MARKER)
        .map(|value| value.1.split_once(ERROR_PROPS_MARKER).map_or(value.1, |props| props.0))
}

/// Reads a caught error's own custom property from the trailing
/// `\u{5}<json>` bag `describe_tagged_exception` (thaw-quickjs) appends --
/// real trigger: `catch (e) { e.status }` for an `http-errors` Error thrown
/// by koa, whose `status`/`expose` live on the error's prototype. Returns
/// the value rendered as a string (`"418"`, `"true"`, a string itself),
/// or `None` when the tag carries no such property (the caller then
/// yields `undefined`, matching an absent property). Strings are returned
/// without JSON quoting.
fn error_property(message: &str, name: &str) -> Option<String> {
    let bag = message.split_once(ERROR_PROPS_MARKER)?.1;
    let value: serde_json::Value = serde_json::from_str(bag).ok()?;
    let value = value.as_object()?.get(name)?;
    Some(match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    })
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// # Safety
/// `name` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_property(
    message: *const c_char,
    name: *const c_char,
) -> *const c_char {
    if message.is_null() || name.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    match error_property(&text, &name) {
        Some(value) => arena_c_string(&value).map_or(std::ptr::null(), |value| value.cast()),
        None => std::ptr::null(),
    }
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
    let name = resolved_error_name(&text);
    arena_c_string(&name).map_or(std::ptr::null(), |value| value.cast())
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

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_cause(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    arena_c_string(split_error_cause(&text).unwrap_or_default())
        .map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_code(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    arena_c_string(split_error_code(&text).unwrap_or("undefined"))
        .map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// The returned pointer is either `message` itself or an arena allocation.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_to_string(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    if !text.starts_with(ERROR_TAG_MARKER) {
        return message;
    }
    let (_, body) = split_error_tag(&text);
    let name = resolved_error_name(&text);
    let rendered = if body.is_empty() {
        name
    } else {
        format!("{name}: {body}")
    };
    arena_c_string(&rendered).map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// The returned pointer, if non-null, is arena-allocated and must not be
/// freed by the caller.
///
/// ponytail: no real call-stack frames -- compiled native code has no
/// JS-style frame tracking to walk, so this only ever renders the
/// mandatory first line real `Error.prototype.stack` always starts
/// with (`"name: message"`), never the frame list after it. Unlike
/// `thaw_error_to_string` (`.toString()`/`String(error)`, where an
/// *untagged* plain-string throw's real JS semantics return the string
/// itself unchanged), `.stack` always applies the same "defaults to
/// `Error`" convention `.name`/`.message` already use, tagged or not --
/// a bare `throw "boom"`'s `.stack` reads `"Error: boom"`, matching how
/// its `.name`/`.message` already read `"Error"`/`"boom"`. Upgrade
/// path: capture real frames if/when this compiler grows stack-
/// unwinding support.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_stack(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let (_, body) = split_error_tag(&text);
    let name = resolved_error_name(&text);
    let rendered = if body.is_empty() {
        name
    } else {
        format!("{name}: {body}")
    };
    arena_c_string(&rendered).map_or(std::ptr::null(), |value| value.cast())
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

/// `Error.isError(value)` for a caught/tagged error string: true when the
/// value carries the leading error tag that a `new Error(...)`-family
/// construction (or a QuickJS-thrown error) produces. An untagged string
/// is a plain value, not an `Error`.
///
/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_is_error(message: *const c_char) -> bool {
    if message.is_null() {
        return false;
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    text.starts_with(ERROR_TAG_MARKER)
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

    fn call_code(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_code(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_property(message: &str, name: &str) -> Option<String> {
        let message = CString::new(message).unwrap();
        let name = CString::new(name).unwrap();
        let result = unsafe { thaw_error_property(message.as_ptr(), name.as_ptr()) };
        if result.is_null() {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(result) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    fn call_to_string(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_to_string(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_stack(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_stack(message.as_ptr()) };
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
    fn a_labeled_rejection_keeps_its_labeled_text_and_drops_the_trailing_tag() {
        // thaw-quickjs's `describe_promise_exception` wraps a plain-Error
        // rejection in a human-readable label and appends the original
        // `\u{1}Error\u{1}message\u{5}<props>` so a later property
        // reconstruction can read it; `.message`/`String()` still show
        // only the labeled text.
        let labeled = "`pkg::boom`'s promise rejected: boom\u{1}Error\u{1}boom\u{5}{\"status\":418}";
        assert_eq!(call_message(labeled), "`pkg::boom`'s promise rejected: boom");
        assert_eq!(call_name(labeled), "Error");
    }

    #[test]
    fn a_caught_errors_custom_property_round_trips_from_its_json_bag() {
        // `.status`/`.expose` live in the trailing `\u{5}<json>` bag a
        // caught JS error carries (real trigger: koa's `onerror` reading
        // an `http-errors` error). A number renders as itself, a boolean
        // as `true`/`false`, a string unquoted; an absent property is
        // `None`.
        let tagged = "\u{1}ImATeapotError\u{1}teapot\u{5}{\"status\":418,\"expose\":true,\"label\":\"x\"}";
        assert_eq!(call_property(tagged, "status").as_deref(), Some("418"));
        assert_eq!(call_property(tagged, "expose").as_deref(), Some("true"));
        assert_eq!(call_property(tagged, "label").as_deref(), Some("x"));
        assert_eq!(call_property(tagged, "missing"), None);
        assert_eq!(call_property("plain", "status"), None);
    }

    #[test]
    fn a_tagged_error_reports_its_own_class_and_message() {
        let tagged = "\u{1}TypeError\u{1}not a function";
        assert_eq!(call_name(tagged), "TypeError");
        assert_eq!(call_message(tagged), "not a function");
        assert!(call_is_instance(tagged, "Error"));
        assert!(call_is_instance(tagged, "TypeError"));
        assert!(!call_is_instance(tagged, "RangeError"));
        assert_eq!(call_to_string(tagged), "TypeError: not a function");
        assert_eq!(call_to_string("plain"), "plain");
    }

    #[test]
    fn a_tagged_host_error_preserves_its_code() {
        assert_eq!(call_code("\u{1}Error\u{1}missing\u{3}ENOENT"), "ENOENT");
        assert_eq!(call_code("\u{1}Error\u{1}plain"), "undefined");
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

    #[test]
    fn stack_always_applies_the_error_default_unlike_to_string() {
        // Unlike `.toString()`/`String(error)`, whose real JS semantics
        // return an untagged plain string unchanged, `.stack` always
        // applies the same "defaults to `Error`" convention `.name`/
        // `.message` already use.
        assert_eq!(call_stack("boom"), "Error: boom");
        assert_eq!(call_to_string("boom"), "boom");
        let tagged = "\u{1}TypeError\u{1}not a function";
        assert_eq!(call_stack(tagged), "TypeError: not a function");
        assert_eq!(call_stack(tagged), call_to_string(tagged));
    }

    #[test]
    fn a_name_override_takes_priority_over_the_static_identity_chain() {
        // A user class's `this.name = "..."` (or its real-JS-matching
        // default, the nearest native ancestor's name -- see
        // `lower/invocations/calls.rs`) is embedded as a `\u{4}`-tagged
        // override, read by `.name`/`.stack`/`.toString()` in place of
        // the *static*, compile-time-derived identity chain (which
        // `instanceof` still uses unchanged -- an override never
        // affects `thaw_error_is_instance`).
        let tagged = "\u{1}MyError$Error\u{1}oops\u{4}MyError";
        assert_eq!(call_name(tagged), "MyError");
        assert_eq!(call_message(tagged), "oops");
        assert_eq!(call_stack(tagged), "MyError: oops");
        assert_eq!(call_to_string(tagged), "MyError: oops");
        assert!(call_is_instance(tagged, "MyError"));
        assert!(call_is_instance(tagged, "Error"));

        // No override present -- falls back to the identity chain's
        // most-derived segment, exactly like before this marker existed.
        let unoverridden = "\u{1}MyError$Error\u{1}oops";
        assert_eq!(call_name(unoverridden), "MyError");
        assert_eq!(call_stack(unoverridden), "MyError: oops");
    }

    #[test]
    fn an_error_cause_is_separate_from_its_message() {
        let tagged = "\u{1}Error\u{1}outer\u{2}\u{1}TypeError\u{1}root";
        assert_eq!(call_message(tagged), "outer");
        let tagged = CString::new(tagged).unwrap();
        let cause = unsafe { thaw_error_cause(tagged.as_ptr()) };
        assert_eq!(unsafe { CStr::from_ptr(cause) }.to_string_lossy(), "\u{1}TypeError\u{1}root");
    }
}
