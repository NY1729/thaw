thread_local! {
    // Keyed by the 16-byte array handle, not its replaceable backing buffer.
    static ARRAY_PROVENANCE: RefCell<std::collections::HashMap<usize, *mut u8>> =
        RefCell::new(std::collections::HashMap::new());
}

fn reset_array_provenance(tracing: bool) {
    ARRAY_PROVENANCE.with(|entries| {
        let mut entries = entries.borrow_mut();
        if tracing {
            entries.retain(|handle, _| !thaw_arena::was_reclaimed(*handle));
        } else {
            entries.clear();
        }
    });
}

unsafe fn array_handle_length(handle: *const u8) -> Option<usize> {
    if handle.is_null() { return None; }
    let buffer = unsafe { handle.cast::<*const u8>().read() };
    if buffer.is_null() { return None; }
    usize::try_from(unsafe { buffer.cast::<u64>().read() }).ok()
}

unsafe fn array_handle_index_present(handle: *const u8, index: usize) -> bool {
    let Some(length) = (unsafe { array_handle_length(handle) }) else { return false; };
    if index >= length { return false; }
    let presence = unsafe { handle.add(8).cast::<*const u8>().read() };
    if presence.is_null() { return true; }
    let mask_length = unsafe { presence.cast::<u64>().read() } as usize;
    index >= mask_length || unsafe { presence.add(8 + index).read() == 1 }
}

/// Returns only source-proven records on present indices of a stable array
/// handle. Null denotes missing sidecar, hole, out-of-range, or null record.
/// # Safety
/// `handle` must be null or a live 16-byte Thaw array handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_array_provenance_get(
    handle: *const u8, index: usize,
) -> *const ExceptionProvenance {
    if !unsafe { array_handle_index_present(handle, index) } {
        return std::ptr::null();
    }
    ARRAY_PROVENANCE.with(|entries| {
        let entries = entries.borrow();
        let Some(&sidecar) = entries.get(&(handle as usize)) else { return std::ptr::null(); };
        let length = unsafe { sidecar.cast::<u64>().read() } as usize;
        if index >= length { return std::ptr::null(); }
        unsafe { sidecar.add(8 + index * 8).cast::<*const ExceptionProvenance>().read() }
    })
}

/// Reserves the record sidecar before a caller mutates its visible buffer.
/// A failed reservation leaves the visible array untouched and preserves its
/// old sidecar. A successful reservation may grow an otherwise empty sidecar;
/// that is harmless if the later visible operation fails.
/// # Safety
/// `handle` must be a live stable Thaw array handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_array_provenance_prepare(
    handle: *mut u8, required_capacity: usize,
) -> u8 {
    if handle.is_null() { return 0; }
    thaw_arena::register_reset_hook(reset_array_provenance);
    ARRAY_PROVENANCE.with(|entries| {
        let mut entries = entries.borrow_mut();
        let previous = entries.get(&(handle as usize)).copied().unwrap_or(std::ptr::null_mut());
        let old_capacity = if previous.is_null() { 0 } else {
            unsafe { previous.cast::<u64>().read() as usize }
        };
        if old_capacity >= required_capacity { return 1; }
        let Some(bytes) = required_capacity.checked_mul(8).and_then(|n| n.checked_add(8)) else {
            return 0;
        };
        if previous.is_null() && entries.try_reserve(1).is_err() { return 0; }
        let next = thaw_arena::thaw_arena_alloc(bytes, 8);
        if next.is_null() { return 0; }
        unsafe {
            next.cast::<u64>().write(required_capacity as u64);
            std::ptr::write_bytes(next.add(8), 0, required_capacity * 8);
            if !previous.is_null() {
                std::ptr::copy_nonoverlapping(previous.add(8), next.add(8), old_capacity * 8);
            }
        }
        thaw_arena::replace_reference(handle as usize, previous as usize, next as usize);
        entries.insert(handle as usize, next);
        1
    })
}

/// Commits one record after a matching visible element/presence write. This
/// operation never allocates; callers must prepare enough capacity first.
/// Zero is a contract failure and must enter the caller's synthetic error
/// path, never silently downgrade to a provenance-free value.
/// # Safety
/// `handle` must be a live mutable Thaw array handle and `record` null or a
/// live private arena record.
#[no_mangle]
pub unsafe extern "C" fn thaw_array_provenance_set(
    handle: *mut u8, index: usize, record: *const ExceptionProvenance,
) -> u8 {
    let Some(length) = (unsafe { array_handle_length(handle) }) else { return 0; };
    if index >= length { return 0; }
    if !unsafe { array_handle_index_present(handle, index) } && !record.is_null() { return 0; }
    ARRAY_PROVENANCE.with(|entries| {
        let entries = entries.borrow();
        let sidecar = entries.get(&(handle as usize)).copied().unwrap_or(std::ptr::null_mut());
        if sidecar.is_null() { return u8::from(record.is_null()); }
        let capacity = unsafe { sidecar.cast::<u64>().read() as usize };
        if index >= capacity { return u8::from(record.is_null()); }
        unsafe { sidecar.add(8 + index * 8).cast::<*const ExceptionProvenance>().write(record) };
        1
    })
}

/// Clears records at and after `new_length` after a successful array shrink.
/// This is required even when the current buffer is later regrown within the
/// sidecar's old capacity: stale provenance must not reappear.
/// # Safety
/// `handle` must be null or a live stable Thaw array handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_array_provenance_truncate(
    handle: *mut u8, new_length: usize,
) {
    if handle.is_null() { return; }
    ARRAY_PROVENANCE.with(|entries| {
        let entries = entries.borrow();
        let Some(&sidecar) = entries.get(&(handle as usize)) else { return; };
        let stored = unsafe { sidecar.cast::<u64>().read() as usize };
        if new_length >= stored { return; }
        unsafe { std::ptr::write_bytes(sidecar.add(8 + new_length * 8), 0, (stored - new_length) * 8) };
    });
}
