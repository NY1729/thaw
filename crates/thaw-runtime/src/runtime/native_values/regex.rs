thread_local! {
    static REGEX_CACHE: RefCell<std::collections::HashMap<(String, String), regex::Regex>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Compiles (or reuses a cached compilation of) the regex named by `source`
/// and `flags`, then calls `f` with it. Returns `None` when the pattern
/// fails to compile with the `regex` crate's syntax (which lacks
/// backreferences and lookaround) or an unsupported flag combination.
///
/// Only the `i` (case-insensitive), `m` (multiline) and `s` (dot-all) flags
/// are honored; `g`/`y` sticky/global `lastIndex` state and the `u`/`v`
/// unicode-mode flags are not tracked, so every call matches as if searching
/// from the start of the string.
fn with_compiled_regex<T>(
    source: &str,
    flags: &str,
    f: impl FnOnce(&regex::Regex) -> T,
) -> Option<T> {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (source.to_string(), flags.to_string());
        if !cache.contains_key(&key) {
            let compiled = regex::RegexBuilder::new(source)
                .case_insensitive(flags.contains('i'))
                .multi_line(flags.contains('m'))
                .dot_matches_new_line(flags.contains('s'))
                .build()
                .ok()?;
            cache.insert(key.clone(), compiled);
        }
        cache.get(&key).map(f)
    })
}

/// Writes `parts` into a fresh arena-allocated native `string[]` (length
/// prefix, then one pointer-sized slot per element), returning null on
/// allocation failure.
fn arena_string_array(parts: Vec<String>) -> *mut u8 {
    let output = thaw_arena::thaw_arena_alloc((parts.len() + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(parts.len() as u64);
    }
    for (index, part) in parts.into_iter().enumerate() {
        let Some(part) = arena_c_string(&part) else {
            return std::ptr::null_mut();
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<*const u8>()
                .write_unaligned(part);
        }
    }
    output
}

#[no_mangle]
/// Tests whether `value` matches the regex named by `source`/`flags`,
/// matching `RegExp.prototype.test` (without `g`/`y` `lastIndex` state).
/// Returns `0` for a null argument or a pattern the `regex` crate cannot
/// compile (for example one that uses backreferences or lookaround).
///
/// # Safety
/// `source`, `flags` and `value` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_test(
    source: *const c_char,
    flags: *const c_char,
    value: *const c_char,
) -> u8 {
    if source.is_null() || flags.is_null() || value.is_null() {
        return 0;
    }
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    with_compiled_regex(&source, &flags, |regex| regex.is_match(&value)).unwrap_or(false) as u8
}

#[no_mangle]
/// Returns the JavaScript UTF-16 code-unit index of the first match of the
/// regex named by `source`/`flags` in `value`, matching
/// `String.prototype.search`. Returns `-1` for a null argument, no match, or
/// a pattern the `regex` crate cannot compile, the same way
/// `RegExp.prototype.test` cannot distinguish "no match" from "failed to
/// compile".
///
/// # Safety
/// `value`, `source` and `flags` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_search(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
) -> f64 {
    if value.is_null() || source.is_null() || flags.is_null() {
        return -1.0;
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    with_compiled_regex(&source, &flags, |regex| {
        regex
            .find(&value)
            .map(|found| value[..found.start()].encode_utf16().count() as f64)
    })
    .flatten()
    .unwrap_or(-1.0)
}

/// # Safety
/// `value`, `source`, `flags` and `replacement` must be null or point to
/// valid NUL-terminated UTF-8 strings.
unsafe fn thaw_regex_replace_impl(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
    replacement: *const c_char,
    all: bool,
) -> *const c_char {
    if value.is_null() || source.is_null() || flags.is_null() || replacement.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let replacement = unsafe { CStr::from_ptr(replacement) }.to_string_lossy();
    let global = all || flags.contains('g');
    let Some(replaced) = with_compiled_regex(&source, &flags, |regex| {
        if global {
            regex
                .replace_all(&value, regex::NoExpand(&replacement))
                .into_owned()
        } else {
            regex
                .replacen(&value, 1, regex::NoExpand(&replacement))
                .into_owned()
        }
    }) else {
        return std::ptr::null();
    };
    arena_c_string(&replaced).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Replaces the first match of the regex named by `source`/`flags` in
/// `value` with `replacement`, or every match when `flags` contains `g`,
/// matching `String.prototype.replace` for a `RegExp` search value.
/// `replacement` is inserted literally: `$1`/`$&`-style capture-group
/// interpolation is not supported. Returns a null pointer for a null
/// argument or a pattern the `regex` crate cannot compile.
///
/// # Safety
/// `value`, `source`, `flags` and `replacement` must be null or point to
/// valid NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_replace(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
    replacement: *const c_char,
) -> *const c_char {
    unsafe { thaw_regex_replace_impl(value, source, flags, replacement, false) }
}

#[no_mangle]
/// Replaces every match of the regex named by `source`/`flags` in `value`
/// with `replacement`, matching `String.prototype.replaceAll` for a
/// `RegExp` search value. Returns a null pointer (to be reported as a
/// `TypeError`, matching the specification) when `flags` does not contain
/// `g`, and otherwise the same null-pointer failure cases as
/// `thaw_regex_replace`.
///
/// # Safety
/// `value`, `source`, `flags` and `replacement` must be null or point to
/// valid NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_replace_all(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
    replacement: *const c_char,
) -> *const c_char {
    if flags.is_null() {
        return std::ptr::null();
    }
    let flag_text = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    if !flag_text.contains('g') {
        return std::ptr::null();
    }
    unsafe { thaw_regex_replace_impl(value, source, flags, replacement, true) }
}

#[no_mangle]
/// Splits `value` on every match of the regex named by `source`/`flags`
/// into the native `string[]` array layout, matching
/// `String.prototype.split` for a `RegExp` separator. Returns a null
/// pointer for a null argument or a pattern the `regex` crate cannot
/// compile.
///
/// # Safety
/// `value`, `source` and `flags` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_split(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let Some(parts) = with_compiled_regex(&source, &flags, |regex| {
        regex.split(&value).map(str::to_string).collect::<Vec<_>>()
    }) else {
        return std::ptr::null_mut();
    };
    arena_string_array(parts)
}

/// Returns the whole match followed by each capture group's text, or an
/// empty vector when nothing matches. A group that did not participate in
/// the match (for example one inside an unmatched alternative) is reported
/// as an empty string rather than `undefined`, since the native array
/// element type is a plain `string`.
fn capture_strings(regex: &regex::Regex, value: &str) -> Vec<String> {
    regex
        .captures(value)
        .map(|captures| {
            (0..captures.len())
                .map(|index| {
                    captures
                        .get(index)
                        .map(|group| group.as_str().to_string())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[no_mangle]
/// Matches `value` against the regex named by `source`/`flags`, matching
/// `RegExp.prototype.exec` without `g`/`y` `lastIndex` state -- every call
/// searches from the start of `value`, matching `RegExp.prototype.test`'s
/// own simplification. Returns the whole match followed by each capture
/// group's text (see `capture_strings`), or a null pointer when nothing
/// matches or `source`/`flags` fails to compile.
///
/// # Safety
/// `source`, `flags` and `value` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_exec(
    source: *const c_char,
    flags: *const c_char,
    value: *const c_char,
) -> *mut u8 {
    if source.is_null() || flags.is_null() || value.is_null() {
        return std::ptr::null_mut();
    }
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let Some(matches) =
        with_compiled_regex(&source, &flags, |regex| capture_strings(regex, &value))
    else {
        return std::ptr::null_mut();
    };
    if matches.is_empty() {
        return std::ptr::null_mut();
    }
    arena_string_array(matches)
}

#[no_mangle]
/// Matches `value` against the regex named by `source`/`flags`, matching
/// `String.prototype.match`. Returns every whole match when `flags`
/// contains `g` (capture groups are not exposed in this mode, matching
/// `String.prototype.match`'s own behavior for a global pattern), or the
/// whole match followed by each capture group's text when it matches
/// otherwise -- a group that did not participate in the match (for example
/// one inside an unmatched alternative) is reported as an empty string
/// rather than `undefined`, since the native array element type is a plain
/// `string`. `.index` and `.input` are not exposed either way. Returns a
/// null pointer both when nothing matches and when `source`/`flags` fails
/// to compile; the generated code distinguishes these only in that both
/// report "no match" (`undefined`), matching what a caller observes for
/// either case.
///
/// # Safety
/// `value`, `source` and `flags` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_match(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let global = flags.contains('g');
    let Some(matches) = with_compiled_regex(&source, &flags, |regex| {
        if global {
            regex
                .find_iter(&value)
                .map(|found| found.as_str().to_string())
                .collect::<Vec<_>>()
        } else {
            capture_strings(regex, &value)
        }
    }) else {
        return std::ptr::null_mut();
    };
    if matches.is_empty() {
        return std::ptr::null_mut();
    }
    arena_string_array(matches)
}
