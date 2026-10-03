thread_local! {
    /// A template's raw sibling is the original stable array handle, keyed
    /// by its cooked handle. Backing buffers may change without changing identity.
    static TEMPLATE_RAW_STRINGS: RefCell<std::collections::HashMap<usize, *mut u8>> =
        RefCell::new(std::collections::HashMap::new());
}

fn reset_template_strings(tracing: bool) {
    TEMPLATE_RAW_STRINGS.with(|stored| {
        let mut stored = stored.borrow_mut();
        if tracing {
            stored.retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer));
        } else {
            stored.clear();
        }
    });
}

#[no_mangle]
/// Associates a tagged template's cooked array handle with its raw sibling.
///
/// # Safety
/// `cooked` and `raw` must be live Thaw `Array<Str>` handles.
pub unsafe extern "C" fn thaw_template_strings_register(cooked: *const u8, raw: *const u8) {
    if cooked.is_null() || raw.is_null() {
        return;
    }
    let cooked_buffer = unsafe { cooked.cast::<*const u8>().read_unaligned() };
    let raw_buffer = unsafe { raw.cast::<*const u8>().read_unaligned() };
    if cooked_buffer.is_null() || raw_buffer.is_null() {
        return;
    }
    thaw_arena::register_reset_hook(reset_template_strings);
    let previous = TEMPLATE_RAW_STRINGS.with(|stored| {
        stored.borrow_mut().insert(cooked as usize, raw.cast_mut())
    }).unwrap_or(std::ptr::null_mut());
    thaw_arena::replace_reference(cooked as usize, previous as usize, raw as usize);
}

#[no_mangle]
/// Returns the registered raw array handle. Unrelated/null arrays retain the
/// existing empty-array fallback. No handle or string is copied for a template.
///
/// # Safety
/// `cooked` must be null or a live Thaw array handle.
pub unsafe extern "C" fn thaw_template_strings_raw(cooked: *const u8) -> *mut u8 {
    TEMPLATE_RAW_STRINGS.with(|stored| stored.borrow().get(&(cooked as usize)).copied())
        .unwrap_or_else(|| wrap_array_handle(arena_pointer_array(Vec::new())))
}

#[cfg(test)]
mod reset_tests {
    use super::*;

    #[test]
    fn raw_reads_reuse_the_registered_handle_and_exact_string_bytes() {
        let bytes = b"raw\0\xed\xa0\x80";
        let text = thaw_arena::arena_string(bytes);
        let cooked = wrap_array_handle(arena_pointer_array(Vec::new()));
        let raw = wrap_array_handle(arena_pointer_array(vec![text.cast()]));
        unsafe {
            thaw_template_strings_register(cooked, raw);
            assert_eq!(thaw_template_strings_raw(cooked), raw);
            assert_eq!(thaw_template_strings_raw(cooked), raw);
            let buffer = raw.cast::<*mut u8>().read();
            let value = buffer.add(8).cast::<*const c_char>().read();
            assert_eq!(CStr::from_ptr(value).to_bytes(), bytes);
        }
        thaw_arena::thaw_arena_reset();
        TEMPLATE_RAW_STRINGS.with(|stored| assert!(!stored.borrow().contains_key(&(cooked as usize))));
    }

    #[test]
    fn cooked_root_retains_raw_until_the_cooked_handle_expires() {
        thaw_arena::thaw_arena_enable_tracing();
        let text = thaw_arena::arena_string(b"raw\0text");
        let cooked = wrap_array_handle(arena_pointer_array(Vec::new()));
        let raw = wrap_array_handle(arena_pointer_array(vec![text.cast()]));
        unsafe { thaw_template_strings_register(cooked, raw); }
        let root = thaw_arena::ArenaRoot::new(cooked as usize);
        thaw_arena::thaw_arena_reset();
        assert!(!thaw_arena::was_reclaimed(raw as usize));
        unsafe {
            assert_eq!(thaw_template_strings_raw(cooked), raw);
            let buffer = raw.cast::<*mut u8>().read();
            let value = buffer.add(8).cast::<*const c_char>().read();
            assert_eq!(CStr::from_ptr(value).to_bytes(), b"raw\0text");
        }
        drop(root);
        thaw_arena::thaw_arena_reset();
        TEMPLATE_RAW_STRINGS.with(|stored| assert!(!stored.borrow().contains_key(&(cooked as usize))));
    }
}
