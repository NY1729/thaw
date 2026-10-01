thread_local! {
    static REGEX_CACHE: RefCell<std::collections::HashMap<(String, String), CompiledRegex>> =
        RefCell::new(std::collections::HashMap::new());
    /// Match metadata (named groups, UTF-16 match index, and input string)
    /// keyed by the raw `[length][elem...]` buffer a successful
    /// `exec`/`match`/`matchAll` produced. The generated code reads these
    /// back through `.groups`/`.index`/`.input`; the buffer is the stable
    /// identity every accessor can recover from its array handle.
    static REGEX_META: RefCell<std::collections::HashMap<usize, RegexMatchMeta>> =
        RefCell::new(std::collections::HashMap::new());
}

struct RegexMatchMeta {
    groups: *mut u8,
    index: f64,
    input: String,
}

fn reset_regex_meta(tracing: bool) {
    REGEX_META.with(|stored| {
        let mut stored = stored.borrow_mut();
        if tracing {
            stored.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            stored.clear();
        }
    });
}

fn store_regex_meta(pointer: *mut u8, meta: RegexMatchMeta) {
    thaw_arena::register_reset_hook(reset_regex_meta);
    REGEX_META.with(|stored| {
        stored.borrow_mut().insert(pointer as usize, meta);
    });
}

// `thaw-runtime` has no Rust-level access to thaw-std's `Json` `Value`
// type (see `AnyKey`'s own doc comment in `maps.rs`), so the named-capture
// object is built through thaw-std's exported JSON entry point: serialize
// the (string-valued) groups to JSON text and parse it back as a live
// `Json` value. `thaw_json_parse` returns a leaked, shared value, so the
// pointer stays valid/identity-stable for the process lifetime.
unsafe extern "C" {
    fn thaw_json_parse(text: *const std::os::raw::c_char) -> *mut u8;
}

fn groups_json_pointer(groups: serde_json::Map<String, serde_json::Value>) -> *mut u8 {
    let text = serde_json::to_string(&serde_json::Value::Object(groups)).unwrap_or_default();
    let Ok(text) = std::ffi::CString::new(text) else {
        return std::ptr::null_mut();
    };
    unsafe { thaw_json_parse(text.as_ptr()) }
}

/// A compiled pattern: the `regex` crate when it can compile the pattern,
/// otherwise `fancy-regex`, which adds backreferences and lookaround. A
/// `fancy-regex` pattern that exceeds its backtracking limit reports an
/// error at match time; those calls degrade to "no match" at the call
/// sites below.
enum CompiledRegex {
    Native(regex::Regex),
    Fancy(fancy_regex::Regex),
}

impl CompiledRegex {
    fn compile(source: &str, flags: &str) -> Option<Self> {
        if let Ok(native) = regex::RegexBuilder::new(source)
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'))
            .build()
        {
            return Some(Self::Native(native));
        }
        fancy_regex::RegexBuilder::new(source)
            .case_insensitive(flags.contains('i'))
            .multi_line(flags.contains('m'))
            .dot_matches_new_line(flags.contains('s'))
            .build()
            .ok()
            .map(Self::Fancy)
    }

    fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Native(regex) => regex.is_match(text),
            Self::Fancy(regex) => regex.is_match(text).unwrap_or(false),
        }
    }

    fn captures(&self, text: &str) -> Option<RegexCaptures> {
        match self {
            Self::Native(regex) => regex.captures(text).map(RegexCaptures::from_native),
            Self::Fancy(regex) => regex
                .captures(text)
                .ok()
                .flatten()
                .map(RegexCaptures::from_fancy),
        }
    }

    fn captures_at(&self, text: &str, start: usize) -> Option<RegexCaptures> {
        match self {
            Self::Native(regex) => regex
                .captures_at(text, start)
                .map(RegexCaptures::from_native),
            Self::Fancy(regex) => regex
                .captures_from_pos(text, start)
                .ok()
                .flatten()
                .map(RegexCaptures::from_fancy),
        }
    }

    fn find_at(&self, text: &str, start: usize) -> Option<(usize, usize)> {
        match self {
            Self::Native(regex) => regex.find_at(text, start).map(|found| (found.start(), found.end())),
            Self::Fancy(regex) => regex
                .find_from_pos(text, start)
                .ok()
                .flatten()
                .map(|found| (found.start(), found.end())),
        }
    }

    fn find_iter(&self, text: &str) -> Vec<String> {
        match self {
            Self::Native(regex) => regex
                .find_iter(text)
                .map(|found| found.as_str().to_string())
                .collect(),
            Self::Fancy(regex) => regex
                .find_iter(text)
                .filter_map(|found| found.ok())
                .map(|found| found.as_str().to_string())
                .collect(),
        }
    }

    fn captures_iter(&self, text: &str) -> Vec<RegexCaptures> {
        match self {
            Self::Native(regex) => regex
                .captures_iter(text)
                .map(RegexCaptures::from_native)
                .collect(),
            Self::Fancy(regex) => regex
                .captures_iter(text)
                .filter_map(|captures| captures.ok())
                .map(RegexCaptures::from_fancy)
                .collect(),
        }
    }

    fn capture_names(&self) -> Vec<Option<&str>> {
        match self {
            Self::Native(regex) => regex.capture_names().collect(),
            Self::Fancy(regex) => regex.capture_names().collect(),
        }
    }

    fn split(&self, text: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut cursor = 0;
        for captures in self.captures_iter(text) {
            let matched = captures.get(0).unwrap_or("");
            let start = captures.start;
            let end = start + matched.len();
            if start == end && (start == 0 || start == text.len()) {
                continue;
            }
            parts.push(text[cursor..start].to_string());
            parts.extend(
                captures.groups[1..]
                    .iter()
                    .map(|group| group.clone().unwrap_or_default()),
            );
            cursor = end;
        }
        parts.push(text[cursor..].to_string());
        parts
    }
}

/// A match's captured groups as owned strings (index 0 is the whole match).
/// A group that did not participate is an empty string, matching the native
/// array element type.
struct RegexCaptures {
    start: usize,
    /// `None` for a group that did not participate in the match.
    groups: Vec<Option<String>>,
}

impl RegexCaptures {
    fn from_native(captures: regex::Captures<'_>) -> Self {
        Self {
            start: captures.get(0).map_or(0, |matched| matched.start()),
            groups: (0..captures.len())
                .map(|index| captures.get(index).map(|group| group.as_str().to_string()))
                .collect(),
        }
    }

    fn from_fancy(captures: fancy_regex::Captures<'_>) -> Self {
        Self {
            start: captures.get(0).map_or(0, |matched| matched.start()),
            groups: (0..captures.len())
                .map(|index| captures.get(index).map(|group| group.as_str().to_string()))
                .collect(),
        }
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.groups.get(index).and_then(|group| group.as_deref())
    }

    /// The whole match followed by each group, a non-participating group as
    /// an empty string (the native array element type is a plain string).
    fn into_positional(self) -> Vec<String> {
        self.groups
            .into_iter()
            .map(|group| group.unwrap_or_default())
            .collect()
    }
}

/// Compiles (or reuses a cached compilation of) the regex named by `source`
/// and `flags`, then calls `f` with it. Returns `None` when neither the
/// `regex` crate nor `fancy-regex` can compile the pattern.
///
/// Only the `i` (case-insensitive), `m` (multiline) and `s` (dot-all) flags
/// are honored; the `u`/`v` unicode-mode flags are not tracked. `g`/`y`
/// `lastIndex` state is tracked by `RegExp.prototype.exec`/`test` alone (see
/// `thaw_regex_exec`/`thaw_regex_exec_advance`) -- `match`, `matchAll`,
/// `replace`/`replaceAll` and `split` all still match as if searching from
/// the start of the string every call.
fn with_compiled_regex<T>(
    source: &str,
    flags: &str,
    f: impl FnOnce(&CompiledRegex) -> T,
) -> Option<T> {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (source.to_string(), flags.to_string());
        if !cache.contains_key(&key) {
            cache.insert(key.clone(), CompiledRegex::compile(source, flags)?);
        }
        cache.get(&key).map(f)
    })
}

/// Wraps a freshly built native `[length][elem...]` array/tuple `buffer` in
/// a two-word handle cell, matching thaw-llvm's `compile_array_wrap` --
/// every `Array`/`Tuple` value is a handle now, including one built
/// entirely in Rust like `matchAll`'s per-match capture array, since it
/// becomes an *element* of the outer matches array and gets indexed back
/// out expecting a handle. Returns null if `buffer` is null (propagating an
/// earlier allocation failure) or if the handle's own allocation fails.
fn wrap_array_handle(buffer: *mut u8) -> *mut u8 {
    if buffer.is_null() {
        return std::ptr::null_mut();
    }
    let handle = thaw_arena::thaw_arena_alloc(16, 8);
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        handle.cast::<*mut u8>().write_unaligned(buffer);
        handle.add(8).cast::<*mut u8>().write_unaligned(std::ptr::null_mut());
    }
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
            .find_at(&value, 0)
            .map(|(start, _)| value[..start].encode_utf16().count() as f64)
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
        let captures = if global {
            regex.captures_iter(&value)
        } else {
            regex.captures(&value).into_iter().collect()
        };
        let mut output = String::new();
        let mut cursor = 0;
        for captures in captures {
            let matched = captures.get(0).unwrap_or("");
            let start = captures.start;
            let end = start + matched.len();
            let names = regex.capture_names();
            let named_captures: Vec<_> = names
                .into_iter()
                .zip(captures.groups.iter())
                .filter_map(|(name, value)| name.map(|name| (name.to_string(), value.clone())))
                .collect();
            output.push_str(&value[cursor..start]);
            output.push_str(&expand_replacement(
                &replacement,
                matched,
                &value[..start],
                &value[end..],
                &captures.groups[1..],
                &named_captures,
            ));
            cursor = end;
        }
        output.push_str(&value[cursor..]);
        output
    }) else {
        return std::ptr::null();
    };
    arena_c_string(&replaced).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Replaces the first match of the regex named by `source`/`flags` in
/// `value` with `replacement`, or every match when `flags` contains `g`,
/// matching `String.prototype.replace` for a `RegExp` search value.
/// Returns a null pointer for a null argument or a pattern the `regex`
/// crate cannot compile.
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
    let Some(mut parts) = with_compiled_regex(&source, &flags, |regex| regex.split(&value)) else {
        return std::ptr::null_mut();
    };
    // Truncated after computing the full split, not via the regex crate's
    // own `splitn` (which keeps the unsplit remainder in its last piece
    // instead) -- matching thaw_string_split's own limit handling, and
    // JavaScript's own `String.prototype.split(separator, limit)`, which
    // truncates the result rather than limiting how many splits happen.
    let limit = if limit == -1.0 {
        usize::MAX
    } else if limit.is_finite() {
        limit.trunc().rem_euclid(4_294_967_296.0) as usize
    } else {
        0
    };
    parts.truncate(limit);
    arena_string_array(parts)
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
    let Some((matches, groups, index)) = with_compiled_regex(&source, &flags, |regex| {
        regex.captures_at(&value, byte_start).and_then(|captures| {
            if flags.contains('y') && captures.start != byte_start {
                return None;
            }
            let index = value[..captures.start].encode_utf16().count() as f64;
            let groups = regex
                .capture_names()
                .into_iter()
                .enumerate()
                .filter_map(|(group_index, name)| {
                    let name = name?;
                    captures.get(group_index).map(|capture| {
                        (
                            name.to_string(),
                            serde_json::Value::String(capture.to_string()),
                        )
                    })
                })
                .collect();
            Some((
                captures.into_positional(),
                groups_json_pointer(groups),
                index,
            ))
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
        store_regex_meta(result, RegexMatchMeta {
            groups,
            index,
            input: value.to_string(),
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
pub unsafe extern "C" fn thaw_regex_exec_groups(matches: *const u8) -> *mut u8 {
    if matches.is_null() {
        return std::ptr::null_mut();
    }
    let matches = unsafe { matches.cast::<*const u8>().read_unaligned() };
    if matches.is_null() {
        return std::ptr::null_mut();
    }
    REGEX_META.with(|stored| {
        stored
            .borrow()
            .get(&(matches as usize))
            .map_or(std::ptr::null_mut(), |meta| meta.groups)
    })
}

#[no_mangle]
/// Returns the UTF-16 code-unit index of the match associated with a
/// successful `exec`/`match`/`matchAll` result, matching the array's own
/// `.index` property. `-1` for an unrelated array (the generated code only
/// reaches this for a regex result, but a plain `string[]` shares the same
/// native type).
///
/// # Safety
/// `matches` must be null or an array handle returned by Thaw.
pub unsafe extern "C" fn thaw_regex_exec_index(matches: *const u8) -> f64 {
    if matches.is_null() {
        return -1.0;
    }
    let matches = unsafe { matches.cast::<*const u8>().read_unaligned() };
    if matches.is_null() {
        return -1.0;
    }
    REGEX_META.with(|stored| {
        stored
            .borrow()
            .get(&(matches as usize))
            .map_or(-1.0, |meta| meta.index)
    })
}

#[no_mangle]
/// Returns the input string the match associated with a successful
/// `exec`/`match`/`matchAll` result searched, matching the array's own
/// `.input` property. Returns null for an unrelated array.
///
/// # Safety
/// `matches` must be null or an array handle returned by Thaw.
pub unsafe extern "C" fn thaw_regex_exec_input(matches: *const u8) -> *const c_char {
    if matches.is_null() {
        return std::ptr::null();
    }
    let matches = unsafe { matches.cast::<*const u8>().read_unaligned() };
    if matches.is_null() {
        return std::ptr::null();
    }
    let input = REGEX_META.with(|stored| {
        stored
            .borrow()
            .get(&(matches as usize))
            .map(|meta| meta.input.clone())
    });
    input
        .and_then(|input| arena_c_string(&input))
        .map_or(std::ptr::null(), |pointer| pointer.cast())
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
        let Some((start, end)) = regex.find_at(&value, byte_start) else {
            return -1.0;
        };
        if sticky && start != byte_start {
            return -1.0;
        }
        let mut units = value[..end].encode_utf16().count();
        if start == end {
            units += 1;
        }
        units as f64
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
/// `string`. For the non-global form the result's `.index`/`.input`/`.groups`
/// metadata is recorded with `REGEX_META`, matching `RegExpBuiltinExec`;
/// the global form exposes none of it, matching `.match()` with `g`. Returns
/// a null pointer both when nothing matches and when `source`/`flags` fails
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
    let mut meta = None;
    let Some(matches) = with_compiled_regex(&source, &flags, |regex| {
        if global {
            regex.find_iter(&value)
        } else {
            match regex.captures(&value) {
                Some(captures) => {
                    let index = value[..captures.start].encode_utf16().count() as f64;
                    let groups = regex
                        .capture_names()
                        .into_iter()
                        .enumerate()
                        .filter_map(|(group_index, name)| {
                            let name = name?;
                            captures.get(group_index).map(|capture| {
                                (
                                    name.to_string(),
                                    serde_json::Value::String(capture.to_string()),
                                )
                            })
                        })
                        .collect();
                    meta = Some((groups_json_pointer(groups), index));
                    captures.into_positional()
                }
                None => Vec::new(),
            }
        }
    }) else {
        return std::ptr::null_mut();
    };
    if matches.is_empty() {
        return std::ptr::null_mut();
    }
    let result = arena_string_array(matches);
    if !result.is_null() {
        if let Some((groups, index)) = meta {
            store_regex_meta(result, RegexMatchMeta {
                groups,
                index,
                input: value.to_string(),
            });
        }
    }
    result
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
        let names = regex.capture_names();
        regex
            .captures_iter(&value)
            .into_iter()
            .map(|captures| {
                let index = value[..captures.start].encode_utf16().count() as f64;
                let groups = names
                    .iter()
                    .enumerate()
                    .filter_map(|(group_index, name)| {
                        let name = (*name)?;
                        captures.get(group_index).map(|capture| {
                            (
                                name.to_string(),
                                serde_json::Value::String(capture.to_string()),
                            )
                        })
                    })
                    .collect();
                (
                    captures.into_positional(),
                    groups_json_pointer(groups),
                    index,
                )
            })
            .collect::<Vec<_>>()
    }) else {
        return std::ptr::null_mut();
    };
    let mut inner_arrays = Vec::with_capacity(matches.len());
    for (captures, groups, index) in matches {
        let buffer = arena_string_array(captures);
        let inner = wrap_array_handle(buffer);
        if inner.is_null() {
            return std::ptr::null_mut();
        }
        if !buffer.is_null() {
            store_regex_meta(buffer, RegexMatchMeta {
                groups,
                index,
                input: value.to_string(),
            });
        }
        inner_arrays.push(inner);
    }
    arena_pointer_array(inner_arrays)
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn match_metadata_expires_with_its_arena_buffer() {
        let buffer = thaw_arena::thaw_arena_alloc(8, 8);
        store_regex_meta(buffer, RegexMatchMeta {
            groups: std::ptr::null_mut(),
            index: 3.0,
            input: "old".into(),
        });
        thaw_arena::thaw_arena_reset();
        REGEX_META.with(|stored| assert!(!stored.borrow().contains_key(&(buffer as usize))));
    }
}
