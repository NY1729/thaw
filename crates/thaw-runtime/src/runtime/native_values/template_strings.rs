thread_local! {
    /// A tagged template's raw (unescaped) strings, keyed by its cooked
    /// array's own buffer address -- the same "metadata keyed by the
    /// result buffer's stable identity" pattern `RegexMatchMeta`
    /// (`regex.rs`) uses for `.index`/`.input`/`.groups`. The cooked array
    /// is what the tag function actually receives and the identity every
    /// `.raw` read recovers from its own array handle.
    static TEMPLATE_RAW_STRINGS: RefCell<std::collections::HashMap<usize, Vec<String>>> =
        RefCell::new(std::collections::HashMap::new());
}

#[no_mangle]
/// Associates a tagged template's cooked-strings array with its raw
/// sibling, called once per tagged-template evaluation right after the
/// cooked array is built and before the tag function runs.
///
/// # Safety
/// `cooked` and `raw` must be non-null array handles returned by Thaw
/// (`Array<Str>`).
pub unsafe extern "C" fn thaw_template_strings_register(cooked: *const u8, raw: *const u8) {
    if cooked.is_null() || raw.is_null() {
        return;
    }
    let cooked_buffer = unsafe { cooked.cast::<*const u8>().read_unaligned() };
    let raw_buffer = unsafe { raw.cast::<*const u8>().read_unaligned() };
    if cooked_buffer.is_null() || raw_buffer.is_null() {
        return;
    }
    let len = unsafe { raw_buffer.cast::<u64>().read_unaligned() } as usize;
    let mut strings = Vec::with_capacity(len);
    for index in 0..len {
        let ptr = unsafe {
            raw_buffer
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        let text = if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
        };
        strings.push(text);
    }
    TEMPLATE_RAW_STRINGS.with(|stored| {
        stored.borrow_mut().insert(cooked_buffer as usize, strings);
    });
}

#[no_mangle]
/// Returns the raw (unescaped) strings for a tagged template's cooked
/// array, matching `TemplateStringsArray.raw`. An unrelated `string[]`
/// (no registered metadata) degrades to an empty array, same convention
/// `.groups`/`.index`/`.input` use for an unrelated RegExp match result
/// (`regex.rs`).
///
/// # Safety
/// `cooked` must be null or an array handle returned by Thaw.
pub unsafe extern "C" fn thaw_template_strings_raw(cooked: *const u8) -> *mut u8 {
    if cooked.is_null() {
        return arena_string_array(Vec::new());
    }
    let cooked_buffer = unsafe { cooked.cast::<*const u8>().read_unaligned() };
    if cooked_buffer.is_null() {
        return arena_string_array(Vec::new());
    }
    let strings = TEMPLATE_RAW_STRINGS
        .with(|stored| stored.borrow().get(&(cooked_buffer as usize)).cloned())
        .unwrap_or_default();
    arena_string_array(strings)
}
