thread_local! {
    static OBJECT_STATES: RefCell<HashMap<usize, u8>> = RefCell::new(HashMap::new());
    // ponytail: side-table keeps the native object ABI stable; use object headers if profiling
    // shows the pointer lookup matters.
    static OBJECT_ACCESSORS: RefCell<HashMap<usize, HashMap<String, [usize; 2]>>> =
        RefCell::new(HashMap::new());
    // Descriptor attributes belong to the original native allocation, not a
    // transient Json/QuickJS projection. Three bits model writable,
    // enumerable, and configurable for a visible fixed data field.
    static OBJECT_PROPERTY_FLAGS: RefCell<HashMap<usize, HashMap<String, u8>>> =
        RefCell::new(HashMap::new());
    // The full typed view is retained when a pointer is narrowed to a prefix
    // layout. The closure is an arena child of its owner and runs only on the
    // first reference projection.
    static OBJECT_PROJECTORS: RefCell<HashMap<usize, (String, usize)>> =
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
    // None keeps the static field-order fallback for ordinary fixed objects.
    // A copied object's physical slots can precede their actual creation.
    own_key_order: Option<Vec<String>>,
}

const NON_EXTENSIBLE: u8 = 1;
const SEALED: u8 = 2;
const FROZEN: u8 = 4;
const PROPERTY_WRITABLE: u8 = 1;
const PROPERTY_ENUMERABLE: u8 = 2;
const PROPERTY_CONFIGURABLE: u8 = 4;
const PROPERTY_DEFAULT: u8 = PROPERTY_WRITABLE | PROPERTY_ENUMERABLE | PROPERTY_CONFIGURABLE;

/// Effective descriptor flags for one physically present native data field.
/// A zero result is a valid fully restricted property; callers establish
/// existence from the allocation's trusted field layout before querying.
fn object_property_flags(object: *const u8, property: &str) -> u8 {
    let mut flags = OBJECT_PROPERTY_FLAGS.with(|stored| stored.borrow()
        .get(&(object as usize)).and_then(|fields| fields.get(property)).copied()
        .unwrap_or(PROPERTY_DEFAULT));
    let state = OBJECT_STATES.with(|states| states.borrow()
        .get(&(object as usize)).copied().unwrap_or_default());
    if state & SEALED != 0 { flags &= !PROPERTY_CONFIGURABLE; }
    if state & FROZEN != 0 { flags &= !PROPERTY_WRITABLE; }
    flags
}

#[no_mangle]
/// # Safety
/// `property` points to a live native string; `object` is an opaque identity.
pub unsafe extern "C" fn thaw_object_property_flags(object: *const u8, property: *const c_char) -> u8 {
    if object.is_null() || property.is_null() { return 0; }
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else { return 0; };
    object_property_flags(object, property)
}

/// Records a successful descriptor update on the original native object.
/// The caller must first verify that the property is a visible physical
/// field, and must apply the corresponding JS target descriptor only when
/// this transition is allowed.
#[no_mangle]
/// # Safety
/// `property` points to a live native string; `object` is an opaque identity.
pub unsafe extern "C" fn thaw_object_set_property_flags(
    object: *const u8, property: *const c_char, requested: u8,
) -> bool {
    if object.is_null() || property.is_null() || requested & !PROPERTY_DEFAULT != 0 {
        return false;
    }
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else { return false; };
    let current = object_property_flags(object, property);
    let configurable = current & PROPERTY_CONFIGURABLE != 0;
    if !configurable && (requested & PROPERTY_CONFIGURABLE != 0
        || (requested ^ current) & PROPERTY_ENUMERABLE != 0
        || (current & PROPERTY_WRITABLE == 0 && requested & PROPERTY_WRITABLE != 0)) {
        return false;
    }
    OBJECT_PROPERTY_FLAGS.with(|stored| {
        stored.borrow_mut().entry(object as usize).or_default()
            .insert(property.to_owned(), requested);
    });
    true
}

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
        if let Some(order) = metadata.own_key_order.as_mut() {
            order.retain(|name| name != marker);
        }
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
/// `marker` must be a live native string for a compiler marker name. `object`
/// is an opaque arena allocation identity and is never dereferenced.
pub unsafe extern "C" fn thaw_object_hide_marker(
    object: *const u8,
    marker: *const c_char,
) -> bool {
    if object.is_null() || marker.is_null() {
        return false;
    }
    let Ok(marker) = thaw_arena::NativeStr::from_ptr(marker).to_str() else {
        return false;
    };
    if !marker.starts_with("__thaw_class_identity_\u{1e}") {
        return false;
    }
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        let mut stored = stored.borrow_mut();
        let metadata = stored.entry(object as usize).or_default();
        metadata.hidden_markers.insert(marker.to_owned());
        if let Some(order) = metadata.own_key_order.as_mut() {
            order.retain(|name| name != marker);
        }
    });
    true
}

/// Makes a physically present marker field public after a successful data
/// write, without changing the object's nominal class ancestry.
#[no_mangle]
/// # Safety
/// `marker` must be a live native string; `object` is an opaque identity.
pub unsafe extern "C" fn thaw_object_reveal_marker(
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
        stored.borrow_mut().get_mut(&(object as usize))
            .is_some_and(|metadata| {
                if !metadata.hidden_markers.remove(marker) {
                    return false;
                }
                if let Some(order) = metadata.own_key_order.as_mut() {
                    if !order.iter().any(|name| name == marker) {
                        order.push(marker.to_owned());
                    }
                }
                true
            })
    })
}

/// Starts explicit own-key ordering without changing nominal ancestry or the
/// set of hidden compiler markers. Call before copying into a zeroed superset.
#[no_mangle]
pub extern "C" fn thaw_object_order_begin(object: *const u8) -> bool {
    if object.is_null() {
        return false;
    }
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored.borrow_mut().entry(object as usize).or_default().own_key_order = Some(Vec::new());
    });
    true
}

/// Records a successfully created visible field in an active order list.
/// Ordinary objects without an order list keep their static-layout fallback.
#[no_mangle]
/// # Safety
/// `key` must point to a live native string; `object` is only an identity.
pub unsafe extern "C" fn thaw_object_order_seed(object: *const u8, key: *const c_char) -> bool {
    if object.is_null() || key.is_null() {
        return false;
    }
    let Ok(key) = thaw_arena::NativeStr::from_ptr(key).to_str() else {
        return false;
    };
    OBJECT_CLASS_IDENTITIES.with(|stored| {
        let mut stored = stored.borrow_mut();
        let Some(metadata) = stored.get_mut(&(object as usize)) else {
            return true;
        };
        if metadata.hidden_markers.contains(key) {
            return true;
        }
        if let Some(order) = metadata.own_key_order.as_mut() {
            if !order.iter().any(|name| name == key) {
                order.push(key.to_owned());
            }
        }
        true
    })
}

/// Returns a sortable own-key rank. Canonical array indices precede string
/// keys, while an active list distinguishes an uncreated physical slot from
/// an ordinary object using static order. `i64::MAX` means absent/invalid.
#[no_mangle]
/// # Safety
/// `key` must point to a live native string; `object` is only an identity.
pub unsafe extern "C" fn thaw_object_order_rank(
    object: *const u8,
    key: *const c_char,
    static_rank: i64,
) -> i64 {
    if object.is_null() || key.is_null() {
        return i64::MAX;
    }
    let Ok(key) = thaw_arena::NativeStr::from_ptr(key).to_str() else {
        return i64::MAX;
    };
    let index = key.parse::<u32>().ok()
        .filter(|&index| index != u32::MAX && index.to_string() == key);
    let string_rank = OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored.borrow().get(&(object as usize))
            .and_then(|metadata| metadata.own_key_order.as_ref())
            .map_or(static_rank, |order| {
                order.iter().position(|name| name == key)
                    .map_or(i64::MAX, |index| i64::try_from(index).unwrap_or(i64::MAX))
            })
    });
    if string_rank == i64::MAX {
        return i64::MAX;
    }
    if let Some(index) = index {
        return i64::from(index);
    }
    (1_i64 << 32).checked_add(string_rank).unwrap_or(i64::MAX)
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

#[no_mangle]
/// # Safety
/// `layout` is a live native string; `owner` and `closure` are arena objects.
pub unsafe extern "C" fn thaw_object_register_projector(
    owner: *mut u8, layout: *const c_char, closure: *mut u8,
) -> bool {
    if owner.is_null() || layout.is_null() || closure.is_null()
        || !thaw_arena::contains_allocation(owner as usize) { return false; }
    let Ok(layout) = thaw_arena::NativeStr::from_ptr(layout).to_str() else { return false; };
    OBJECT_PROJECTORS.with(|projectors| {
        let mut projectors = projectors.borrow_mut();
        let previous = projectors.get(&(owner as usize));
        if let Some((existing, _)) = previous {
            if existing.starts_with(layout) { return true; }
            if !layout.starts_with(existing) { return false; }
        }
        let old = projectors.insert(owner as usize, (layout.to_owned(), closure as usize))
            .map(|(_, pointer)| pointer).unwrap_or_default();
        thaw_arena::replace_reference(owner as usize, old, closure as usize);
        true
    })
}

#[no_mangle]
/// # Safety
/// `layout` is a live native string; `owner` is an arena object.
pub unsafe extern "C" fn thaw_object_projector(
    owner: *const u8, layout: *const c_char,
) -> *mut u8 {
    if owner.is_null() || layout.is_null() { return std::ptr::null_mut(); }
    let Ok(layout) = thaw_arena::NativeStr::from_ptr(layout).to_str() else {
        return std::ptr::null_mut();
    };
    OBJECT_PROJECTORS.with(|projectors| projectors.borrow().get(&(owner as usize))
        .filter(|(actual, _)| actual.starts_with(layout))
        .map(|(_, closure)| *closure as *mut u8)
        .unwrap_or(std::ptr::null_mut()))
}

fn clear_object_states() {
    OBJECT_STATES.with(|states| states.borrow_mut().clear());
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().clear());
    OBJECT_PROPERTY_FLAGS.with(|flags| flags.borrow_mut().clear());
    OBJECT_PROJECTORS.with(|projectors| projectors.borrow_mut().clear());
    OBJECT_CLASS_IDENTITIES.with(|identities| identities.borrow_mut().clear());
}

fn prune_object_states() {
    if !thaw_arena::is_tracing() { clear_object_states(); return; }
    OBJECT_STATES.with(|states| states.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_PROPERTY_FLAGS.with(|flags| flags.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_PROJECTORS.with(|projectors| projectors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_CLASS_IDENTITIES.with(|identities| identities.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
}

#[cfg(test)]
mod object_state_tests {
    use super::*;

    #[test]
    fn own_key_order_tracks_creation_hide_reveal_and_full_native_names() {
        clear_object_states();
        let object = 0_u8;
        let object = &object as *const u8;
        let marker = std::ffi::CString::new("__thaw_class_identity_\u{1e}A").unwrap();
        let x = unsafe { thaw_arena::thaw_string_register_literal(b"x".as_ptr().cast(), 1) };
        let marker_name = marker.as_bytes();
        let marker_key = unsafe { thaw_arena::thaw_string_register_literal(
            marker_name.as_ptr().cast(), marker_name.len(),
        ) };
        let nul_name = b"x\0second";
        let nul_key = unsafe { thaw_arena::thaw_string_register_literal(
            nul_name.as_ptr().cast(), nul_name.len(),
        ) };
        let numeric = unsafe { thaw_arena::thaw_string_register_literal(b"2".as_ptr().cast(), 1) };

        assert_eq!(unsafe { thaw_object_order_rank(object, x, 7) }, (1_i64 << 32) + 7);
        assert!(unsafe { thaw_object_set_class_identity(object, marker.as_ptr()) });
        assert!(thaw_object_order_begin(object));
        assert!(unsafe { thaw_object_order_seed(object, marker_key) });
        assert_eq!(unsafe { thaw_object_order_rank(object, marker_key, 0) }, i64::MAX);
        assert!(unsafe { thaw_object_order_seed(object, x) });
        assert!(unsafe { thaw_object_order_seed(object, nul_key) });
        assert!(unsafe { thaw_object_order_seed(object, numeric) });
        assert_eq!(unsafe { thaw_object_order_rank(object, numeric, 99) }, 2);
        assert_eq!(unsafe { thaw_object_order_rank(object, x, 9) }, 1_i64 << 32);
        assert_eq!(unsafe { thaw_object_order_rank(object, nul_key, 9) }, (1_i64 << 32) + 1);
        assert!(unsafe { thaw_object_reveal_marker(object, marker_key) });
        assert_eq!(unsafe { thaw_object_order_rank(object, marker_key, 9) }, (1_i64 << 32) + 3);
        assert!(unsafe { thaw_object_hide_marker(object, marker_key) });
        assert_eq!(unsafe { thaw_object_order_rank(object, marker_key, 9) }, i64::MAX);
        assert!(unsafe { thaw_object_reveal_marker(object, marker_key) });
        assert_eq!(unsafe { thaw_object_order_rank(object, marker_key, 9) }, (1_i64 << 32) + 3);
        clear_object_states();
        assert_eq!(unsafe { thaw_object_order_rank(object, x, 7) }, (1_i64 << 32) + 7);
    }

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

#[cfg(test)]
mod marker_reveal_tests {
    use super::*;

    #[test]
    fn reveal_removes_only_the_exact_hidden_key_and_keeps_ancestry() {
        clear_object_states();
        let object = 0_u8;
        let object = &object as *const u8;
        let first = std::ffi::CString::new("__thaw_class_identity_\u{1e}Leaf\u{1f}Base").unwrap();
        let sibling = std::ffi::CString::new("__thaw_class_identity_\u{1e}Other").unwrap();
        let leaf = std::ffi::CString::new("Leaf").unwrap();
        assert!(unsafe { thaw_object_set_class_identity(object, first.as_ptr()) });
        assert!(unsafe { thaw_object_hide_marker(object, sibling.as_ptr()) });
        assert!(unsafe { thaw_object_reveal_marker(object, first.as_ptr()) });
        assert!(!unsafe { thaw_object_marker_hidden(object, first.as_ptr()) });
        assert!(unsafe { thaw_object_marker_hidden(object, sibling.as_ptr()) });
        assert!(unsafe { thaw_object_has_class_identity(object, leaf.as_ptr()) });
        assert!(!unsafe { thaw_object_reveal_marker(object, first.as_ptr()) });
        assert!(unsafe { thaw_object_reveal_marker(object, sibling.as_ptr()) });
        assert!(!unsafe { thaw_object_marker_hidden(object, sibling.as_ptr()) });

        static EMBEDDED: &[u8] = b"__thaw_class_identity_\x1eOther\0tail\0";
        let full = unsafe { thaw_arena::thaw_string_register_literal(
            EMBEDDED.as_ptr().cast(), EMBEDDED.len() - 1,
        ) };
        assert!(unsafe { thaw_object_hide_marker(object, full) });
        assert!(!unsafe { thaw_object_reveal_marker(object, sibling.as_ptr()) });
        assert!(unsafe { thaw_object_marker_hidden(object, full) });
        assert!(unsafe { thaw_object_reveal_marker(object, full) });
        assert!(!unsafe { thaw_object_marker_hidden(object, full) });
        clear_object_states();
        assert!(!unsafe { thaw_object_has_class_identity(object, leaf.as_ptr()) });
    }
}
