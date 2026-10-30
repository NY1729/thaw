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
/// `Sub\u{1f}MyError\u{1f}Error` for `class Sub extends MyError extends Error`, see
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
/// A `SuppressedError`'s `.error` sub-error tag, appended after the message
/// as `\u{6}<byte length>:<error><suppressed>`. The length prefix preserves
/// nested `SuppressedError` values without escaping or delimiter ambiguity.
const ERROR_SUPPRESSED_ERROR_MARKER: char = '\u{6}';
/// A `SuppressedError`'s `.suppressed` sub-error tag.
const ERROR_SUPPRESSED_MARKER: char = '\u{7}';

fn framed_error(message: &str) -> Option<thaw_arena::error_wire::ErrorFrame<'_>> {
    thaw_arena::error_wire::parse_tagged(message.as_bytes())
}

fn metadata_suffix(message: &str) -> &str {
    if let Some(frame) = framed_error(message) {
        return std::str::from_utf8(frame.suffix).unwrap_or("");
    }
    message
}

/// Length-frame a structured error message without changing the pointer ABI. Also records the
/// caller's call stack for `err.stack`: this trampoline hands the caller's frame pointer and
/// return address to the implementation, so compiled code needs no extra call.
/// # Safety
/// Both pointers must reference live native strings.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame(name: *const c_char, message: *const c_char) -> *const c_char {
    core::arch::naked_asm!(
        "mov rdx, rbp",
        "mov rcx, [rsp]",
        "jmp {inner}",
        inner = sym thaw_error_frame_at,
    )
}

/// See the x86-64 variant.
/// # Safety
/// Both pointers must reference live native strings.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame(name: *const c_char, message: *const c_char) -> *const c_char {
    core::arch::naked_asm!(
        "mov x2, x29",
        "mov x3, x30",
        "b {inner}",
        inner = sym thaw_error_frame_at,
    )
}

/// See the x86-64 variant; without a known frame layout no stack is captured.
/// # Safety
/// Both pointers must reference live native strings.
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame(name: *const c_char, message: *const c_char) -> *const c_char {
    unsafe { thaw_error_frame_at(name, message, 0, 0) }
}

unsafe extern "C" fn thaw_error_frame_at(
    name: *const c_char,
    message: *const c_char,
    frame: usize,
    ra: usize,
) -> *const c_char {
    if name.is_null() || message.is_null() { return std::ptr::null(); }
    let name = unsafe { wtf8_bytes(name) };
    let message = unsafe { wtf8_bytes(message) };
    let error = thaw_arena::error_wire::encode_tagged(name, message, None)
        .and_then(|bytes| arena_wtf8(&bytes))
        .map_or(std::ptr::null(), |value| value.cast());
    capture_frames(error, frame, ra);
    error
}

/// Ensures a literal framed error uses an arena-owned pointer so the optional QuickJS
/// Error bridge can preserve identity across repeated projections. Unframed strings and
/// already-owned frames keep their original pointer. The copy of a runtime-synthesised error
/// (`TypeError: Cannot read properties...`, thrown as a literal) also records the throwing call
/// stack, like `thaw_error_frame` does for `new Error`.
/// # Safety
/// `value` must reference a live native string.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame_own(value: *const c_char) -> *const c_char {
    core::arch::naked_asm!(
        "mov rsi, rbp",
        "mov rdx, [rsp]",
        "jmp {inner}",
        inner = sym thaw_error_frame_own_at,
    )
}

/// See the x86-64 variant.
/// # Safety
/// `value` must reference a live native string.
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame_own(value: *const c_char) -> *const c_char {
    core::arch::naked_asm!(
        "mov x1, x29",
        "mov x2, x30",
        "b {inner}",
        inner = sym thaw_error_frame_own_at,
    )
}

/// See the x86-64 variant; without a known frame layout no stack is captured.
/// # Safety
/// `value` must reference a live native string.
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frame_own(value: *const c_char) -> *const c_char {
    unsafe { thaw_error_frame_own_at(value, 0, 0) }
}

unsafe extern "C" fn thaw_error_frame_own_at(value: *const c_char, frame: usize, ra: usize) -> *const c_char {
    if value.is_null() || thaw_arena::contains_allocation(value as usize) {
        return value;
    }
    let bytes = unsafe { wtf8_bytes(value) };
    let framed = thaw_arena::error_wire::parse_tagged(bytes).is_some()
        || (bytes.first() == Some(&(ERROR_TAG_MARKER as u8))
            && bytes[1..].contains(&(ERROR_TAG_MARKER as u8)));
    if framed {
        let owned = arena_wtf8(bytes).map_or(std::ptr::null(), |owned| owned.cast());
        capture_frames(owned, frame, ra);
        owned
    } else {
        value
    }
}


/// Drops a `SuppressedError`'s trailing `.error`/`.suppressed` sub-tags
/// (and anything after them) so `.message`/`.cause`/`.code`/`.name` read
/// only the real fields. Nested sub-error tags carry `\u{1}`-`\u{5}`, never
/// `\u{6}`.
fn strip_suppressed_segments(message: &str) -> &str {
    message
        .split_once(ERROR_SUPPRESSED_ERROR_MARKER)
        .map_or(message, |value| value.0)
}

fn split_once_error_marker(message: &[u8], marker: u8) -> Option<(&[u8], &[u8])> {
    let index = message.iter().position(|&byte| byte == marker)?;
    Some((&message[..index], &message[index + 1..]))
}

// The markers are ASCII bytes, so splitting raw WTF-8 also preserves lone
// surrogate sequences and embedded NULs in the message body.
fn split_error_tag_bytes(message: &[u8]) -> (&[u8], &[u8]) {
    if let Some(frame) = thaw_arena::error_wire::parse_tagged(message) {
        return (frame.chain, frame.display);
    }
    let message = split_once_error_marker(message, ERROR_SUPPRESSED_ERROR_MARKER as u8)
        .map_or(message, |value| value.0);
    let Some(rest) = message.strip_prefix(&[ERROR_TAG_MARKER as u8]) else {
        // Untagged rejections may still have trailing identity/property tags.
        let body = split_once_error_marker(message, ERROR_PROPS_MARKER as u8)
            .map_or(message, |value| value.0);
        let body = split_once_error_marker(body, ERROR_TAG_MARKER as u8)
            .map_or(body, |value| value.0);
        return (b"Error", body);
    };
    let (name, body) = split_once_error_marker(rest, ERROR_TAG_MARKER as u8)
        .unwrap_or((b"Error", message));
    let body = split_once_error_marker(body, ERROR_PROPS_MARKER as u8)
        .map_or(body, |value| value.0);
    let body = split_once_error_marker(body, ERROR_NAME_OVERRIDE_MARKER as u8)
        .map_or(body, |value| value.0);
    let body = split_once_error_marker(body, ERROR_CAUSE_MARKER as u8)
        .map_or(body, |value| value.0);
    (name, split_once_error_marker(body, ERROR_CODE_MARKER as u8)
        .map_or(body, |value| value.0))
}

fn split_error_tag(message: &str) -> (&str, &str) {
    let (name, body) = split_error_tag_bytes(message.as_bytes());
    // ASCII markers cannot split a UTF-8 code point in a valid `str`.
    (std::str::from_utf8(name).unwrap(), std::str::from_utf8(body).unwrap())
}

// Native class errors carry a versioned ancestry chain. Legacy tagged
// errors from older/external bridges still use '$'; the explicit version
// prefix keeps a legal class name containing '$' unambiguous.
fn error_ancestry(chain: &str) -> std::str::Split<'_, char> {
    if let Some(versioned) = chain.strip_prefix('\u{1e}') {
        versioned.split('\u{1f}')
    } else {
        chain.split('$')
    }
}

/// The runtime `.name` override embedded after the message, if any --
/// see `ERROR_NAME_OVERRIDE_MARKER`.
fn split_error_name_override(message: &str) -> Option<&str> {
    let message = strip_suppressed_segments(metadata_suffix(message));
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
    error_ancestry(chain).next().unwrap_or(chain).to_string()
}

fn split_error_cause(message: &str) -> Option<&str> {
    let message = strip_suppressed_segments(metadata_suffix(message));
    let after = message.split_once(ERROR_CAUSE_MARKER)?.1;
    let after = after.split_once(ERROR_CODE_MARKER).map_or(after, |value| value.0);
    Some(after.split_once(ERROR_PROPS_MARKER).map_or(after, |value| value.0))
}

fn split_error_code(message: &str) -> Option<&str> {
    let message = strip_suppressed_segments(metadata_suffix(message));
    message
        .split_once(ERROR_CODE_MARKER)
        .map(|value| value.1.split_once(ERROR_PROPS_MARKER).map_or(value.1, |props| props.0))
}

/// A `SuppressedError`'s `.error` sub-error tag, or `None`.
fn split_suppressed_error(message: &str) -> Option<&str> {
    let after = metadata_suffix(message).split_once(ERROR_SUPPRESSED_ERROR_MARKER)?.1;
    if let Some((error, _)) = split_length_prefixed(after) {
        return Some(error);
    }
    let segment = after
        .split_once(ERROR_SUPPRESSED_MARKER)
        .map_or(after, |value| value.0);
    Some(
        segment
            .split_once(ERROR_PROPS_MARKER)
            .map_or(segment, |value| value.0),
    )
}

/// A `SuppressedError`'s `.suppressed` sub-error tag, or `None`.
fn split_suppressed(message: &str) -> Option<&str> {
    let after_error = metadata_suffix(message).split_once(ERROR_SUPPRESSED_ERROR_MARKER)?.1;
    if let Some((_, suppressed)) = split_length_prefixed(after_error) {
        return Some(suppressed);
    }
    let after = after_error.split_once(ERROR_SUPPRESSED_MARKER)?.1;
    Some(
        after
            .split_once(ERROR_PROPS_MARKER)
            .map_or(after, |value| value.0),
    )
}

fn split_length_prefixed(value: &str) -> Option<(&str, &str)> {
    let (length, body) = value.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    Some((body.get(..length)?, body.get(length..)?))
}

/// Builds the canonical, nesting-safe `SuppressedError` representation.
///
/// # Safety
/// Every argument must be a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_suppress(
    error: *const c_char,
    suppressed: *const c_char,
    message: *const c_char,
) -> *const c_char {
    if error.is_null() || suppressed.is_null() || message.is_null() {
        return std::ptr::null();
    }
    let error = unsafe { CStr::from_ptr(error) }.to_string_lossy();
    let suppressed = unsafe { CStr::from_ptr(suppressed) }.to_string_lossy();
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let Some(mut tagged) = thaw_arena::error_wire::encode_tagged(
        b"SuppressedError", message.as_bytes(), None,
    ) else { return std::ptr::null(); };
    tagged.push(ERROR_SUPPRESSED_ERROR_MARKER as u8);
    tagged.extend_from_slice(format!("{}:{error}{suppressed}", error.len()).as_bytes());
    arena_wtf8(&tagged).map_or(std::ptr::null(), |value| value.cast())
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
    let bag = metadata_suffix(message).split_once(ERROR_PROPS_MARKER)?.1;
    let value: serde_json::Value = serde_json::from_str(bag).ok()?;
    let object = value.as_object()?;
    let value = object
        .get(name)
        .or_else(|| if name == "cause" { object.get(HIDDEN_CAUSE_KEY) } else { None })?;
    Some(match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => return None,
        other => other.to_string(),
    })
}

fn error_property_json(message: &str, name: &str) -> Option<String> {
    let bag = metadata_suffix(message).split_once(ERROR_PROPS_MARKER)?.1;
    let value: serde_json::Value = serde_json::from_str(bag).ok()?;
    let object = value.as_object()?;
    let value = object
        .get(name)
        .or_else(|| if name == "cause" { object.get(HIDDEN_CAUSE_KEY) } else { None })?;
    serde_json::to_string(value).ok()
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
    if let Some(json) = with_extras(message, |extras| {
        extras.lookup(&name)
    }) {
        return arena_c_string(&json_text_as_plain(&json)).map_or(std::ptr::null(), |value| value.cast());
    }
    match error_property(&text, &name) {
        Some(value) => arena_c_string(&value).map_or(std::ptr::null(), |value| value.cast()),
        None => std::ptr::null(),
    }
}

/// Reads a legacy framed error property's JSON representation. Missing keys
/// return null; present values, including null, retain their JSON type.
///
/// # Safety
/// `message` and `name` must be null or valid, NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_property_json(
    message: *const c_char,
    name: *const c_char,
) -> *const c_char {
    if message.is_null() || name.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    if let Some(json) = with_extras(message, |extras| {
        extras.lookup(&name)
    }) {
        return arena_c_string(&json).map_or(std::ptr::null(), |value| value.cast());
    }
    match error_property_json(&text, &name) {
        Some(value) => arena_c_string(&value).map_or(std::ptr::null(), |value| value.cast()),
        None => std::ptr::null(),
    }
}

/// The global constructor name behind a caught error's `.constructor`: the most-derived class of
/// the static identity chain, ignoring any runtime `.name` override.
///
/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_constructor_name(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let (chain, _) = split_error_tag(&text);
    let name = error_ancestry(chain).next().unwrap_or(chain);
    arena_c_string(name).map_or(std::ptr::null(), |value| value.cast())
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
    if let Some(name) = with_extras(message, |extras| extras.name.clone()) {
        return arena_c_string(&name).map_or(std::ptr::null(), |value| value.cast());
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
    if let Some(text) = with_extras(message, |extras| extras.message.clone()) {
        return arena_wtf8(text.as_bytes()).map_or(std::ptr::null(), |value| value.cast());
    }
    if with_extras(message, |extras| Some(extras.deleted.iter().any(|key| key == "message"))) == Some(true) {
        return arena_wtf8(b"").map_or(std::ptr::null(), |value| value.cast());
    }
    let bytes = unsafe { wtf8_bytes(message) };
    let (_, body) = split_error_tag_bytes(bytes);
    arena_wtf8(body).map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_cause(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    if let Some(json) = with_extras(message, |extras| {
        extras.lookup("cause")
    }) {
        return arena_c_string(&json_text_as_plain(&json)).map_or(std::ptr::null(), |value| value.cast());
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    if let Some(value) = error_property(&text, "cause") {
        return arena_c_string(&value).map_or(std::ptr::null(), |value| value.cast());
    }
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
    if let Some(json) = with_extras(message, |extras| {
        extras.props.iter().find(|prop| prop.0 == "code").map(|prop| prop.1.clone())
    }) {
        return arena_c_string(&json_text_as_plain(&json)).map_or(std::ptr::null(), |value| value.cast());
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    // A JS error crossing the boundary carries its own `code` (`ENOENT`,
    // `Z_DATA_ERROR`) in the `\u{5}<json>` property bag, not a `\u{3}` tag.
    let code = split_error_code(&text)
        .map(str::to_owned)
        .or_else(|| error_property(&text, "code"));
    arena_c_string(code.as_deref().unwrap_or("undefined"))
        .map_or(std::ptr::null(), |value| value.cast())
}

/// `SuppressedError.prototype.error` -- the original sub-error tag (empty
/// when `message` isn't a `SuppressedError`), matching `.cause`'s
/// empty-string-for-absent convention.
///
/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_suppressed_error(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    arena_c_string(split_suppressed_error(&text).unwrap_or_default())
        .map_or(std::ptr::null(), |value| value.cast())
}

/// `SuppressedError.prototype.suppressed` -- the suppressed sub-error tag
/// (empty when `message` isn't a `SuppressedError`).
///
/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_suppressed(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    arena_c_string(split_suppressed(&text).unwrap_or_default())
        .map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
///
/// The returned pointer is either `message` itself or an arena allocation.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_to_string(message: *const c_char) -> *const c_char {
    if message.is_null() {
        return arena_c_string("undefined").map_or(std::ptr::null(), |value| value.cast());
    }
    // Hot path (`+`/template operands): untagged strings return after one byte check.
    if unsafe { *message } != ERROR_TAG_MARKER as c_char {
        return message;
    }
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let (name, body) = effective_name_message(message, &text);
    let rendered = error_header(&name, &body);
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
    unsafe { error_stack(message, false) }
}

/// `thaw_error_stack` plus the generated frame line (see `stack_frames`): what `err.stack` reads on
/// an Error the program holds. A caught value keeps the header-only `thaw_error_stack` form.
///
/// # Safety
/// `message` must be null or a valid, NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_stack_with_frames(message: *const c_char) -> *const c_char {
    unsafe { error_stack(message, true) }
}

unsafe fn error_stack(message: *const c_char, frames: bool) -> *const c_char {
    if message.is_null() {
        return std::ptr::null();
    }
    let rendered = unsafe { stack_text(message, frames) };
    // V8 serializes `stack` on first access: later name/message writes no longer change it. Only the
    // header is cached, so the frame line stays out of `console.log(error)` like before.
    if is_framed_error_text(message) {
        let header = rendered.split("\n    at ").next().unwrap_or_default().to_string();
        thaw_arena::register_metadata_reset_hook(prune_error_extras);
        let key = extras_key(message);
        ERROR_EXTRAS.with(|table| {
            extras_entry(&mut table.borrow_mut(), key).stack.get_or_insert(header);
        });
    }
    arena_c_string(&rendered).map_or(std::ptr::null(), |value| value.cast())
}

/// Whether `message`'s tagged identity chain includes `class_name`. Untagged
/// values are primitive throws, not Error instances. A multi-level chain
/// (`Sub\u{1f}MyError\u{1f}Error`) matches any ancestor's name, not just the most-derived
/// one.
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
    if !text.starts_with(ERROR_TAG_MARKER) {
        return false;
    }
    let (chain, _) = split_error_tag(&text);
    class_name == "Error" || error_ancestry(chain).any(|name| name == class_name)
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

// ---- Own extensible properties of an Error value -------------------------------------------
//
// An Error is a framed string, so `err.code = x` has nowhere to live in the value itself. The
// extras live in a side table keyed by the canonical framed-string pointer (a caught copy maps
// back to the thrown pointer through thaw-std's `thaw_error_canonical_pointer`). Entries die
// with the arena reset that frees the string (`prune_error_extras`).

unsafe extern "C" {
    /// thaw-std: canonical identity pointer of a possibly copied caught Error string.
    fn thaw_error_canonical_pointer(pointer: *const u8) -> usize;
}

#[derive(Default)]
struct ErrorExtras {
    /// Text of the keyed string when the entry was created: an entry whose address now holds
    /// different text (a freed non-arena string whose address was reused) is stale and ignored.
    text: Vec<u8>,
    /// Own enumerable properties in insertion order: key -> JSON text.
    // ponytail: props are JSON text, so a prop holding an object loses reference identity
    // (x.foo.a = 2 after x.foo = o does not mutate o). Upgrade: store thaw-std Json handles via
    // thaw_json_* externs once their ownership/decode-scope rules are confirmed.
    props: Vec<(String, String)>,
    message: Option<String>,
    name: Option<String>,
    stack: Option<String>,
    /// Hidden (non-enumerable) own `cause` from the constructor options, as JSON text.
    cause: Option<String>,
    /// `stack` was assigned by the program (kept verbatim); otherwise `stack` caches only the header.
    stack_user: bool,
    /// Deleted own `message`/`stack`/`cause`/`code`/`name`.
    deleted: Vec<String>,
    /// Code addresses of the call stack at construction (innermost first), see `capture_frames`.
    frames: Vec<usize>,
}

impl ErrorExtras {
    /// JSON text of own property `key`: an enumerable prop, or the hidden constructor `cause`.
    fn lookup(&self, key: &str) -> Option<String> {
        self.props
            .iter()
            .find(|prop| prop.0 == key)
            .map(|prop| prop.1.clone())
            .or_else(|| if key == "cause" { self.cause.clone() } else { None })
    }
}

/// Key under which the constructor's hidden `cause` travels through `thaw_error_set_prop` and the
/// `\u{5}` property bag; it never names an enumerable property.
const HIDDEN_CAUSE_KEY: &str = "\u{2}cause";

thread_local! {
    static ERROR_EXTRAS: RefCell<HashMap<usize, ErrorExtras>> = RefCell::new(HashMap::new());
}

fn prune_error_extras(tracing: bool) {
    ERROR_EXTRAS.with(|table| {
        let mut table = table.borrow_mut();
        if tracing {
            table.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            table.clear();
        }
    });
    OWNER_PROPS.with(|table| {
        let mut table = table.borrow_mut();
        if tracing {
            table.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            table.clear();
        }
    });
}

fn extras_key(pointer: *const c_char) -> usize {
    unsafe { thaw_error_canonical_pointer(pointer.cast()) }
}

/// Runs `read` on the extras of `pointer`; the empty-table early-out keeps error-free programs
/// at one TLS read.
fn with_extras<R>(pointer: *const c_char, read: impl FnOnce(&ErrorExtras) -> Option<R>) -> Option<R> {
    if ERROR_EXTRAS.with(|table| table.borrow().is_empty()) {
        return None;
    }
    let key = extras_key(pointer);
    ERROR_EXTRAS.with(|table| {
        let table = table.borrow();
        let extras = table.get(&key)?;
        (extras.text == key_bytes(key)).then_some(extras).and_then(read)
    })
}

fn key_bytes<'a>(key: usize) -> &'a [u8] {
    unsafe { CStr::from_ptr(key as *const c_char) }.to_bytes()
}

/// The live entry for `key`, replacing a stale one (see `ErrorExtras::text`).
fn extras_entry(table: &mut HashMap<usize, ErrorExtras>, key: usize) -> &mut ErrorExtras {
    let text = key_bytes(key);
    let extras = table.entry(key).or_default();
    if extras.text != text {
        *extras = ErrorExtras { text: text.to_vec(), ..ErrorExtras::default() };
    }
    extras
}

fn json_text_as_plain(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::String(text)) => text,
        Ok(other) => other.to_string(),
        Err(_) => json.to_string(),
    }
}

fn error_header(name: &str, body: &str) -> String {
    if name.is_empty() {
        body.to_string()
    } else if body.is_empty() {
        name.to_string()
    } else {
        format!("{name}: {body}")
    }
}

/// The error's current name/message after extras overrides.
fn effective_name_message(pointer: *const c_char, text: &str) -> (String, String) {
    let name = with_extras(pointer, |extras| extras.name.clone())
        .unwrap_or_else(|| resolved_error_name(text));
    let message = with_extras(pointer, |extras| extras.message.clone())
        .or_else(|| {
            // A deleted own `message` falls back to `Error.prototype.message` (empty).
            with_extras(pointer, |extras| extras.deleted.iter().any(|key| key == "message").then(String::new))
        })
        .unwrap_or_else(|| split_error_tag(text).1.to_string());
    (name, message)
}

fn is_framed_error_text(pointer: *const c_char) -> bool {
    // A frame is `\u{1}Name\u{1}message`; a user string that merely starts with U+0001
    // has no closing delimiter and is plain text.
    !pointer.is_null()
        && unsafe { *pointer } == ERROR_TAG_MARKER as c_char
        && unsafe { CStr::from_ptr(pointer) }.to_bytes()[1..].contains(&(ERROR_TAG_MARKER as u8))
}

/// Keys of the JS-origin `\u{5}` property bag, in bag order.
fn bag_keys(text: &str) -> Vec<String> {
    let Some(bag) = metadata_suffix(text).split_once(ERROR_PROPS_MARKER).map(|v| v.1) else {
        return Vec::new();
    };
    match serde_json::from_str::<serde_json::Value>(bag) {
        Ok(serde_json::Value::Object(map)) => {
            map.keys()
                .filter(|key| *key != HIDDEN_CAUSE_KEY && !key.starts_with('\0'))
                .cloned()
                .collect()
        }
        _ => Vec::new(),
    }
}

fn arena_json(value: impl Into<serde_json::Value>) -> *const c_char {
    serde_json::to_string(&value.into())
        .ok()
        .and_then(|text| arena_c_string(&text))
        .map_or(std::ptr::null(), |value| value.cast())
}

fn arena_key_object(keys: Vec<String>) -> *const c_char {
    arena_json(keys.into_iter().map(|key| (key, serde_json::Value::Null)).collect::<serde_json::Map<_, _>>())
}

/// Own enumerable keys, in order: assigned props, then the JS-origin bag.
fn error_enumerable_keys(pointer: *const c_char, text: &str) -> Vec<String> {
    let mut keys: Vec<String> =
        with_extras(pointer, |extras| Some(extras.props.iter().map(|p| p.0.clone()).collect()))
            .unwrap_or_default();
    for key in bag_keys(text) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// `err.key = value`. `json` is the JSON text of the assigned value. Returns 0 when `error` is
/// not an Error value (the caller throws a TypeError).
///
/// # Safety
/// All pointers must be null or valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_set_prop(
    error: *const c_char,
    key: *const c_char,
    json: *const c_char,
) -> bool {
    if key.is_null() || json.is_null() || !is_framed_error_text(error) {
        return false;
    }
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy().into_owned();
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    thaw_arena::register_metadata_reset_hook(prune_error_extras);
    let table_key = extras_key(error);
    ERROR_EXTRAS.with(|table| {
        let mut table = table.borrow_mut();
        let extras = extras_entry(&mut table, table_key);
        extras.deleted.retain(|deleted| *deleted != key);
        let plain = json_text_as_plain(&json);
        if key == HIDDEN_CAUSE_KEY || (key == "cause" && extras.cause.is_some()) {
            // `{ cause: undefined }` still creates the own property: keep Json's undefined sentinel.
            extras.cause = Some(if json == "undefined" {
                r#"{"$__thaw_napi_undefined$":true}"#.to_string()
            } else {
                json
            });
            return;
        }
        match key.as_str() {
            "message" | "name" => {
                if key == "message" {
                    extras.message = Some(plain);
                    return;
                }
                extras.name = Some(plain);
            }
            "stack" => {
                extras.stack = Some(plain);
                extras.stack_user = true;
                return;
            }
            _ => {}
        }
        // `name` (inherited) becomes an own enumerable prop on assignment; message/stack stay hidden.
        match extras.props.iter_mut().find(|prop| prop.0 == key) {
            Some(prop) => prop.1 = json,
            None => extras.props.push((key, json)),
        }
    });
    true
}

/// JSON text of own property `key`, or null when absent.
///
/// # Safety
/// All pointers must be null or valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_get_prop_json(error: *const c_char, key: *const c_char) -> *const c_char {
    if key.is_null() || error.is_null() {
        return std::ptr::null();
    }
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy().into_owned();
    if let Some(json) = with_extras(error, |extras| {
        extras.lookup(&key)
    }) {
        return arena_c_string(&json).map_or(std::ptr::null(), |value| value.cast());
    }
    if !is_framed_error_text(error) {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    let deleted = with_extras(error, |extras| Some(extras.deleted.contains(&key))).unwrap_or(false);
    if deleted {
        return std::ptr::null();
    }
    match key.as_str() {
        "message" => arena_json(effective_name_message(error, &text).1),
        "name" => arena_json(effective_name_message(error, &text).0),
        "stack" => arena_json(unsafe { stack_text(error, true) }),
        "cause" if error_property_json(&text, &key).is_none() => split_error_cause(&text)
            .map_or(std::ptr::null(), |cause| arena_json(cause.to_string())),
        _ => error_property_json(&text, &key)
            .and_then(|json| arena_c_string(&json))
            .map_or(std::ptr::null(), |value| value.cast()),
    }
}

/// # Safety
/// All pointers must be null or valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_has_prop(error: *const c_char, key: *const c_char) -> bool {
    if key.is_null() || error.is_null() {
        return false;
    }
    let name = unsafe { CStr::from_ptr(key) }.to_string_lossy().into_owned();
    if with_extras(error, |extras| Some(extras.lookup(&name).is_some())) == Some(true) {
        return true;
    }
    if !is_framed_error_text(error) {
        return false;
    }
    if with_extras(error, |extras| Some(extras.deleted.contains(&name))) == Some(true) {
        return false;
    }
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    matches!(name.as_str(), "message" | "stack")
        || bag_keys(&text).contains(&name)
        || (name == "cause" && split_error_cause(&text).is_some())
}

/// `delete err.key`. Returns 1 (JS `delete` is true for absent keys too).
///
/// # Safety
/// All pointers must be null or valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_delete_prop(error: *const c_char, key: *const c_char) -> bool {
    if key.is_null() || !is_framed_error_text(error) {
        return true;
    }
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy().into_owned();
    let table_key = extras_key(error);
    thaw_arena::register_metadata_reset_hook(prune_error_extras);
    ERROR_EXTRAS.with(|table| {
        let mut table = table.borrow_mut();
        let extras = extras_entry(&mut table, table_key);
        extras.props.retain(|prop| prop.0 != key);
        match key.as_str() {
            "message" => extras.message = None,
            "name" => extras.name = None,
            "stack" => {
                extras.stack = None;
                extras.stack_user = false;
            }
            "cause" => extras.cause = None,
            _ => {}
        }
        if matches!(key.as_str(), "message" | "stack" | "cause" | "code") && !extras.deleted.contains(&key) {
            extras.deleted.push(key);
        }
    });
    true
}

/// JSON object text (`{"key":null,..}`, so a parse plus `Object.keys` yields the ordered key list)
/// of own keys. Errors: enumerable keys, or with `include_hidden` the Node
/// `getOwnPropertyNames` order (`stack`, `message`, then the rest). A non-Error string yields
/// its index keys (`Object.keys("ab")`), plus `length` when hidden keys are requested.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_own_keys(error: *const c_char, include_hidden: u8) -> *const c_char {
    if error.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    if !is_framed_error_text(error) {
        let mut keys: Vec<String> = (0..text.encode_utf16().count()).map(|i| i.to_string()).collect();
        if include_hidden != 0 {
            keys.push("length".to_string());
        }
        return arena_key_object(keys);
    }
    let mut keys = Vec::new();
    if include_hidden != 0 {
        let deleted = with_extras(error, |extras| Some(extras.deleted.clone())).unwrap_or_default();
        for hidden in ["stack", "message"] {
            if !deleted.iter().any(|key| key == hidden) {
                keys.push(hidden.to_string());
            }
        }
        if split_error_cause(&text).is_some()
            || with_extras(error, |extras| extras.cause.as_ref().map(|_| ())).is_some()
        {
            keys.push("cause".to_string());
        }
    }
    for key in error_enumerable_keys(error, &text) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    arena_key_object(keys)
}

/// `Object.assign(err, source)` for a JSON-object `source`; returns 1, or 0 when `error` is not an
/// Error (nothing set, the caller throws a TypeError).
///
/// # Safety
/// Both pointers must be null or valid NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_assign_json(error: *const c_char, json_object: *const c_char) -> bool {
    if json_object.is_null() || !is_framed_error_text(error) {
        return false;
    }
    let text = unsafe { CStr::from_ptr(json_object) }.to_string_lossy().into_owned();
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(&text) else {
        return true;
    };
    for (key, value) in map {
        let (Ok(key), Ok(value)) = (std::ffi::CString::new(key), std::ffi::CString::new(value.to_string()))
        else {
            continue;
        };
        unsafe { thaw_error_set_prop(error, key.as_ptr(), value.as_ptr()) };
    }
    true
}

/// Own enumerable props of a framed Error as a JSON object: assigned props, then the JS-origin bag.
fn error_props_object(error: *const c_char, text: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut object = serde_json::Map::new();
    let props = with_extras(error, |extras| Some(extras.props.clone())).unwrap_or_default();
    for (key, json) in props {
        if let Ok(value) = serde_json::from_str(&json) {
            object.insert(key, value);
        }
    }
    if let Some(bag) = metadata_suffix(text).split_once(ERROR_PROPS_MARKER).map(|v| v.1) {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(bag) {
            for (key, value) in map.into_iter().filter(|(key, _)| !key.starts_with('\0')) {
                if key == HIDDEN_CAUSE_KEY {
                    continue;
                }
                object.entry(key).or_insert(value);
            }
        }
    }
    object
}

/// `JSON.stringify(err)`: own enumerable props for an Error, a quoted string otherwise.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_json_stringify(error: *const c_char) -> *const c_char {
    if error.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    if !is_framed_error_text(error) {
        return arena_json(text);
    }
    arena_json(error_props_object(error, &text))
}

/// JSON object of the own enumerable entries of an Error (`Object.entries(err)`); a plain string
/// yields its index entries like `Object.entries("ab")`. Null input yields null.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_entries_json(error: *const c_char) -> *const c_char {
    if error.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    if !is_framed_error_text(error) {
        return arena_json(
            text.encode_utf16()
                .enumerate()
                .map(|(i, unit)| (i.to_string(), serde_json::Value::String(String::from_utf16_lossy(&[unit]))))
                .collect::<serde_json::Map<_, _>>(),
        );
    }
    arena_json(error_props_object(error, &text))
}

/// The error's text with its side-table state baked into the legacy `\u{5}` property bag, so a
/// copy of the text (a Json string, a QuickJS bridge value) carries the props, and the `message`
/// and `stack` overrides, without needing the pointer identity. Returns `error` itself when there is
/// nothing to add.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_with_props(error: *const c_char) -> *const c_char {
    unsafe { error_text_with_props(error, false) }
}

/// Reserved bag key holding the canonical pointer of the Error a Json string value refers to.
/// Keys starting with NUL are never properties.
const ERROR_ID_KEY: &str = "\u{0}id";

/// `thaw_error_with_props` for a text copy that becomes a Json string value: the bag also records
/// the canonical identity pointer of the Error (`ERROR_ID_KEY`), so the Json value stays a
/// reference to the same Error (props, `===`) instead of an independent copy.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_json_text(error: *const c_char) -> *const c_char {
    unsafe { error_text_with_props(error, true) }
}

unsafe fn error_text_with_props(error: *const c_char, with_id: bool) -> *const c_char {
    if error.is_null() || !is_framed_error_text(error) {
        return error;
    }
    let mut overlay = with_extras(error, |extras| {
        let mut pairs = extras.props.clone();
        for (key, value) in [("message", &extras.message), ("stack", &extras.stack)] {
            if let Some(value) = value {
                pairs.push((key.to_string(), serde_json::Value::String(value.clone()).to_string()));
            }
        }
        if let Some(cause) = &extras.cause {
            pairs.push((HIDDEN_CAUSE_KEY.to_string(), cause.clone()));
        }
        Some(pairs)
    })
    .unwrap_or_default();
    if with_id {
        overlay.push((ERROR_ID_KEY.to_string(), extras_key(error).to_string()));
    }
    if overlay.is_empty() {
        return error;
    }
    let bytes = unsafe { CStr::from_ptr(error) }.to_bytes();
    let suffix_start = thaw_arena::error_wire::parse_tagged(bytes).map_or(0, |frame| bytes.len() - frame.suffix.len());
    let bag_start = bytes[suffix_start..]
        .iter()
        .position(|byte| *byte == ERROR_PROPS_MARKER as u8)
        .map(|offset| suffix_start + offset);
    let mut bag = bag_start
        .and_then(|start| serde_json::from_slice::<serde_json::Value>(&bytes[start + 1..]).ok())
        .and_then(|value| match value {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    for (key, json) in overlay {
        // An id already in the bag (a re-widened Json copy) names the original Error.
        if key == ERROR_ID_KEY && bag.contains_key(ERROR_ID_KEY) {
            continue;
        }
        if let Ok(value) = serde_json::from_str(&json) {
            bag.insert(key, value);
        }
    }
    let mut out = bytes[..bag_start.unwrap_or(bytes.len())].to_vec();
    out.push(ERROR_PROPS_MARKER as u8);
    out.extend_from_slice(serde_json::Value::Object(bag).to_string().as_bytes());
    thaw_arena::owned_string(out)
}

thread_local! {
    /// Thrown Error-family class instance (owner pointer) -> JSON object text of its own enumerable
    /// props at the throw (`message` excluded, `name` only when assigned), for a catch's
    /// `Object.keys`/`entries`/`getOwnPropertyNames`.
    static OWNER_PROPS: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

/// Records the own enumerable props (a JSON object text) of a thrown Error-family instance.
///
/// # Safety
/// `json` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_register_owner_props(owner: *const u8, json: *const c_char) -> bool {
    if owner.is_null() || json.is_null() {
        return false;
    }
    thaw_arena::register_metadata_reset_hook(prune_error_extras);
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    OWNER_PROPS.with(|table| table.borrow_mut().insert(owner as usize, text));
    true
}

/// The text registered by `thaw_error_register_owner_props`, or `""` for an owner without one.
/// With `hidden` set, `stack` and `message` lead the object (for `getOwnPropertyNames`).
#[no_mangle]
pub extern "C" fn thaw_error_owner_props(owner: *const u8, hidden: u8) -> *const c_char {
    let Some(text) = OWNER_PROPS.with(|table| table.borrow().get(&(owner as usize)).cloned()) else {
        return arena_c_string("").map_or(std::ptr::null(), |value| value.cast());
    };
    if hidden == 0 {
        return arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast());
    }
    let mut keys = serde_json::Map::new();
    keys.insert("stack".into(), serde_json::Value::Null);
    keys.insert("message".into(), serde_json::Value::Null);
    if let Ok(serde_json::Value::Object(props)) = serde_json::from_str(&text) {
        keys.extend(props);
    }
    arena_json(keys)
}

thread_local! {
    /// Own `stack` of a record that has no `stack` slot (an Error-family class instance after
    /// `Error.captureStackTrace`, or any class instance captured into): owner pointer -> text.
    static OBJECT_STACKS: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
    /// Owners whose side-table `stack` was assigned (enumerable) rather than captured (hidden).
    static OBJECT_STACK_VISIBLE: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
}

/// `Error.captureStackTrace(owner)` for a record: remembers `text` as the owner's `stack`.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_object_stack_set(owner: *const u8, text: *const c_char) -> bool {
    if owner.is_null() || text.is_null() {
        return false;
    }
    thaw_arena::register_metadata_reset_hook(prune_object_stacks);
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    OBJECT_STACKS.with(|table| table.borrow_mut().insert(owner as usize, text));
    true
}

/// `owner.stack = text` on a record without a `stack` slot: like `thaw_object_stack_set`, but the
/// property is enumerable (`Error.captureStackTrace` defines a hidden one).
///
/// # Safety
/// `text` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_object_stack_assign(owner: *const u8, text: *const c_char) -> bool {
    if !unsafe { thaw_object_stack_set(owner, text) } {
        return false;
    }
    OBJECT_STACK_VISIBLE.with(|table| table.borrow_mut().insert(owner as usize));
    true
}

#[no_mangle]
pub extern "C" fn thaw_object_stack_visible(owner: *const u8) -> bool {
    OBJECT_STACK_VISIBLE.with(|table| table.borrow().contains(&(owner as usize)))
}

#[no_mangle]
pub extern "C" fn thaw_object_stack_has(owner: *const u8) -> bool {
    OBJECT_STACKS.with(|table| table.borrow().contains_key(&(owner as usize)))
}

/// The remembered `stack` of `owner`, or `fallback` when none was captured.
///
/// # Safety
/// `fallback` must be a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_object_stack_get(owner: *const u8, fallback: *const c_char) -> *const c_char {
    match OBJECT_STACKS.with(|table| table.borrow().get(&(owner as usize)).cloned()) {
        Some(text) => arena_c_string(&text).map_or(fallback, |value| value.cast()),
        None => fallback,
    }
}

thread_local! {
    /// Native `Map`/`Set`/`Array` allocation -> name of the subclass it was handed on from.
    static NATIVE_SUBCLASS_NAMES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

/// Remembers that the native collection `base` backs an instance of class `name`.
///
/// # Safety
/// `name` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_native_subclass_mark(base: *const u8, name: *const c_char) {
    if base.is_null() || name.is_null() {
        return;
    }
    thaw_arena::register_metadata_reset_hook(prune_native_subclass_names);
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    NATIVE_SUBCLASS_NAMES.with(|table| table.borrow_mut().insert(base as usize, name));
}

/// The subclass name remembered for `base`, or null.
#[no_mangle]
pub extern "C" fn thaw_native_subclass_name(base: *const u8) -> *const c_char {
    let name = NATIVE_SUBCLASS_NAMES.with(|table| table.borrow().get(&(base as usize)).cloned());
    name.and_then(|name| arena_c_string(&name)).map_or(std::ptr::null(), |value| value.cast())
}

fn prune_native_subclass_names(tracing: bool) {
    NATIVE_SUBCLASS_NAMES.with(|table| {
        let mut table = table.borrow_mut();
        if tracing {
            table.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            table.clear();
        }
    });
}

fn prune_object_stacks(tracing: bool) {
    OBJECT_STACK_VISIBLE.with(|table| {
        let mut table = table.borrow_mut();
        if tracing {
            table.retain(|pointer| !thaw_arena::was_reclaimed(*pointer));
        } else {
            table.clear();
        }
    });
    OBJECT_STACKS.with(|table| {
        let mut table = table.borrow_mut();
        if tracing {
            table.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            table.clear();
        }
    });
}

/// # Safety
/// `error` must be a valid NUL-terminated string.
unsafe fn stack_text(error: *const c_char, frames: bool) -> String {
    let (cached, assigned) =
        with_extras(error, |extras| Some((extras.stack.clone(), extras.stack_user))).unwrap_or_default();
    if assigned {
        return cached.unwrap_or_default();
    }
    let frames = if frames { stack_frames(error) } else { String::new() };
    format!("{}{}", unsafe { stack_header(error, cached) }, frames)
}

/// The first line of a generated `stack`: the cached one (V8 serializes on first read), else
/// `name: message` as of now.
unsafe fn stack_header(error: *const c_char, cached: Option<String>) -> String {
    cached.unwrap_or_else(|| {
        let text = unsafe { CStr::from_ptr(error) }.to_string_lossy();
        let (name, body) = effective_name_message(error, &text);
        error_header(&name, &body)
    })
}

thread_local! {
    /// `Error.stackTraceLimit`.
    static STACK_TRACE_LIMIT: Cell<f64> = const { Cell::new(10.0) };
}

/// `Error.stackTraceLimit`.
#[no_mangle]
pub extern "C" fn thaw_error_stack_trace_limit() -> f64 {
    STACK_TRACE_LIMIT.with(Cell::get)
}

/// `Error.stackTraceLimit = limit`; returns the assigned value.
#[no_mangle]
pub extern "C" fn thaw_error_set_stack_trace_limit(limit: f64) -> f64 {
    STACK_TRACE_LIMIT.with(|slot| slot.set(limit));
    limit
}

/// The frame lines of a generated `stack`: the frames captured when `error` was constructed
/// (`thaw_error_frame`), else one fixed frame of V8's anonymous shape, while
/// `Error.stackTraceLimit` allows at least one.
// ponytail: definition line:col only (not the call site), and a frame
// inlined by LLVM or reached through frame-pointer-less runtime code is not shown.
fn stack_frames(error: *const c_char) -> String {
    let limit = thaw_error_stack_trace_limit();
    if limit < 1.0 {
        return String::new();
    }
    let pcs = with_extras(error, |extras| Some(extras.frames.clone())).unwrap_or_default();
    let lines = render_frames(&pcs, limit as usize);
    if lines.is_empty() {
        "\n    at <anonymous>".to_string()
    } else {
        lines
    }
}

/// The frame lines a generated `stack` of the framed Error `error` carries (empty for anything
/// else): what `console.log(error)` appends to the `Name: message` header like Node.
///
/// # Safety
/// `error` must be null or a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_error_frames(error: *const c_char) -> *const c_char {
    let lines = if is_framed_error_text(error) { stack_frames(error) } else { String::new() };
    arena_c_string(&lines).map_or(std::ptr::null(), |value| value.cast())
}

/// The frame lines `stack_frames` appends, for the `stack` that `Error.captureStackTrace` builds
/// on a plain object: `frame` and `ra` are the compiled caller's frame pointer and return address
/// (see `caller_pcs`); without compiled frames it is V8's single anonymous frame.
#[no_mangle]
pub extern "C" fn thaw_error_stack_frames(frame: usize, ra: usize) -> *const c_char {
    let limit = thaw_error_stack_trace_limit();
    let lines = if limit < 1.0 {
        String::new()
    } else {
        let lines = render_frames(&caller_pcs(frame, ra, (limit as usize).min(200)), limit as usize);
        if lines.is_empty() { "\n    at <anonymous>".to_string() } else { lines }
    };
    arena_c_string(&lines).map_or(std::ptr::null(), |value| value.cast())
}

/// Compiler-emitted table of the program's functions (`thaw_register_fn_table`).
struct FnTable {
    /// Sorted by entry address.
    entries: Vec<FnInfo>,
    /// Bounds of the section holding every compiled function.
    lo: usize,
    hi: usize,
    file: String,
}

static FN_TABLE: std::sync::OnceLock<FnTable> = std::sync::OnceLock::new();

/// One compiled function: an empty name is anonymous, a `None` file/line is unknown.
struct FnInfo {
    address: usize,
    name: String,
    file: Option<String>,
    line: u32,
    col: u32,
}

#[repr(C)]
pub struct FnEntry {
    function: *const u8,
    name: *const c_char,
    /// Null when the function's definition site is unknown.
    file: *const c_char,
    line: u32,
    col: u32,
}

/// Registers the compiled functions' names; called once at process start by generated code.
///
/// # Safety
/// `entries` must point to `count` valid entries; `file` and every name must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn thaw_register_fn_table(
    entries: *const FnEntry,
    count: usize,
    lo: *const u8,
    hi: *const u8,
    file: *const c_char,
) {
    let text = |pointer: *const c_char| unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned();
    let mut entries: Vec<FnInfo> = (0..count)
        .map(|index| {
            let entry = unsafe { &*entries.add(index) };
            FnInfo {
                address: entry.function as usize,
                name: text(entry.name),
                file: (!entry.file.is_null()).then(|| text(entry.file)),
                line: entry.line,
                col: entry.col,
            }
        })
        .collect();
    entries.sort_by_key(|entry| entry.address);
    let _ = FN_TABLE.set(FnTable { entries, lo: lo as usize, hi: hi as usize, file: text(file) });
}

/// `[low, high)` of the calling thread's stack, cached per thread; `(0, 0)` where unknown.
fn stack_bounds() -> (usize, usize) {
    thread_local! {
        static BOUNDS: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
    }
    BOUNDS.with(|slot| {
        slot.get().unwrap_or_else(|| {
            let bounds = query_stack_bounds();
            slot.set(Some(bounds));
            bounds
        })
    })
}

#[cfg(target_os = "linux")]
fn query_stack_bounds() -> (usize, usize) {
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return (0, 0);
        }
        let (mut base, mut size) = (std::ptr::null_mut(), 0usize);
        let ok = libc::pthread_attr_getstack(&attr, &mut base, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        if ok { (base as usize, base as usize + size) } else { (0, 0) }
    }
}

#[cfg(not(target_os = "linux"))]
fn query_stack_bounds() -> (usize, usize) {
    (0, 0)
}

/// Frame-pointer chain of the compiled code that called a runtime entry, innermost first:
/// `ra` is the entry's return address and `frame` the caller's frame pointer (both taken by the
/// naked trampoline in front of the entry, so no IR is needed at the call site). Callers are
/// found by following the saved frame-pointer chain of compiled code (every compiled function
/// keeps one). Best effort: the walk stops at the first record that is not on this thread's stack
/// or does not move up, and non-compiled addresses are filtered out when rendering.
fn caller_pcs(frame: usize, ra: usize, limit: usize) -> Vec<usize> {
    let Some(table) = FN_TABLE.get() else {
        return Vec::new();
    };
    let compiled = |pc: usize| pc >= table.lo && pc < table.hi;
    let mut pcs = vec![ra.wrapping_sub(1)];
    let mut seen = usize::from(compiled(pcs[0]));
    let (low, high) = stack_bounds();
    let mut frame = frame;
    while seen < limit && pcs.len() < 256 && frame.is_multiple_of(8) && frame >= low && frame + 16 <= high {
        let (next, ra) = unsafe { (*(frame as *const usize), *((frame + 8) as *const usize)) };
        let pc = ra.wrapping_sub(1);
        seen += usize::from(compiled(pc));
        pcs.push(pc);
        if next <= frame {
            break;
        }
        frame = next;
    }
    pcs
}

/// Records the call stack at `error`'s construction for `err.stack`.
fn capture_frames(error: *const c_char, frame: usize, ra: usize) {
    let limit = thaw_error_stack_trace_limit();
    if error.is_null() || limit < 1.0 || FN_TABLE.get().is_none() {
        return;
    }
    let pcs = caller_pcs(frame, ra, (limit as usize).min(200));
    thaw_arena::register_metadata_reset_hook(prune_error_extras);
    let key = extras_key(error);
    ERROR_EXTRAS.with(|table| extras_entry(&mut table.borrow_mut(), key).frames = pcs);
}

/// `\n    at name (file)` lines for the program functions among `pcs`, at most `limit`.
fn render_frames(pcs: &[usize], limit: usize) -> String {
    use std::fmt::Write as _;
    let Some(table) = FN_TABLE.get() else {
        return String::new();
    };
    let mut out = String::new();
    let mut shown = 0;
    for &pc in pcs {
        if shown == limit {
            break;
        }
        if pc < table.lo || pc >= table.hi {
            continue;
        }
        let index = table.entries.partition_point(|entry| entry.address <= pc);
        let Some(info) = index.checked_sub(1).map(|index| &table.entries[index]) else {
            continue;
        };
        if info.name == "\u{1}" {
            continue;
        }
        shown += 1;
        let file = info.file.as_deref().unwrap_or(&table.file);
        let _ = if info.line == 0 {
            if info.name.is_empty() {
                write!(out, "\n    at {file}")
            } else {
                write!(out, "\n    at {} ({file})", info.name)
            }
        } else if info.name.is_empty() {
            write!(out, "\n    at {file}:{}:{}", info.line, info.col)
        } else {
            write!(out, "\n    at {} ({file}:{}:{})", info.name, info.line, info.col)
        };
    }
    out
}

#[cfg(test)]
mod error_native_tests {
    use super::*;

    #[test]
    fn stack_frame_line_follows_the_trace_limit_and_assigned_stacks() {
        // The limit and the side table are thread-local.
        std::thread::spawn(|| unsafe {
            let text = |pointer: *const c_char| CStr::from_ptr(pointer).to_string_lossy().into_owned();
            let error = thaw_error_frame(c"TypeError".as_ptr(), c"m".as_ptr());
            assert_eq!(text(thaw_error_stack(error)), "TypeError: m");
            assert_eq!(text(thaw_error_stack_with_frames(error)), "TypeError: m\n    at <anonymous>");
            thaw_error_set_stack_trace_limit(0.0);
            assert_eq!(text(thaw_error_stack_with_frames(error)), "TypeError: m");
            thaw_error_set_stack_trace_limit(10.0);
            assert!(thaw_error_set_prop(error, c"stack".as_ptr(), c"\"custom\"".as_ptr()));
            assert_eq!(text(thaw_error_stack_with_frames(error)), "custom");
            assert_eq!(text(thaw_error_stack(error)), "custom");
        })
        .join()
        .unwrap();
    }

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

    #[test]
    fn to_string_converts_the_unset_string_sentinel_to_undefined() {
        let result = unsafe { thaw_error_to_string(std::ptr::null()) };
        assert!(!result.is_null());
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_bytes(), b"undefined");
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
        assert!(!call_is_instance("boom", "Error"));
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
        let tagged = "\u{1}\u{1e}Sub\u{1f}MyError\u{1f}Error\u{1}deep failure";
        assert_eq!(call_name(tagged), "Sub");
        assert_eq!(call_message(tagged), "deep failure");
        assert!(call_is_instance(tagged, "Sub"));
        assert!(call_is_instance(tagged, "MyError"));
        assert!(call_is_instance(tagged, "Error"));
        assert!(!call_is_instance(tagged, "TypeError"));
    }

    #[test]
    fn dollar_sign_in_versioned_error_class_name_is_not_an_ancestor_separator() {
        let tagged = "\u{1}\u{1e}Leaf\u{1f}Base$Name\u{1f}Error\u{1}failure";
        assert_eq!(call_name(tagged), "Leaf");
        assert!(call_is_instance(tagged, "Base$Name"));
        assert!(!call_is_instance(tagged, "Base"));
        let legacy = "\u{1}Leaf$Base$Error\u{1}failure";
        assert!(call_is_instance(legacy, "Base"));
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
        let tagged = "\u{1}\u{1e}MyError\u{1f}Error\u{1}oops\u{4}MyError";
        assert_eq!(call_name(tagged), "MyError");
        assert_eq!(call_message(tagged), "oops");
        assert_eq!(call_stack(tagged), "MyError: oops");
        assert_eq!(call_to_string(tagged), "MyError: oops");
        let empty_name = "\u{1}\u{1e}MyError\u{1f}Error\u{1}oops\u{4}";
        assert_eq!(call_name(empty_name), "");
        assert_eq!(call_message(empty_name), "oops");
        assert_eq!(call_to_string(empty_name), "oops");
        assert_eq!(call_stack(empty_name), "oops");
        assert_eq!(call_to_string("\u{1}Error\u{1}\u{4}"), "");
        assert!(call_is_instance(tagged, "MyError"));
        assert!(call_is_instance(tagged, "Error"));

        // No override present -- falls back to the identity chain's
        // most-derived segment, exactly like before this marker existed.
        let unoverridden = "\u{1}\u{1e}MyError\u{1f}Error\u{1}oops";
        assert_eq!(call_name(unoverridden), "MyError");
        assert_eq!(call_stack(unoverridden), "MyError: oops");
    }

    fn call_suppressed_error(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_suppressed_error(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_suppressed(message: &str) -> String {
        let message = CString::new(message).unwrap();
        let result = unsafe { thaw_error_suppressed(message.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    fn call_suppress(error: &str, suppressed: &str, message: &str) -> String {
        let error = CString::new(error).unwrap();
        let suppressed = CString::new(suppressed).unwrap();
        let message = CString::new(message).unwrap();
        let result = unsafe {
            thaw_error_suppress(error.as_ptr(), suppressed.as_ptr(), message.as_ptr())
        };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn a_suppressed_error_keeps_its_sub_errors_separate_from_its_message() {
        let tagged = "\u{1}SuppressedError\u{1}both\u{6}\u{1}Error\u{1}first\u{7}\u{1}TypeError\u{1}second";
        assert_eq!(call_name(tagged), "SuppressedError");
        assert_eq!(call_message(tagged), "both");
        assert_eq!(call_suppressed_error(tagged), "\u{1}Error\u{1}first");
        assert_eq!(call_suppressed(tagged), "\u{1}TypeError\u{1}second");
        // A plain error has neither segment.
        assert_eq!(call_suppressed_error("boom"), "");
        assert_eq!(call_suppressed("\u{1}Error\u{1}x"), "");
    }

    #[test]
    fn nested_suppressed_errors_round_trip_without_delimiter_collisions() {
        let inner = call_suppress(
            "\u{1}Error\u{1}first",
            "\u{1}TypeError\u{1}second",
            "inner",
        );
        let outer = call_suppress(&inner, "\u{1}RangeError\u{1}third", "outer");
        assert_eq!(call_message(&outer), "outer");
        assert_eq!(call_suppressed_error(&outer), inner);
        assert_eq!(call_suppressed(&outer), "\u{1}RangeError\u{1}third");
        assert_eq!(call_suppressed_error(&inner), "\u{1}Error\u{1}first");
        assert_eq!(call_suppressed(&inner), "\u{1}TypeError\u{1}second");
    }

    #[test]
    fn error_message_preserves_wtf8_surrogates_and_embedded_nul() {
        let body = b"high\xed\xa0\x80\0low\xed\xb0\x80";
        for source in [
            body.to_vec(),
            [b"\x01TypeError\x01".as_slice(), body, b"\x03ERR_TEST"].concat(),
        ] {
            let source = arena_wtf8(&source).unwrap();
            let result = unsafe { thaw_error_message(source.cast()) };
            assert!(!result.is_null());
            assert_eq!(unsafe { wtf8_bytes(result) }, body);
        }
        assert_eq!(call_message("\u{1}TypeError\u{1}plain\u{3}ERR_TEST"), "plain");
        assert_eq!(call_message("plain"), "plain");
    }

    #[test]
    fn legacy_error_property_json_preserves_types_and_null() {
        let source = "legacy\u{5}{\"status\":418,\"expose\":true,\"payload\":{\"id\":9},\"explicitNull\":null}";
        assert_eq!(error_property_json(source, "status").as_deref(), Some("418"));
        assert_eq!(error_property_json(source, "expose").as_deref(), Some("true"));
        assert_eq!(error_property_json(source, "payload").as_deref(), Some("{\"id\":9}"));
        assert_eq!(error_property_json(source, "explicitNull").as_deref(), Some("null"));
        assert_eq!(error_property_json(source, "missing"), None);
    }

    #[test]
    fn framed_error_message_keeps_control_text_before_real_suffix() {
        let body = "first\u{2}second\u{3}third\u{4}fourth\u{5}fifth\u{6}sixth";
        let name = c"TypeError";
        let body_arg = thaw_arena::arena_string(body.as_bytes());
        let framed = unsafe { thaw_error_frame(name.as_ptr(), body_arg) };
        assert!(!framed.is_null());
        let mut wire = unsafe { wtf8_bytes(framed) }.to_vec();
        wire.extend_from_slice(b"\x02real cause\x03REAL_CODE\x05{\"status\":418}");
        let source = arena_wtf8(&wire).unwrap();
        let result = unsafe { thaw_error_message(source.cast()) };
        assert_eq!(unsafe { wtf8_bytes(result) }, body.as_bytes());
        let source = unsafe { CStr::from_ptr(source.cast()) }.to_string_lossy();
        assert_eq!(resolved_error_name(&source), "TypeError");
        assert_eq!(split_error_cause(&source), Some("real cause"));
        assert_eq!(split_error_code(&source), Some("REAL_CODE"));
        assert_eq!(error_property(&source, "status"), Some("418".into()));
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

/// Lowest stack address compiled code may use before it throws `RangeError`: the main thread's
/// stack end (from `RLIMIT_STACK`, 8 MiB when unlimited) plus a 1 MiB margin for native callees.
/// Returns 0 (guard off) when the limit is too small to leave a margin.
#[no_mangle]
pub extern "C" fn thaw_stack_guard_limit() -> u64 {
    const MARGIN: u64 = 1 << 20;
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    let size = if unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut limit) } == 0
        && limit.rlim_cur != libc::RLIM_INFINITY
    {
        limit.rlim_cur
    } else {
        8 << 20
    };
    let marker = 0u8;
    let here = std::ptr::addr_of!(marker) as u64;
    if size <= 2 * MARGIN {
        return 0;
    }
    here.saturating_sub(size - MARGIN)
}

#[cfg(test)]
mod error_extras_tests {
    use super::*;
    use std::ffi::CString;

    fn c(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    fn frame(name: &str, message: &str) -> *const c_char {
        let (name, message) = (c(name), c(message));
        let framed = unsafe { thaw_error_frame(name.as_ptr(), message.as_ptr()) };
        assert!(!framed.is_null());
        framed
    }

    fn set(error: *const c_char, key: &str, json: &str) -> bool {
        unsafe { thaw_error_set_prop(error, c(key).as_ptr(), c(json).as_ptr()) }
    }

    fn get(error: *const c_char, key: &str) -> Option<String> {
        let value = unsafe { thaw_error_get_prop_json(error, c(key).as_ptr()) };
        (!value.is_null()).then(|| unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned())
    }

    fn text(value: *const c_char) -> String {
        assert!(!value.is_null());
        unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned()
    }

    /// Each test runs on its own thread: the table is thread-local.
    fn isolated(body: impl FnOnce() + Send + 'static) {
        std::thread::spawn(body).join().unwrap();
    }

    #[test]
    fn set_get_keys_delete_round_trip() {
        isolated(|| {
            let error = frame("Error", "boom");
            assert!(set(error, "code", "\"E_X\""));
            assert!(set(error, "meta", "{\"a\":[1,2]}"));
            assert_eq!(get(error, "code").as_deref(), Some("\"E_X\""));
            assert_eq!(get(error, "meta").as_deref(), Some("{\"a\":[1,2]}"));
            assert_eq!(get(error, "absent"), None);
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), r#"{"code":null,"meta":null}"#);
            assert_eq!(
                text(unsafe { thaw_error_own_keys(error, 1) }),
                r#"{"stack":null,"message":null,"code":null,"meta":null}"#
            );
            assert!(unsafe { thaw_error_has_prop(error, c("code").as_ptr()) });
            assert!(unsafe { thaw_error_delete_prop(error, c("code").as_ptr()) });
            assert!(!unsafe { thaw_error_has_prop(error, c("code").as_ptr()) });
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), r#"{"meta":null}"#);
            assert_eq!(text(unsafe { thaw_error_json_stringify(error) }), r#"{"meta":{"a":[1,2]}}"#);
        });
    }

    #[test]
    fn frames_with_identical_text_have_independent_props() {
        isolated(|| {
            let (first, second) = (frame("Error", "x"), frame("Error", "x"));
            assert_ne!(first, second);
            assert!(set(first, "tag", "\"A\""));
            assert_eq!(get(first, "tag").as_deref(), Some("\"A\""));
            assert_eq!(get(second, "tag"), None);
            assert_eq!(text(unsafe { thaw_error_own_keys(second, 0) }), "{}");
        });
    }

    #[test]
    fn message_override_changes_message_and_to_string_not_name() {
        isolated(|| {
            let error = frame("TypeError", "old");
            assert!(set(error, "message", "\"new\""));
            assert_eq!(text(unsafe { thaw_error_message(error) }), "new");
            assert_eq!(text(unsafe { thaw_error_to_string(error) }), "TypeError: new");
            assert_eq!(text(unsafe { thaw_error_name(error) }), "TypeError");
            // `message` stays a hidden property; `name` becomes an enumerable own one.
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), "{}");
            assert!(set(error, "name", "\"Custom\""));
            assert_eq!(text(unsafe { thaw_error_name(error) }), "Custom");
            assert_eq!(text(unsafe { thaw_error_to_string(error) }), "Custom: new");
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), r#"{"name":null}"#);
            assert!(unsafe { thaw_error_delete_prop(error, c("message").as_ptr()) });
            assert_eq!(text(unsafe { thaw_error_message(error) }), "");
        });
    }

    #[test]
    fn stack_is_serialized_on_first_read_and_overridable() {
        isolated(|| {
            let error = frame("Error", "s");
            assert_eq!(text(unsafe { thaw_error_stack(error) }), "Error: s");
            assert!(set(error, "message", "\"later\""));
            assert_eq!(text(unsafe { thaw_error_stack(error) }), "Error: s");
            assert_eq!(text(unsafe { thaw_error_to_string(error) }), "Error: later");
            assert!(set(error, "stack", "\"custom\""));
            assert_eq!(text(unsafe { thaw_error_stack(error) }), "custom");
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), "{}");
        });
    }

    #[test]
    fn code_and_cause_prefer_assigned_props() {
        isolated(|| {
            let error = frame("Error", "c");
            assert_eq!(text(unsafe { thaw_error_code(error) }), "undefined");
            assert!(set(error, "code", "\"E_CODE\""));
            assert!(set(error, "cause", "42"));
            assert_eq!(text(unsafe { thaw_error_code(error) }), "E_CODE");
            assert_eq!(text(unsafe { thaw_error_cause(error) }), "42");
            assert_eq!(
                text(unsafe { thaw_error_property_json(error, c("code").as_ptr()) }),
                "\"E_CODE\""
            );
        });
    }

    #[test]
    fn constructor_cause_is_hidden_but_readable_and_baked() {
        isolated(|| {
            let error = frame("Error", "c");
            assert!(set(error, HIDDEN_CAUSE_KEY, r#"{"code":5}"#));
            assert_eq!(get(error, "cause").as_deref(), Some(r#"{"code":5}"#));
            assert!(unsafe { thaw_error_has_prop(error, c("cause").as_ptr()) });
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), "{}");
            assert_eq!(
                text(unsafe { thaw_error_own_keys(error, 1) }),
                r#"{"stack":null,"message":null,"cause":null}"#
            );
            assert_eq!(text(unsafe { thaw_error_json_stringify(error) }), "{}");
            // Assigning `cause` later keeps the property hidden; deleting removes it.
            assert!(set(error, "cause", "7"));
            assert_eq!(get(error, "cause").as_deref(), Some("7"));
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), "{}");
            // A copy of the text carries the cause through the property bag.
            let baked = unsafe { thaw_error_with_props(error) };
            assert_eq!(text(unsafe { thaw_error_cause(baked) }), "7");
            assert_eq!(text(unsafe { thaw_error_own_keys(baked, 0) }), "{}");
            assert!(unsafe { thaw_error_delete_prop(error, c("cause").as_ptr()) });
            assert!(!unsafe { thaw_error_has_prop(error, c("cause").as_ptr()) });
        });
    }

    #[test]
    fn record_stacks_use_the_owner_side_table() {
        isolated(|| {
            let owner = 0x1000usize as *const u8;
            let fallback = c("Error: fallback");
            assert!(!thaw_object_stack_has(owner));
            assert_eq!(text(unsafe { thaw_object_stack_get(owner, fallback.as_ptr()) }), "Error: fallback");
            assert!(unsafe { thaw_object_stack_set(owner, c("Named: kept").as_ptr()) });
            assert!(thaw_object_stack_has(owner));
            assert_eq!(text(unsafe { thaw_object_stack_get(owner, fallback.as_ptr()) }), "Named: kept");
            assert!(!unsafe { thaw_object_stack_set(std::ptr::null(), c("x").as_ptr()) });
        });
    }

    #[test]
    fn plain_strings_are_not_errors() {
        isolated(|| {
            let plain = c("plain");
            assert!(!unsafe { thaw_error_set_prop(plain.as_ptr(), c("k").as_ptr(), c("1").as_ptr()) });
            assert_eq!(get(plain.as_ptr(), "k"), None);
            assert_eq!(text(unsafe { thaw_error_own_keys(plain.as_ptr(), 0) }), r#"{"0":null,"1":null,"2":null,"3":null,"4":null}"#);
            assert_eq!(text(unsafe { thaw_error_json_stringify(plain.as_ptr()) }), "\"plain\"");
            assert!(unsafe { thaw_error_delete_prop(plain.as_ptr(), c("k").as_ptr()) });
            assert!(ERROR_EXTRAS.with(|table| table.borrow().is_empty()));
        });
    }

    #[test]
    fn assign_json_sets_every_field_in_order() {
        isolated(|| {
            let error = frame("Error", "a");
            assert!(unsafe { thaw_error_assign_json(error, c(r#"{"b":1,"a":"x"}"#).as_ptr()) });
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), r#"{"b":null,"a":null}"#);
            assert!(!unsafe { thaw_error_assign_json(c("plain").as_ptr(), c("{}").as_ptr()) });
        });
    }

    #[test]
    fn an_entry_whose_address_now_holds_different_text_is_stale() {
        isolated(|| {
            // Stands in for a freed non-arena string whose address is reused.
            let mut buffer = b"\x01Error\x01a\0".to_vec();
            let error = buffer.as_ptr().cast::<c_char>();
            assert!(set(error, "k", "1"));
            assert_eq!(get(error, "k").as_deref(), Some("1"));
            buffer[7] = b'b';
            assert_eq!(get(error, "k"), None);
            assert!(set(error, "other", "2"));
            assert_eq!(text(unsafe { thaw_error_own_keys(error, 0) }), r#"{"other":null}"#);
        });
    }

    #[test]
    fn arena_reset_prunes_the_table() {
        isolated(|| {
            let error = frame("Error", "reset");
            assert!(set(error, "k", "1"));
            assert!(ERROR_EXTRAS.with(|table| !table.borrow().is_empty()));
            thaw_arena::thaw_arena_reset();
            assert!(ERROR_EXTRAS.with(|table| table.borrow().is_empty()));
        });
    }

    #[test]
    fn with_props_bakes_props_and_overrides_into_the_property_bag() {
        isolated(|| {
            let error = frame("Error", "boom");
            // Nothing to add: the text itself comes back.
            assert_eq!(unsafe { thaw_error_with_props(error) }, error);
            assert!(set(error, "code", "\"E_X\""));
            assert!(set(error, "stack", "\"Error: boom\\n    at x\""));
            let baked = unsafe { thaw_error_with_props(error) };
            assert_ne!(baked, error);
            let baked = text(baked);
            let (_, bag) = baked.split_once(ERROR_PROPS_MARKER).unwrap();
            assert_eq!(bag, r#"{"code":"E_X","stack":"Error: boom\n    at x"}"#);
            // A second bake merges into the existing bag instead of appending another.
            assert!(set(error, "n", "1"));
            let baked = text(unsafe { thaw_error_with_props(error) });
            assert_eq!(baked.matches(ERROR_PROPS_MARKER).count(), 1);
            assert_eq!(text(unsafe { thaw_error_with_props(c("plain").as_ptr()) }), "plain");
        });
    }

    #[test]
    fn entries_json_lists_props_and_index_entries_of_plain_strings() {
        isolated(|| {
            let error = frame("Error", "e");
            assert!(set(error, "a", "1"));
            assert!(set(error, "b", "\"two\""));
            assert_eq!(text(unsafe { thaw_error_entries_json(error) }), r#"{"a":1,"b":"two"}"#);
            assert_eq!(text(unsafe { thaw_error_entries_json(c("ab").as_ptr()) }), r#"{"0":"a","1":"b"}"#);
            assert!(unsafe { thaw_error_entries_json(std::ptr::null()) }.is_null());
        });
    }

    #[test]
    fn owner_props_round_trip_with_hidden_keys() {
        isolated(|| {
            let owner = 0x1000_usize as *const u8;
            assert_eq!(text(unsafe { thaw_error_owner_props(owner, 0) }), "");
            assert!(unsafe { thaw_error_register_owner_props(owner, c(r#"{"status":404,"name":"HttpError"}"#).as_ptr()) });
            assert_eq!(
                text(unsafe { thaw_error_owner_props(owner, 0) }),
                r#"{"status":404,"name":"HttpError"}"#
            );
            assert_eq!(
                text(unsafe { thaw_error_owner_props(owner, 1) }),
                r#"{"stack":null,"message":null,"status":404,"name":"HttpError"}"#
            );
        });
    }

    #[test]
    fn strict_equal_compares_errors_by_identity_and_other_strings_by_text() {
        isolated(|| {
            let first = frame("Error", "x");
            let second = frame("Error", "x");
            assert_ne!(unsafe { thaw_string_strict_equal(first, second) }, 0);
            assert_eq!(unsafe { thaw_string_strict_equal(first, first) }, 0);
            assert_eq!(unsafe { thaw_string_strict_equal(c("x").as_ptr(), c("x").as_ptr()) }, 0);
            assert_ne!(unsafe { thaw_string_strict_equal(c("x").as_ptr(), c("y").as_ptr()) }, 0);
        });
    }
}
