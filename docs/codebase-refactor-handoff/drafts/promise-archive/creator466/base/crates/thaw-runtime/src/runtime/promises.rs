#[cfg(test)]
thread_local! {
    // Exercise the same reentry point as an owned Host value's final release
    // without installing process-global Host operations in these tests.
    static PROMISE_RELEASE_TEST_HOOK: Cell<Option<fn(u64)>> = const { Cell::new(None) };
}

/// Private, source-proven exception tuple. This is pointer-aligned arena
/// storage; user objects cannot acquire this type by matching its fields.
#[repr(C)]
pub struct ExceptionProvenance {
    pub original: *const u8,
    pub native_text: *const u8,
    pub aggregate_errors: *const u8,
    pub tag: u64,
    pub f64_value: f64,
    pub i64_value: i64,
    pub bool_value: u64,
    pub object: *const u8,
}
const _: [(); 64] = [(); std::mem::size_of::<ExceptionProvenance>()];

/// Allocates one immutable compiler-private tuple. Null means allocation
/// failed, not a provenance-free value. The caller must handle that failure.
#[no_mangle]
pub extern "C" fn thaw_exception_provenance_new(
    original: *const u8,
    native_text: *const u8,
    aggregate_errors: *const u8,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: u64,
    object: *const u8,
) -> *const ExceptionProvenance {
    let allocation = thaw_arena::thaw_arena_alloc(
        std::mem::size_of::<ExceptionProvenance>(),
        std::mem::align_of::<ExceptionProvenance>(),
    ).cast::<ExceptionProvenance>();
    if allocation.is_null() { return std::ptr::null(); }
    unsafe { allocation.write(ExceptionProvenance {
        original, native_text, aggregate_errors, tag, f64_value, i64_value,
        bool_value, object,
    }) };
    allocation
}

/// Allocates an unresolved promise with a creator share and a separate
/// request-boundary share. Consume the creator share exactly once with
/// `thaw_promise_destroy`; request cleanup releases its own share. Queued
/// subscribers, stored fields and JS wrappers retain their own shares.
#[no_mangle]
pub extern "C" fn thaw_promise_new() -> *mut ThawPromise {
    let Some(identity) = NEXT_PROMISE_ID.with(|next| {
        let identity = next.get();
        next.set(identity.checked_add(1)?);
        Some(identity)
    }) else { return std::ptr::null_mut(); };
    let promise = Box::into_raw(Box::new(ThawPromise {
        // One creator share plus one independent request-boundary share.
        // Persistent fields, queued continuations and JS wrappers acquire
        // their own shares before the request share can be released.
        owners: 2,
        creator_live: true,
        base_live: true,
        identity,
        graph_finisher: std::ptr::null(),
        root: None,
        result: None,
        result_kind: 0,
        fulfilled_provenance: std::ptr::null(),
        rejection_text: None,
        rejected: false,
        exception_tag: 0,
        exception_f64: 0.0,
        exception_i64: 0,
        exception_bool: false,
        exception_object: std::ptr::null(),
        rejection_json: None,
        rejection_source: None,
        aggregate_reason_owner: None,
        aggregate_errors: std::ptr::null(),
        handled: false,
        reported_unhandled: false,
        subscribers: Vec::new(),
    }));
    unsafe { (*promise).root = Some(thaw_arena::ArenaRoot::new(promise as usize)) };
    ACTIVE_PROMISES.with(|active| active.borrow_mut().push(promise));
    PROMISE_BASES.with(|bases| bases.borrow_mut().push((identity, thaw_arena::reset_epoch())));
    thaw_arena::register_reset_hook(reset_promise_bases);
    promise
}

fn reset_promise_bases(tracing: bool) {
    // Snapshot only promises created before this reset. Earlier hook cleanup
    // can reenter and create a new Promise at the current epoch; it belongs to
    // the next request even if it enters PROMISE_BASES before this hook runs.
    let epoch = thaw_arena::reset_epoch();
    let identities = PROMISE_BASES.with(|bases| {
        let mut bases = bases.borrow_mut();
        let mut due = Vec::new();
        bases.retain(|entry| {
            if entry.1 < epoch { due.push(*entry); false } else { true }
        });
        due
    });
    if !reconcile_promise_array_handles(tracing) {
        // Reconciliation failed before any base owner was retired. Retry the
        // same identities at the next boundary without dropping their share.
        PROMISE_BASES.with(|bases| bases.borrow_mut().extend(identities));
        return;
    }
    // A module global can point at a Promise created in this request. Keep
    // its independent share before releasing the request-boundary share.
    let global_retired = reconcile_promise_global_slots_staged();
    if !release_promise_shares_until_staged(global_retired) {
        PROMISE_BASES.with(|bases| bases.borrow_mut().extend(identities));
        return;
    }
    let pins = PROMISE_REQUEST_PINS.with(|pins| std::mem::take(&mut *pins.borrow_mut()));
    if !release_promise_shares_until_staged(pins.into_values().collect()) {
        PROMISE_BASES.with(|bases| bases.borrow_mut().extend(identities));
        return;
    }
    let retired = PROMISE_RETIRED_FIELDS.with(|fields| std::mem::take(&mut *fields.borrow_mut()));
    if !release_promise_shares_until_staged(retired) {
        PROMISE_BASES.with(|bases| bases.borrow_mut().extend(identities));
        return;
    }
    let mut due = identities.into_iter();
    while let Some((identity, epoch)) = due.next() {
        if PROMISE_ARRAY_FIELDS_DEFERRED.with(Cell::get) {
            PROMISE_BASES.with(|bases| {
                let mut bases = bases.borrow_mut();
                bases.push((identity, epoch));
                bases.extend(due);
            });
            return;
        }
        let promise = thaw_promise_from_identity(identity);
        if promise.is_null() { continue; }
        let creator_live = unsafe { (*promise).creator_live };
        if creator_live {
            unsafe { (*promise).creator_live = false };
            unsafe { thaw_promise_release(promise) };
        }
        // `creator_live` may have been the last non-base share. Recheck the
        // allocation incarnation before releasing its request share.
        unsafe { thaw_promise_release_request_base(thaw_promise_from_identity(identity)) };
    }
}

fn release_promise_shares_until_staged(shares: Vec<*mut ThawPromise>) -> bool {
    let mut remaining = shares.into_iter();
    while let Some(promise) = remaining.next() {
        if PROMISE_ARRAY_FIELDS_DEFERRED.with(Cell::get) {
            PROMISE_RETIRED_FIELDS.with(|queue| {
                let mut queue = queue.borrow_mut();
                queue.push(promise);
                queue.extend(remaining);
            });
            return false;
        }
        unsafe { thaw_promise_release(promise) };
    }
    true
}

/// Pin a native Promise for the remainder of the current request when HIR
/// code reads it from a field, global, union or callback result. A generated
/// expression may be evaluated repeatedly; one share per identity suffices.
#[no_mangle]
pub extern "C" fn thaw_promise_pin_request(promise: *mut ThawPromise) -> u8 {
    let identity = thaw_promise_identity(promise);
    if identity == 0 { return 0; }
    PROMISE_REQUEST_PINS.with(|pins| {
        let mut pins = pins.borrow_mut();
        if pins.contains_key(&identity) { return 1; }
        let retained = thaw_promise_retain(promise);
        if retained.is_null() { return 0; }
        pins.insert(identity, retained);
        1
    })
}

/// Track a stable native `Array<Promise<T>>` handle. The handle cell points
/// to the current raw buffer; mutators may replace that buffer or reorder its
/// elements before the next request boundary. Registration is idempotent.
#[no_mangle]
pub extern "C" fn thaw_promise_track_array(handle: *mut u8) -> u8 {
    if PROMISE_ARRAY_HANDLES.with(|handles| handles.borrow().iter()
        .any(|entry| entry.handle == handle as usize)) { return 1; }
    unsafe { thaw_promise_track_array_layout(handle, 8, None) }
}

/// Registers the physical element layout on a newly produced native Array.
/// `element` is a compiler-generated, non-reentrant tag decoder; it receives
/// one fully initialized element slot and returns its active Promise or null.
/// An alias may repeat the same registration, but cannot replace the layout.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_track_array_layout(
    handle: *mut u8, stride: usize, element: Option<PromiseArrayElement>,
) -> u8 {
    if handle.is_null() || stride == 0 || !thaw_arena::contains_allocation(handle as usize) {
        return 0;
    }
    let layout = PromiseArrayLayout { handle: handle as usize, stride, element };
    let registration = PROMISE_ARRAY_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        if let Some(existing) = handles.iter().find(|entry| entry.handle == handle as usize) {
            return u8::from(existing.stride == stride &&
                existing.element.map(|callback| callback as usize)
                    == element.map(|callback| callback as usize));
        }
        if handles.try_reserve(1).is_err() { return 0; }
        handles.push(layout);
        2
    });
    if registration != 2 { return registration; }
    if element.is_none() { return 1; }
    // Take output shares at the producer, before an earlier source array's
    // dead field owners can be released at the next reset boundary.
    let buffer = unsafe { handle.cast::<*const u8>().read() };
    if !buffer.is_null() {
        if let Some(old) = unsafe { reconcile_promise_array_layout_staged(layout, buffer) } {
            for promise in old { unsafe { thaw_promise_release(promise) } }
            return 1;
        }
    }
    PROMISE_ARRAY_HANDLES.with(|handles| {
        handles.borrow_mut().retain(|entry| entry.handle != handle as usize);
    });
    0
}

fn reconcile_promise_array_handles(tracing: bool) -> bool {
    let failure_generation = PROMISE_ARRAY_STAGE_FAILURES.with(Cell::get);
    // On a failed live-array stage, dead source fields may be the last owner
    // of a pointer copied into that live array. Reserve retirement storage
    // before modifying either registry so those shares can survive a retry.
    let fields_len = PROMISE_FIELDS.with(|fields| fields.borrow().len());
    #[cfg(test)]
    let force_reserve_failure = PROMISE_ARRAY_FORCE_RETIRE_RESERVE_FAILURE
        .with(|force| force.replace(false));
    #[cfg(not(test))]
    let force_reserve_failure = false;
    if force_reserve_failure || PROMISE_RETIRED_FIELDS.with(|retired| retired.borrow_mut()
        .try_reserve(fields_len).is_err()) {
        defer_promise_array_fields();
        return false;
    }
    let (live, dead) = PROMISE_ARRAY_HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        let dead = handles.iter().copied()
            .filter(|entry| !tracing || thaw_arena::was_reclaimed(entry.handle))
            .collect::<Vec<_>>();
        handles.retain(|entry| !dead.iter().any(|old| old.handle == entry.handle));
        (handles.clone(), dead)
    });
    // Releasing dead field owners can reenter an arena reset through a Host
    // lease. Pin every live array handle before that first release, so the
    // outer pass never dereferences an array reclaimed by the nested reset.
    let live_roots = live.iter().map(|entry| thaw_arena::ArenaRoot::new(entry.handle))
        .collect::<Vec<_>>();
    let retire_dead = || PROMISE_FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        let keys = fields.keys().filter(|(owner, _)| dead.iter().any(|entry| entry.handle == *owner))
            .copied().collect::<Vec<_>>();
        keys.into_iter().filter_map(|key| fields.remove(&key)).collect::<Vec<_>>()
    });
    #[cfg(test)]
    if PROMISE_ARRAY_FORCE_STAGE_FAILURE.with(|force| force.replace(false)) {
        PROMISE_RETIRED_FIELDS.with(|queue| queue.borrow_mut().extend(retire_dead()));
        defer_promise_array_fields();
        return false;
    }
    // Retain every live array's published elements before dropping any old
    // field share. A live array may contain a Promise copied from a now-dead
    // array, including a raw-buffer mutator that did not stage its own share.
    let mut replaced = Vec::new();
    if replaced.try_reserve(live.len()).is_err() {
        PROMISE_RETIRED_FIELDS.with(|queue| queue.borrow_mut().extend(retire_dead()));
        defer_promise_array_fields();
        return false;
    }
    for entry in live {
        let buffer = unsafe { (entry.handle as *const *const u8).read() };
        let Some(old) = (!buffer.is_null()).then(|| unsafe {
            reconcile_promise_array_layout_staged(entry, buffer)
        }).flatten() else {
            PROMISE_RETIRED_FIELDS.with(|queue| queue.borrow_mut().extend(retire_dead()));
            defer_promise_array_fields();
            PROMISE_RETIRED_FIELDS.with(|queue| {
                let mut queue = queue.borrow_mut();
                for group in replaced { queue.extend(group); }
            });
            return false;
        };
        replaced.push(old);
    }
    // Detach every dead handle before the first old-share release. Host
    // cleanup may reset the arena and reuse a former dead handle address.
    let retired = retire_dead();
    PROMISE_RETIRED_FIELDS.with(|queue| {
        let mut queue = queue.borrow_mut();
        for group in replaced { queue.extend(group); }
        queue.extend(retired);
    });
    drop(live_roots);
    // Reentrant Host cleanup can run a nested reset. Its failed stage must
    // remain visible to the later field hook of both reset invocations.
    PROMISE_ARRAY_STAGE_FAILURES.with(|failures| {
        if failures.get() == failure_generation {
            PROMISE_ARRAY_FIELDS_DEFERRED.with(|deferred| deferred.set(false));
        }
    });
    !PROMISE_ARRAY_FIELDS_DEFERRED.with(Cell::get)
}

fn defer_promise_array_fields() {
    PROMISE_ARRAY_STAGE_FAILURES.with(|failures| {
        failures.set(failures.get().checked_add(1).expect("Promise array failure generation overflow"));
    });
    PROMISE_ARRAY_FIELDS_DEFERRED.with(|deferred| deferred.set(true));
}

/// Register a process-lifetime generated module slot whose declared type is
/// directly `Promise<T>`. The slot is read only during arena reset, while the
/// generated module remains loaded.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_register_global_slot(
    slot: *const *mut ThawPromise,
) {
    if slot.is_null() { return; }
    PROMISE_GLOBAL_SLOTS.with(|slots| {
        let mut slots = slots.borrow_mut();
        if slots.iter().any(|(current, _)| *current == slot) { return; }
        slots.push((slot, std::ptr::null_mut()));
    })
}

fn reconcile_promise_global_slots_staged() -> Vec<*mut ThawPromise> {
    // Publish every acquired current share before any old share is dropped.
    // A Host destructor may replace a global and reenter reset, so the table
    // must remain visible to that nested reconciliation throughout cleanup.
    let retired = PROMISE_GLOBAL_SLOTS.with(|registry| {
        let mut slots = registry.borrow_mut();
        let mut retired = Vec::new();
        for (slot, retained) in slots.iter_mut() {
            let current = unsafe { (*slot).read() };
            if current == *retained { continue; }
            let acquired = if current.is_null() { current } else { thaw_promise_retain(current) };
            if !current.is_null() && acquired.is_null() {
                // Preserve the prior share and retry the changed slot later.
                continue;
            }
            let prior = std::mem::replace(retained, acquired);
            if !prior.is_null() { retired.push(prior); }
        }
        retired
    });
    retired
}

#[cfg(test)]
fn reconcile_promise_global_slots() {
    let retired = reconcile_promise_global_slots_staged();
    assert!(release_promise_shares_until_staged(retired));
}

/// Acquires an independent owner share for a live native Promise. The
/// address is checked against this thread's active allocation set before it
/// is dereferenced; a borrowed field read can use this before a consuming
/// await, callback finisher, or combinator receives the pointer.
#[no_mangle]
pub extern "C" fn thaw_promise_retain(promise: *mut ThawPromise) -> *mut ThawPromise {
    let live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&promise));
    if !live { return std::ptr::null_mut(); }
    // Promise runtime operations are confined to one thread. A failed
    // increment must leave the existing owner shares intact.
    let promise_ref = unsafe { &mut *promise };
    let Some(owners) = promise_ref.owners.checked_add(1) else {
        return std::ptr::null_mut();
    };
    promise_ref.owners = owners;
    promise
}

/// Returns a unique allocation-incarnation ticket, or zero for an invalid
/// pointer. A ticket is safe to retain in a JS WeakMap; a raw pointer is not.
#[no_mangle]
pub extern "C" fn thaw_promise_identity(promise: *const ThawPromise) -> u64 {
    let live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&(promise as *mut ThawPromise)));
    if live { unsafe { (*promise).identity } } else { 0 }
}

/// Resolve a ticket only while its native allocation still has an owner.
/// This returns a borrowed pointer; callers that will consume it must call
/// `thaw_promise_retain` before releasing the active runtime context.
#[no_mangle]
pub extern "C" fn thaw_promise_from_identity(identity: u64) -> *mut ThawPromise {
    if identity == 0 { return std::ptr::null_mut(); }
    ACTIVE_PROMISES.with(|active| {
        active.borrow().iter().copied()
            .find(|promise| unsafe { (**promise).identity == identity })
            .unwrap_or(std::ptr::null_mut())
    })
}

/// Attach the compiler-generated finisher for this Promise's resolved type.
/// A different finisher cannot reinterpret one live Promise through another
/// field type. The address stays in native state and never enters graph wire.
#[no_mangle]
pub extern "C" fn thaw_promise_register_graph_finisher(
    promise: *mut ThawPromise, finisher: *const u8,
) -> u8 {
    if finisher.is_null() || thaw_promise_identity(promise) == 0 { return 0; }
    let state = unsafe { &mut *promise };
    if !state.graph_finisher.is_null() && state.graph_finisher != finisher { return 0; }
    state.graph_finisher = finisher;
    1
}

/// Read a finisher only by allocation-incarnation ticket. Callers must hold
/// an independent Promise share across asynchronous polling.
#[no_mangle]
pub extern "C" fn thaw_promise_graph_finisher(identity: u64) -> *const u8 {
    let promise = thaw_promise_from_identity(identity);
    if promise.is_null() { std::ptr::null() }
    else { unsafe { (*promise).graph_finisher } }
}

/// The JSON graph owns Promise tickets by allocation identity rather than a
/// raw address. The ticket lease prevents arena/reset or callback cleanup
/// from reusing the address while the shared JSON object still exists.
#[no_mangle]
pub extern "C" fn thaw_promise_retain_identity(identity: u64) -> u64 {
    let promise = thaw_promise_from_identity(identity);
    if thaw_promise_retain(promise).is_null() { 0 } else { identity }
}

#[no_mangle]
pub extern "C" fn thaw_promise_release_identity(identity: u64) -> u8 {
    let promise = thaw_promise_from_identity(identity);
    if promise.is_null() { return 0; }
    unsafe { thaw_promise_release(promise) };
    1
}

fn reset_promise_fields(tracing: bool) {
    // The array stage may fail even before retirement storage is reserved.
    // Preserve dead-source shares for the next pass; otherwise a copied live
    // array can contain a pointer to the released final Promise owner.
    if PROMISE_ARRAY_FIELDS_DEFERRED.with(Cell::get) { return; }
    // Snapshot and remove all dead entries under one borrow. A release may
    // reenter reset and register a new field at the same address; the outer
    // pass must not remove that new incarnation by a stale key.
    let retired = PROMISE_FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        let dead = fields.keys().filter_map(|&(owner, slot)| {
            (!tracing || thaw_arena::was_reclaimed(owner)).then_some((owner, slot))
        }).collect::<Vec<_>>();
        dead.into_iter().filter_map(|key| fields.remove(&key)).collect::<Vec<_>>()
    });
    for old in retired { unsafe { thaw_promise_release(old) } }
}

/// Replace one physical arena field's independently owned Promise share.
/// The caller must invoke this before storing the new pointer. A zero return
/// means the native field and its previous share remain unchanged.
/// `slot` is the physical byte address for an object field or the numerical
/// index for a stable array handle. Structural aliases must use the original
/// physical object slot; arrays use their handle rather than a movable buffer.
#[no_mangle]
pub extern "C" fn thaw_promise_field_replace(
    owner: *const u8, slot: usize, promise: *mut ThawPromise,
) -> u8 {
    if owner.is_null() { return 0; }
    let key = (owner as usize, slot);
    let retained = if promise.is_null() {
        std::ptr::null_mut()
    } else {
        thaw_promise_retain(promise)
    };
    if !promise.is_null() && retained.is_null() { return 0; }
    let changed = PROMISE_FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        if !fields.contains_key(&key) && fields.try_reserve(1).is_err() {
            return None;
        }
        Some(if retained.is_null() { fields.remove(&key) }
            else { fields.insert(key, retained) })
    });
    let Some(old) = changed else {
        if !retained.is_null() { unsafe { thaw_promise_release(retained) } }
        return 0;
    };
    thaw_arena::register_reset_hook(reset_promise_fields);
    if let Some(old) = old {
        PROMISE_RETIRED_FIELDS.with(|retired| retired.borrow_mut().push(old));
    }
    1
}

/// Stage the complete ownership shape of a native `Array<Promise<T>>` after
/// a copy/move mutation and before publishing the new buffer in its stable
/// handle. Every future slot is retained first; on failure the previous
/// side-table shape and handle contents remain unchanged. A caller must pass
/// a readable pointer-width array buffer generated by this runtime.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_array_reconcile(
    handle: *mut u8, next_buffer: *const u8,
) -> u8 {
    let old = unsafe { reconcile_promise_array_layout_staged(PromiseArrayLayout {
        handle: handle as usize, stride: 8, element: None,
    }, next_buffer) };
    let Some(old) = old else { return 0; };
    for promise in old { unsafe { thaw_promise_release(promise) } }
    1
}

unsafe fn reconcile_promise_array_layout_staged(
    layout: PromiseArrayLayout, next_buffer: *const u8,
) -> Option<Vec<*mut ThawPromise>> {
    let handle = layout.handle as *mut u8;
    if handle.is_null() || next_buffer.is_null() { return None; }
    let length = usize::try_from(unsafe { next_buffer.cast::<u64>().read() })
        .ok();
    let Some(length) = length else { return None; };
    let Some(_) = length.checked_mul(layout.stride) else {
        return None;
    };
    let mut desired = Vec::new();
    if desired.try_reserve(length).is_err() { return None; }
    let published = unsafe { handle.cast::<*const u8>().read() == next_buffer };
    for index in 0..length {
        if published && !unsafe { array_handle_index_present(handle, index) } { continue; }
        let Some(offset) = index.checked_mul(layout.stride).and_then(|n| n.checked_add(8)) else {
            for (_, previous) in desired { unsafe { thaw_promise_release(previous) } }
            return None;
        };
        let slot = unsafe { next_buffer.add(offset) };
        let pointer = match layout.element {
            Some(extract) => unsafe { extract(index, slot) },
            None => unsafe { slot.cast::<*mut ThawPromise>().read_unaligned() },
        };
        if !pointer.is_null() {
            let retained = thaw_promise_retain(pointer);
            if retained.is_null() {
                for (_, previous) in desired {
                    unsafe { thaw_promise_release(previous) }
                }
                return None;
            }
            desired.push((index, retained));
        }
    }
    let owner = handle as usize;
    let updated = PROMISE_FIELDS.with(|fields| {
        let mut fields = fields.borrow_mut();
        if fields.try_reserve(desired.len()).is_err() { return None; }
        let mut old_keys = Vec::new();
        if old_keys.try_reserve(fields.len()).is_err() { return None; }
        old_keys.extend(fields.keys().filter(|(key_owner, _)| *key_owner == owner).copied());
        let mut old = Vec::new();
        // Capacity is already proven available for every removed key.
        if old.try_reserve(old_keys.len()).is_err() { return None; }
        old.extend(old_keys.into_iter().filter_map(|key| fields.remove(&key)));
        for &(index, promise) in &desired {
            fields.insert((owner, index), promise);
        }
        Some(old)
    });
    let Some(old) = updated else {
        for (_, promise) in desired { unsafe { thaw_promise_release(promise) } }
        return None;
    };
    thaw_arena::register_reset_hook(reset_promise_fields);
    Some(old)
}

/// Returns 0 for pending, 1 for fulfilled, 2 for rejected, and 255 for an
/// invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live `ThawPromise` that is not being
/// mutated or destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_state(promise: *const ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_ref() }) else {
        return u8::MAX;
    };
    match (promise.result.is_some(), promise.rejected) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    }
}

/// Marks a promise as observed by a non-coroutine consumer such as the JS
/// bridge, which polls settlement instead of registering a continuation.
///
/// # Safety
///
/// `promise` must be null or point to a live, exclusively accessible
/// `ThawPromise`.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_mark_handled(promise: *mut ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if promise.reported_unhandled && !promise.handled {
        PENDING_REJECTION_HANDLED.with(|pending| pending.set(pending.get() + 1));
    }
    promise.handled = true;
    1
}

/// Registers a coroutine continuation. If already resolved, the callback is
/// queued immediately, but never invoked reentrantly inside this function.
/// Returns 1 on success and 0 for an invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live, exclusively accessible
/// `ThawPromise`. `frame` must remain valid whenever `resume` can be invoked.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_subscribe(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    frame: *mut u8,
) -> u8 {
    unsafe { subscribe_with_cancel(promise, resume, None, frame) }
}

/// Compiler-private subscription for split async frames. The frame stores a
/// native completion pointer that arena tracing cannot keep alive. This
/// subscription owns a separate completion share through resume or purge.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_subscribe_with_completion(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    frame: *mut u8,
    completion: *mut ThawPromise,
) -> u8 {
    if completion.is_null() { return 0; }
    unsafe { subscribe_with_cancel_and_completion(
        promise, resume, None, frame, completion,
    ) }
}

/// Compiler-private split-frame subscription whose cancellation callback
/// runs before the subscription's ArenaRoot or Promise shares are released.
/// The generated callback may therefore retire frame-owned catch carriers
/// without dereferencing a reclaimed frame.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_subscribe_with_cancel_and_completion(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    cancel: extern "C" fn(*mut u8),
    frame: *mut u8,
    completion: *mut ThawPromise,
) -> u8 {
    if completion.is_null() { return 0; }
    unsafe { subscribe_with_cancel_and_completion(
        promise, resume, Some(cancel), frame, completion,
    ) }
}

unsafe fn subscribe_with_cancel(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    cancel: Option<extern "C" fn(*mut u8)>,
    frame: *mut u8,
) -> u8 {
    unsafe { subscribe_with_cancel_and_completion(
        promise, resume, cancel, frame, std::ptr::null_mut(),
    ) }
}

unsafe fn subscribe_with_cancel_and_completion(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    cancel: Option<extern "C" fn(*mut u8)>,
    frame: *mut u8,
    completion: *mut ThawPromise,
) -> u8 {
    if PURGING_ASYNC_STATE.with(Cell::get) { return 0; }
    let completion_share = if completion.is_null() {
        std::ptr::null_mut()
    } else {
        let retained = thaw_promise_retain(completion);
        if retained.is_null() { return 0; }
        retained
    };
    // The queued continuation owns its share until its resume callback
    // releases it. The allocation's initial request share may disappear at
    // arena reset while this continuation is still pending.
    if thaw_promise_retain(promise).is_null() {
        if !completion_share.is_null() {
            unsafe { thaw_promise_release(completion_share) };
        }
        return 0;
    }
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        unsafe { thaw_promise_release(completion_share) };
        return 0;
    };
    unsafe { thaw_promise_mark_handled(promise) };
    if let Some(result) = promise.result {
        enqueue_continuation(PromiseSubscription {
            resume, frame, cancel, delivered: false, source: promise,
            completion: completion_share,
            _frame_root: thaw_arena::ArenaRoot::new(frame as usize),
        }, result);
    } else {
        promise
            .subscribers
            .push(PromiseSubscription {
                resume, frame, cancel, delivered: false, source: promise,
                completion: completion_share,
                _frame_root: thaw_arena::ArenaRoot::new(frame as usize),
            });
    }
    1
}

/// Resolves a promise exactly once and queues every current subscriber in
/// registration order. Returns 0 for a null handle or repeated resolution.
#[no_mangle]
pub extern "C" fn thaw_promise_resolve(promise: *mut ThawPromise, result: *const u8) -> u8 {
    settle_promise(promise, result, false, std::ptr::null())
}

unsafe extern "C" {
    fn thaw_json_share(value: *const u8) -> *mut u8;
    fn thaw_json_destroy(value: *mut u8);
    fn thaw_json_track_arena_owned_root(value: *mut u8) -> *mut u8;
    fn thaw_json_detach_arena_owned_root(value: *mut u8) -> u8;
}

/// The private result representation of a settled Promise. Native producers
/// use 0; JS-origin promises own one canonical Json Box and use 1. Never
/// infer a physical native T from the first contextual view.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_result_kind(promise: *const ThawPromise) -> u8 {
    unsafe { promise.as_ref() }.map_or(u8::MAX, |promise| promise.result_kind)
}

/// Copy an exception object into the pending exception/frame lifetime while
/// its source Promise is still live. A tag-7 pointer is owned by that Promise
/// and would dangle if an await released the last Promise share before its
/// catch block ran. The arena-owned share follows the pending pointer through
/// registered module roots and async frames. Other tags retain their existing
/// borrowed native object/aggregate pointer contract.
///
/// A null result is valid for non-7 tags. For tag 7 it reports a JSON HostError
/// if sharing or rooting failed; the caller must check that case before
/// releasing the source Promise. `thaw_json_track_arena_owned_root` destroys
/// its input share on failure.
///
/// # Safety
/// `promise` must be null or a live Promise on this runtime thread.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_exception_pending_object(
    promise: *const ThawPromise,
) -> *const u8 {
    let Some(promise) = (unsafe { promise.as_ref() }) else { return std::ptr::null(); };
    if promise.exception_tag != 7 { return promise.exception_object; }
    let owned = unsafe { thaw_json_share(promise.exception_object) };
    if owned.is_null() { return std::ptr::null(); }
    unsafe { thaw_json_track_arena_owned_root(owned) }.cast_const()
}

/// Resolve from a borrowed, trusted Json value. The Promise owns a distinct
/// clone so a later release of the decoder's temporary cannot invalidate a
/// pending subscriber or another contextual Promise<T> view.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_resolve_js_origin(
    promise: *mut ThawPromise,
    value: *const u8,
) -> u8 {
    if promise.is_null() || value.is_null() { return 0; }
    let owned = unsafe { thaw_json_share(value) };
    if owned.is_null() { return 0; }
    if settle_promise(promise, owned.cast_const(), false, std::ptr::null()) == 0 {
        unsafe { thaw_json_destroy(owned) };
        return 0;
    }
    unsafe { (*promise).result_kind = 1 };
    1
}

/// Compiler-private fulfillment with a trusted provenance record. The record
/// is attached only if this call wins settlement. Repeated calls return zero.
///
/// # Safety
/// `promise` must be null or a live Promise, and `provenance` null or a live
/// `ExceptionProvenance` in the invocation arena.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_resolve_with_provenance(
    promise: *mut ThawPromise,
    result: *const u8,
    provenance: *const ExceptionProvenance,
) -> u8 {
    settle_promise(promise, result, false, provenance)
}

/// Returns the record attached to an already fulfilled promise, or null.
/// Rejected, pending, and invalid promises never expose a fulfilled record.
///
/// # Safety
/// `promise` must be null or a live Promise.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_fulfilled_provenance(
    promise: *const ThawPromise,
) -> *const ExceptionProvenance {
    let Some(promise) = (unsafe { promise.as_ref() }) else { return std::ptr::null(); };
    if promise.result.is_none() || promise.rejected { return std::ptr::null(); }
    promise.fulfilled_provenance
}

fn forward_promise_fulfillment(
    output: *mut ThawPromise,
    input: *const ThawPromise,
    result: *const u8,
) -> u8 {
    if unsafe { input.as_ref() }.is_some_and(|input| input.result_kind == 1) {
        return unsafe { thaw_promise_resolve_js_origin(output, result) };
    }
    let record = unsafe { thaw_promise_fulfilled_provenance(input) };
    unsafe { thaw_promise_resolve_with_provenance(output, result, record) }
}

/// Rejects a promise exactly once and queues all subscribers. The error is an
/// opaque producer-owned pointer, using the same lifetime contract as a
/// fulfilled result. Returns 0 for a null handle or repeated settlement.
#[no_mangle]
pub extern "C" fn thaw_promise_reject(promise: *mut ThawPromise, error: *const u8) -> u8 {
    settle_promise(promise, error, true, std::ptr::null())
}

/// Native runtime error producers call this only for known native/C strings.
/// The public `thaw_promise_reject` remains an opaque pointer ABI.
/// # Safety
/// `error` must be null or a live native string. Registered NativeStr
/// allocations retain their full byte length, including embedded NULs.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_native_text(promise: *mut ThawPromise, error: *const u8) -> u8 {
    reject_native_text(promise, error)
}

fn reject_native_text(promise: *mut ThawPromise, error: *const u8) -> u8 {
    let text = (!error.is_null())
        .then(|| unsafe { thaw_arena::NativeStr::from_ptr(error.cast()).to_bytes().to_vec() });
    if settle_promise(promise, error, true, std::ptr::null()) == 0 { return 0; }
    unsafe { (*promise).rejection_text = text; }
    1
}

#[no_mangle]
/// Rejects a live promise and attaches typed exception metadata.
///
/// # Safety
/// `promise` must be null or point to a live `ThawPromise` allocated by this runtime.
pub unsafe extern "C" fn thaw_promise_reject_typed(
    promise: *mut ThawPromise,
    error: *const u8,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: bool,
    object: *const u8,
) -> u8 {
    if tag == 7 {
        // The tagged object is a borrowed Json Box. A Promise must acquire
        // its own Value share before the producer clears its pending tuple.
        return unsafe { thaw_promise_reject_js_origin(promise, object) };
    }
    if settle_promise(promise, error, true, std::ptr::null()) == 0 {
        return 0;
    }
    let promise = unsafe { &mut *promise };
    promise.exception_tag = tag;
    promise.exception_f64 = f64_value;
    promise.exception_i64 = i64_value;
    promise.exception_bool = bool_value;
    thaw_arena::replace_reference(
        promise as *mut ThawPromise as usize,
        promise.exception_object as usize,
        object as usize,
    );
    promise.exception_object = object;
    1
}

/// Reject with an independently owned graph-decoded JS reason. `reason` is
/// borrowed; this Promise takes its own shallow Value share only if it wins
/// settlement. The exception tuple's tag 7 identifies that owned Json Box.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_js_origin(
    promise: *mut ThawPromise,
    reason: *const u8,
) -> u8 {
    if promise.is_null() || reason.is_null() { return 0; }
    let owned = unsafe { thaw_json_share(reason) };
    if owned.is_null() { return 0; }
    if settle_promise(promise, owned.cast_const(), true, std::ptr::null()) == 0 {
        unsafe { thaw_json_destroy(owned) };
        return 0;
    }
    let promise = unsafe { &mut *promise };
    promise.exception_tag = 7;
    promise.exception_object = owned.cast_const();
    promise.rejection_json = Some(owned);
    1
}

/// Settle using the producer's original, explicitly arena-owned Json Box.
/// This private lane avoids an additional Host lease clone when the pending
/// tuple carries a unique owner token. Status zero leaves that Box with the
/// producer. Status one transfers it to the Promise, which destroys it on
/// final retirement. Borrowed Json values must use the sharing ABI above.
///
/// # Safety
///
/// `reason` must be the producer's unique, arena-tracked Box and `promise`
/// must be a live Promise retained by the caller throughout this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_js_origin_take(
    promise: *mut ThawPromise,
    reason: *mut u8,
) -> u8 {
    unsafe { thaw_promise_reject_js_origin_take_with_aggregate(
        promise, std::ptr::null(), reason, std::ptr::null(), std::ptr::null(),
    ) }
}

/// Compiler-only companion for the pending tuple's exact aggregate and
/// trusted native-text channels. The caller detaches its pending owner token
/// before this call and restores it only when the returned status is zero.
///
/// # Safety
///
/// The `reason` Box satisfies `thaw_promise_reject_js_origin_take`; `error`
/// and `native_text` are null or live NativeStr pointers, and `aggregate`
/// is null or a live invocation-arena array handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_js_origin_take_with_aggregate(
    promise: *mut ThawPromise,
    error: *const u8,
    reason: *mut u8,
    native_text: *const u8,
    aggregate: *const u8,
) -> u8 {
    if reason.is_null() || promise.is_null() { return 0; }
    // During invocation purge, enqueue_continuation drops subscribers and
    // runs their cancellation hooks synchronously. The caller has detached
    // its pending owner token but has not yet classified reentrant globals.
    // Leave the Box with the producer on that path.
    if PURGING_ASYNC_STATE.with(Cell::get) { return 0; }
    let live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&promise));
    if !live || unsafe { (*promise).result.is_some() } { return 0; }
    let text = (!error.is_null() && error == native_text).then(|| unsafe {
        thaw_arena::NativeStr::from_ptr(error.cast()).to_bytes().to_vec()
    });
    if unsafe { thaw_json_detach_arena_owned_root(reason) } == 0 { return 0; }
    // Publish all typed channels before making any subscriber ready. The
    // ordinary settle helper queues immediately and would let a cancellation
    // observer inspect half-installed metadata during nested runtime work.
    let subscribers = {
        let promise = unsafe { &mut *promise };
        promise.result = Some(reason.cast_const());
        promise.rejected = true;
        promise.exception_tag = 7;
        promise.exception_object = reason.cast_const();
        promise.rejection_json = Some(reason);
        promise.rejection_text = text;
        std::mem::take(&mut promise.subscribers)
    };
    thaw_arena::replace_reference(promise as usize, 0, reason as usize);
    set_promise_aggregate_errors(promise, aggregate);
    for subscriber in subscribers { enqueue_continuation(subscriber, reason.cast_const()); }
    1
}

/// Return or install one source-owned canonical JS Error for producer-proven
/// native tagged text or a legacy nested AggregateError. The caller must hold
/// an independent Promise share throughout this call. A null candidate reads
/// only; a nonnull candidate is an arena-owned borrowed Json Host from a
/// captured-intrinsic constructor, shared independently by this Promise.
/// The original native result/text remains for existing reporting paths.
///
/// # Safety
/// `promise` must be retained across this call; `candidate` must be null or a
/// live Json Box created by the compiler's trusted Error projector.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_source_canonical_error(
    promise: *mut ThawPromise, candidate: *const u8,
) -> *const u8 {
    if thaw_promise_identity(promise) == 0 { return std::ptr::null(); }
    let current = unsafe { &*promise };
    if current.result.is_none() || !current.rejected { return std::ptr::null(); }
    if current.exception_tag == 7 {
        // An exact borrowed-source rejection owns the source Promise rather
        // than another Json Box. Its stable tag-7 object is still canonical.
        return current.exception_object;
    }
    if candidate.is_null() || current.exception_tag != 0
        || current.exception_object != std::ptr::null()
        || current.rejection_text.is_none()
    { return std::ptr::null(); }
    let owned = unsafe { thaw_json_share(candidate) };
    if owned.is_null() { return std::ptr::null(); }
    // Sharing a Host may reenter the runtime. The state-held source share
    // keeps the pointer live, but another projector may have installed first.
    let current = unsafe { &mut *promise };
    if current.exception_tag == 7 {
        let installed = current.exception_object;
        unsafe { thaw_json_destroy(owned) };
        return installed;
    }
    if current.exception_tag != 0 || !current.exception_object.is_null()
        || current.rejection_text.is_none()
    {
        unsafe { thaw_json_destroy(owned) };
        return std::ptr::null();
    }
    thaw_arena::replace_reference(promise as usize, 0, owned as usize);
    current.exception_tag = 7;
    current.exception_object = owned.cast_const();
    current.rejection_json = Some(owned);
    owned.cast_const()
}

/// Promise.any has only a callback-scoped raw source field, so validate its
/// serial-checked reason record before using the shared source cache. Direct
/// await/catch callers instead retain the source Promise before invoking it.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any_source_canonical_error(
    promise: *mut ThawPromise, candidate: *const u8,
) -> *const u8 {
    if !active_promise_any_record_source(promise) { return std::ptr::null(); }
    unsafe { thaw_promise_source_canonical_error(promise, candidate) }
}

/// Compiler-only typed rejection. `native_text` is the exact static/native
/// pointer proven by the producer; no arbitrary rejection pointer is read.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_typed_with_native_text(
    promise: *mut ThawPromise,
    error: *const u8,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: bool,
    object: *const u8,
    native_text: *const u8,
) -> u8 {
    let settled = unsafe { thaw_promise_reject_typed(
        promise, error, tag, f64_value, i64_value, bool_value, object,
    ) };
    if settled != 0 && !error.is_null() && error == native_text {
        unsafe { (*promise).rejection_text = Some(thaw_arena::NativeStr::from_ptr(error.cast()).to_bytes().to_vec()); }
    }
    settled
}

/// Compiler-only typed rejection with the Promise.any errors companion.
/// The companion is attached only when this call actually settles `promise`;
/// a repeated rejection cannot overwrite an earlier reason array.
///
/// # Safety
///
/// `promise` must be null or a live Promise. `native_text` must have the same
/// provenance contract as `thaw_promise_reject_typed_with_native_text`, and
/// `aggregate_errors` must be null or a live invocation-arena array handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_typed_with_aggregate(
    promise: *mut ThawPromise,
    error: *const u8,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: bool,
    object: *const u8,
    native_text: *const u8,
    aggregate_errors: *const u8,
) -> u8 {
    let settled = unsafe { thaw_promise_reject_typed_with_native_text(
        promise, error, tag, f64_value, i64_value, bool_value, object, native_text,
    ) };
    if settled != 0 {
        set_promise_aggregate_errors(promise, aggregate_errors);
    }
    settled
}

/// Tracks the Promise's aggregate handle beside its ordinary result edge.
/// `replace_reference` takes the previous child pointer, not a slot number;
/// retaining both children under the same boxed Promise owner lets a rooted
/// Promise keep the errors array alive across invocation-arena resets.
fn set_promise_aggregate_errors(promise: *mut ThawPromise, errors: *const u8) {
    if promise.is_null() { return; }
    // Catch/async propagation carries only the arena buffer pointer. Recover
    // its exact Box owner; a reused address alone is not authority.
    let current_serial = thaw_arena::allocation_serial(errors as usize);
    let current_epoch = thaw_arena::reset_epoch();
    let next_owner = AGGREGATE_REASON_OWNERS.with(|owners| {
        owners.borrow().get(&(errors as usize))
            .and_then(|(epoch, serial, owner)| {
                let same_allocation = if thaw_arena::is_tracing() {
                    serial.is_some() && *serial == current_serial
                } else { *epoch == current_epoch };
                same_allocation.then(|| owner.clone())
            })
    });
    let previous_owner = {
        let promise_ref = unsafe { &mut *promise };
        let previous = promise_ref.aggregate_errors;
        thaw_arena::replace_reference(promise as usize, previous as usize, errors as usize);
        promise_ref.aggregate_errors = errors;
        std::mem::replace(&mut promise_ref.aggregate_reason_owner, next_owner)
    };
    // Owner destruction can release Host leases and reenter this Promise.
    drop(previous_owner);
}

macro_rules! promise_exception_getter {
    ($name:ident, $field:ident, $ty:ty, $default:expr) => {
        #[no_mangle]
        #[doc = "Returns typed exception metadata from a live promise."]
        #[doc = ""]
        #[doc = "# Safety"]
        #[doc = "`promise` must be null or point to a live `ThawPromise` allocated by this runtime."]
        pub unsafe extern "C" fn $name(promise: *const ThawPromise) -> $ty {
            unsafe { promise.as_ref() }.map_or($default, |promise| promise.$field)
        }
    };
}

promise_exception_getter!(thaw_promise_exception_tag, exception_tag, u64, 0);
promise_exception_getter!(thaw_promise_exception_f64, exception_f64, f64, 0.0);
promise_exception_getter!(thaw_promise_exception_i64, exception_i64, i64, 0);
promise_exception_getter!(thaw_promise_exception_bool, exception_bool, bool, false);
promise_exception_getter!(
    thaw_promise_exception_object,
    exception_object,
    *const u8,
    std::ptr::null()
);
promise_exception_getter!(
    thaw_promise_exception_aggregate_errors,
    aggregate_errors,
    *const u8,
    std::ptr::null()
);

/// Copies trusted rejection text into the same invocation arena as an async
/// frame. Null means the Promise carries only an opaque producer value.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_exception_native_text_copy(
    promise: *const ThawPromise,
) -> *const u8 {
    unsafe { promise.as_ref() }
        .and_then(|promise| promise.rejection_text.as_ref())
        .map_or(std::ptr::null(), |text| thaw_arena::arena_string(text).cast())
}


fn forward_promise_rejection(
    output: *mut ThawPromise,
    input: *const ThawPromise,
    error: *const u8,
) -> u8 {
    let Some(input) = (unsafe { input.as_ref() }) else {
        return thaw_promise_reject(output, error);
    };
    let aggregate_reason_owner = input.aggregate_reason_owner.clone();
    let text = input.rejection_text.clone();
    let settled = if input.exception_tag == 7 {
        unsafe { thaw_promise_reject_js_origin(output, input.exception_object) }
    } else {
        unsafe { thaw_promise_reject_typed(
            output,
            error,
            input.exception_tag,
            input.exception_f64,
            input.exception_i64,
            input.exception_bool,
            input.exception_object,
        ) }
    };
    if settled != 0 {
        unsafe {
            (*output).rejection_text = text;
            (*output).aggregate_reason_owner = aggregate_reason_owner;
            set_promise_aggregate_errors(output, input.aggregate_errors);
        }
    }
    settled
}

/// For compiler-generated forwarding from a live source Promise. Preserve
/// both typed exception fields and the explicitly owned native-text snapshot.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_forward_rejection(
    output: *mut ThawPromise,
    source: *const ThawPromise,
    error: *const u8,
) -> u8 {
    if output.cast_const() == source { return 0; }
    forward_promise_rejection(output, source, error)
}

/// Compiler-private fallback when cloning a tag-7 rejection Box failed.
/// The output takes an independent Promise share instead of a new Json Box;
/// the source's exact reason remains live until output retirement. This is
/// only valid for a settled, rejected tag-7 source, such as the waiting
/// Promise still retained by a split-async resume subscription.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_borrowed_source(
    output: *mut ThawPromise,
    source: *mut ThawPromise,
) -> u8 {
    if output.is_null() || source.is_null() || output == source { return 0; }
    let source_live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&source));
    let output_live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&output));
    if !source_live || !output_live { return 0; }
    let retained = thaw_promise_retain(source);
    if retained.is_null() { return 0; }
    let source_ref = unsafe { &*source };
    if source_ref.result.is_none() || !source_ref.rejected || source_ref.exception_tag != 7
        || source_ref.exception_object.is_null()
        || (source_ref.rejection_json.is_none() && source_ref.rejection_source.is_none())
    {
        unsafe { thaw_promise_release(retained) };
        return 0;
    }
    if unsafe { (*output).result.is_some() } {
        unsafe { thaw_promise_release(retained) };
        return 0;
    }
    // Snapshot metadata before publication; no callback runs while taking
    // the source share. The source is settled, so its tag-7 reason is stable.
    let reason = source_ref.exception_object;
    let text = source_ref.rejection_text.clone();
    let aggregate = source_ref.aggregate_errors;
    let aggregate_owner = source_ref.aggregate_reason_owner.clone();
    let subscribers = {
        let output_ref = unsafe { &mut *output };
        output_ref.result = Some(reason);
        output_ref.rejected = true;
        output_ref.exception_tag = 7;
        output_ref.exception_object = reason;
        output_ref.rejection_text = text;
        output_ref.rejection_source = Some(retained);
        output_ref.aggregate_errors = aggregate;
        output_ref.aggregate_reason_owner = aggregate_owner;
        std::mem::take(&mut output_ref.subscribers)
    };
    thaw_arena::replace_reference(output as usize, 0, reason as usize);
    for subscriber in subscribers { enqueue_continuation(subscriber, reason); }
    1
}

struct DetachedPromise {
    promise: *mut ThawPromise,
    pending_exception: *mut *const u8,
    owned_report: bool,
}

fn promise_report_bytes(source: &ThawPromise) -> Vec<u8> {
    source.rejection_text.clone().unwrap_or_else(|| match source.exception_tag {
        1 => javascript_number_string(source.exception_f64).into_bytes(),
        2 => source.exception_i64.to_string().into_bytes(),
        3 => source.exception_bool.to_string().into_bytes(),
        5 => b"undefined".to_vec(),
        6 => b"null".to_vec(),
        4 => b"Unhandled opaque string Promise rejection".to_vec(),
        _ => b"Unhandled opaque Promise rejection".to_vec(),
    })
}

extern "C" fn destroy_detached_promise(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<DetachedPromise>()) };
    if unsafe { thaw_promise_state(state.promise) } == 2 && !state.pending_exception.is_null() {
        let pending = unsafe { &mut *state.pending_exception };
        if pending.is_null() {
            *pending = if state.owned_report {
                // Snapshot producer-owned text and typed fields while the source
                // Promise is live. An arbitrary rejection result is opaque.
                let source = unsafe { &*state.promise };
                thaw_arena::owned_string(promise_report_bytes(source)).cast()
            } else {
                result
            };
        }
    }
}

/// Observes a fire-and-forget Promise after it settles. The subscription
/// retains the source through the callback; the caller keeps its own share.
///
/// # Safety
/// `promise` must be null or a live, exclusively accessible `ThawPromise`.
/// `pending_exception` must remain writable until the promise settles.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_detach(
    promise: *mut ThawPromise,
    pending_exception: *mut *const u8,
) -> u8 {
    unsafe { detach_promise(promise, pending_exception, false) }
}

/// Compiler-private variant for the terminal rejection slot. The slot owns
/// its diagnostic string and must destroy it after reporting.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_detach_for_report(
    promise: *mut ThawPromise,
    pending_rejection: *mut *const u8,
) -> u8 {
    unsafe { detach_promise(promise, pending_rejection, true) }
}

unsafe fn detach_promise(
    promise: *mut ThawPromise,
    pending_exception: *mut *const u8,
    owned_report: bool,
) -> u8 {
    if promise.is_null() {
        return 0;
    }
    let state = Box::into_raw(Box::new(DetachedPromise {
        promise,
        pending_exception,
        owned_report,
    }));
    let subscribed = unsafe { subscribe_with_cancel(
        promise, destroy_detached_promise, Some(cancel_detached_promise), state.cast::<u8>(),
    ) };
    if subscribed == 0 {
        unsafe { drop(Box::from_raw(state)) };
    }
    subscribed
}

extern "C" fn cancel_detached_promise(frame: *mut u8) {
    unsafe { drop(Box::from_raw(frame.cast::<DetachedPromise>())) };
}

// External join-state Boxes are not Promise owners. Keep their output alive
// independently of the request's creator/base shares until the last queued
// child callback retires the state. The subscription owns the input share.
struct PromiseOutputShare(*mut ThawPromise);

impl PromiseOutputShare {
    fn acquire(output: *mut ThawPromise) -> Option<Self> {
        if thaw_promise_retain(output).is_null() { None } else { Some(Self(output)) }
    }
}

impl Drop for PromiseOutputShare {
    fn drop(&mut self) {
        unsafe { thaw_promise_release(self.0) };
    }
}

struct PromiseChainState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    input: *mut ThawPromise,
    fulfilled: Option<(PromiseTransformFn, *mut u8)>,
    rejected: Option<(PromiseTransformFn, *mut u8)>,
}

impl Drop for PromiseChainState {
    fn drop(&mut self) {
        // A subscriber pins this external Box, but the arena scanner cannot
        // inspect Box fields. Its callback contexts are explicit child edges.
        thaw_arena::forget_references(self as *const Self as usize);
    }
}

extern "C" fn resume_promise_chain(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseChainState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    if rejected {
        if let Some((callback, context)) = state.rejected {
            callback(context, state.output, state.input, result);
        } else {
            forward_promise_rejection(state.output, state.input, result);
        }
    } else if let Some((callback, context)) = state.fulfilled {
        callback(context, state.output, state.input, result);
    } else {
        forward_promise_fulfillment(state.output, state.input, result);
    }
}

extern "C" fn cancel_promise_chain(frame: *mut u8) {
    unsafe { drop(Box::from_raw(frame.cast::<PromiseChainState>())) };
}

/// Creates the Promise returned by `.then` or `.catch`. The input handle is
/// borrowed; the subscription owns its share. The callback is invoked only for
/// the selected settlement kind;
/// the other kind is forwarded without changing its payload.
///
/// # Safety
///
/// `input` must point to a live `ThawPromise`. `context` must remain valid
/// until `callback` runs.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_chain(
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: u8,
) -> *mut ThawPromise {
    let selected = Some((callback, context));
    let (fulfilled, rejected) = if on_rejected != 0 {
        (None, selected)
    } else {
        (selected, None)
    };
    unsafe { promise_chain_with_handlers(input, fulfilled, rejected) }
}

/// Registers both handlers against one original input and returns one output.
/// The optional fulfillment callback forwards unchanged fulfillment when
/// absent. A failure of the selected callback only settles the output; it
/// cannot invoke the other callback.
///
/// # Safety
/// `input` is borrowed. Both contexts remain valid through settlement.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_chain_both(
    input: *mut ThawPromise,
    on_fulfilled: Option<PromiseTransformFn>,
    fulfilled_context: *mut u8,
    on_rejected: PromiseTransformFn,
    rejected_context: *mut u8,
) -> *mut ThawPromise {
    unsafe { promise_chain_with_handlers(
        input,
        on_fulfilled.map(|callback| (callback, fulfilled_context)),
        Some((on_rejected, rejected_context)),
    ) }
}

unsafe fn promise_chain_with_handlers(
    input: *mut ThawPromise,
    fulfilled: Option<(PromiseTransformFn, *mut u8)>,
    rejected: Option<(PromiseTransformFn, *mut u8)>,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return std::ptr::null_mut(); }
    if input.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseChainState {
        output,
        _output_share: output_share,
        input,
        fulfilled,
        rejected,
    }));
    for (_, context) in fulfilled.into_iter().chain(rejected) {
        if !context.is_null() {
            thaw_arena::replace_reference(state as usize, 0, context as usize);
        }
    }
    if unsafe { subscribe_with_cancel(input, resume_promise_chain, Some(cancel_promise_chain), state.cast()) } == 0 {
        unsafe { drop(Box::from_raw(state)) };
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    output
}

struct PromiseAdoptState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    input: *mut ThawPromise,
}

extern "C" fn resume_promise_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        forward_promise_rejection(state.output, state.input, result);
    } else {
        forward_promise_fulfillment(state.output, state.input, result);
    }
}

extern "C" fn cancel_promise_adopt(frame: *mut u8) {
    unsafe { drop(Box::from_raw(frame.cast::<PromiseAdoptState>())) };
}

/// Makes `output` follow `input`, implementing Promise callback flattening.
/// The input handle is borrowed; the subscription owns its share.
///
/// # Safety
///
/// Both arguments must point to distinct live promises.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
) -> u8 {
    if output.is_null() || input.is_null() {
        return 0;
    }
    if output == input {
        reject_native_text(output, PROMISE_CYCLE_ERROR.as_ptr());
        return 0;
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else { return 0; };
    let state = Box::into_raw(Box::new(PromiseAdoptState { output, _output_share: output_share, input }));
    if unsafe { subscribe_with_cancel(input, resume_promise_adopt, Some(cancel_promise_adopt), state.cast()) } == 0 {
        unsafe { drop(Box::from_raw(state)) };
        return 0;
    }
    1
}

struct PromiseFinallyState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
}

impl Drop for PromiseFinallyState {
    fn drop(&mut self) {
        // The subscription pins this external Box, but the arena tracer
        // cannot inspect its callback context field.
        thaw_arena::forget_references(self as *const Self as usize);
    }
}

extern "C" fn resume_promise_finally(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    (state.callback)(
        state.context,
        state.output,
        state.input,
        result,
        u8::from(rejected),
    );
}

extern "C" fn cancel_promise_finally(frame: *mut u8) {
    unsafe { drop(Box::from_raw(frame.cast::<PromiseFinallyState>())) };
}

/// Runs a `.finally` callback for either settlement kind. The callback owns
/// forwarding or replacing the original settlement. The input is borrowed.
///
/// # Safety
///
/// `input` must point to a live Promise and `context` must outlive callback
/// dispatch.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally(
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return output; }
    if input.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseFinallyState {
        output,
        _output_share: output_share,
        input,
        callback,
        context,
    }));
    if !context.is_null() {
        thaw_arena::replace_reference(state as usize, 0, context as usize);
    }
    if unsafe { subscribe_with_cancel(input, resume_promise_finally, Some(cancel_promise_finally), state.cast()) } == 0 {
        unsafe { drop(Box::from_raw(state)) };
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    output
}

// These Box-held callbacks are not arena allocations. Purge releases only
// their temporary arena pins and marks them inert; a late Promise callback
// may still own the Box frame and must not read request-arena payloads.
thread_local! {
    static PROMISE_FINALLY_ADOPT_STATES: RefCell<Vec<*mut PromiseFinallyAdoptState>> =
        const { RefCell::new(Vec::new()) };
    static PROMISE_ANY_STATES: RefCell<Vec<*mut PromiseAnyState>> =
        const { RefCell::new(Vec::new()) };
}

fn pin_promise_pointer_once(roots: &mut Vec<(usize, thaw_arena::ArenaRoot)>, pointer: usize) {
    if pointer != 0 && !roots.iter().any(|(current, _)| *current == pointer) {
        roots.push((pointer, thaw_arena::ArenaRoot::new(pointer)));
    }
}

fn cancel_pending_aggregate_states() {
    let finally_states = PROMISE_FINALLY_ADOPT_STATES.with(|states| {
        std::mem::take(&mut *states.borrow_mut())
    });
    let mut retired_originals = Vec::new();
    let mut retired_aggregate_owners = Vec::new();
    let mut retired_reason_sources = Vec::new();
    let mut retired_output_shares = Vec::new();
    for pointer in finally_states {
        let state = unsafe { &mut *pointer };
        state.cancelled = true;
        if let Some(share) = state._output_share.take() {
            retired_output_shares.push(share);
        }
        state.original_roots.clear();
        if let Some(value) = state.original_js_clone.take() {
            retired_originals.push(value);
        }
        if let Some(value) = state.original_rejection_json.take() {
            retired_originals.push(value);
        }
        if let Some(owner) = state.original_aggregate_reason_owner.take() {
            retired_aggregate_owners.push(owner);
        }
    }
    let any_states = PROMISE_ANY_STATES.with(|states| {
        std::mem::take(&mut *states.borrow_mut())
    });
    for pointer in any_states {
        let state = unsafe { &mut *pointer };
        state.cancelled = true;
        state.join_counted = false;
        if let Some(share) = state._output_share.take() {
            retired_output_shares.push(share);
        }
        state.reason_roots.clear();
        state.output_root = None;
        retired_originals.extend(std::mem::take(&mut state.owned_reason_json));
        retired_aggregate_owners.extend(std::mem::take(&mut state.nested_reason_owners));
        retired_reason_sources.extend(std::mem::take(&mut state.native_reason_sources));
    }
    // A late callback may still own its Box. Retire all states first, then
    // release cloned Host payloads without a registry or state borrow held.
    for value in retired_originals {
        unsafe { thaw_json_destroy(value) };
    }
    drop(retired_aggregate_owners);
    for source in retired_reason_sources { unsafe { thaw_promise_release(source) } }
    drop(retired_output_shares);
}

struct PromiseFinallyAdoptState {
    output: *mut ThawPromise,
    _output_share: Option<PromiseOutputShare>,
    input: *mut ThawPromise,
    original: *const u8,
    original_result_kind: u8,
    // The original source may release its last share while the returned
    // Promise is still pending. Hold our own Json clone until this state is
    // drained or cancelled; an arena root alone cannot own a Box<Value>.
    original_js_clone: Option<*mut u8>,
    original_rejection_json: Option<*mut u8>,
    original_aggregate_reason_owner: Option<std::rc::Rc<AggregateReasonOwner>>,
    original_rejected: bool,
    original_tag: u64,
    original_f64: f64,
    original_i64: i64,
    original_bool: bool,
    original_object: *const u8,
    original_aggregate_errors: *const u8,
    original_provenance: *const ExceptionProvenance,
    original_text: Option<Vec<u8>>,
    original_roots: Vec<(usize, thaw_arena::ArenaRoot)>,
    cancelled: bool,
}

impl Drop for PromiseFinallyAdoptState {
    fn drop(&mut self) {
        if let Some(value) = self.original_js_clone.take() {
            unsafe { thaw_json_destroy(value) };
        }
        if let Some(value) = self.original_rejection_json.take() {
            unsafe { thaw_json_destroy(value) };
        }
        self.original_aggregate_reason_owner = None;
    }
}

extern "C" fn resume_promise_finally_adopt(frame: *mut u8, result: *const u8) {
    PROMISE_FINALLY_ADOPT_STATES.with(|states| {
        states.borrow_mut().retain(|pointer| *pointer != frame.cast());
    });
    let mut state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyAdoptState>()) };
    if state.cancelled {
        return;
    }
    if unsafe { thaw_promise_state(state.input) } == 2 {
        forward_promise_rejection(state.output, state.input, result);
    } else if state.original_rejected {
        let settled = if state.original_tag == 7 {
            unsafe { thaw_promise_reject_js_origin(state.output, state.original_object) }
        } else {
            unsafe { thaw_promise_reject_typed(
                state.output,
                state.original,
                state.original_tag,
                state.original_f64,
                state.original_i64,
                state.original_bool,
                state.original_object,
            ) }
        };
        if settled != 0 {
            unsafe {
                (*state.output).rejection_text = state.original_text.clone();
                (*state.output).aggregate_reason_owner =
                    state.original_aggregate_reason_owner.take();
                set_promise_aggregate_errors(state.output, state.original_aggregate_errors);
            }
        }
    } else {
        if state.original_result_kind == 1 {
            unsafe { thaw_promise_resolve_js_origin(state.output, state.original) };
        } else {
            unsafe { thaw_promise_resolve_with_provenance(
                state.output, state.original, state.original_provenance,
            ) };
        }
    }
}

extern "C" fn cancel_promise_finally_adopt(frame: *mut u8) {
    PROMISE_FINALLY_ADOPT_STATES.with(|states| {
        states.borrow_mut().retain(|pointer| *pointer != frame.cast());
    });
    unsafe { drop(Box::from_raw(frame.cast::<PromiseFinallyAdoptState>())) };
}

/// Waits for a Promise returned by `.finally`, then forwards the original
/// settlement unless the returned Promise rejects.
///
/// # Safety
///
/// `output` and `input` must be distinct live Promises. `original` must remain
/// valid until `input` settles.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: u8,
    original_tag: u64,
    original_f64: f64,
    original_i64: i64,
    original_bool: bool,
    original_object: *const u8,
) -> u8 {
    finally_adopt_with_source(output, input, original, original_rejected, original_tag,
        original_f64, original_i64, original_bool, original_object, std::ptr::null())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally_adopt_with_source(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: u8,
    original_tag: u64,
    original_f64: f64,
    original_i64: i64,
    original_bool: bool,
    original_object: *const u8,
    source: *const ThawPromise,
) -> u8 {
    finally_adopt_with_source(output, input, original, original_rejected, original_tag,
        original_f64, original_i64, original_bool, original_object, source)
}

fn finally_adopt_with_source(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: u8,
    original_tag: u64,
    original_f64: f64,
    original_i64: i64,
    original_bool: bool,
    original_object: *const u8,
    source: *const ThawPromise,
) -> u8 {
    if output.is_null() || input.is_null() || output == input {
        return 0;
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else { return 0; };
    let aggregate_errors = unsafe { source.as_ref() }
        .map_or(std::ptr::null(), |source| source.aggregate_errors);
    let original_aggregate_reason_owner = unsafe { source.as_ref() }
        .and_then(|source| source.aggregate_reason_owner.clone());
    let original_provenance = if original_rejected == 0 {
        unsafe { thaw_promise_fulfilled_provenance(source) }
    } else { std::ptr::null() };
    let original_result_kind = unsafe { source.as_ref() }
        .map_or(0, |source| source.result_kind);
    let original_js_clone = if original_rejected == 0 && original_result_kind == 1 {
        let cloned = unsafe { thaw_json_share(original) };
        if cloned.is_null() { return 0; }
        Some(cloned)
    } else { None };
    let original = original_js_clone
        .map_or(original, |cloned| cloned.cast_const());
    let original_rejection_json = if original_rejected != 0 && original_tag == 7 {
        let cloned = unsafe { thaw_json_share(original_object) };
        if cloned.is_null() { return 0; }
        Some(cloned)
    } else { None };
    let original_object = original_rejection_json
        .map_or(original_object, |cloned| cloned.cast_const());
    let original = original_rejection_json
        .map_or(original, |cloned| cloned.cast_const());
    let mut original_roots = Vec::new();
    for pointer in [original, original_object, aggregate_errors, original_provenance.cast()] {
        pin_promise_pointer_once(&mut original_roots, pointer as usize);
    }
    let state = Box::into_raw(Box::new(PromiseFinallyAdoptState {
        output,
        _output_share: Some(output_share),
        input,
        original,
        original_result_kind,
        original_js_clone,
        original_rejection_json,
        original_aggregate_reason_owner,
        original_rejected: original_rejected != 0,
        original_tag,
        original_f64,
        original_i64,
        original_bool,
        original_object,
        original_aggregate_errors: aggregate_errors,
        original_provenance,
        original_text: unsafe { source.as_ref() }.and_then(|source| source.rejection_text.clone()),
        original_roots,
        cancelled: false,
    }));
    PROMISE_FINALLY_ADOPT_STATES.with(|states| states.borrow_mut().push(state));
    if unsafe { subscribe_with_cancel(input, resume_promise_finally_adopt,
        Some(cancel_promise_finally_adopt), state.cast()) } == 0 {
        PROMISE_FINALLY_ADOPT_STATES.with(|states| {
            states.borrow_mut().retain(|pointer| *pointer != state);
        });
        unsafe { drop(Box::from_raw(state)) };
        return 0;
    }
    1
}

thread_local! {
    static PROMISE_ALL_STATES: RefCell<Vec<*mut PromiseAllState>> = const { RefCell::new(Vec::new()) };
    static PROMISE_ALL_SETTLED_STATES: RefCell<Vec<*mut PromiseAllSettledState>> = const { RefCell::new(Vec::new()) };
    static PROMISE_RACE_STATES: RefCell<Vec<*mut PromiseRaceState>> = const { RefCell::new(Vec::new()) };
}

fn uncount_pending_promise_all_states_for_invocation() {
    // The native Promise state and its ArenaRoot may outlive one Lambda
    // request when an input is intentionally retained. The request-local
    // detached-work counter is reset at purge; a later child callback must
    // not decrement a new invocation's counter.
    PROMISE_ALL_STATES.with(|states| {
        for pointer in states.borrow().iter().copied() {
            unsafe { (*pointer).join_counted = false };
        }
    });
    PROMISE_ALL_SETTLED_STATES.with(|states| {
        for pointer in states.borrow().iter().copied() {
            unsafe { (*pointer).join_counted = false };
        }
    });
    PROMISE_RACE_STATES.with(|states| {
        for pointer in states.borrow().iter().copied() {
            unsafe { (*pointer).join_counted = false };
        }
    });
}

struct PromiseAllState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    remaining: usize,
    rejected: bool,
    result: *mut u8,
    result_slot: *mut *const u8,
    result_handle: *mut u8,
    result_root: Option<thaw_arena::ArenaRoot>,
    join_counted: bool,
    element_sizes: Vec<usize>,
    element_offsets: Vec<usize>,
}

struct PromiseAllChild {
    state: *mut PromiseAllState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllChild>()) };
    let state = unsafe { &mut *child.state };
    let child_state = unsafe { thaw_promise_state(child.promise) };
    if child_state == 2 {
        if !state.rejected {
            state.rejected = true;
            forward_promise_rejection(state.output, child.promise, result);
            state.result_root = None;
        }
    } else if !state.rejected {
        let record = unsafe { thaw_promise_fulfilled_provenance(child.promise) };
        for index in &child.indices {
            let destination = unsafe { state.result.add(state.element_offsets[*index]) };
            unsafe {
                destination.write_bytes(0, state.element_sizes[*index].max(size_of::<u64>()));
                std::ptr::copy_nonoverlapping(result, destination, state.element_sizes[*index]);
            }
            if unsafe { thaw_array_provenance_set(state.result_handle, *index, record) } == 0 {
                state.rejected = true;
                reject_native_text(state.output, PROMISE_ALL_INVALID_ERROR.as_ptr());
                state.result_root = None;
                break;
            }
        }
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.rejected {
            thaw_promise_resolve(state.output, state.result_slot.cast());
        }
        state.result_root = None;
        PROMISE_ALL_STATES.with(|states| states.borrow_mut().retain(|current| *current != child.state));
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

extern "C" fn cancel_promise_all_child(frame: *mut u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.rejected {
        state.rejected = true;
        state.result_root = None;
        reject_native_text(state.output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_ALL_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != child.state));
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Joins homogeneous Promise handles without serializing them.
/// The result uses Thaw's `[i64 len][8-byte slots...]` array layout in the
/// request arena and therefore remains valid after the returned promise is
/// destroyed. Child handles are borrowed by this call; each subscription
/// retains its source through its own completion callback.
///
/// # Safety
///
/// `promises` must reference `len` live handles returned by Thaw async
/// functions. Each handle must be live; aliases are grouped by pointer and
/// remain owned by their callers after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_slots(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let element_sizes = vec![element_size; len];
    unsafe { thaw_promise_all_typed(promises, element_sizes.as_ptr(), len) }
}

/// Joins Promise handles using one result-copy size per input position.
/// Each value occupies at least one eight-byte array slot; wider values use
/// their complete byte width.
///
/// # Safety
///
/// `promises` and `element_sizes` must each reference `len` readable entries.
/// Promise handles are borrowed and remain owned by their callers.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_typed(
    promises: *const *mut ThawPromise,
    element_sizes: *const usize,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return output; }
    if len != 0 && element_sizes.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let element_sizes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(element_sizes, len) }.to_vec()
    };
    if element_sizes.contains(&0) {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut element_offsets = Vec::with_capacity(len);
    let mut result_bytes = size_of::<u64>();
    for size in &element_sizes {
        element_offsets.push(result_bytes);
        let Some(next) = result_bytes.checked_add((*size).max(size_of::<u64>())) else {
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        };
        result_bytes = next;
    }
    let Some(allocation_bytes) = result_bytes.checked_add(size_of::<*const u8>()) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let allocation = thaw_arena::thaw_arena_alloc(allocation_bytes, align_of::<u64>());
    if allocation.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(size_of::<*const u8>()) };
    // The Promise's own resolved value has Array/Tuple type, so it must be a
    // handle - not the raw buffer - by the time codegen's generic await
    // extraction loads it back out of `result_slot`. See `wrap_array_handle`.
    let result_handle = wrap_array_handle(result);
    if result_handle.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    unsafe { result_slot.write(result_handle.cast()) };
    unsafe { result.cast::<u64>().write(len as u64) };
    if unsafe { thaw_array_provenance_prepare(result_handle, len) } == 0 {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    if promises.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else {
            if let Some((_, indices)) = grouped
                .iter_mut()
                .find(|(existing, _)| *existing == promise)
            {
                indices.push(index);
            } else {
                grouped.push((promise, vec![index]));
            }
        }
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseAllState {
        output,
        _output_share: output_share,
        remaining: grouped.len(),
        rejected: invalid,
        result,
        result_slot,
        result_handle,
        result_root: Some(thaw_arena::ArenaRoot::new(result_slot as usize)),
        join_counted: !grouped.is_empty(),
        element_sizes,
        element_offsets,
    }));
    if !grouped.is_empty() {
        PROMISE_ALL_STATES.with(|states| states.borrow_mut().push(state));
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { (*state).result_root = None };
    }
    if grouped.is_empty() {
        unsafe { drop(Box::from_raw(state)) };
        return output;
    }
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllChild {
            state,
            promise,
            indices,
        }));
        if unsafe { subscribe_with_cancel(promise, resume_promise_all_child,
            Some(cancel_promise_all_child), child.cast()) } == 0 {
            unsafe { drop(Box::from_raw(child)) };
            let state_ref = unsafe { &mut *state };
            state_ref.rejected = true;
            state_ref.result_root = None;
            state_ref.remaining -= 1;
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        PROMISE_ALL_STATES.with(|states| states.borrow_mut().retain(|current| *current != state));
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

/// Backward-compatible number-only entry point.
///
/// # Safety
///
/// The same contract as [`thaw_promise_all_slots`] applies.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_f64(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    unsafe { thaw_promise_all_slots(promises, len, size_of::<f64>()) }
}

struct PromiseRaceState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    remaining: usize,
    settled: bool,
    join_counted: bool,
}

struct PromiseRaceChild {
    state: *mut PromiseRaceState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_race_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseRaceChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.settled {
        state.settled = true;
        if unsafe { thaw_promise_state(child.promise) } == 2 {
            forward_promise_rejection(state.output, child.promise, result);
        } else {
            forward_promise_fulfillment(state.output, child.promise, result);
        }
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_RACE_STATES.with(|states| {
            states.borrow_mut().retain(|pointer| *pointer != child.state);
        });
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

extern "C" fn cancel_promise_race_child(frame: *mut u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseRaceChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.settled {
        state.settled = true;
        reject_native_text(state.output, PROMISE_RACE_INVALID_ERROR.as_ptr());
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_RACE_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != child.state));
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Settles with the first input Promise to fulfill or reject. Input handles
/// are deduplicated and borrowed; slower children continue to be drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle must be live; the caller retains ownership after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_race(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return output; }
    if len == 0 {
        return output;
    }
    if promises.is_null() {
        reject_native_text(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else if !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseRaceState {
        output,
        _output_share: output_share,
        remaining: unique.len(),
        settled: invalid,
        join_counted: !unique.is_empty(),
    }));
    let join_counted = !unique.is_empty();
    if join_counted {
        PROMISE_RACE_STATES.with(|states| states.borrow_mut().push(state));
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        reject_native_text(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
    }
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseRaceChild { state, promise }));
        if unsafe { subscribe_with_cancel(promise, resume_promise_race_child,
            Some(cancel_promise_race_child), child.cast()) } == 0 {
            unsafe { drop(Box::from_raw(child)) };
            let state_ref = unsafe { &mut *state };
            state_ref.settled = true;
            state_ref.remaining -= 1;
            reject_native_text(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        PROMISE_RACE_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != state));
        if join_counted {
            // Each child was rejected before registration; no callback will
            // retire the join count for this state.
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

// The private 16-byte reason slot uses the same {i8 tag, i64 payload}
// storage as a compiler Union. Tags: 0 number, 1 bigint, 2 boolean,
// 3 string, 4 undefined, 5 null, 6 native object pointer, 7 nested
// AggregateError record, 8 opaque pointer, 9 native error-text record,
// 10 independently owned Json reason.
// This precursor exposes it only through the Promise-owned aggregate handle;
// HIR consumers are a separate unit.
struct AggregateReasonOwner {
    boxes: Vec<*mut u8>,
    nested: Vec<std::rc::Rc<AggregateReasonOwner>>,
    // Legacy nested tag9 records can be projected after the originating
    // Promise.any state has gone. Keep their native source Promises live for
    // exactly as long as the reason buffer remains observable.
    sources: Vec<*mut ThawPromise>,
}

thread_local! {
    static AGGREGATE_REASON_OWNERS:
        RefCell<std::collections::HashMap<usize, (u128, Option<u64>, std::rc::Rc<AggregateReasonOwner>)>> =
        RefCell::new(std::collections::HashMap::new());
}

fn reset_aggregate_reason_owners(traced: bool) {
    let retired = AGGREGATE_REASON_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        let mut retained = std::collections::HashMap::new();
        let mut retired = Vec::new();
        let current_epoch = thaw_arena::reset_epoch();
        for (buffer, (registered_epoch, serial, owner)) in std::mem::take(&mut *owners) {
            // Epoch protects registrations made by earlier hooks during an
            // untraced reset; serial identifies the actual tracked allocation
            // even if a nested reset overwrites RECLAIMED before this hook.
            let live = if traced {
                serial.is_some() && serial == thaw_arena::allocation_serial(buffer)
            } else {
                registered_epoch == current_epoch
            };
            if live {
                retained.insert(buffer, (registered_epoch, serial, owner));
            } else {
                retired.push(owner);
            }
        }
        *owners = retained;
        retired
    });
    // Releasing a Host lease may reenter compiled code. No registry borrow is
    // live here, and all dead entries are already removed.
    drop(retired);
}

fn register_aggregate_reason_owner(
    errors: *const u8,
    owner: std::rc::Rc<AggregateReasonOwner>,
) {
    if errors.is_null() { return; }
    let registered_epoch = thaw_arena::reset_epoch();
    let serial = thaw_arena::allocation_serial(errors as usize);
    let replaced = AGGREGATE_REASON_OWNERS.with(|owners| {
        owners.borrow_mut().insert(errors as usize, (registered_epoch, serial, owner))
    });
    // A replaced owner can release Host leases and reenter this registry.
    drop(replaced);
    thaw_arena::register_reset_hook(reset_aggregate_reason_owners);
}

// A missing reason array is an allocation/shape failure, not a successful
// AggregateError with an empty or absent `errors` property. Keep any captured
// Host reasons alive until rejection has copied its native text, then retire
// them exactly once if the array could not be installed.
type PromiseAnyProjector = extern "C" fn(*const u8, *mut ThawPromise) -> *mut u8;

#[derive(Clone, Copy)]
struct ActivePromiseAnyReasons {
    handle: usize,
    handle_serial: u64,
    buffer: usize,
    buffer_serial: u64,
}
thread_local! {
    static ACTIVE_PROMISE_ANY_REASONS: RefCell<Vec<ActivePromiseAnyReasons>> =
        RefCell::new(Vec::new());
}
struct PromiseAnyReasonAccess { start_len: usize }
impl PromiseAnyReasonAccess {
    fn enter(handle: *const u8) -> Option<Self> {
        // Only a source-live, arena-tracked reason array can cross the
        // projector ABI. The arena serial distinguishes reused addresses.
        let handle_serial = thaw_arena::allocation_serial(handle as usize)?;
        let buffer = unsafe { handle.cast::<*const u8>().read() };
        let buffer_serial = thaw_arena::allocation_serial(buffer as usize)?;
        let start_len = ACTIVE_PROMISE_ANY_REASONS.with(|active| {
            let mut active = active.borrow_mut();
            let start_len = active.len();
            active.push(ActivePromiseAnyReasons {
                handle: handle as usize, handle_serial,
                buffer: buffer as usize, buffer_serial,
            });
            start_len
        });
        Some(Self { start_len })
    }
}
impl Drop for PromiseAnyReasonAccess {
    fn drop(&mut self) {
        ACTIVE_PROMISE_ANY_REASONS.with(|active| {
            active.borrow_mut().truncate(self.start_len);
        });
    }
}
fn active_promise_any_reason_buffer(handle: *const u8) -> Option<*const u8> {
    let access = ACTIVE_PROMISE_ANY_REASONS.with(|active| active.borrow().iter().rev()
        .find(|entry| entry.handle == handle as usize).copied())?;
    if thaw_arena::allocation_serial(access.handle) != Some(access.handle_serial)
        || thaw_arena::allocation_serial(access.buffer) != Some(access.buffer_serial)
    { return None; }
    Some(access.buffer as *const u8)
}

/// Trusted projector-only accessors. Invalid or expired handles return zero;
/// callers must check count/index before interpreting a reason slot.
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_count(handle: *const u8) -> usize {
    let Some(buffer) = active_promise_any_reason_buffer(handle) else { return 0; };
    unsafe { buffer.cast::<u64>().read() as usize }
}
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_tag(handle: *const u8, index: usize) -> u8 {
    let Some(buffer) = active_promise_any_reason_buffer(handle) else { return u8::MAX; };
    let count = unsafe { buffer.cast::<u64>().read() as usize };
    if index >= count { return u8::MAX; }
    unsafe { buffer.add(8 + index * 16).read() }
}
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_payload(handle: *const u8, index: usize) -> u64 {
    let Some(buffer) = active_promise_any_reason_buffer(handle) else { return 0; };
    let count = unsafe { buffer.cast::<u64>().read() as usize };
    if index >= count { return 0; }
    unsafe { buffer.add(16 + index * 16).cast::<u64>().read_unaligned() }
}

fn active_promise_any_reason_record(
    parent: *const u8, index: usize,
) -> Option<*const PromiseAnyErrorRecord> {
    let tag = thaw_promise_any_reason_tag(parent, index);
    if tag != 7 && tag != 9 { return None; }
    let record = thaw_promise_any_reason_payload(parent, index) as *const PromiseAnyErrorRecord;
    thaw_arena::allocation_serial(record as usize)?;
    Some(record)
}

fn active_promise_any_record_source(source: *mut ThawPromise) -> bool {
    ACTIVE_PROMISE_ANY_REASONS.with(|active| active.borrow().iter().rev().any(|entry| {
        if thaw_arena::allocation_serial(entry.handle) != Some(entry.handle_serial)
            || thaw_arena::allocation_serial(entry.buffer) != Some(entry.buffer_serial)
        { return false; }
        let buffer = entry.buffer as *const u8;
        let count = unsafe { buffer.cast::<u64>().read() as usize };
        for index in 0..count {
            let slot = unsafe { buffer.add(8 + index * 16) };
            if !matches!(unsafe { slot.read() }, 7 | 9) { continue; }
            let record = unsafe { slot.add(8).cast::<u64>().read_unaligned() }
                as *const PromiseAnyErrorRecord;
            if thaw_arena::allocation_serial(record as usize).is_some()
                && unsafe { (*record).source } == source
            { return true; }
        }
        false
    }))
}

/// Native text copied while the rejected child was live. An untracked or
/// expired pointer cannot cross this private projector boundary.
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_record_text(
    parent: *const u8, index: usize,
) -> *const u8 {
    let Some(record) = active_promise_any_reason_record(parent, index) else {
        return std::ptr::null();
    };
    let text = unsafe { (*record).text };
    if thaw_arena::allocation_serial(text as usize).is_none() {
        return std::ptr::null();
    }
    text
}

/// A native Error owner may be projected only through its registered live
/// arena owner; external opaque pointers do not receive this authority.
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_record_object(
    parent: *const u8, index: usize,
) -> *const u8 {
    let Some(record) = active_promise_any_reason_record(parent, index) else {
        return std::ptr::null();
    };
    let object = unsafe { (*record).object };
    if thaw_arena::allocation_serial(object as usize).is_none() {
        return std::ptr::null();
    }
    object
}

/// Native-text and legacy nested-AggregateError records retain their source
/// Promise through the reason owner. The pointer is exposed only during the
/// validated callback, so both forms can cache one canonical JS Error.
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_record_source(
    parent: *const u8, index: usize,
) -> *mut ThawPromise {
    if !matches!(thaw_promise_any_reason_tag(parent, index), 7 | 9) {
        return std::ptr::null_mut();
    }
    let Some(record) = active_promise_any_reason_record(parent, index) else {
        return std::ptr::null_mut();
    };
    let source = unsafe { (*record).source };
    if thaw_promise_identity(source) == 0 { return std::ptr::null_mut(); }
    source
}

/// Authorize one nested AggregateError reason array for the duration of the
/// current projector callback. This validates the parent slot before reading
/// the native record; the outer owner keeps the nested owner live.
#[no_mangle]
pub extern "C" fn thaw_promise_any_reason_nested_errors(
    parent: *const u8, index: usize,
) -> *const u8 {
    if thaw_promise_any_reason_tag(parent, index) != 7 { return std::ptr::null(); }
    let Some(record) = active_promise_any_reason_record(parent, index) else {
        return std::ptr::null();
    };
    let nested = unsafe { (*record).errors };
    let Some(handle_serial) = thaw_arena::allocation_serial(nested as usize) else {
        return std::ptr::null();
    };
    let buffer = unsafe { nested.cast::<*const u8>().read() };
    let Some(buffer_serial) = thaw_arena::allocation_serial(buffer as usize) else {
        return std::ptr::null();
    };
    ACTIVE_PROMISE_ANY_REASONS.with(|active| active.borrow_mut().push(
        ActivePromiseAnyReasons {
            handle: nested as usize, handle_serial,
            buffer: buffer as usize, buffer_serial,
        },
    ));
    nested
}

fn finish_promise_any_rejection(
    output: *mut ThawPromise,
    errors: *const u8,
    owner: Option<std::rc::Rc<AggregateReasonOwner>>,
    projector: Option<PromiseAnyProjector>,
) {
    if errors.is_null() {
        reject_native_text(output, PROMISE_ANY_ALLOCATION_ERROR.as_ptr());
        return;
    }
    if let Some(projector) = projector {
        // A compiled projector materializes one real AggregateError while the
        // ordered reason slots and their Host leases remain live. Its return
        // is an arena-managed borrowed Json Box. It may reject `output` itself
        // with the exact thrown value if construction fails.
        let _reason_root = thaw_arena::ArenaRoot::new(errors as usize);
        let Some(_access) = PromiseAnyReasonAccess::enter(errors) else {
            reject_native_text(output, PROMISE_ANY_ALLOCATION_ERROR.as_ptr());
            return;
        };
        let projected = projector(errors, output);
        if !projected.is_null() {
            if unsafe { thaw_promise_state(output) } == 0
                && unsafe { thaw_promise_reject_js_origin(output, projected.cast_const()) } == 0
                && unsafe { thaw_promise_state(output) } == 0
            {
                reject_native_text(output, PROMISE_ANY_ALLOCATION_ERROR.as_ptr());
            }
        } else if unsafe { thaw_promise_state(output) } == 0 {
            reject_native_text(output, PROMISE_ANY_ALLOCATION_ERROR.as_ptr());
        }
        // `owner` is released after the projector and Promise settlement.
        return;
    }
    if reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr()) != 0 {
        if let Some(owner) = owner {
            register_aggregate_reason_owner(errors, owner);
        }
        set_promise_aggregate_errors(output, errors);
    }
}

impl Drop for AggregateReasonOwner {
    fn drop(&mut self) {
        for value in self.boxes.drain(..) {
            unsafe { thaw_json_destroy(value) };
        }
        for source in self.sources.drain(..) {
            unsafe { thaw_promise_release(source) };
        }
        // Nested owners drop after these boxes so their Host leases remain
        // live through a reentrant destructor of an outer reason.
    }
}

#[derive(Clone, Copy)]
struct PromiseAnyReason {
    tag: u8,
    payload: u64,
}

#[repr(C)]
struct PromiseAnyErrorRecord {
    text: *const u8,
    errors: *const u8,
    object: *const u8,
    source: *mut ThawPromise,
}

// Snapshot while the child Promise is still live. Native-owned text is copied
// into the invocation arena; arbitrary opaque rejection pointers are stored
// as opaque bits and are never dereferenced here.
fn promise_any_reason(
    promise: &ThawPromise,
    result: *const u8,
    owned_json: &mut Vec<*mut u8>,
    nested: &mut Vec<std::rc::Rc<AggregateReasonOwner>>,
    source_shares: &mut Vec<*mut ThawPromise>,
) -> Option<PromiseAnyReason> {
    if promise.exception_tag == 7 {
        let shared = unsafe { thaw_json_share(promise.exception_object) };
        if shared.is_null() { return None; }
        owned_json.push(shared);
        return Some(PromiseAnyReason { tag: 10, payload: shared as u64 });
    }
    if !promise.aggregate_errors.is_null()
        || (promise.exception_tag == 0 && promise.exception_object.is_null()
            && promise.rejection_text.is_some())
    {
        let text = promise.rejection_text.as_deref()?;
        let source = promise as *const ThawPromise as *mut ThawPromise;
        let native_source = thaw_promise_retain(source);
        if native_source.is_null() { return None; }
        source_shares.push(native_source);
        let text = thaw_arena::arena_string(text).cast::<u8>();
        if text.is_null() { return None; }
        let record = thaw_arena::thaw_arena_alloc(
            size_of::<PromiseAnyErrorRecord>(), align_of::<PromiseAnyErrorRecord>(),
        ).cast::<PromiseAnyErrorRecord>();
        if record.is_null() { return None; }
        // Arena tracing scans pointer-sized words, including struct padding.
        // Initialize the entire record before its field pointers become live.
        unsafe {
            record.cast::<u8>().write_bytes(0, size_of::<PromiseAnyErrorRecord>());
            std::ptr::addr_of_mut!((*record).text).write(text);
            std::ptr::addr_of_mut!((*record).errors).write(promise.aggregate_errors);
            std::ptr::addr_of_mut!((*record).object).write(promise.exception_object);
            std::ptr::addr_of_mut!((*record).source).write(native_source);
        }
        if let Some(owner) = &promise.aggregate_reason_owner {
            nested.push(owner.clone());
        }
        return Some(PromiseAnyReason {
            tag: if promise.aggregate_errors.is_null() { 9 } else { 7 },
            payload: record as u64,
        });
    }
    let (tag, payload) = match promise.exception_tag {
        1 => (0, promise.exception_f64.to_bits()),
        2 => (1, promise.exception_i64 as u64),
        3 => (2, u64::from(promise.exception_bool)),
        4 => {
            let text = match promise.rejection_text.as_deref() {
                Some(bytes) => thaw_arena::arena_string(bytes).cast::<u8>(),
                None => result,
            };
            if text.is_null() { return None; }
            (3, text as u64)
        }
        5 => (4, 0),
        6 => (5, 0), // Explicit null tag is supplied by the HIR stage.
        0 if !promise.exception_object.is_null() => (6, promise.exception_object as u64),
        _ => (8, result as u64), // Opaque: no text/object dereference.
    };
    Some(PromiseAnyReason { tag, payload })
}

fn promise_any_errors(reasons: &[Option<PromiseAnyReason>]) -> *const u8 {
    let Some(bytes) = reasons.len().checked_mul(16).and_then(|n| n.checked_add(8)) else {
        return std::ptr::null();
    };
    let buffer = thaw_arena::thaw_arena_alloc(bytes, 8);
    if buffer.is_null() { return std::ptr::null(); }
    unsafe { buffer.cast::<u64>().write(reasons.len() as u64) };
    for (index, reason) in reasons.iter().enumerate() {
        let Some(reason) = reason else { return std::ptr::null(); };
        let slot = unsafe { buffer.add(8 + index * 16) };
        unsafe {
            // The scanner sees the whole first word, not just its tag byte.
            // Keep the tag at byte zero on either endian layout.
            slot.write_bytes(0, 8);
            slot.write(reason.tag);
            slot.add(8).cast::<u64>().write_unaligned(reason.payload);
        }
    }
    wrap_array_handle(buffer).cast()
}

struct PromiseAnyState {
    output: *mut ThawPromise,
    _output_share: Option<PromiseOutputShare>,
    remaining: usize,
    fulfilled: bool,
    reasons: Vec<Option<PromiseAnyReason>>,
    owned_reason_json: Vec<*mut u8>,
    nested_reason_owners: Vec<std::rc::Rc<AggregateReasonOwner>>,
    native_reason_sources: Vec<*mut ThawPromise>,
    reason_roots: Vec<(usize, thaw_arena::ArenaRoot)>,
    output_root: Option<thaw_arena::ArenaRoot>,
    projector: Option<PromiseAnyProjector>,
    cancelled: bool,
    join_counted: bool,
}

impl Drop for PromiseAnyState {
    fn drop(&mut self) {
        // An unsuccessful subscription can stop the join after earlier
        // children have already supplied owned Json reasons. The normal
        // rejection path moves these into AggregateReasonOwner; retire any
        // reasons left in the state on every other exit.
        for value in std::mem::take(&mut self.owned_reason_json) {
            unsafe { thaw_json_destroy(value) };
        }
        for source in std::mem::take(&mut self.native_reason_sources) {
            unsafe { thaw_promise_release(source) };
        }
    }
}

struct PromiseAnyChild {
    state: *mut PromiseAnyState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_any_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let mut retired_json = Vec::new();
    let mut retired_owners = Vec::new();
    let mut retired_sources = Vec::new();
    let mut forward = false;
    let mut reject_errors = None;
    let mut done = false;
    {
        let state = unsafe { &mut *child.state };
        if !state.cancelled && !state.fulfilled {
            if unsafe { thaw_promise_state(child.promise) } == 1 {
                state.fulfilled = true;
                state.reason_roots.clear();
                retired_json = std::mem::take(&mut state.owned_reason_json);
                retired_owners = std::mem::take(&mut state.nested_reason_owners);
                retired_sources = std::mem::take(&mut state.native_reason_sources);
                forward = true;
            } else {
                let reason = promise_any_reason(
                    unsafe { &*child.promise }, result, &mut state.owned_reason_json,
                    &mut state.nested_reason_owners,
                    &mut state.native_reason_sources,
                );
                if let Some(reason) = reason {
                    if matches!(reason.tag, 3 | 6 | 7 | 8 | 9) {
                        pin_promise_pointer_once(&mut state.reason_roots, reason.payload as usize);
                    }
                }
                for index in &child.indices { state.reasons[*index] = reason; }
            }
        }
        state.remaining -= 1;
        done = state.remaining == 0;
        if done {
            PROMISE_ANY_STATES.with(|states| {
                states.borrow_mut().retain(|pointer| *pointer != child.state);
            });
            if !state.cancelled && !state.fulfilled {
                reject_errors = Some(promise_any_errors(&state.reasons));
            }
            state.reason_roots.clear();
            if state.join_counted {
                ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
            }
        }
    }
    if forward {
        let output = unsafe { (*child.state).output };
        forward_promise_fulfillment(output, child.promise, result);
    }
    if let Some(errors) = reject_errors {
        let output = unsafe { (*child.state).output };
        // The same Box addresses stay in arena reason slots across forwarding.
        // The owner is also responsible for releasing them on every failure.
        let (owner, projector) = {
            let state = unsafe { &mut *child.state };
            (std::rc::Rc::new(AggregateReasonOwner {
                boxes: std::mem::take(&mut state.owned_reason_json),
                nested: std::mem::take(&mut state.nested_reason_owners),
                sources: std::mem::take(&mut state.native_reason_sources),
            }), state.projector)
        };
        finish_promise_any_rejection(output, errors, Some(owner), projector);
    }
    if done {
        unsafe { drop(Box::from_raw(child.state)) };
    }
    for value in retired_json { unsafe { thaw_json_destroy(value) } }
    drop(retired_owners);
    for source in retired_sources { unsafe { thaw_promise_release(source) } }
}

extern "C" fn cancel_promise_any_child(frame: *mut u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.cancelled && !state.fulfilled {
        state.cancelled = true;
        reject_native_text(state.output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_ANY_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != child.state));
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Resolves with the first fulfilled input Promise. Every rejected input
/// position keeps its own reason, even when one Promise handle occurs twice.
/// Slower children are still drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// non-null handle is borrowed. A child that outlives the output still has
/// its own subscription share until its callback returns.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    unsafe { thaw_promise_any_with_projector(promises, len, None) }
}

/// Compiler-private Promise.any with one source-live AggregateError projector.
/// The callback must not retain `errors_handle` past its invocation. It may
/// reject `output` on a construction throw and then return null; otherwise it
/// returns an arena-owned borrowed Json Box of the canonical AggregateError.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any_with_projector(
    promises: *const *mut ThawPromise,
    len: usize,
    projector: Option<PromiseAnyProjector>,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return output; }
    if len == 0 {
        let errors = promise_any_errors(&[]);
        finish_promise_any_rejection(output, errors, None, projector);
        return output;
    }
    if promises.is_null() {
        reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() { continue; }
        if let Some((_, indices)) = grouped.iter_mut().find(|(existing, _)| *existing == promise) {
            indices.push(index);
        } else {
            grouped.push((promise, vec![index]));
        }
    }
    if grouped.is_empty() {
        reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseAnyState {
        output,
        _output_share: Some(output_share),
        remaining: grouped.len(),
        fulfilled: false,
        reasons: vec![None; len],
        owned_reason_json: Vec::new(),
        nested_reason_owners: Vec::new(),
        native_reason_sources: Vec::new(),
        reason_roots: Vec::new(),
        output_root: Some(thaw_arena::ArenaRoot::new(output as usize)),
        projector,
        cancelled: false,
        join_counted: true,
    }));
    PROMISE_ANY_STATES.with(|states| states.borrow_mut().push(state));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAnyChild { state, promise, indices }));
        if unsafe { subscribe_with_cancel(promise, resume_promise_any_child,
            Some(cancel_promise_any_child), child.cast()) } == 0 {
            unsafe { drop(Box::from_raw(child)) };
            let state_ref = unsafe { &mut *state };
            state_ref.fulfilled = true;
            state_ref.remaining -= 1;
            reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        PROMISE_ANY_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != state));
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

// The compiler can construct the selected result shape while the original
// input Promise and its rejection provenance are still live.
type PromiseSettledConstructor = unsafe extern "C" fn(*mut ThawPromise, *const u8, *mut u8, *mut ThawPromise) -> *mut u8;

struct PromiseAllSettledState {
    output: *mut ThawPromise,
    _output_share: PromiseOutputShare,
    remaining: usize,
    result_slot: *mut *const u8,
    result: *mut u64,
    result_stride: usize,
    objects: *mut u8,
    element_size: usize,
    object_stride: usize,
    constructor: Option<PromiseSettledConstructor>,
    construction_failed: bool,
    _result_root: thaw_arena::ArenaRoot,
    _objects_root: Option<thaw_arena::ArenaRoot>,
    join_counted: bool,
}

struct PromiseAllSettledChild {
    state: *mut PromiseAllSettledState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_settled_child(frame: *mut u8, value: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllSettledChild>()) };
    let rejected = unsafe { thaw_promise_state(child.promise) } == 2;
    if !unsafe { (*child.state).construction_failed } {
    for index in &child.indices {
        if let Some(constructor) = unsafe { (*child.state).constructor } {
            // A failed materializer settles the output once. Drain consumed
            // children without calling further user-visible projectors.
            if unsafe { (*child.state).construction_failed } { continue; }
            let (destination, output) = unsafe {
                let state = &*child.state;
                (state.result.cast::<u8>().add(size_of::<u64>() + index * state.result_stride), state.output)
            };
            // Projection may reenter the continuation queue. Do not keep a
            // Rust reference to shared join state across generated callbacks.
            let object = unsafe { constructor(child.promise, value, destination, output) };
            let state = unsafe { &mut *child.state };
            if object.is_null() {
                state.construction_failed = true;
                reject_native_text(state.output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            } else {
                thaw_arena::replace_reference(state.result_slot as usize, 0, object as usize);
            }
            continue;
        }
        let state = unsafe { &mut *child.state };
        let object = unsafe { state.objects.add(index * state.object_stride) };
        let value_slot = unsafe { object.add(size_of::<u64>()) };
        let reason_slot = unsafe { value_slot.add(state.element_size.max(size_of::<u64>())) };
        unsafe {
            object.cast::<u64>().write(if rejected {
                PROMISE_SETTLED_REJECTED.as_ptr() as u64
            } else {
                PROMISE_SETTLED_FULFILLED.as_ptr() as u64
            });
            value_slot.write_bytes(0, state.element_size.max(size_of::<u64>()));
            if !rejected {
                std::ptr::copy_nonoverlapping(value, value_slot, state.element_size);
            }
            reason_slot.cast::<u64>().write(if rejected {
                value as u64
            } else {
                PROMISE_SETTLED_EMPTY_REASON.as_ptr() as u64
            });
            state.result.add(index + 1).write(object as u64);
        }
    }
    }
    let state = unsafe { &mut *child.state };
    state.remaining -= 1;
    if state.remaining == 0 {
        thaw_promise_resolve(state.output, state.result_slot.cast());
        PROMISE_ALL_SETTLED_STATES.with(|states| states.borrow_mut().retain(|current| *current != child.state));
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

extern "C" fn cancel_promise_all_settled_child(frame: *mut u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllSettledChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.construction_failed {
        state.construction_failed = true;
        reject_native_text(state.output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_ALL_SETTLED_STATES.with(|states| {
            states.borrow_mut().retain(|pointer| *pointer != child.state);
        });
        if state.join_counted {
            ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        }
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Waits for every distinct input and resolves with an input-ordered array of
/// `{ status, value, reason }` object pointers. Rejections become result
/// entries and never reject the output Promise.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle is borrowed. Its subscription owns a share through resume.
/// Values wider than one array slot are copied in full.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_settled(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    unsafe { promise_all_settled_with_constructor(promises, len, element_size, size_of::<u64>(), None) }
}

/// Compiler-private result construction while input provenance is live.
/// The callback writes exactly one `result_stride`-byte native element and
/// returns its object owner, or null on failure. On a thrown exception the
/// callback rejects `output` through the compiler's existing typed rejection
/// path before returning null; the fallback rejection cannot overwrite it.
/// The returned owner is retained before the subscription releases its source share.
///
/// # Safety
/// Input handles obey `thaw_promise_all_settled`'s borrowed-input contract.
/// `constructor` must be a live compiler-generated function with that layout.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_settled_constructed(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
    result_stride: usize,
    constructor: Option<unsafe extern "C" fn(*mut ThawPromise, *const u8, *mut u8, *mut ThawPromise) -> *mut u8>,
) -> *mut ThawPromise {
    if constructor.is_none() {
        let output = thaw_promise_new();
        if output.is_null() { return output; }
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    unsafe { promise_all_settled_with_constructor(promises, len, element_size, result_stride, constructor) }
}

unsafe fn promise_all_settled_with_constructor(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
    result_stride: usize,
    constructor: Option<PromiseSettledConstructor>,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if output.is_null() { return output; }
    if element_size == 0 || result_stride < size_of::<u64>()
        || result_stride % size_of::<u64>() != 0
        || (constructor.is_none() && result_stride != size_of::<u64>())
        || (len != 0 && promises.is_null()) {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let Some(allocation_size) = len.checked_mul(result_stride)
        .and_then(|bytes| bytes.checked_add(2 * size_of::<u64>())) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let allocation = thaw_arena::thaw_arena_alloc(allocation_size, align_of::<u64>()).cast::<u64>();
    let (object_stride, objects_size) = if constructor.is_none() {
        let Some(stride) = size_of::<u64>()
            .checked_add(element_size.max(size_of::<u64>()))
            .and_then(|size| size.checked_add(size_of::<u64>()))
        else {
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        };
        let Some(bytes) = len.checked_mul(stride) else {
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        };
        (stride, bytes)
    } else {
        (0, 0)
    };
    let objects = if constructor.is_none() && len != 0 {
        thaw_arena::thaw_arena_alloc(objects_size, align_of::<u64>())
    } else { std::ptr::null_mut() };
    if allocation.is_null() || (constructor.is_none() && len != 0 && objects.is_null()) {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    unsafe {
        allocation.cast::<u8>().write_bytes(0, allocation_size);
        if !objects.is_null() { objects.write_bytes(0, objects_size); }
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(1) };
    let result_handle = wrap_array_handle(result.cast());
    if result_handle.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    unsafe {
        // The Promise's own resolved value has Array type, so it must be a
        // handle - not the raw buffer - by the time codegen's generic await
        // extraction loads it back out of `result_slot`. See
        // `wrap_array_handle` (defined alongside `thaw_regex_match_all`,
        // which needs the identical wrap for its own nested arrays).
        result_slot.write(result_handle.cast());
        result.write(len as u64);
    }
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        }
        if let Some((_, indices)) = grouped
            .iter_mut()
            .find(|(existing, _)| *existing == promise)
        {
            indices.push(index);
        } else {
            grouped.push((promise, vec![index]));
        }
    }
    let Some(output_share) = PromiseOutputShare::acquire(output) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let state = Box::into_raw(Box::new(PromiseAllSettledState {
        output,
        _output_share: output_share,
        remaining: grouped.len(),
        result_slot,
        result,
        result_stride,
        objects,
        element_size,
        object_stride,
        constructor,
        construction_failed: false,
        _result_root: thaw_arena::ArenaRoot::new(result_slot as usize),
        _objects_root: (!objects.is_null()).then(|| thaw_arena::ArenaRoot::new(objects as usize)),
        join_counted: true,
    }));
    PROMISE_ALL_SETTLED_STATES.with(|states| states.borrow_mut().push(state));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllSettledChild {
            state,
            promise,
            indices,
        }));
        if unsafe { subscribe_with_cancel(promise, resume_promise_all_settled_child,
            Some(cancel_promise_all_settled_child), child.cast()) } == 0 {
            unsafe { drop(Box::from_raw(child)) };
            let state_ref = unsafe { &mut *state };
            state_ref.construction_failed = true;
            state_ref.remaining -= 1;
            reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        PROMISE_ALL_SETTLED_STATES.with(|states| states.borrow_mut().retain(|pointer| *pointer != state));
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

fn settle_promise(
    promise: *mut ThawPromise, result: *const u8, rejected: bool,
    provenance: *const ExceptionProvenance,
) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if promise.result.is_some() {
        return 0;
    }
    promise.result = Some(result);
    let owner = promise as *mut ThawPromise as usize;
    thaw_arena::replace_reference(owner, 0, result as usize);
    if !rejected && !provenance.is_null() {
        thaw_arena::replace_reference(owner, 0, provenance as usize);
        promise.fulfilled_provenance = provenance;
    }
    promise.rejected = rejected;
    let subscribers = std::mem::take(&mut promise.subscribers);
    for subscriber in subscribers {
        enqueue_continuation(subscriber, result);
    }
    1
}

/// Consumes the creator share of a promise handle. Passing null is a no-op.
/// A queued callback or stored alias remains live through its independent
/// share, including when this runs from inside a resume callback.
///
/// # Safety
///
/// `promise` must be null or a live pointer returned by `thaw_promise_new`
/// whose creator share has not already been consumed. This call does not
/// release the independent request, subscriber, field or JS-wrapper shares.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_destroy(promise: *mut ThawPromise) {
    if promise.is_null() { return; }
    let live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&promise));
    if !live || !unsafe { (*promise).creator_live } { return; }
    unsafe { (*promise).creator_live = false };
    unsafe { thaw_promise_release(promise) };
    // Keep the request share until its boundary even if no other owner is
    // currently known: a native array may store this pointer and mutate its
    // stable handle before reset-time ownership reconciliation runs.
}

/// Consume this Promise's request-base share at most once. A fresh output may
/// already have subscriber or JS-wrapper shares when setup fails; releasing
/// the base as an undifferentiated share would leave `base_live` set and let
/// request reset release one of those other owners a second time.
///
/// # Safety
/// `promise` must be null or a live pointer returned by this runtime.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_release_request_base(promise: *mut ThawPromise) {
    let identity = thaw_promise_identity(promise);
    if identity == 0 { return; }
    let promise = thaw_promise_from_identity(identity);
    if promise.is_null() || !unsafe { (*promise).base_live } { return; }
    unsafe { (*promise).base_live = false };
    unsafe { thaw_promise_release(promise) };
}

/// Releases one independently retained subscriber, field or JS-wrapper
/// share. It never consumes the creator share; the request reset owns a
/// separate base share so pending work can survive until its own release.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_release(promise: *mut ThawPromise) {
    if !promise.is_null() {
        let live = ACTIVE_PROMISES.with(|active| active.borrow().contains(&promise));
        if !live { return; }
        let promise_ref = unsafe { &mut *promise };
        if promise_ref.owners > 1 {
            promise_ref.owners -= 1;
            return;
        }
        ACTIVE_PROMISES.with(|active| active.borrow_mut().retain(|current| *current != promise));
        TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.promise != promise));
        FD_WAITS.with(|waits| waits.borrow_mut().retain(|wait| wait.promise != promise));
        let owned_json = (promise_ref.result_kind == 1)
            .then(|| promise_ref.result)
            .flatten()
            .map(|result| result.cast_mut());
        let rejection_json = promise_ref.rejection_json.take();
        let rejection_source = promise_ref.rejection_source.take();
        let aggregate_reason_owner = promise_ref.aggregate_reason_owner.take();
        #[cfg(test)]
        let released_identity = promise_ref.identity;
        thaw_arena::forget_references(promise as usize);
        drop(Box::from_raw(promise));
        #[cfg(test)]
        if let Some(hook) = PROMISE_RELEASE_TEST_HOOK.with(|hook| hook.replace(None)) {
            hook(released_identity);
        }
        if let Some(value) = owned_json {
            unsafe { thaw_json_destroy(value) };
        }
        if let Some(value) = rejection_json {
            unsafe { thaw_json_destroy(value) };
        }
        drop(aggregate_reason_owner);
        if let Some(source) = rejection_source {
            unsafe { thaw_promise_release(source) };
        }
    }
}

pub type PromiseUnhandledFn = extern "C" fn(*const u8) -> u8;
pub type PromiseRejectionHandledFn = extern "C" fn();

/// Same C layout as thaw-quickjs::ThawHandleResult; keep runtime independent
/// of the QuickJS crate. `error` is an owned C string, null on success.
#[repr(C)]
pub struct PromiseReportResult {
    pub value: u64,
    pub error: *const c_char,
}
pub type PromiseUnhandledResultFn = extern "C" fn(*const u8) -> PromiseReportResult;
pub type PromiseRejectionHandledResultFn = extern "C" fn() -> PromiseReportResult;

fn report_listener_error(error: *const c_char) -> bool {
    if error.is_null() { return false; }
    unsafe {
        thaw_runtime_report_uncaught(error);
        thaw_arena::destroy_string(error.cast_mut());
    }
    UNHANDLED_FAILURE.with(|failed| failed.set(true));
    true
}

#[no_mangle]
pub extern "C" fn thaw_promise_set_unhandled_reporter_result(reporter: Option<PromiseUnhandledResultFn>) {
    UNHANDLED_REPORTER_RESULT.with(|registered| registered.set(reporter));
    UNHANDLED_REPORTER_TEXT_RESULT.with(|registered| registered.set(None));
    UNHANDLED_REPORTER.with(|registered| registered.set(None));
}

/// Compiler-private reporter: its callback receives a temporary owned native
/// diagnostic string, never the public Promise result pointer.
#[no_mangle]
pub extern "C" fn thaw_promise_set_unhandled_reporter_text_result(reporter: Option<PromiseUnhandledResultFn>) {
    UNHANDLED_REPORTER_TEXT_RESULT.with(|registered| registered.set(reporter));
    UNHANDLED_REPORTER_RESULT.with(|registered| registered.set(None));
    UNHANDLED_REPORTER.with(|registered| registered.set(None));
}

#[no_mangle]
pub extern "C" fn thaw_promise_set_rejection_handled_reporter_result(
    reporter: Option<PromiseRejectionHandledResultFn>,
) {
    REJECTION_HANDLED_REPORTER_RESULT.with(|registered| registered.set(reporter));
    REJECTION_HANDLED_REPORTER.with(|registered| registered.set(None));
}


#[no_mangle]
pub extern "C" fn thaw_promise_set_unhandled_reporter(reporter: Option<PromiseUnhandledFn>) {
    UNHANDLED_REPORTER.with(|registered| registered.set(reporter));
    UNHANDLED_REPORTER_RESULT.with(|registered| registered.set(None));
    UNHANDLED_REPORTER_TEXT_RESULT.with(|registered| registered.set(None));
}

#[no_mangle]
pub extern "C" fn thaw_promise_set_rejection_handled_reporter(
    reporter: Option<PromiseRejectionHandledFn>,
) {
    REJECTION_HANDLED_REPORTER.with(|registered| registered.set(reporter));
    REJECTION_HANDLED_REPORTER_RESULT.with(|registered| registered.set(None));
}

fn report_registered_unhandled_rejections() {
    let reporter = UNHANDLED_REPORTER.with(Cell::get);
    let result_reporter = UNHANDLED_REPORTER_RESULT.with(Cell::get);
    let text_reporter = UNHANDLED_REPORTER_TEXT_RESULT.with(Cell::get);
    let failed = if text_reporter.is_some() {
        thaw_promise_drain_unhandled_text_result(text_reporter)
    } else if result_reporter.is_some() {
        thaw_promise_drain_unhandled_result(result_reporter)
    } else {
        thaw_promise_drain_unhandled(reporter)
    };
    if failed != 0 {
        UNHANDLED_FAILURE.with(|failed| failed.set(true));
    }
    report_pending_rejection_handled();
}

fn report_pending_rejection_handled() {
    let count = PENDING_REJECTION_HANDLED.with(|pending| pending.replace(0));
    if let Some(reporter) = REJECTION_HANDLED_REPORTER_RESULT.with(Cell::get) {
        PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(activity.get().saturating_add(count)));
        for _ in 0..count {
            report_listener_error(reporter().error);
        }
    } else if let Some(reporter) = REJECTION_HANDLED_REPORTER.with(Cell::get) {
        PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(activity.get().saturating_add(count)));
        for _ in 0..count {
            reporter();
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_promise_take_unhandled_failure() -> u8 {
    UNHANDLED_FAILURE.with(|failed| u8::from(failed.replace(false)))
}

/// Notification progress is separate from the unhandled-failure latch.
#[no_mangle]
pub extern "C" fn thaw_promise_take_report_activity() -> usize {
    PROMISE_REPORT_ACTIVITY.with(|activity| activity.replace(0))
}

// Only rejection_text marks a pointer as a native C string. Public reject
// values are opaque, so an unhandled one needs a safe generic diagnostic.
fn unhandled_rejection_report_text(error: *const u8, trusted_text: bool) -> *const c_char {
    if trusted_text { error.cast() } else { c"Unhandled opaque Promise rejection".as_ptr() }
}

/// Reports every live rejected Promise which never gained a subscriber.
/// Returns 1 when at least one rejection had no host handler.
#[no_mangle]
pub extern "C" fn thaw_promise_drain_unhandled_result(
    reporter: Option<PromiseUnhandledResultFn>,
) -> u8 {
    let rejected = collect_unhandled_rejections();
    PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(activity.get().saturating_add(rejected.len())));
    let mut failed = false;
    for rejection in rejected {
        let text = rejection.text.as_ref().map(thaw_arena::owned_string);
        let error_ptr = text.as_ref().map_or(rejection.error, |text| text.cast_const().cast());
        let result = reporter.map(|reporter| reporter(error_ptr));
        let handled = result.as_ref().is_some_and(|result| result.value != 0 && result.error.is_null());
        if !handled {
            unsafe { thaw_runtime_report_uncaught(unhandled_rejection_report_text(error_ptr, text.is_some())) };
            failed = true;
        }
        if let Some(result) = result {
            failed |= report_listener_error(result.error);
        }
        if let Some(text) = text { unsafe { thaw_arena::destroy_string(text) }; }
    }
    u8::from(failed)
}

/// Compiler-private text adapter for QuickJS process events. This keeps the
/// public Result reporter's opaque-pointer ABI unchanged.
#[no_mangle]
pub extern "C" fn thaw_promise_drain_unhandled_text_result(
    reporter: Option<PromiseUnhandledResultFn>,
) -> u8 {
    let rejected = collect_unhandled_rejections();
    PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(activity.get().saturating_add(rejected.len())));
    let mut failed = false;
    for rejection in rejected {
        let text = thaw_arena::owned_string(&rejection.diagnostic);
        let result = reporter.map(|reporter| reporter(text.cast()));
        let handled = result.as_ref().is_some_and(|result| result.value != 0 && result.error.is_null());
        if !handled {
            unsafe { thaw_runtime_report_uncaught(text) };
            failed = true;
        }
        if let Some(result) = result {
            failed |= report_listener_error(result.error);
        }
        unsafe { thaw_arena::destroy_string(text) };
    }
    u8::from(failed)
}

#[no_mangle]
pub extern "C" fn thaw_promise_drain_unhandled(
    reporter: Option<PromiseUnhandledFn>,
) -> u8 {
    let rejected = collect_unhandled_rejections();
    PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(activity.get().saturating_add(rejected.len())));
    let mut failed = false;
    for rejection in rejected {
        let text = rejection.text.as_ref().map(thaw_arena::owned_string);
        let error_ptr = text.as_ref().map_or(rejection.error, |text| text.cast_const().cast());
        let handled = reporter.is_some_and(|reporter| reporter(error_ptr) != 0);
        if !handled {
            unsafe { thaw_runtime_report_uncaught(unhandled_rejection_report_text(error_ptr, text.is_some())) };
            failed = true;
        }
        if let Some(text) = text { unsafe { thaw_arena::destroy_string(text) }; }
    }
    u8::from(failed)
}

// Snapshot only the text explicitly supplied by trusted native producers.
// Public reject/typed reject errors remain opaque and are forwarded unchanged.
struct UnhandledRejection {
    error: *const u8,
    text: Option<Vec<u8>>,
    diagnostic: Vec<u8>,
}

fn collect_unhandled_rejections() -> Vec<UnhandledRejection> {
    ACTIVE_PROMISES.with(|active| {
        active
            .borrow()
            .iter()
            .filter_map(|promise| {
                let promise = unsafe { &mut **promise };
                (promise.rejected && !promise.handled && !promise.reported_unhandled).then(|| {
                    promise.reported_unhandled = true;
                    UnhandledRejection {
                        error: promise.result.unwrap_or(std::ptr::null()),
                        text: promise.rejection_text.clone(),
                        diagnostic: promise_report_bytes(promise),
                    }
                })
            })
            .collect::<Vec<_>>()
    })
}
