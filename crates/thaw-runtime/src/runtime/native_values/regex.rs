thread_local! {
    static REGEX_CACHE: RefCell<std::collections::HashMap<(String, String), regex::Regex>> =
        RefCell::new(std::collections::HashMap::new());
    static REGEX_GROUPS: RefCell<std::collections::HashMap<usize, Box<serde_json::Value>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Compiles (or reuses a cached compilation of) the regex named by `source`
/// and `flags`, then calls `f` with it. Returns `None` when the pattern
/// fails to compile with the `regex` crate's syntax (which lacks
/// backreferences and lookaround) or an unsupported flag combination.
///
/// Only the `i` (case-insensitive), `m` (multiline) and `s` (dot-all) flags
/// are honored; the `u`/`v` unicode-mode flags are not tracked. `g`/`y`
/// `lastIndex` state is tracked by `RegExp.prototype.exec` alone (see
/// `thaw_regex_exec`/`thaw_regex_exec_advance`) -- `test`, `match`,
/// `matchAll`, `replace`/`replaceAll` and `split` all still match as if
/// searching from the start of the string every call.
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

/// Wraps a freshly built native `[length][elem...]` array/tuple `buffer` in
/// a one-word "handle" cell, matching thaw-llvm's `compile_array_wrap` --
/// every `Array`/`Tuple` value is a handle now, including one built
/// entirely in Rust like `matchAll`'s per-match capture array, since it
/// becomes an *element* of the outer matches array and gets indexed back
/// out expecting a handle. Returns null if `buffer` is null (propagating an
/// earlier allocation failure) or if the handle's own allocation fails.
fn wrap_array_handle(buffer: *mut u8) -> *mut u8 {
    if buffer.is_null() {
        return std::ptr::null_mut();
    }
    let handle = thaw_arena::thaw_arena_alloc(8, 8);
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { handle.cast::<*mut u8>().write_unaligned(buffer) };
    handle
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
    limit: f64,
) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let Some(mut parts) = with_compiled_regex(&source, &flags, |regex| {
        regex.split(&value).map(str::to_string).collect::<Vec<_>>()
    }) else {
        return std::ptr::null_mut();
    };
    // Truncated after computing the full split, not via the regex crate's
    // own `splitn` (which keeps the unsplit remainder in its last piece
    // instead) -- matching thaw_string_split's own limit handling, and
    // JavaScript's own `String.prototype.split(separator, limit)`, which
    // truncates the result rather than limiting how many splits happen.
    let limit = if limit.is_finite() && limit >= 0.0 {
        limit as usize
    } else {
        usize::MAX
    };
    parts.truncate(limit);
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

/// Converts a JavaScript `lastIndex` value (`ToLength` semantics: `NaN` and
/// negative values clamp to `0`) into a UTF-16 code-unit count, matching how
/// `thaw_regex_search` already reports match positions in UTF-16 units for
/// JS compatibility.
fn last_index_to_length(last_index: f64) -> usize {
    if last_index.is_nan() || last_index <= 0.0 {
        0
    } else if last_index >= usize::MAX as f64 {
        usize::MAX
    } else {
        last_index.trunc() as usize
    }
}

/// Converts a UTF-16 code-unit index into a byte offset into `value`'s UTF-8
/// representation. Returns `None` when the index falls past the end of
/// `value` or lands inside a UTF-16 surrogate pair split off by an earlier
/// lossy conversion -- both treated as "no match" by callers, the same way
/// an out-of-range `lastIndex` resets to `0` without matching in the
/// specification.
fn utf16_index_to_byte_offset(value: &str, utf16_index: usize) -> Option<usize> {
    if utf16_index == 0 {
        return Some(0);
    }
    let mut utf16_count = 0usize;
    for (byte_index, ch) in value.char_indices() {
        if utf16_count == utf16_index {
            return Some(byte_index);
        }
        utf16_count += ch.len_utf16();
    }
    (utf16_count == utf16_index).then_some(value.len())
}

fn capture_strings_from(captures: &regex::Captures) -> Vec<String> {
    (0..captures.len())
        .map(|index| {
            captures
                .get(index)
                .map(|group| group.as_str().to_string())
                .unwrap_or_default()
        })
        .collect()
}

#[no_mangle]
/// Matches `value` against the regex named by `source`/`flags`, starting the
/// search at the UTF-16 code-unit index `last_index` -- `RegExp.prototype
/// .exec`'s caller passes `0` for a non-global, non-sticky pattern (matching
/// `RegExp.prototype.test`'s own simplification of always searching from the
/// start), and its own `lastIndex` property otherwise. Returns the whole
/// match followed by each capture group's text (see `capture_strings_from`),
/// or a null pointer when nothing matches, `last_index` is out of range, or
/// `source`/`flags` fails to compile.
///
/// # Safety
/// `source`, `flags` and `value` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_exec(
    source: *const c_char,
    flags: *const c_char,
    value: *const c_char,
    last_index: f64,
) -> *mut u8 {
    if source.is_null() || flags.is_null() || value.is_null() {
        return std::ptr::null_mut();
    }
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let Some(byte_start) = utf16_index_to_byte_offset(&value, last_index_to_length(last_index))
    else {
        return std::ptr::null_mut();
    };
    let Some((matches, groups)) = with_compiled_regex(&source, &flags, |regex| {
        regex.captures_at(&value, byte_start).map(|captures| {
            let groups = regex
                .capture_names()
                .flatten()
                .filter_map(|name| {
                    captures.name(name).map(|capture| {
                        (
                            name.to_string(),
                            serde_json::Value::String(capture.as_str().to_string()),
                        )
                    })
                })
                .collect();
            (capture_strings_from(&captures), serde_json::Value::Object(groups))
        })
    })
    .flatten() else {
        return std::ptr::null_mut();
    };
    if matches.is_empty() {
        return std::ptr::null_mut();
    }
    let result = arena_string_array(matches);
    if !result.is_null() {
        REGEX_GROUPS.with(|stored| {
            stored.borrow_mut().insert(result as usize, Box::new(groups));
        });
    }
    result
}

#[no_mangle]
/// Returns the named capture object associated with a successful
/// `thaw_regex_exec` result, or null for an unrelated array.
///
/// # Safety
/// `matches` must be null or an array handle returned by Thaw.
pub unsafe extern "C" fn thaw_regex_exec_groups(matches: *const u8) -> *mut serde_json::Value {
    if matches.is_null() {
        return std::ptr::null_mut();
    }
    let matches = unsafe { matches.cast::<*const u8>().read_unaligned() };
    if matches.is_null() {
        return std::ptr::null_mut();
    }
    REGEX_GROUPS.with(|stored| {
        stored
            .borrow_mut()
            .get_mut(&(matches as usize))
            .map_or(std::ptr::null_mut(), |groups| groups.as_mut())
    })
}

#[no_mangle]
/// Computes the `lastIndex` a stateful (`g` or `y` flagged) `RegExp` should
/// hold after a successful `exec`/`test` call that searched `value` starting
/// at the UTF-16 code-unit index `last_index`, matching the specification's
/// `AdvanceStringIndex` (a zero-length match advances by one code unit
/// rather than looping forever) and its sticky-flag requirement that the
/// match start exactly at `last_index`. Returns `-1` when there is no such
/// match, `last_index` is out of range, or `source`/`flags` fails to
/// compile -- the caller resets `lastIndex` to `0` in that case.
///
/// # Safety
/// `value`, `source` and `flags` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_exec_advance(
    value: *const c_char,
    source: *const c_char,
    flags: *const c_char,
    last_index: f64,
) -> f64 {
    if value.is_null() || source.is_null() || flags.is_null() {
        return -1.0;
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let source = unsafe { CStr::from_ptr(source) }.to_string_lossy();
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let sticky = flags.contains('y');
    let Some(byte_start) = utf16_index_to_byte_offset(&value, last_index_to_length(last_index))
    else {
        return -1.0;
    };
    with_compiled_regex(&source, &flags, |regex| {
        let Some(found) = regex.find_at(&value, byte_start) else {
            return -1.0;
        };
        if sticky && found.start() != byte_start {
            return -1.0;
        }
        let mut end = value[..found.end()].encode_utf16().count();
        if found.start() == found.end() {
            end += 1;
        }
        end as f64
    })
    .unwrap_or(-1.0)
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

/// Writes `pointers` into a fresh arena-allocated array of raw pointers
/// (length prefix, then one pointer-sized slot per element), returning
/// null on allocation failure. Unlike `arena_string_array`, the pointers
/// are written as-is rather than built from owned strings -- used for an
/// array whose elements are themselves other arena-allocated arrays.
fn arena_pointer_array(pointers: Vec<*mut u8>) -> *mut u8 {
    let output = thaw_arena::thaw_arena_alloc((pointers.len() + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(pointers.len() as u64);
    }
    for (index, pointer) in pointers.into_iter().enumerate() {
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<*mut u8>()
                .write_unaligned(pointer);
        }
    }
    output
}

#[no_mangle]
/// `String.prototype.matchAll`: an array of match-info arrays (the whole
/// match followed by each capture group's text, same shape and
/// empty-string-for-non-participating-group behavior as non-global
/// `.match()`), one per match found, in order. Unlike `.match()`, capture
/// groups are always included here even though every match is found (that
/// is the entire point of `matchAll` over a global `.match()`). Returns a
/// null pointer for a null argument, a pattern the `regex` crate cannot
/// compile, or a `flags` that lacks `g` -- the generated code always
/// checks for `g` itself first and throws a specific message for that
/// case, so this only needs to return *some* failure signal for it, not
/// distinguish it from a compile failure.
///
/// # Safety
/// `value`, `source` and `flags` must be null or point to valid
/// NUL-terminated UTF-8 strings.
pub unsafe extern "C" fn thaw_regex_match_all(
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
    if !flags.contains('g') {
        return std::ptr::null_mut();
    }
    let Some(matches) = with_compiled_regex(&source, &flags, |regex| {
        regex
            .captures_iter(&value)
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
            .collect::<Vec<_>>()
    }) else {
        return std::ptr::null_mut();
    };
    let mut inner_arrays = Vec::with_capacity(matches.len());
    for captures in matches {
        let inner = wrap_array_handle(arena_string_array(captures));
        if inner.is_null() {
            return std::ptr::null_mut();
        }
        inner_arrays.push(inner);
    }
    arena_pointer_array(inner_arrays)
}
