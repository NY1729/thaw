thread_local! {
    static OBJECT_STATES: RefCell<HashMap<usize, u8>> = RefCell::new(HashMap::new());
    // ponytail: side-table keeps the native object ABI stable; use object headers if profiling
    // shows the pointer lookup matters.
    static OBJECT_ACCESSORS: RefCell<HashMap<usize, HashMap<String, [usize; 2]>>> =
        RefCell::new(HashMap::new());
    // The source HIR type carries class ancestry in the marker field name;
    // the compiled object stores only that field's Boolean value. Keep the
    // marker beside the allocation so an extracted method can validate the
    // actual receiver after function values flow through aliases/containers.
    static OBJECT_CLASS_IDENTITIES: RefCell<HashMap<usize, ObjectClassMetadata>> = RefCell::new(HashMap::new());
}

#[derive(Default)]
struct ObjectClassMetadata {
    ancestry: Option<String>,
    hidden_markers: std::collections::HashSet<String>,
}

const NON_EXTENSIBLE: u8 = 1;
const SEALED: u8 = 2;
const FROZEN: u8 = 4;

/// A dynamic JSON object's last shared handle has been dropped.
#[no_mangle]
pub extern "C" fn thaw_object_clear_state(object: *const u8) {
    let _ = OBJECT_STATES.try_with(|states| {
        states.borrow_mut().remove(&(object as usize));
    });
}

#[no_mangle]
/// # Safety
/// `marker` must be a valid NUL-terminated class identity field name.
/// `object` is an arena-owned native object identity and is not dereferenced.
pub unsafe extern "C" fn thaw_object_set_class_identity(
    object: *const u8,
    marker: *const c_char,
) -> bool {
    if object.is_null() || marker.is_null() {
        return false;
    }
    let Ok(marker) = CStr::from_ptr(marker).to_str() else {
        return false;
    };
    let Some(identities) = marker.strip_prefix("__thaw_class_identity_\u{1e}") else {
        return false;
    };
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        let mut stored = stored.borrow_mut();
        let metadata = stored.entry(object as usize).or_default();
        metadata.ancestry = Some(identities.to_owned());
        metadata.hidden_markers.insert(marker.to_owned());
    });
    true
}

#[no_mangle]
/// # Safety
/// `expected` must be a valid NUL-terminated class name. `object` is used
/// only as an identity; no unverified receiver pointer is dereferenced.
pub unsafe extern "C" fn thaw_object_has_class_identity(
    object: *const u8,
    expected: *const c_char,
) -> bool {
    if object.is_null() || expected.is_null() {
        return false;
    }
    let Ok(expected) = CStr::from_ptr(expected).to_str() else {
        return false;
    };
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored
            .borrow()
            .get(&(object as usize))
            .and_then(|metadata| metadata.ancestry.as_deref())
            .is_some_and(|identities| identities.split('\u{1f}').any(|name| name == expected))
    })
}

/// Marks a compiler-created physical marker as hidden without granting nominal
/// class identity to an ordinary object allocation.
#[no_mangle]
/// # Safety
/// `marker` must be a valid NUL-terminated compiler marker name. `object`
/// is an opaque arena allocation identity and is never dereferenced.
pub unsafe extern "C" fn thaw_object_hide_marker(
    object: *const u8,
    marker: *const c_char,
) -> bool {
    if object.is_null() || marker.is_null() {
        return false;
    }
    let Ok(marker) = CStr::from_ptr(marker).to_str() else {
        return false;
    };
    if !marker.starts_with("__thaw_class_identity_\u{1e}") {
        return false;
    }
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored
            .borrow_mut()
            .entry(object as usize)
            .or_default()
            .hidden_markers
            .insert(marker.to_owned());
    });
    true
}

#[no_mangle]
/// # Safety
/// `marker` must be a valid NUL-terminated field name. `object` is used
/// only as an opaque identity; a user-written same-name field is not hidden.
pub unsafe extern "C" fn thaw_object_marker_hidden(
    object: *const u8,
    marker: *const c_char,
) -> bool {
    if object.is_null() || marker.is_null() {
        return false;
    }
    let Ok(marker) = thaw_arena::NativeStr::from_ptr(marker).to_str() else {
        return false;
    };
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored
            .borrow()
            .get(&(object as usize))
            .is_some_and(|metadata| metadata.hidden_markers.contains(marker))
    })
}

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
    OBJECT_CLASS_IDENTITIES.with(|identities| identities.borrow_mut().clear());
}

fn prune_object_states() {
    if !thaw_arena::is_tracing() { clear_object_states(); return; }
    OBJECT_STATES.with(|states| states.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_CLASS_IDENTITIES.with(|identities| identities.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
}

#[cfg(test)]
mod object_state_tests {
    use super::*;

    #[test]
    fn class_identity_tracks_actual_allocation_and_inherited_names() {
        clear_object_states();
        let object = 0_u8;
        let identity = &object as *const u8;
        let marker = std::ffi::CString::new("__thaw_class_identity_\u{1e}Leaf\u{1f}Base$Name").unwrap();
        let leaf = std::ffi::CString::new("Leaf").unwrap();
        let base = std::ffi::CString::new("Base$Name").unwrap();
        let false_base = std::ffi::CString::new("Base").unwrap();
        let other = std::ffi::CString::new("Other").unwrap();
        assert!(unsafe { thaw_object_set_class_identity(identity, marker.as_ptr()) });
        assert!(unsafe { thaw_object_marker_hidden(identity, marker.as_ptr()) });
        assert!(unsafe { thaw_object_has_class_identity(identity, leaf.as_ptr()) });
        assert!(unsafe { thaw_object_has_class_identity(identity, base.as_ptr()) });
        assert!(!unsafe { thaw_object_has_class_identity(identity, false_base.as_ptr()) });
        assert!(!unsafe { thaw_object_has_class_identity(identity, other.as_ptr()) });
        clear_object_states();
        assert!(!unsafe { thaw_object_has_class_identity(identity, leaf.as_ptr()) });
    }

    #[test]
    fn hidden_marker_is_provenance_not_a_reserved_field_name() {
        clear_object_states();
        let ordinary = 0_u8;
        let builtin = 0_u8;
        let ordinary = &ordinary as *const u8;
        let builtin = &builtin as *const u8;
        let marker = std::ffi::CString::new("__thaw_class_identity_\u{1e}AggregateError\u{1f}Error").unwrap();
        let aggregate = std::ffi::CString::new("AggregateError").unwrap();
        let invalid = std::ffi::CString::new("ordinary").unwrap();
        assert!(!unsafe { thaw_object_marker_hidden(std::ptr::null(), marker.as_ptr()) });
        assert!(!unsafe { thaw_object_marker_hidden(ordinary, std::ptr::null()) });
        assert!(!unsafe { thaw_object_hide_marker(ordinary, invalid.as_ptr()) });
        assert!(!unsafe { thaw_object_marker_hidden(ordinary, marker.as_ptr()) });
        assert!(unsafe { thaw_object_hide_marker(builtin, marker.as_ptr()) });
        assert!(unsafe { thaw_object_marker_hidden(builtin, marker.as_ptr()) });
        assert!(!unsafe { thaw_object_has_class_identity(builtin, aggregate.as_ptr()) });

        assert!(!unsafe { thaw_object_marker_hidden(ordinary, marker.as_ptr()) });
        // A registered native string can contain NUL after a real marker.
        // Query the full bytes, not its C-string prefix.
        static EMBEDDED_NUL: &[u8] = b"__thaw_class_identity_\x1eAggregateError\x1fError\0extra\0";
        let embedded = unsafe { thaw_arena::thaw_string_register_literal(
            EMBEDDED_NUL.as_ptr().cast(), EMBEDDED_NUL.len() - 1,
        ) };
        assert!(!unsafe { thaw_object_marker_hidden(builtin, embedded) });
        clear_object_states();
        assert!(!unsafe { thaw_object_marker_hidden(builtin, marker.as_ptr()) });
    }

    #[test]
    fn hidden_markers_follow_retained_and_reclaimed_arena_identities() {
        std::thread::spawn(|| {
            clear_object_states();
            thaw_arena::thaw_arena_enable_tracing();
            let retained = thaw_arena::thaw_arena_alloc(8, 8);
            let reclaimed = thaw_arena::thaw_arena_alloc(8, 8);
            assert!(!retained.is_null() && !reclaimed.is_null());
            let marker = std::ffi::CString::new("__thaw_class_identity_\u{1e}Leaf").unwrap();
            let leaf = std::ffi::CString::new("Leaf").unwrap();
            assert!(unsafe { thaw_object_set_class_identity(retained, marker.as_ptr()) });
            assert!(unsafe { thaw_object_set_class_identity(reclaimed, marker.as_ptr()) });
            let root = thaw_arena::ArenaRoot::new(retained as usize);
            thaw_arena::thaw_arena_reset();
            prune_object_states();
            assert!(unsafe { thaw_object_marker_hidden(retained, marker.as_ptr()) });
            assert!(unsafe { thaw_object_has_class_identity(retained, leaf.as_ptr()) });
            assert!(!unsafe { thaw_object_marker_hidden(reclaimed, marker.as_ptr()) });
            assert!(!unsafe { thaw_object_has_class_identity(reclaimed, leaf.as_ptr()) });
            let reused = thaw_arena::thaw_arena_alloc(8, 8);
            assert!(!reused.is_null());
            assert!(!unsafe { thaw_object_marker_hidden(reused, marker.as_ptr()) });
            drop(root);
            thaw_arena::thaw_arena_reset();
            prune_object_states();
            assert!(!unsafe { thaw_object_marker_hidden(retained, marker.as_ptr()) });
        }).join().unwrap();
    }

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
