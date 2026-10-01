thread_local! {
    static OBJECT_STATES: RefCell<HashMap<usize, u8>> = RefCell::new(HashMap::new());
    // ponytail: side-table keeps the native object ABI stable; use object headers if profiling
    // shows the pointer lookup matters.
    static OBJECT_ACCESSORS: RefCell<HashMap<usize, HashMap<String, [usize; 2]>>> =
        RefCell::new(HashMap::new());
}

const NON_EXTENSIBLE: u8 = 1;
const SEALED: u8 = 2;
const FROZEN: u8 = 4;

/// Records `preventExtensions` (1), `seal` (2), or `freeze` (3) for one
/// native object identity. Native layouts are fixed already; this table only
/// carries the observable integrity state across aliases and control flow.
#[no_mangle]
pub extern "C" fn thaw_object_set_state(object: *const u8, operation: u8) -> bool {
    if object.is_null() {
        return false;
    }
    let flags = match operation {
        1 => NON_EXTENSIBLE,
        2 => NON_EXTENSIBLE | SEALED,
        3 => NON_EXTENSIBLE | SEALED | FROZEN,
        _ => return false,
    };
    OBJECT_STATES.with(|states| {
        *states.borrow_mut().entry(object as usize).or_default() |= flags;
    });
    true
}

/// Propagates `source`'s own recorded integrity state (if any) onto
/// `target` too -- used when a statically `Object`-typed value crosses
/// into a `Json`-encoded representation (e.g. `Object.freeze({...})`
/// assigned into an `any`-typed binding). `Object`'s native layout and
/// `Json`'s boxed `serde_json::Value` are different allocations with
/// different addresses, so a frozen/sealed object's own state -- keyed
/// purely by pointer identity -- would otherwise silently vanish the
/// moment its JSON encoding (a genuinely new pointer) is built. A no-op
/// (returns `false`) when `source` was never frozen/sealed/prevented, so
/// this is safe to call unconditionally after every native-object-to-JSON
/// encoding.
#[no_mangle]
pub extern "C" fn thaw_object_copy_state(source: *const u8, target: *const u8) -> bool {
    if source.is_null() || target.is_null() {
        return false;
    }
    let Some(flags) = OBJECT_STATES.with(|states| states.borrow().get(&(source as usize)).copied())
    else {
        return false;
    };
    OBJECT_STATES.with(|states| {
        *states.borrow_mut().entry(target as usize).or_default() |= flags;
    });
    true
}

/// Queries extensible (0), sealed (1), or frozen (2) state.
#[no_mangle]
pub extern "C" fn thaw_object_state(object: *const u8, query: u8) -> bool {
    let flags = OBJECT_STATES.with(|states| {
        states
            .borrow()
            .get(&(object as usize))
            .copied()
            .unwrap_or_default()
    });
    match query {
        0 => flags & NON_EXTENSIBLE == 0,
        1 => flags & SEALED != 0,
        2 => flags & FROZEN != 0,
        _ => false,
    }
}

#[no_mangle]
/// # Safety
/// `property` must point to a valid NUL-terminated string for the duration of
/// this call. `object` and `closure` are opaque identities and are not read.
pub unsafe extern "C" fn thaw_object_set_accessor(
    object: *const u8,
    property: *const c_char,
    closure: *mut u8,
    setter: bool,
) -> bool {
    if object.is_null() || property.is_null() || closure.is_null() {
        return false;
    }
    let Ok(property) = CStr::from_ptr(property).to_str() else {
        return false;
    };
    OBJECT_ACCESSORS.with(|accessors| {
        let mut accessors = accessors.borrow_mut();
        let slot = &mut accessors
            .entry(object as usize)
            .or_default()
            .entry(property.to_owned())
            .or_default()[setter as usize];
        let previous = std::mem::replace(slot, closure as usize);
        thaw_arena::replace_reference(object as usize, previous, closure as usize);
    });
    true
}

#[no_mangle]
/// # Safety
/// `property` must point to a valid NUL-terminated string for the duration of
/// this call. `object` is used only as an opaque identity.
pub unsafe extern "C" fn thaw_object_accessor(
    object: *const u8,
    property: *const c_char,
    setter: bool,
) -> *mut u8 {
    if object.is_null() || property.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(property) = CStr::from_ptr(property).to_str() else {
        return std::ptr::null_mut();
    };
    OBJECT_ACCESSORS.with(|accessors| {
        accessors
            .borrow()
            .get(&(object as usize))
            .and_then(|properties| properties.get(property))
            .map(|slots| slots[setter as usize] as *mut u8)
            .unwrap_or(std::ptr::null_mut())
    })
}

fn clear_object_states() {
    OBJECT_STATES.with(|states| states.borrow_mut().clear());
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().clear());
}

fn prune_object_states() {
    if !thaw_arena::is_tracing() { clear_object_states(); return; }
    OBJECT_STATES.with(|states| states.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
}

#[cfg(test)]
mod object_state_tests {
    use super::*;

    #[test]
    fn integrity_state_follows_pointer_identity_and_resets() {
        clear_object_states();
        let object = 0_u8;
        let identity = &object as *const u8;
        assert!(thaw_object_state(identity, 0));
        assert!(thaw_object_set_state(identity, 3));
        assert!(thaw_object_state(identity, 2));
        assert!(thaw_object_state(identity, 1));
        assert!(!thaw_object_state(identity, 0));
        clear_object_states();
        assert!(thaw_object_state(identity, 0));
    }

    #[test]
    fn accessors_follow_object_and_property_identity_and_reset() {
        clear_object_states();
        let object = 0_u8;
        let closure = 0_u8;
        let property = c"value";
        unsafe {
            assert!(thaw_object_set_accessor(
                &object,
                property.as_ptr(),
                &closure as *const u8 as *mut u8,
                false,
            ));
            assert_eq!(
                thaw_object_accessor(&object, property.as_ptr(), false),
                &closure as *const u8 as *mut u8
            );
            assert!(thaw_object_accessor(&object, property.as_ptr(), true).is_null());
            clear_object_states();
            assert!(thaw_object_accessor(&object, property.as_ptr(), false).is_null());
        }
    }
}
