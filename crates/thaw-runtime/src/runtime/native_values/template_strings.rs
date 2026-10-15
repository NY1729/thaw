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
    thaw_arena::register_metadata_reset_hook(reset_template_strings);
    let previous = TEMPLATE_RAW_STRINGS.with(|stored| {
        stored.borrow_mut().insert(cooked as usize, raw.cast_mut())
    }).unwrap_or(std::ptr::null_mut());
    thaw_arena::replace_reference(cooked as usize, previous as usize, raw as usize);
}

thread_local! {
    /// One frozen template object per tagged-template site, rooted for the process.
    static TEMPLATE_SITES: RefCell<std::collections::HashMap<u64, (usize, thaw_arena::ArenaRoot)>> =
        RefCell::new(std::collections::HashMap::new());
}

fn reset_template_sites(tracing: bool) {
    if !tracing {
        TEMPLATE_SITES.with(|sites| sites.borrow_mut().clear());
    }
}

#[no_mangle]
/// Returns the site's template object, creating it from `cooked`/`raw` on the
/// first evaluation: registers `.raw`, freezes both arrays, and keeps the
/// object alive so every later evaluation of the same site is identical.
///
/// # Safety
/// `cooked` and `raw` must be live Thaw `Array<Str>` handles.
pub unsafe extern "C" fn thaw_template_site(site: f64, cooked: *mut u8, raw: *mut u8) -> *mut u8 {
    let site = site as u64;
    if let Some(existing) = TEMPLATE_SITES.with(|sites| sites.borrow().get(&site).map(|entry| entry.0)) {
        return existing as *mut u8;
    }
    unsafe { thaw_template_strings_register(cooked, raw) };
    thaw_object_set_state(cooked, 3);
    thaw_object_set_state(raw, 3);
    thaw_arena::register_metadata_reset_hook(reset_template_sites);
    TEMPLATE_SITES.with(|sites| {
        sites.borrow_mut().insert(site, (cooked as usize, thaw_arena::ArenaRoot::new(cooked as usize)));
    });
    cooked
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
mod template_strings_reset_tests {
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

    #[test]
    fn a_site_returns_one_frozen_template_object_every_time() {
        let text = thaw_arena::arena_string(b"a\0");
        let make = || {
            (
                wrap_array_handle(arena_pointer_array(vec![text.cast()])),
                wrap_array_handle(arena_pointer_array(vec![text.cast()])),
            )
        };
        let (cooked, raw) = make();
        let (other_cooked, other_raw) = make();
        unsafe {
            let first = thaw_template_site(7.0, cooked, raw);
            assert_eq!(first, cooked);
            assert_eq!(thaw_template_site(7.0, other_cooked, other_raw), cooked);
            assert_eq!(thaw_template_site(8.0, other_cooked, other_raw), other_cooked);
            assert_eq!(thaw_template_strings_raw(cooked), raw);
        }
        assert!(thaw_object_state(cooked, 2) && thaw_object_state(raw, 2));
        thaw_arena::thaw_arena_reset();
        TEMPLATE_SITES.with(|sites| assert!(sites.borrow().is_empty()));
    }
}
