/// Allocates an unresolved promise. Pair every successful call with
/// `thaw_promise_destroy` after no coroutine can reference the handle.
#[no_mangle]
pub extern "C" fn thaw_promise_new() -> *mut ThawPromise {
    let promise = Box::into_raw(Box::new(ThawPromise {
        result: None,
        rejected: false,
        handled: false,
        reported_unhandled: false,
        subscribers: Vec::new(),
    }));
    ACTIVE_PROMISES.with(|active| active.borrow_mut().push(promise));
    promise
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
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    unsafe { thaw_promise_mark_handled(promise) };
    if let Some(result) = promise.result {
        enqueue_continuation(PromiseSubscription { resume, frame }, result);
    } else {
        promise
            .subscribers
            .push(PromiseSubscription { resume, frame });
    }
    1
}

/// Resolves a promise exactly once and queues every current subscriber in
/// registration order. Returns 0 for a null handle or repeated resolution.
#[no_mangle]
pub extern "C" fn thaw_promise_resolve(promise: *mut ThawPromise, result: *const u8) -> u8 {
    settle_promise(promise, result, false)
}

/// Rejects a promise exactly once and queues all subscribers. The error is an
/// opaque producer-owned pointer, using the same lifetime contract as a
/// fulfilled result. Returns 0 for a null handle or repeated settlement.
#[no_mangle]
pub extern "C" fn thaw_promise_reject(promise: *mut ThawPromise, error: *const u8) -> u8 {
    settle_promise(promise, error, true)
}

struct DetachedPromise {
    promise: *mut ThawPromise,
    pending_exception: *mut *const u8,
}

extern "C" fn destroy_detached_promise(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<DetachedPromise>()) };
    if unsafe { thaw_promise_state(state.promise) } == 2 && !state.pending_exception.is_null() {
        let pending = unsafe { &mut *state.pending_exception };
        if pending.is_null() {
            *pending = result;
        }
    }
    unsafe { thaw_promise_destroy(state.promise) };
}

/// Consumes a fire-and-forget Promise after it settles.
///
/// # Safety
/// `promise` must be null or a live, exclusively accessible `ThawPromise`.
/// `pending_exception` must remain writable until the promise settles.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_detach(
    promise: *mut ThawPromise,
    pending_exception: *mut *const u8,
) -> u8 {
    if promise.is_null() {
        return 0;
    }
    let state = Box::into_raw(Box::new(DetachedPromise {
        promise,
        pending_exception,
    }));
    let subscribed = unsafe {
        thaw_promise_subscribe(promise, destroy_detached_promise, state.cast::<u8>())
    };
    if subscribed == 0 {
        unsafe { drop(Box::from_raw(state)) };
    }
    subscribed
}

struct PromiseChainState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: bool,
}

extern "C" fn resume_promise_chain(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseChainState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    if rejected == state.on_rejected {
        (state.callback)(state.context, state.output, result);
    } else if rejected {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Creates the Promise returned by `.then` or `.catch`. The input handle is
/// consumed. The callback is invoked only for the selected settlement kind;
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
    let output = thaw_promise_new();
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseChainState {
        output,
        input,
        callback,
        context,
        on_rejected: on_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_chain, state.cast());
    output
}

struct PromiseAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
}

extern "C" fn resume_promise_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Makes `output` follow `input`, implementing Promise callback flattening.
/// The input handle is consumed.
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
        thaw_promise_reject(output, PROMISE_CYCLE_ERROR.as_ptr());
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseAdoptState { output, input }));
    thaw_promise_subscribe(input, resume_promise_adopt, state.cast());
    1
}

struct PromiseFinallyState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
}

extern "C" fn resume_promise_finally(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    (state.callback)(state.context, state.output, result, u8::from(rejected));
    unsafe { thaw_promise_destroy(state.input) };
}

/// Runs a `.finally` callback for either settlement kind. The callback owns
/// forwarding or replacing the original settlement. The input is consumed.
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
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyState {
        output,
        input,
        callback,
        context,
    }));
    thaw_promise_subscribe(input, resume_promise_finally, state.cast());
    output
}

struct PromiseFinallyAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: bool,
}

extern "C" fn resume_promise_finally_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else if state.original_rejected {
        thaw_promise_reject(state.output, state.original);
    } else {
        thaw_promise_resolve(state.output, state.original);
    }
    unsafe { thaw_promise_destroy(state.input) };
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
) -> u8 {
    if output.is_null() || input.is_null() || output == input {
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyAdoptState {
        output,
        input,
        original,
        original_rejected: original_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_finally_adopt, state.cast());
    1
}

struct PromiseAllState {
    output: *mut ThawPromise,
    remaining: usize,
    rejected: bool,
    first_error: *const u8,
    result: *mut u8,
    result_slot: *mut *const u8,
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
            state.first_error = result;
            thaw_promise_reject(state.output, result);
        }
    } else if !state.rejected {
        for index in &child.indices {
            let destination = unsafe { state.result.add(state.element_offsets[*index]) };
            unsafe {
                destination.write_bytes(0, state.element_sizes[*index].max(size_of::<u64>()));
                std::ptr::copy_nonoverlapping(result, destination, state.element_sizes[*index]);
            }
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.rejected {
            thaw_promise_resolve(state.output, state.result_slot.cast());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Joins homogeneous Promise handles without serializing them.
/// The result uses Thaw's `[i64 len][8-byte slots...]` array layout in the
/// request arena and therefore remains valid after the returned promise is
/// destroyed. Child handles are consumed by this call.
///
/// # Safety
///
/// `promises` must reference `len` live handles returned by Thaw async
/// functions. Each handle must be unique and must not be used after this call.
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
/// Promise handles are consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_typed(
    promises: *const *mut ThawPromise,
    element_sizes: *const usize,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len != 0 && element_sizes.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let element_sizes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(element_sizes, len) }.to_vec()
    };
    if element_sizes.contains(&0) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut element_offsets = Vec::with_capacity(len);
    let mut result_bytes = size_of::<u64>();
    for size in &element_sizes {
        element_offsets.push(result_bytes);
        let Some(next) = result_bytes.checked_add((*size).max(size_of::<u64>())) else {
            thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        };
        result_bytes = next;
    }
    let Some(allocation_bytes) = result_bytes.checked_add(size_of::<*const u8>()) else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let allocation = thaw_arena::thaw_arena_alloc(allocation_bytes, align_of::<u64>());
    if allocation.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(size_of::<*const u8>()) };
    // The Promise's own resolved value has Array/Tuple type, so it must be a
    // handle - not the raw buffer - by the time codegen's generic await
    // extraction loads it back out of `result_slot`. See `wrap_array_handle`.
    unsafe { result_slot.write(wrap_array_handle(result).cast()) };
    unsafe { result.cast::<u64>().write(len as u64) };
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
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
    let state = Box::into_raw(Box::new(PromiseAllState {
        output,
        remaining: grouped.len(),
        rejected: invalid,
        first_error: if invalid {
            PROMISE_ALL_INVALID_ERROR.as_ptr()
        } else {
            std::ptr::null()
        },
        result,
        result_slot,
        element_sizes,
        element_offsets,
    }));
    if !grouped.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllChild {
            state,
            promise,
            indices,
        }));
        unsafe {
            thaw_promise_subscribe(promise, resume_promise_all_child, child.cast());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        let error = unsafe { (*state).first_error };
        thaw_promise_reject(output, error);
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
    remaining: usize,
    settled: bool,
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
            thaw_promise_reject(state.output, result);
        } else {
            thaw_promise_resolve(state.output, result);
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Settles with the first input Promise to fulfill or reject. Input handles
/// are deduplicated and consumed; slower children continue to be drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle must be live and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_race(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 {
        thaw_promise_reject(output, PROMISE_RACE_EMPTY_ERROR.as_ptr());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
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
    let state = Box::into_raw(Box::new(PromiseRaceState {
        output,
        remaining: unique.len(),
        settled: invalid,
    }));
    if !unique.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
    }
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseRaceChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_race_child, child.cast()) };
    }
    if unsafe { (*state).remaining } == 0 {
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

struct PromiseAnyState {
    output: *mut ThawPromise,
    remaining: usize,
    fulfilled: bool,
}

struct PromiseAnyChild {
    state: *mut PromiseAnyState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_any_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.fulfilled && unsafe { thaw_promise_state(child.promise) } == 1 {
        state.fulfilled = true;
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.fulfilled {
            thaw_promise_reject(state.output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Resolves with the first fulfilled input Promise. Rejections are ignored
/// until every distinct input has rejected, at which point an aggregate error
/// message is used to reject the output. Slower children are still drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// non-null handle is consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 || promises.is_null() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if !promise.is_null() && !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    if unique.is_empty() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseAnyState {
        output,
        remaining: unique.len(),
        fulfilled: false,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseAnyChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_any_child, child.cast()) };
    }
    output
}

struct PromiseAllSettledState {
    output: *mut ThawPromise,
    remaining: usize,
    result_slot: *mut *const u8,
    result: *mut u64,
    objects: *mut u8,
    element_size: usize,
    object_stride: usize,
}

struct PromiseAllSettledChild {
    state: *mut PromiseAllSettledState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_settled_child(frame: *mut u8, value: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllSettledChild>()) };
    let state = unsafe { &mut *child.state };
    let rejected = unsafe { thaw_promise_state(child.promise) } == 2;
    for index in &child.indices {
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
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        thaw_promise_resolve(state.output, state.result_slot.cast());
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
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
/// handle is consumed. Values wider than one array slot are copied in full.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_settled(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if element_size == 0 || (len != 0 && promises.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let allocation =
        thaw_arena::thaw_arena_alloc((len + 2) * size_of::<u64>(), align_of::<u64>()).cast::<u64>();
    let Some(object_stride) = size_of::<u64>()
        .checked_add(element_size.max(size_of::<u64>()))
        .and_then(|size| size.checked_add(size_of::<u64>()))
    else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let Some(objects_size) = len.checked_mul(object_stride) else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let objects = thaw_arena::thaw_arena_alloc(objects_size, align_of::<u64>());
    if allocation.is_null() || (len != 0 && objects.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(1) };
    unsafe {
        // The Promise's own resolved value has Array type, so it must be a
        // handle - not the raw buffer - by the time codegen's generic await
        // extraction loads it back out of `result_slot`. See
        // `wrap_array_handle` (defined alongside `thaw_regex_match_all`,
        // which needs the identical wrap for its own nested arrays).
        result_slot.write(wrap_array_handle(result.cast()).cast());
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
            thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
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
    let state = Box::into_raw(Box::new(PromiseAllSettledState {
        output,
        remaining: grouped.len(),
        result_slot,
        result,
        objects,
        element_size,
        object_stride,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllSettledChild {
            state,
            promise,
            indices,
        }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_all_settled_child, child.cast()) };
    }
    output
}

fn settle_promise(promise: *mut ThawPromise, result: *const u8, rejected: bool) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if promise.result.is_some() {
        return 0;
    }
    promise.result = Some(result);
    promise.rejected = rejected;
    let subscribers = std::mem::take(&mut promise.subscribers);
    for subscriber in subscribers {
        enqueue_continuation(subscriber, result);
    }
    1
}

/// Destroys a promise handle. Passing null is a no-op. The caller must not
/// destroy a promise from inside one of its resume callbacks.
///
/// # Safety
///
/// `promise` must be null or a pointer returned by `thaw_promise_new` that has
/// not already been destroyed and is no longer referenced by any subscriber.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_destroy(promise: *mut ThawPromise) {
    if !promise.is_null() {
        ACTIVE_PROMISES.with(|active| active.borrow_mut().retain(|current| *current != promise));
        TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.promise != promise));
        FD_WAITS.with(|waits| waits.borrow_mut().retain(|wait| wait.promise != promise));
        drop(Box::from_raw(promise));
    }
}

pub type PromiseUnhandledFn = extern "C" fn(*const u8) -> u8;
pub type PromiseRejectionHandledFn = extern "C" fn();

#[no_mangle]
pub extern "C" fn thaw_promise_set_unhandled_reporter(reporter: Option<PromiseUnhandledFn>) {
    UNHANDLED_REPORTER.with(|registered| registered.set(reporter));
}

#[no_mangle]
pub extern "C" fn thaw_promise_set_rejection_handled_reporter(
    reporter: Option<PromiseRejectionHandledFn>,
) {
    REJECTION_HANDLED_REPORTER.with(|registered| registered.set(reporter));
}

fn report_registered_unhandled_rejections() {
    let reporter = UNHANDLED_REPORTER.with(Cell::get);
    if thaw_promise_drain_unhandled(reporter) != 0 {
        UNHANDLED_FAILURE.with(|failed| failed.set(true));
    }
    report_pending_rejection_handled();
}

fn report_pending_rejection_handled() {
    let count = PENDING_REJECTION_HANDLED.with(|pending| pending.replace(0));
    if let Some(reporter) = REJECTION_HANDLED_REPORTER.with(Cell::get) {
        for _ in 0..count {
            reporter();
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_promise_take_unhandled_failure() -> u8 {
    UNHANDLED_FAILURE.with(|failed| u8::from(failed.replace(false)))
}

/// Reports every live rejected Promise which never gained a subscriber.
/// Returns 1 when at least one rejection had no host handler.
#[no_mangle]
pub extern "C" fn thaw_promise_drain_unhandled(
    reporter: Option<PromiseUnhandledFn>,
) -> u8 {
    let rejected = ACTIVE_PROMISES.with(|active| {
        active
            .borrow()
            .iter()
            .filter_map(|promise| {
                let promise = unsafe { &mut **promise };
                (promise.rejected && !promise.handled && !promise.reported_unhandled).then(|| {
                    promise.reported_unhandled = true;
                    promise.result.unwrap_or(std::ptr::null())
                })
            })
            .collect::<Vec<_>>()
    });
    let mut failed = false;
    for error in rejected {
        let handled = reporter.is_some_and(|reporter| reporter(error) != 0);
        if !handled {
            unsafe { thaw_runtime_report_uncaught(error.cast()) };
            failed = true;
        }
    }
    u8::from(failed)
}
