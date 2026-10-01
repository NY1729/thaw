thread_local! {
    static REGEX_CACHE: RefCell<std::collections::HashMap<(Vec<u16>, String), regress::Regex>> =
        RefCell::new(std::collections::HashMap::new());
    /// Match metadata (named groups, UTF-16 match index, and input string)
    /// keyed by the stable array handle. Array mutations replace the backing
    /// buffer while preserving this handle.
    static REGEX_META: RefCell<std::collections::HashMap<usize, RegexMatchMeta>> =
        RefCell::new(std::collections::HashMap::new());
}

struct RegexMatchMeta {
    groups: *mut u8,
    index: f64,
    input: Vec<u8>,
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

// `thaw-runtime` has no Rust-level access to thaw-std's `Json` value type.
// Build the named-capture object through its existing exported setters so
// nonparticipating groups are undefined and lone-surrogate captures remain
// exact WTF-8 values.
unsafe extern "C" {
    fn thaw_json_object_new() -> *mut u8;
    fn thaw_json_object_set_string(object: *mut u8, key: *const c_char, value: *const c_char);
    fn thaw_json_undefined() -> *mut u8;
    fn thaw_json_object_set_json_owned(object: *mut u8, key: *const c_char, value: *mut u8);
}

fn groups_json_pointer(groups: Vec<(String, Option<Vec<u16>>)>) -> *mut u8 {
    if groups.is_empty() {
        return std::ptr::null_mut();
    }
    let object = unsafe { thaw_json_object_new() };
    if object.is_null() {
        return object;
    }
    for (name, value) in groups {
        let name = thaw_arena::arena_string(name.as_bytes());
        if name.is_null() { return std::ptr::null_mut(); }
        if let Some(value) = value {
            let value = thaw_arena::arena_string(&wtf8_encode_utf16(&value));
            if value.is_null() { return std::ptr::null_mut(); }
            unsafe { thaw_json_object_set_string(object, name, value) };
        } else {
            unsafe { thaw_json_object_set_json_owned(object, name, thaw_json_undefined()) };
        }
    }
    object
}

/// Decode every native string as UTF-16, retaining lone surrogate code units.
unsafe fn regex_units(value: *const c_char) -> Vec<u16> {
    wtf8_decode_utf16(unsafe { CStr::from_ptr(value) }.to_bytes())
}

fn compiled_match(regex: &regress::Regex, text: &[u16], flags: &str, start: usize) -> Option<RegexCaptures> {
    if start > text.len() {
        return None;
    }
    let matched = if flags.contains('u') || flags.contains('v') {
        regex.find_from_utf16(text, start).next()
    } else {
        regex.find_from_ucs2(text, start).next()
    }?;
    Some(RegexCaptures::from_match(matched, text))
}

fn advance_string_index(text: &[u16], index: usize, flags: &str) -> usize {
    if (flags.contains('u') || flags.contains('v'))
        && text.get(index).is_some_and(|unit| (0xD800..=0xDBFF).contains(unit))
        && text.get(index + 1).is_some_and(|unit| (0xDC00..=0xDFFF).contains(unit))
    {
        index + 2
    } else {
        index + 1
    }
}

/// Captures retain UTF-16 ranges so a half-surrogate match is representable.
struct RegexCaptures {
    start: usize,
    end: usize,
    groups: Vec<Option<Vec<u16>>>,
    names: Vec<(String, Option<Vec<u16>>)>,
}

impl RegexCaptures {
    fn from_match(matched: regress::Match, text: &[u16]) -> Self {
        let capture = |range: Option<std::ops::Range<usize>>| range.map(|range| text[range].to_vec());
        let groups = (0..=matched.captures.len())
            .map(|index| capture(matched.group(index)))
            .collect();
        let names = matched.named_groups()
            .map(|(name, range)| (name.to_string(), capture(range)))
            .collect();
        Self { start: matched.start(), end: matched.end(), groups, names }
    }

    fn into_positional(self) -> Vec<Option<Vec<u16>>> {
        self.groups
    }
}

fn pattern_codepoints(source: &[u16], flags: &str) -> Vec<u32> {
    let unicode = flags.contains('u') || flags.contains('v');
    let mut result = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        let first = source[index];
        if unicode && (0xD800..=0xDBFF).contains(&first)
            && source.get(index + 1).is_some_and(|next| (0xDC00..=0xDFFF).contains(next))
        {
            let low = source[index + 1];
            result.push(0x10000 + ((u32::from(first) - 0xD800) << 10) + u32::from(low) - 0xDC00);
            index += 2;
        } else {
            result.push(u32::from(first));
            index += 1;
        }
    }
    result
}

/// Keep compiled JavaScript syntax cached; `g` and `y` remain caller state.
fn with_compiled_regex<T>(
    source: &[u16],
    flags: &str,
    f: impl FnOnce(&regress::Regex) -> T,
) -> Option<T> {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (source.to_vec(), flags.to_string());
        if !cache.contains_key(&key) {
            let mut options = regress::Flags::from(flags);
            // ECMAScript `v` implies Unicode code-point input as well as sets.
            if options.unicode_sets { options.unicode = true; }
            let compiled = regress::Regex::from_unicode(
                pattern_codepoints(source, flags).into_iter(), options,
            ).ok()?;
            cache.insert(key.clone(), compiled);
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

fn arena_optional_string_array(parts: Vec<Option<Vec<u16>>>) -> *mut u8 {
    let output = thaw_arena::thaw_arena_alloc(8 + parts.len() * 16, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.write_bytes(0, 8 + parts.len() * 16);
        output.cast::<u64>().write(parts.len() as u64);
    }
    for (index, part) in parts.into_iter().enumerate() {
        let Some(part) = part else { continue };
        let pointer = thaw_arena::arena_string(&wtf8_encode_utf16(&part));
        if pointer.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            output.add(8 + index * 16).write(1);
            output.add(16 + index * 16).cast::<*const c_char>().write_unaligned(pointer);
        }
    }
    output
}

#[no_mangle]
/// Tests a JavaScript regular expression without caller-managed lastIndex state.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_test(source: *const c_char, flags: *const c_char, value: *const c_char) -> u8 {
    if source.is_null() || flags.is_null() || value.is_null() { return 0; }
    let source = unsafe { regex_units(source) };
    let text = unsafe { regex_units(value) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    u8::from(with_compiled_regex(&source, &flags, |regex| compiled_match(regex, &text, &flags, 0).is_some()).unwrap_or(false))
}

#[no_mangle]
/// Returns the first match's UTF-16 start, respecting a sticky search at zero.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_search(value: *const c_char, source: *const c_char, flags: *const c_char) -> f64 {
    if source.is_null() || flags.is_null() || value.is_null() { return -1.0; }
    let source = unsafe { regex_units(source) };
    let text = unsafe { regex_units(value) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    with_compiled_regex(&source, &flags, |regex| {
        compiled_match(regex, &text, &flags, 0)
            .filter(|matched| !flags.contains('y') || matched.start == 0)
            .map_or(-1.0, |matched| matched.start as f64)
    }).unwrap_or(-1.0)
}

fn expand_regex_replacement(
    replacement: &[u16], matched: &[u16], prefix: &[u16], suffix: &[u16],
    captures: &[Option<Vec<u16>>], names: &[(String, Option<Vec<u16>>)],
) -> Vec<u16> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < replacement.len() {
        if replacement[index] != b'$' as u16 || index + 1 == replacement.len() {
            output.push(replacement[index]);
            index += 1;
            continue;
        }
        let next = replacement[index + 1];
        match next {
            0x24 => output.push(0x24),
            0x26 => output.extend_from_slice(matched),
            0x60 => output.extend_from_slice(prefix),
            0x27 => output.extend_from_slice(suffix),
            0x30..=0x39 if !captures.is_empty() => {
                let first = usize::from(next - 0x30);
                let two = replacement.get(index + 2).copied()
                    .filter(|unit| (0x30..=0x39).contains(unit))
                    .map(|unit| first * 10 + usize::from(unit - 0x30));
                let (capture, consumed) = if two.is_some_and(|value| (1..=captures.len()).contains(&value)) {
                    (two.unwrap(), 3)
                } else { (first, 2) };
                if (1..=captures.len()).contains(&capture) {
                    if let Some(value) = &captures[capture - 1] { output.extend_from_slice(value); }
                    index += consumed;
                    continue;
                }
                output.push(0x24);
                index += 1;
                continue;
            }
            0x3C if !names.is_empty() => {
                if let Some(relative_end) = replacement[index + 2..].iter().position(|unit| *unit == 0x3E) {
                    let end = index + 2 + relative_end;
                    let name = String::from_utf16(&replacement[index + 2..end]).ok();
                    if let Some((_, Some(value))) = names.iter().find(|(candidate, _)| name.as_deref() == Some(candidate.as_str())) {
                        output.extend_from_slice(value);
                    }
                    index = end + 1;
                    continue;
                }
                output.push(0x24);
                index += 1;
                continue;
            }
            _ => {
                output.push(0x24);
                index += 1;
                continue;
            }
        }
        index += 2;
    }
    output
}

/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
unsafe fn thaw_regex_replace_impl(value: *const c_char, source: *const c_char, flags: *const c_char, replacement: *const c_char, all: bool) -> *const c_char {
    if value.is_null() || source.is_null() || flags.is_null() || replacement.is_null() { return std::ptr::null(); }
    let text = unsafe { regex_units(value) };
    let source = unsafe { regex_units(source) };
    let replacement = unsafe { regex_units(replacement) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let global = all || flags.contains('g');
    let Some(output) = with_compiled_regex(&source, &flags, |regex| {
        let mut output = Vec::new();
        let mut append = 0;
        let mut search = 0;
        while search <= text.len() {
            let Some(matched) = compiled_match(regex, &text, &flags, search) else { break };
            if flags.contains('y') && matched.start != search { break; }
            output.extend_from_slice(&text[append..matched.start]);
            output.extend_from_slice(&expand_regex_replacement(
                &replacement, &text[matched.start..matched.end], &text[..matched.start], &text[matched.end..],
                &matched.groups[1..], &matched.names,
            ));
            append = matched.end;
            if !global { break; }
            search = if matched.start == matched.end {
                advance_string_index(&text, matched.end, &flags)
            } else { matched.end };
        }
        output.extend_from_slice(&text[append..]);
        output
    }) else { return std::ptr::null(); };
    arena_wtf8(&wtf8_encode_utf16(&output)).map_or(std::ptr::null(), |pointer| pointer.cast())
}

#[no_mangle]
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_replace(value: *const c_char, source: *const c_char, flags: *const c_char, replacement: *const c_char) -> *const c_char {
    unsafe { thaw_regex_replace_impl(value, source, flags, replacement, false) }
}

#[no_mangle]
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_replace_all(value: *const c_char, source: *const c_char, flags: *const c_char, replacement: *const c_char) -> *const c_char {
    unsafe { thaw_regex_replace_impl(value, source, flags, replacement, true) }
}

#[no_mangle]
/// Regex split with captures, measured in UTF-16 code units.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_split(value: *const c_char, source: *const c_char, flags: *const c_char, limit: f64) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() { return std::ptr::null_mut(); }
    let text = unsafe { regex_units(value) };
    let source = unsafe { regex_units(source) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let limit = if limit == -1.0 { usize::MAX }
        else if limit.is_finite() { limit.trunc().rem_euclid(4_294_967_296.0) as usize }
        else { 0 };
    let Some(parts) = with_compiled_regex(&source, &flags, |regex| {
        let mut parts = Vec::new();
        if limit == 0 { return parts; }
        if text.is_empty() {
            if compiled_match(regex, &text, &flags, 0).is_none() { parts.push(Some(Vec::new())); }
            return parts;
        }
        let mut previous = 0;
        let mut search = 0;
        while search < text.len() {
            let Some(matched) = compiled_match(regex, &text, &flags, search) else { break };
            if matched.start != search {
                search = advance_string_index(&text, search, &flags);
                continue;
            }
            if matched.end == previous {
                search = advance_string_index(&text, search, &flags);
                continue;
            }
            parts.push(Some(text[previous..search].to_vec()));
            if parts.len() >= limit { return parts; }
            for capture in matched.groups.into_iter().skip(1) {
                parts.push(capture);
                if parts.len() >= limit { return parts; }
            }
            previous = matched.end;
            search = previous;
        }
        parts.push(Some(text[previous..].to_vec()));
        parts.truncate(limit);
        parts
    }) else { return std::ptr::null_mut(); };
    arena_optional_string_array(parts)
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

#[no_mangle]
/// Returns an Optional(Str) match handle, preserving captures by UTF-16 range.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_exec(source: *const c_char, flags: *const c_char, value: *const c_char, last_index: f64) -> *mut u8 {
    if source.is_null() || flags.is_null() || value.is_null() { return std::ptr::null_mut(); }
    let source = unsafe { regex_units(source) };
    let text = unsafe { regex_units(value) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let start = last_index_to_length(last_index);
    let Some(matched) = with_compiled_regex(&source, &flags, |regex| {
        compiled_match(regex, &text, &flags, start)
            .filter(|matched| !flags.contains('y') || matched.start == start)
    }).flatten() else { return std::ptr::null_mut(); };
    let index = matched.start as f64;
    let groups = groups_json_pointer(matched.names.clone());
    let result = wrap_array_handle(arena_optional_string_array(matched.into_positional()));
    if !result.is_null() {
        store_regex_meta(result, RegexMatchMeta { groups, index, input: wtf8_encode_utf16(&text) });
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
    let input = REGEX_META.with(|stored| {
        stored
            .borrow()
            .get(&(matches as usize))
            .map(|meta| meta.input.clone())
    });
    input
        .and_then(|input| arena_wtf8(&input))
        .map_or(std::ptr::null(), |pointer| pointer.cast())
}

#[no_mangle]
/// Returns a successful stateful exec/test match end in UTF-16 code units.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_exec_advance(value: *const c_char, source: *const c_char, flags: *const c_char, last_index: f64) -> f64 {
    if value.is_null() || source.is_null() || flags.is_null() { return -1.0; }
    let text = unsafe { regex_units(value) };
    let source = unsafe { regex_units(source) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let start = last_index_to_length(last_index);
    with_compiled_regex(&source, &flags, |regex| {
        compiled_match(regex, &text, &flags, start)
            .filter(|matched| !flags.contains('y') || matched.start == start)
            .map_or(-1.0, |matched| matched.end as f64)
    }).unwrap_or(-1.0)
}

#[no_mangle]
/// String.match result, preserving UTF-16 captures and sticky state.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_match(value: *const c_char, source: *const c_char, flags: *const c_char, last_index: f64) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() { return std::ptr::null_mut(); }
    let text = unsafe { regex_units(value) };
    let source = unsafe { regex_units(source) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    let global = flags.contains('g');
    let sticky = flags.contains('y');
    let start = if sticky && !global { last_index_to_length(last_index) } else { 0 };
    let Some((matches, meta)) = with_compiled_regex(&source, &flags, |regex| {
        if global {
            let mut matches = Vec::new();
            let mut search = 0;
            while search <= text.len() {
                let Some(matched) = compiled_match(regex, &text, &flags, search) else { break };
                if sticky && matched.start != search { break; }
                matches.push(Some(text[matched.start..matched.end].to_vec()));
                search = if matched.start == matched.end {
                    advance_string_index(&text, matched.end, &flags)
                } else { matched.end };
            }
            (matches, None)
        } else {
            let matched = compiled_match(regex, &text, &flags, start)
                .filter(|matched| !sticky || matched.start == start);
            match matched {
                Some(matched) => {
                    let meta = (groups_json_pointer(matched.names.clone()), matched.start as f64);
                    (matched.into_positional(), Some(meta))
                }
                None => (Vec::new(), None),
            }
        }
    }) else { return std::ptr::null_mut(); };
    if matches.is_empty() { return std::ptr::null_mut(); }
    let result = wrap_array_handle(arena_optional_string_array(matches));
    if !result.is_null() {
        if let Some((groups, index)) = meta {
            store_regex_meta(result, RegexMatchMeta { groups, index, input: wtf8_encode_utf16(&text) });
        }
    }
    result
}

/// Writes `pointers` into a fresh arena-allocated array of raw pointers
/// (length prefix, then one pointer-sized slot per element), returning
/// null on allocation failure. Unlike `arena_optional_string_array`, the pointers
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
/// String.matchAll from a copied lastIndex; result and capture arrays are stable handles.
/// # Safety
/// Arguments must be live native strings or NUL-terminated C strings.
pub unsafe extern "C" fn thaw_regex_match_all(value: *const c_char, source: *const c_char, flags: *const c_char, last_index: f64) -> *mut u8 {
    if value.is_null() || source.is_null() || flags.is_null() { return std::ptr::null_mut(); }
    let text = unsafe { regex_units(value) };
    let source = unsafe { regex_units(source) };
    let flags = unsafe { CStr::from_ptr(flags) }.to_string_lossy();
    if !flags.contains('g') { return std::ptr::null_mut(); }
    let start = last_index_to_length(last_index);
    let Some(matches) = with_compiled_regex(&source, &flags, |regex| {
        let mut matches = Vec::new();
        let mut search = start;
        while search <= text.len() {
            let Some(matched) = compiled_match(regex, &text, &flags, search) else { break };
            if flags.contains('y') && matched.start != search { break; }
            let next = if matched.start == matched.end {
                advance_string_index(&text, matched.end, &flags)
            } else { matched.end };
            matches.push(matched);
            search = next;
        }
        matches
    }) else { return std::ptr::null_mut(); };
    let mut inner_arrays = Vec::with_capacity(matches.len());
    for matched in matches {
        let index = matched.start as f64;
        let groups = groups_json_pointer(matched.names.clone());
        let inner = wrap_array_handle(arena_optional_string_array(matched.into_positional()));
        if inner.is_null() { return std::ptr::null_mut(); }
        store_regex_meta(inner, RegexMatchMeta {
            groups, index, input: wtf8_encode_utf16(&text),
        });
        inner_arrays.push(inner);
    }
    wrap_array_handle(arena_pointer_array(inner_arrays))
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn regress_pattern_and_search_keep_utf16_surrogate_semantics() {
        let astral = [0xD83D, 0xDE00];
        assert_eq!(pattern_codepoints(&astral, "u"), vec![0x1F600]);
        assert_eq!(pattern_codepoints(&astral, ""), vec![0xD83D, 0xDE00]);
        for flags in ["", "u"] {
            let expected = (0, 2);
            let found = with_compiled_regex(&astral, flags, |regex| {
                compiled_match(regex, &astral, flags, 0).map(|matched| (matched.start, matched.end))
            }).flatten();
            assert_eq!(found, Some(expected));
        }
        let high_pattern = [0xD83D];
        let high_only = with_compiled_regex(&high_pattern, "", |regex| {
            compiled_match(regex, &astral, "", 0).map(|matched| (matched.start, matched.end))
        }).flatten();
        assert_eq!(high_only, Some((0, 1)));
        let high_unicode = with_compiled_regex(&high_pattern, "u", |regex| {
            compiled_match(regex, &astral, "u", 0).map(|matched| (matched.start, matched.end))
        }).flatten();
        assert_eq!(high_unicode, None);
        let source = wtf8_decode_utf16(b".");
        let inside = with_compiled_regex(&source, "", |regex| {
            compiled_match(regex, &astral, "", 1)
                .map(|matched| (matched.start, matched.end, matched.groups[0].clone()))
        }).flatten();
        assert_eq!(inside, Some((1, 2, Some(vec![0xDE00]))));
    }

    #[test]
    fn regress_empty_iteration_advances_one_nonunicode_code_unit() {
        let text = [0xD83D, 0xDE00];
        let source = wtf8_decode_utf16(b"(?:)");
        for (flags, expected) in [("g", vec![0, 1, 2]), ("gu", vec![0, 2])] {
            let positions = with_compiled_regex(&source, flags, |regex| {
                let mut positions = Vec::new();
                let mut search = 0;
                while search <= text.len() {
                    let Some(matched) = compiled_match(regex, &text, flags, search) else { break };
                    positions.push(matched.start);
                    search = advance_string_index(&text, matched.end, flags);
                }
                positions
            }).unwrap();
            assert_eq!(positions, expected);
        }
    }

    #[test]
    fn regress_js_shorthand_classes_and_named_utf16_captures() {
        let arabic_digit = wtf8_decode_utf16("١".as_bytes());
        let latin_letter = wtf8_decode_utf16("é".as_bytes());
        let nbsp = [0x00A0];
        for (pattern, text, expected) in [
            (r"^\d$", arabic_digit.as_slice(), false),
            (r"^[\D]$", arabic_digit.as_slice(), true),
            (r"^\w$", latin_letter.as_slice(), false),
            (r"^[\W]$", latin_letter.as_slice(), true),
            (r"^\s$", nbsp.as_slice(), true),
            (r"^[\S]$", nbsp.as_slice(), false),
        ] {
            let source = wtf8_decode_utf16(pattern.as_bytes());
            let found = with_compiled_regex(&source, "u", |regex| {
                compiled_match(regex, text, "u", 0).is_some()
            }).unwrap();
            assert_eq!(found, expected, "{pattern}");
        }
        let sets = wtf8_decode_utf16(br"^[\d&&\d]+$");
        for (text, expected) in [(wtf8_decode_utf16(b"7"), true), (arabic_digit, false)] {
            let found = with_compiled_regex(&sets, "v", |regex| {
                compiled_match(regex, &text, "v", 0).is_some()
            }).unwrap();
            assert_eq!(found, expected);
        }
        let astral = [0xD83D, 0xDE00];
        let named = wtf8_decode_utf16(b"(?<x>.)");
        let capture = with_compiled_regex(&named, "", |regex| {
            compiled_match(regex, &astral, "", 1).unwrap()
        }).unwrap();
        assert_eq!(capture.names, vec![("x".to_string(), Some(vec![0xDE00]))]);
        assert_eq!(expand_regex_replacement(
            &wtf8_decode_utf16(b"$<x>"), &astral[1..], &astral[..1], &[],
            &capture.groups[1..], &capture.names,
        ), vec![0xDE00]);
    }

    #[test]
    fn nonunicode_ignore_case_keeps_ascii_word_classes_and_canonicalize() {
        let kelvin = wtf8_decode_utf16("K".as_bytes());
        let long_s = wtf8_decode_utf16("ſ".as_bytes());
        for text in [&kelvin, &long_s] {
            for pattern in [r"^\w$", r"^[\w]$", r"^[a\w]$"] {
                let source = wtf8_decode_utf16(pattern.as_bytes());
                let found = with_compiled_regex(&source, "i", |regex| {
                    compiled_match(regex, text, "i", 0).is_some()
                }).unwrap();
                assert!(!found, "{pattern} must not match a non-ASCII word char under /i");
                for flags in ["iu", "iv"] {
                    let found = with_compiled_regex(&source, flags, |regex| {
                        compiled_match(regex, text, flags, 0).is_some()
                    }).unwrap();
                    assert!(found, "{pattern} must match under /{flags}");
                }
            }
            for pattern in [r"^\W$", r"^[^\w]$"] {
                let source = wtf8_decode_utf16(pattern.as_bytes());
                let found = with_compiled_regex(&source, "i", |regex| {
                    compiled_match(regex, text, "i", 0).is_some()
                }).unwrap();
                assert!(found, "{pattern} must match under /i");
            }
        }
        let source = wtf8_decode_utf16(br"^[s]$");
        assert!(with_compiled_regex(&source, "i", |regex| {
            compiled_match(regex, &long_s, "i", 0).is_none()
        }).unwrap());
        let literal = wtf8_decode_utf16(br"^s$");
        let escaped = wtf8_decode_utf16(br"^\u0073$");
        for source in [&literal, &escaped] {
            assert!(with_compiled_regex(source, "i", |regex| {
                compiled_match(regex, &long_s, "i", 0).is_none()
            }).unwrap());
            assert!(with_compiled_regex(source, "iu", |regex| {
                compiled_match(regex, &long_s, "iu", 0).is_some()
            }).unwrap());
        }
        let backref = wtf8_decode_utf16(br"^(s)\1$");
        let pair = wtf8_decode_utf16("sſ".as_bytes());
        assert!(with_compiled_regex(&backref, "i", |regex| {
            compiled_match(regex, &pair, "i", 0).is_none()
        }).unwrap());
        assert!(with_compiled_regex(&backref, "iu", |regex| {
            compiled_match(regex, &pair, "iu", 0).is_some()
        }).unwrap());
        let boundary = wtf8_decode_utf16(br"^\b\u017F$");
        assert!(with_compiled_regex(&boundary, "i", |regex| {
            compiled_match(regex, &long_s, "i", 0).is_none()
        }).unwrap());
        assert!(with_compiled_regex(&boundary, "iu", |regex| {
            compiled_match(regex, &long_s, "iu", 0).is_some()
        }).unwrap());
    }

    #[test]
    fn regress_match_all_and_split_return_partial_utf16_captures() {
        let input = thaw_arena::arena_string("😀".as_bytes());
        let dot = thaw_arena::arena_string(b".");
        let global = thaw_arena::arena_string(b"g");
        let result = unsafe { thaw_regex_match_all(input, dot, global, 1.0) };
        assert!(!result.is_null());
        let outer = unsafe { result.cast::<*const u8>().read() };
        assert_eq!(unsafe { outer.cast::<u64>().read() }, 1);
        let inner = unsafe { outer.add(8).cast::<*const u8>().read() };
        assert_eq!(unsafe { thaw_regex_exec_index(inner) }, 1.0);
        let buffer = unsafe { inner.cast::<*const u8>().read() };
        let capture = unsafe { buffer.add(16).cast::<*const c_char>().read() };
        assert_eq!(unsafe { CStr::from_ptr(capture) }.to_bytes(), wtf8_encode_utf16(&[0xDE00]));
        let split_source = thaw_arena::arena_string(b"(.)");
        let no_flags = thaw_arena::arena_string(b"");
        let split = unsafe { thaw_regex_split(input, split_source, no_flags, -1.0) };
        assert_eq!(unsafe { split.cast::<u64>().read() }, 5);
        let high = unsafe { split.add(32).cast::<*const c_char>().read() };
        let low = unsafe { split.add(64).cast::<*const c_char>().read() };
        assert_eq!(unsafe { CStr::from_ptr(high) }.to_bytes(), wtf8_encode_utf16(&[0xD83D]));
        assert_eq!(unsafe { CStr::from_ptr(low) }.to_bytes(), wtf8_encode_utf16(&[0xDE00]));
        let replacement = thaw_arena::arena_string(b"$1");
        let replaced = unsafe { thaw_regex_replace(input, split_source, global, replacement) };
        assert_eq!(unsafe { CStr::from_ptr(replaced) }.to_bytes(), "😀".as_bytes());
    }

    #[test]
    fn match_metadata_uses_stable_array_handle() {
        let handle = wrap_array_handle(arena_optional_string_array(vec![Some(vec![b'a' as u16])]));
        store_regex_meta(handle, RegexMatchMeta {
            groups: std::ptr::null_mut(),
            index: 3.0,
            input: b"input".to_vec(),
        });
        let replacement = arena_optional_string_array(vec![Some(vec![b'a' as u16]), Some(vec![b'x' as u16])]);
        unsafe { handle.cast::<*mut u8>().write_unaligned(replacement) };
        assert_eq!(unsafe { thaw_regex_exec_index(handle) }, 3.0);
        thaw_arena::thaw_arena_reset();
    }

    #[test]
    fn match_metadata_expires_with_its_arena_handle() {
        let handle = thaw_arena::thaw_arena_alloc(16, 8);
        store_regex_meta(handle, RegexMatchMeta {
            groups: std::ptr::null_mut(),
            index: 3.0,
            input: b"old".to_vec(),
        });
        thaw_arena::thaw_arena_reset();
        REGEX_META.with(|stored| assert!(!stored.borrow().contains_key(&(handle as usize))));
    }
}
