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
    pub native_layout: *const std::ffi::c_char,
}
const _: [(); 72] = [(); std::mem::size_of::<ExceptionProvenance>()];

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
        bool_value, object, native_layout: std::ptr::null(),
    }) };
    allocation
}

thread_local! {
    static NATIVE_PROMISE_REASON_OWNERS: RefCell<HashMap<usize, *mut ThawPromise>> =
        RefCell::new(HashMap::new());
    static PROMISE_ARENA_SLOT_OWNERS: RefCell<HashMap<usize, (usize, *mut ThawPromise)>> =
        RefCell::new(HashMap::new());
}

fn release_reclaimed_native_promise_internal_owners(tracing: bool) {
    let released = NATIVE_PROMISE_REASON_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        let released = owners.iter()
            .filter_map(|(&record, &promise)|
                (!tracing || thaw_arena::was_reclaimed(record)).then_some((record, promise)))
            .collect::<Vec<_>>();
        for &(record, _) in &released {
            owners.remove(&record);
        }
        released
    });
    let released_slots = PROMISE_ARENA_SLOT_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        let released = owners.iter()
            .filter_map(|(&slot, &(_, promise))|
                (!tracing || thaw_arena::was_reclaimed(slot)).then_some((slot, promise)))
            .collect::<Vec<_>>();
        for &(slot, _) in &released { owners.remove(&slot); }
        released
    });
    for (_, promise) in released.into_iter().chain(released_slots) {
        unsafe { release_native_promise_internal(promise) };
    }
}

/// Build a private native exception descriptor. `owner` is the original
/// native value pointer; `tag` preserves its actual JS typeof category and
/// `layout` is a compiler-generated, static type token used for checked casts.
#[no_mangle]
pub unsafe extern "C" fn thaw_exception_native_provenance_new(
    tag: u64,
    owner: *const u8,
    layout: *const std::ffi::c_char,
) -> *const ExceptionProvenance {
    if owner.is_null() || layout.is_null() { return std::ptr::null(); }
    let allocation = thaw_arena::thaw_arena_alloc(
        std::mem::size_of::<ExceptionProvenance>(),
        std::mem::align_of::<ExceptionProvenance>(),
    ).cast::<ExceptionProvenance>();
    if allocation.is_null() { return std::ptr::null(); }
    unsafe { allocation.write(ExceptionProvenance {
        original: std::ptr::null(), native_text: std::ptr::null(),
        aggregate_errors: std::ptr::null(), tag, f64_value: 0.0,
        i64_value: 0, bool_value: 0, object: owner, native_layout: layout,
    }) };
    if tag == 29 {
        let promise = owner.cast_mut().cast::<ThawPromise>();
        if unsafe { retain_native_promise_internal(promise) } == 0 {
            return std::ptr::null();
        }
        thaw_arena::register_reset_hook(release_reclaimed_native_promise_internal_owners);
        NATIVE_PROMISE_REASON_OWNERS.with(|owners| {
            owners.borrow_mut().insert(allocation as usize, promise);
        });
    }
    allocation
}

/// Return the original owner only when both native category and exact HIR
/// token match. The returned pointer is never interpreted as an Object first.
#[no_mangle]
pub unsafe extern "C" fn thaw_exception_native_owner(
    provenance: *const ExceptionProvenance,
    tag: u64,
    layout: *const std::ffi::c_char,
) -> *const u8 {
    let Some(provenance) = (unsafe { provenance.as_ref() }) else { return std::ptr::null(); };
    if layout.is_null() || provenance.native_layout.is_null() || provenance.tag != tag {
        return std::ptr::null();
    }
    let expected = unsafe { std::ffi::CStr::from_ptr(layout) }.to_bytes();
    let actual = unsafe { std::ffi::CStr::from_ptr(provenance.native_layout) }.to_bytes();
    if expected == actual { provenance.object } else { std::ptr::null() }
}

/// Native strict equality is identity equality on the original owner, not on
/// descriptor allocations which may be copied at catch/rethrow boundaries.
#[no_mangle]
pub unsafe extern "C" fn thaw_exception_native_same(
    left: *const ExceptionProvenance,
    right: *const ExceptionProvenance,
) -> u8 {
    match (unsafe { left.as_ref() }, unsafe { right.as_ref() }) {
        (Some(left), Some(right)) => u8::from(left.tag == right.tag && left.object == right.object),
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_exception_native_tag(
    provenance: *const ExceptionProvenance,
) -> u64 {
    unsafe { provenance.as_ref() }.map_or(0, |value| value.tag)
}

/// Allocates an unresolved promise. Pair every successful call with
/// `thaw_promise_destroy` after no coroutine can reference the handle.
#[no_mangle]
pub extern "C" fn thaw_promise_new() -> *mut ThawPromise {
    let promise = Box::into_raw(Box::new(ThawPromise {
        result: None,
        fulfilled_provenance: std::ptr::null(),
        rejection_text: None,
        rejected: false,
        exception_tag: 0,
        exception_f64: 0.0,
        exception_i64: 0,
        exception_bool: false,
        exception_object: std::ptr::null(),
        exception_native: std::ptr::null(),
        aggregate_errors: std::ptr::null(),
        handled: false,
        reported_unhandled: false,
        subscribers: Vec::new(),
        internal_references: 0,
        references: 1,
        external_root: None,
    }));
    if thaw_arena::is_tracing() {
        unsafe { (*promise).external_root = Some(thaw_arena::ArenaRoot::new(promise as usize)); }
    }
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
        enqueue_continuation(
            PromiseSubscription {
                resume,
                frame,
                _frame_root: thaw_arena::ArenaRoot::new(frame as usize),
            },
            result,
        );
    } else {
        promise
            .subscribers
            .push(PromiseSubscription {
                resume,
                frame,
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
/// `error` must be null or a live NUL-terminated native string.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_native_text(promise: *mut ThawPromise, error: *const u8) -> u8 {
    reject_native_text(promise, error)
}

fn reject_native_text(promise: *mut ThawPromise, error: *const u8) -> u8 {
    let text = (!error.is_null())
        .then(|| unsafe { CStr::from_ptr(error.cast()).to_bytes().to_vec() });
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
        unsafe { (*promise).rejection_text = Some(CStr::from_ptr(error.cast()).to_bytes().to_vec()); }
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

/// Typed rejection handoff that preserves the immutable native value
/// descriptor alongside the legacy scalar/object channels.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_reject_typed_with_native_provenance(
    promise: *mut ThawPromise,
    error: *const u8,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: bool,
    object: *const u8,
    native_text: *const u8,
    aggregate_errors: *const u8,
    native: *const ExceptionProvenance,
) -> u8 {
    let settled = unsafe { thaw_promise_reject_typed_with_aggregate(
        promise, error, tag, f64_value, i64_value, bool_value, object,
        native_text, aggregate_errors,
    ) };
    if settled != 0 { unsafe { thaw_promise_set_exception_native(promise, native) }; }
    settled
}

/// Tracks the Promise's aggregate handle beside its ordinary result edge.
/// `replace_reference` takes the previous child pointer, not a slot number;
/// retaining both children under the same boxed Promise owner lets a rooted
/// Promise keep the errors array alive across invocation-arena resets.
fn set_promise_aggregate_errors(promise: *mut ThawPromise, errors: *const u8) {
    let Some(promise_ref) = (unsafe { promise.as_mut() }) else { return; };
    let previous = promise_ref.aggregate_errors;
    thaw_arena::replace_reference(promise as usize, previous as usize, errors as usize);
    promise_ref.aggregate_errors = errors;
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
    thaw_promise_exception_native,
    exception_native,
    *const ExceptionProvenance,
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
    let text = input.rejection_text.clone();
    let native = input.exception_native;
    let settled = unsafe { thaw_promise_reject_typed(
        output,
        error,
        input.exception_tag,
        input.exception_f64,
        input.exception_i64,
        input.exception_bool,
        input.exception_object,
    ) };
    if settled != 0 {
        unsafe {
            (*output).rejection_text = text;
            set_promise_aggregate_errors(output, input.aggregate_errors);
            let _ = thaw_promise_set_exception_native(output, native);
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
    fulfilled: Option<(PromiseTransformFn, *mut u8)>,
    rejected: Option<(PromiseTransformFn, *mut u8)>,
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
/// `input` is consumed once. Both contexts remain valid through settlement.
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
    if input.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseChainState {
        output,
        input,
        fulfilled,
        rejected,
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
        forward_promise_rejection(state.output, state.input, result);
    } else {
        forward_promise_fulfillment(state.output, state.input, result);
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
        reject_native_text(output, PROMISE_CYCLE_ERROR.as_ptr());
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
    (state.callback)(
        state.context,
        state.output,
        state.input,
        result,
        u8::from(rejected),
    );
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
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
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
    for pointer in finally_states {
        let state = unsafe { &mut *pointer };
        state.cancelled = true;
        state.original_roots.clear();
    }
    let any_states = PROMISE_ANY_STATES.with(|states| {
        std::mem::take(&mut *states.borrow_mut())
    });
    for pointer in any_states {
        let state = unsafe { &mut *pointer };
        state.cancelled = true;
        state.reason_roots.clear();
        state.output_root = None;
    }
}

struct PromiseFinallyAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: bool,
    original_tag: u64,
    original_f64: f64,
    original_i64: i64,
    original_bool: bool,
    original_object: *const u8,
    original_native: *const ExceptionProvenance,
    original_aggregate_errors: *const u8,
    original_provenance: *const ExceptionProvenance,
    original_text: Option<Vec<u8>>,
    original_roots: Vec<(usize, thaw_arena::ArenaRoot)>,
    cancelled: bool,
}

extern "C" fn resume_promise_finally_adopt(frame: *mut u8, result: *const u8) {
    PROMISE_FINALLY_ADOPT_STATES.with(|states| {
        states.borrow_mut().retain(|pointer| *pointer != frame.cast());
    });
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyAdoptState>()) };
    if state.cancelled {
        unsafe { thaw_promise_destroy(state.input) };
        return;
    }
    if unsafe { thaw_promise_state(state.input) } == 2 {
        forward_promise_rejection(state.output, state.input, result);
    } else if state.original_rejected {
        let settled = unsafe { thaw_promise_reject_typed(
            state.output,
            state.original,
            state.original_tag,
            state.original_f64,
            state.original_i64,
            state.original_bool,
            state.original_object,
        ) };
        if settled != 0 {
            unsafe {
                (*state.output).rejection_text = state.original_text.clone();
                set_promise_aggregate_errors(state.output, state.original_aggregate_errors);
                let _ = thaw_promise_set_exception_native(state.output, state.original_native);
            }
        }
    } else {
        unsafe { thaw_promise_resolve_with_provenance(
            state.output, state.original, state.original_provenance,
        ) };
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
    let aggregate_errors = unsafe { source.as_ref() }
        .map_or(std::ptr::null(), |source| source.aggregate_errors);
    let original_native = unsafe { source.as_ref() }
        .map_or(std::ptr::null(), |source| source.exception_native);
    let original_provenance = if original_rejected == 0 {
        unsafe { thaw_promise_fulfilled_provenance(source) }
    } else { std::ptr::null() };
    let mut original_roots = Vec::new();
    for pointer in [original, original_object, aggregate_errors, original_provenance.cast(), original_native.cast()] {
        pin_promise_pointer_once(&mut original_roots, pointer as usize);
    }
    let state = Box::into_raw(Box::new(PromiseFinallyAdoptState {
        output,
        input,
        original,
        original_rejected: original_rejected != 0,
        original_tag,
        original_f64,
        original_i64,
        original_bool,
        original_object,
        original_native,
        original_aggregate_errors: aggregate_errors,
        original_provenance,
        original_text: unsafe { source.as_ref() }.and_then(|source| source.rejection_text.clone()),
        original_roots,
        cancelled: false,
    }));
    PROMISE_FINALLY_ADOPT_STATES.with(|states| states.borrow_mut().push(state));
    thaw_promise_subscribe(input, resume_promise_finally_adopt, state.cast());
    1
}

thread_local! {
    static PROMISE_ALL_STATES: RefCell<Vec<*mut PromiseAllState>> = const { RefCell::new(Vec::new()) };
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
}

struct PromiseAllState {
    output: *mut ThawPromise,
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
    unsafe { thaw_promise_destroy(child.promise) };
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
    let state = Box::into_raw(Box::new(PromiseAllState {
        output,
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
        unsafe {
            thaw_promise_subscribe(promise, resume_promise_all_child, child.cast());
        }
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
            forward_promise_rejection(state.output, child.promise, result);
        } else {
            forward_promise_fulfillment(state.output, child.promise, result);
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
    let state = Box::into_raw(Box::new(PromiseRaceState {
        output,
        remaining: unique.len(),
        settled: invalid,
    }));
    if !unique.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        reject_native_text(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
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

// The private 16-byte reason slot uses the same {i8 tag, i64 payload}
// storage as a compiler Union. Tags: 0 number, 1 bigint, 2 boolean,
// 3 string, 4 undefined, 5 null, 6 native object pointer, 7 nested
// AggregateError record, 8 opaque pointer, 9 native error-text record.
// This precursor exposes it only through the Promise-owned aggregate handle;
// HIR consumers are a separate unit.
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
}

// Snapshot while the child Promise is still live. Native-owned text is copied
// into the invocation arena; arbitrary opaque rejection pointers are stored
// as opaque bits and are never dereferenced here.
fn promise_any_reason(promise: &ThawPromise, result: *const u8) -> Option<PromiseAnyReason> {
    if !promise.aggregate_errors.is_null()
        || (promise.exception_tag == 0 && promise.exception_object.is_null()
            && promise.rejection_text.is_some())
    {
        let text = promise.rejection_text.as_deref()?;
        let text = thaw_arena::arena_string(text).cast::<u8>();
        if text.is_null() { return None; }
        let record = thaw_arena::thaw_arena_alloc(
            size_of::<PromiseAnyErrorRecord>(), align_of::<PromiseAnyErrorRecord>(),
        ).cast::<PromiseAnyErrorRecord>();
        if record.is_null() { return None; }
        unsafe { record.write(PromiseAnyErrorRecord {
            text, errors: promise.aggregate_errors, object: promise.exception_object,
        }); }
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
            slot.write(reason.tag);
            slot.add(8).cast::<u64>().write_unaligned(reason.payload);
        }
    }
    wrap_array_handle(buffer).cast()
}

struct PromiseAnyState {
    output: *mut ThawPromise,
    remaining: usize,
    fulfilled: bool,
    reasons: Vec<Option<PromiseAnyReason>>,
    reason_roots: Vec<(usize, thaw_arena::ArenaRoot)>,
    output_root: Option<thaw_arena::ArenaRoot>,
    cancelled: bool,
}

struct PromiseAnyChild {
    state: *mut PromiseAnyState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_any_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let state = unsafe { &mut *child.state };
    if state.cancelled {
        // Purge released request-arena pins and reset the join count. A
        // still-live child can settle later, but its result is now opaque.
        unsafe { thaw_promise_destroy(child.promise) };
        state.remaining -= 1;
        if state.remaining == 0 {
            unsafe { drop(Box::from_raw(child.state)) };
        }
        return;
    }
    if !state.fulfilled {
        if unsafe { thaw_promise_state(child.promise) } == 1 {
            state.fulfilled = true;
            state.reason_roots.clear();
            forward_promise_fulfillment(state.output, child.promise, result);
        } else {
            let reason = promise_any_reason(unsafe { &*child.promise }, result);
            if let Some(reason) = reason {
                if matches!(reason.tag, 3 | 6 | 7 | 8 | 9) {
                    pin_promise_pointer_once(&mut state.reason_roots, reason.payload as usize);
                }
            }
            for index in &child.indices { state.reasons[*index] = reason; }
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        PROMISE_ANY_STATES.with(|states| {
            states.borrow_mut().retain(|pointer| *pointer != child.state);
        });
        if !state.fulfilled {
            let errors = promise_any_errors(&state.reasons);
            if reject_native_text(state.output, PROMISE_ANY_REJECTED_ERROR.as_ptr()) != 0 {
                set_promise_aggregate_errors(state.output, errors);
            }
        }
        state.reason_roots.clear();
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
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
/// non-null handle is consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 {
        let errors = promise_any_errors(&[]);
        if reject_native_text(output, PROMISE_ANY_REJECTED_ERROR.as_ptr()) != 0 {
            set_promise_aggregate_errors(output, errors);
        }
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
    let state = Box::into_raw(Box::new(PromiseAnyState {
        output,
        remaining: grouped.len(),
        fulfilled: false,
        reasons: vec![None; len],
        reason_roots: Vec::new(),
        output_root: Some(thaw_arena::ArenaRoot::new(output as usize)),
        cancelled: false,
    }));
    PROMISE_ANY_STATES.with(|states| states.borrow_mut().push(state));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAnyChild { state, promise, indices }));
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

// Snapshot before destruction: retiring an input may reenter and reclaim
// the arena array containing these handles.
unsafe fn consume_distinct_promise_inputs(promises: *const *mut ThawPromise, len: usize) {
    if promises.is_null() { return; }
    let mut distinct = Vec::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        // ponytail: preserve first-input retirement order with quadratic
        // deduplication; use an auxiliary HashSet if large joins need it.
        if !promise.is_null() && !distinct.contains(&promise) { distinct.push(promise); }
    }
    for promise in distinct { unsafe { thaw_promise_destroy(promise) }; }
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
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
        return output;
    }
    let Some(allocation_size) = len.checked_add(2)
        .and_then(|words| words.checked_mul(size_of::<u64>())) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
        return output;
    };
    let allocation = thaw_arena::thaw_arena_alloc(allocation_size, align_of::<u64>()).cast::<u64>();
    let Some(object_stride) = size_of::<u64>()
        .checked_add(element_size.max(size_of::<u64>()))
        .and_then(|size| size.checked_add(size_of::<u64>()))
    else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
        return output;
    };
    let Some(objects_size) = len.checked_mul(object_stride) else {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
        return output;
    };
    let objects = thaw_arena::thaw_arena_alloc(objects_size, align_of::<u64>());
    if allocation.is_null() || (len != 0 && objects.is_null()) {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
        return output;
    }
    let result_handle = wrap_array_handle(unsafe { allocation.add(1) }.cast());
    if result_handle.is_null() {
        reject_native_text(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        unsafe { consume_distinct_promise_inputs(promises, len) };
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
            unsafe { consume_distinct_promise_inputs(promises, len) };
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

fn destroy_promise_box(promise: *mut ThawPromise) {
    ACTIVE_PROMISES.with(|active| active.borrow_mut().retain(|current| *current != promise));
    TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.promise != promise));
    FD_WAITS.with(|waits| waits.borrow_mut().retain(|wait| wait.promise != promise));
    thaw_arena::forget_references(promise as usize);
    unsafe { drop(Box::from_raw(promise)); }
}

unsafe fn retain_native_promise_internal(promise: *mut ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else { return 0; };
    let (Some(references), Some(internal_references)) = (
        promise.references.checked_add(1),
        promise.internal_references.checked_add(1),
    ) else { return 0; };
    promise.references = references;
    promise.internal_references = internal_references;
    1
}

unsafe fn release_native_promise_internal(promise: *mut ThawPromise) {
    let Some(promise_ref) = (unsafe { promise.as_mut() }) else { return; };
    if promise_ref.internal_references == 0 || promise_ref.references == 0 { return; }
    promise_ref.internal_references -= 1;
    promise_ref.references -= 1;
    if promise_ref.references == 0 {
        destroy_promise_box(promise);
    }
}

/// Destroys an externally owned Promise handle. Passing null is a no-op.
///
/// # Safety
///
/// `promise` must be null or a pointer returned by `thaw_promise_new` that has
/// not already been destroyed and is no longer referenced by any subscriber.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_destroy(promise: *mut ThawPromise) {
    if !promise.is_null() {
        let Some(promise_ref) = (unsafe { promise.as_mut() }) else { return; };
        if promise_ref.references <= promise_ref.internal_references { return; }
        promise_ref.references -= 1;
        if promise_ref.references == promise_ref.internal_references {
            promise_ref.external_root.take();
        }
        if promise_ref.references == 0 { destroy_promise_box(promise); }
    }
}

/// Retain a Promise as an externally owned alias. This changes only ownership;
/// it does not subscribe, inspect `then`, or mark it handled.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_retain(promise: *mut ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else { return 0; };
    let Some(references) = promise.references.checked_add(1) else { return 0; };
    if promise.references == promise.internal_references && thaw_arena::is_tracing() {
        promise.external_root = Some(thaw_arena::ArenaRoot::new(promise as *mut ThawPromise as usize));
    }
    promise.references = references;
    1
}

/// Replace a Promise pointer stored in an arena-owned cell or frame slot.
/// The owner allocation keeps the Promise as an internal tracing edge; the
/// reset hook releases one internal token when that slot is reclaimed.
///
/// # Safety
/// `owner` and `slot` must point into the same live arena allocation, and
/// `slot` must already contain an initialized null or live Promise pointer.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_arena_slot_replace(
    owner: *mut u8,
    slot: *mut *mut ThawPromise,
    replacement: *mut ThawPromise,
) -> u8 {
    if owner.is_null() || slot.is_null() || (thaw_arena::is_tracing()
        && (!thaw_arena::contains_allocation(owner as usize)
            || !thaw_arena::contains_allocation(slot as usize)))
    {
        return 0;
    }
    let previous = unsafe { slot.read() };
    let key = slot as usize;
    let current = PROMISE_ARENA_SLOT_OWNERS.with(|owners| owners.borrow().get(&key).copied());
    if current.is_some_and(|(current_owner, current_promise)|
        current_owner != owner as usize || current_promise != previous)
        || (previous.is_null() && current.is_some())
        || (!previous.is_null() && current.is_none())
    {
        return 0;
    }
    if previous == replacement { return 1; }
    if !replacement.is_null() && unsafe { retain_native_promise_internal(replacement) } == 0 {
        return 0;
    }
    thaw_arena::replace_reference(owner as usize, previous as usize, replacement as usize);
    PROMISE_ARENA_SLOT_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        owners.remove(&key);
        if !replacement.is_null() {
            owners.insert(key, (owner as usize, replacement));
        }
    });
    unsafe { slot.write(replacement) };
    if !previous.is_null() { unsafe { release_native_promise_internal(previous) }; }
    thaw_arena::register_reset_hook(release_reclaimed_native_promise_internal_owners);
    1
}

/// Attach a native exception descriptor to a rejected Promise and keep its
/// arena record reachable for the Promise's lifetime.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_set_exception_native(
    promise: *mut ThawPromise,
    provenance: *const ExceptionProvenance,
) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else { return 0; };
    let previous = promise.exception_native;
    thaw_arena::replace_reference(
        promise as *mut ThawPromise as usize,
        previous as usize,
        provenance as usize,
    );
    promise.exception_native = provenance;
    1
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
