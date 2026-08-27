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
fn with_compiled_regex<T>(source: &str, flags: &str, f: impl FnOnce(&regex::Regex) -> T) -> Option<T> {
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
