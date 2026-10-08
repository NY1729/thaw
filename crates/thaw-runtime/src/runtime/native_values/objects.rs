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
    // The full typed view is retained when a pointer is structurally narrowed.
    // Layout checks include fields by exact name/type, regardless of order. The closure is an arena child of its owner and runs only on the
    // first reference projection.
    static OBJECT_PROJECTORS: RefCell<HashMap<usize, (String, usize)>> =
        RefCell::new(HashMap::new());
    // A thrown Error record's `() -> Json` describer, so a catch can read the
    // record without any QuickJS projection. Arena child of its owner like a
    // projector.
    static OBJECT_DESCRIBERS: RefCell<HashMap<usize, usize>> =
        RefCell::new(HashMap::new());
    // A structural alias can enumerate fields in a different order from the
    // physical allocation. Record its original byte offsets at the first
    // type-erasure boundary, without copying or changing the object ABI.
    #[allow(clippy::type_complexity)]
    static OBJECT_FIELD_OFFSETS: RefCell<HashMap<usize, (String, HashMap<String, u64>)>> =
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

fn object_can_set_property_flags(object: *const u8, property: &str, requested: u8) -> bool {
    if object.is_null() || requested & !PROPERTY_DEFAULT != 0 {
        return false;
    }
    let current = object_property_flags(object, property);
    let configurable = current & PROPERTY_CONFIGURABLE != 0;
    configurable || !(requested & PROPERTY_CONFIGURABLE != 0
        || (requested ^ current) & PROPERTY_ENUMERABLE != 0
        || (current & PROPERTY_WRITABLE == 0 && requested & PROPERTY_WRITABLE != 0))
}

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
    if property.is_null() { return false; }
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else { return false; };
    if !object_can_set_property_flags(object, property, requested) { return false; }
    OBJECT_PROPERTY_FLAGS.with(|stored| {
        stored.borrow_mut().entry(object as usize).or_default()
            .insert(property.to_owned(), requested);
    });
    true
}

#[no_mangle]
/// Checks a descriptor transition before its value is written. The matching
/// setter rechecks the same owner state when the write has completed.
///
/// # Safety
/// `property` points to a live native string; `object` is an opaque identity.
pub unsafe extern "C" fn thaw_object_can_set_property_flags(
    object: *const u8, property: *const c_char, requested: u8,
) -> bool {
    if property.is_null() { return false; }
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else { return false; };
    object_can_set_property_flags(object, property, requested)
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
    thaw_object_order_rank_for_key(object, key, static_rank)
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
/// `property` must point to a live native string for the duration of
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
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else {
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
/// `property` must point to a live native string for the duration of
/// this call. `object` is used only as an opaque identity.
pub unsafe extern "C" fn thaw_object_accessor(
    object: *const u8,
    property: *const c_char,
    setter: bool,
) -> *mut u8 {
    if object.is_null() || property.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(property) = thaw_arena::NativeStr::from_ptr(property).to_str() else {
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

/// Decodes compiler-owned `offset:hex(field-name);` entries. Hex field names
/// preserve NUL and every UTF-8 source name across the native string ABI.
fn parse_object_offset_layout(layout: &str) -> Option<HashMap<String, u64>> {
    let mut fields = HashMap::new();
    if layout.is_empty() { return Some(fields); }
    for entry in layout.split_terminator(';') {
        let (offset, hex_name) = entry.split_once(':')?;
        let offset = offset.parse::<u64>().ok()?;
        if hex_name.len() % 2 != 0 || !hex_name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        if fields.insert(hex_name.to_owned(), offset).is_some() { return None; }
    }
    Some(fields)
}

/// The first registered descriptor, rather than the current structural
/// view, is the authority for native own-property names.
fn full_object_field_names(owner: *const u8) -> Option<Vec<String>> {
    let descriptor = OBJECT_FIELD_OFFSETS.with(|all| {
        all.borrow().get(&(owner as usize)).map(|(layout, _)| layout.clone())
    })?;
    object_layout_segments(&descriptor)?.into_iter().map(|segment| {
        let colon = segment.find(':')?;
        let length = segment[..colon].parse::<usize>().ok()?;
        let end = colon.checked_add(1)?.checked_add(length.checked_mul(2)?)?;
        let hex = segment.get(colon + 1..end)?;
        let bytes = hex.as_bytes().chunks_exact(2).map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()
        }).collect::<Option<Vec<_>>>()?;
        String::from_utf8(bytes).ok()
    }).collect()
}

fn object_internal_accessor_slot(key: &str) -> bool {
    key.starts_with("__thaw_getter_") || key.starts_with("__thaw_setter_")
}

#[no_mangle]
pub extern "C" fn thaw_object_has_full_layout(owner: *const u8) -> bool {
    OBJECT_FIELD_OFFSETS.with(|all| all.borrow().contains_key(&(owner as usize)))
}

#[no_mangle]
/// # Safety
/// `key` points to a live native string.
pub unsafe extern "C" fn thaw_object_full_has_own(owner: *const u8, key: *const c_char) -> bool {
    if owner.is_null() || key.is_null() { return false; }
    let Ok(key) = thaw_arena::NativeStr::from_ptr(key).to_str() else { return false; };
    full_object_field_names(owner).is_some_and(|fields| {
        fields.iter().any(|field| field == key) && !object_internal_accessor_slot(key)
            && !OBJECT_CLASS_IDENTITIES.with(|states| states.borrow().get(&(owner as usize))
                .is_some_and(|state| state.hidden_markers.contains(key)))
            && thaw_object_order_rank_for_key(owner, key, 0) != i64::MAX
    })
}

/// Uses the same numeric-first and insertion-order rule as
/// `thaw_object_order_rank`, without allocating temporary native strings.
fn thaw_object_order_rank_for_key(owner: *const u8, key: &str, static_rank: i64) -> i64 {
    let index = key.parse::<u32>().ok()
        .filter(|&index| index != u32::MAX && index.to_string() == key);
    let string_rank = OBJECT_CLASS_IDENTITIES.with(|stored| {
        stored.borrow().get(&(owner as usize))
            .and_then(|metadata| metadata.own_key_order.as_ref())
            .map_or(static_rank, |order| order.iter().position(|name| name == key)
                .map_or(i64::MAX, |index| i64::try_from(index).unwrap_or(i64::MAX)))
    });
    if string_rank == i64::MAX { return i64::MAX; }
    index.map_or_else(|| (1_i64 << 32).checked_add(string_rank).unwrap_or(i64::MAX), i64::from)
}

#[no_mangle]
/// Returns the original owner's visible own string keys. `include_non_enumerable`
/// selects the `getOwnPropertyNames`/`Reflect.ownKeys` string-key behavior.
pub extern "C" fn thaw_object_full_own_keys(
    owner: *const u8, include_non_enumerable: bool,
) -> *mut u8 {
    let Some(fields) = full_object_field_names(owner) else { return std::ptr::null_mut(); };
    let hidden = OBJECT_CLASS_IDENTITIES.with(|states| states.borrow()
        .get(&(owner as usize)).map(|state| state.hidden_markers.clone()).unwrap_or_default());
    let mut keys = fields.into_iter().enumerate().filter_map(|(index, key)| {
        if hidden.contains(&key) || object_internal_accessor_slot(&key)
            || (!include_non_enumerable && object_property_flags(owner, &key) & PROPERTY_ENUMERABLE == 0) {
            return None;
        }
        let rank = thaw_object_order_rank_for_key(owner, &key, index as i64);
        (rank != i64::MAX).then_some((rank, key))
    }).collect::<Vec<_>>();
    keys.sort_by_key(|(rank, _)| *rank);
    let output = thaw_arena::thaw_arena_alloc((keys.len() + 1) * 8, 8);
    if output.is_null() { return output; }
    unsafe { output.cast::<i64>().write(keys.len() as i64); }
    for (index, (_, key)) in keys.iter().enumerate() {
        let Some(key) = arena_wtf8(key.as_bytes()) else { return std::ptr::null_mut(); };
        unsafe { output.add(8 + index * 8).cast::<*const u8>().write_unaligned(key); }
    }
    output
}

#[no_mangle]
/// Registers the physical field offsets of an arena-owned object before a
/// nonprefix structural alias erases its full type. A later narrower view may
/// repeat a subset; the first full owner layout remains authoritative.
///
/// # Safety
/// `layout` points to a live native string of compiler-generated entries.
pub unsafe extern "C" fn thaw_object_register_field_offsets(
    owner: *mut u8, layout: *const c_char, descriptor: *const c_char,
) -> bool {
    if owner.is_null() || layout.is_null() || descriptor.is_null()
        || !thaw_arena::contains_allocation(owner as usize) { return false; }
    let Ok(layout) = thaw_arena::NativeStr::from_ptr(layout).to_str() else { return false; };
    let Ok(descriptor) = thaw_arena::NativeStr::from_ptr(descriptor).to_str() else { return false; };
    let Some(fields) = parse_object_offset_layout(layout) else { return false; };
    let Some(segments) = object_layout_segments(descriptor) else { return false; };
    if segments.len() != fields.len() { return false; }
    OBJECT_FIELD_OFFSETS.with(|all| {
        let mut all = all.borrow_mut();
        if let Some((physical, offsets)) = all.get(&(owner as usize)) {
            // A second structural view supplies apparent offsets. Validate
            // names and types against the first physical descriptor, then
            // keep its offsets intact. An unknown field cannot be inferred.
            return object_layout_contains(physical, descriptor)
                && fields.keys().all(|name| offsets.contains_key(name));
        }
        all.insert(owner as usize, (descriptor.to_owned(), fields));
        true
    })
}

#[no_mangle]
/// Resolves an aliased field through its physical owner layout. With no
/// registered alias, the caller's static offset remains correct. A registered
/// owner missing the field returns a sentinel that codegen must reject.
///
/// # Safety
/// `field` points to a live hex-encoded native string.
pub unsafe extern "C" fn thaw_object_field_offset(
    owner: *const u8, field: *const c_char, static_offset: u64,
) -> u64 {
    if owner.is_null() || field.is_null() { return u64::MAX; }
    let Ok(field) = thaw_arena::NativeStr::from_ptr(field).to_str() else { return u64::MAX; };
    OBJECT_FIELD_OFFSETS.with(|all| all.borrow().get(&(owner as usize))
        .map(|(_, fields)| fields.get(field).copied().unwrap_or(u64::MAX))
        .unwrap_or(static_offset))
}

/// Reads field-name/type pairs from the compiler's length-prefixed hex
/// layout token. A structural view may list the same fields in any order.
fn object_layout_segments(layout: &str) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    while cursor < layout.len() {
        let start = cursor;
        for _ in 0..2 {
            let colon = layout.get(cursor..)?.find(':')?.checked_add(cursor)?;
            let bytes = layout.get(cursor..colon)?.parse::<usize>().ok()?;
            let end = colon.checked_add(1)?.checked_add(bytes.checked_mul(2)?)?;
            let hex = layout.get(colon + 1..end)?;
            if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) { return None; }
            cursor = end;
        }
        segments.push(layout.get(start..cursor)?);
    }
    Some(segments)
}

fn object_layout_contains(actual: &str, expected: &str) -> bool {
    if actual == expected { return true; }
    let (Some(actual), Some(expected)) =
        (object_layout_segments(actual), object_layout_segments(expected)) else { return false; };
    let mut used = vec![false; actual.len()];
    expected.iter().all(|field| actual.iter().enumerate().any(|(index, actual)| {
        if !used[index] && actual == field {
            used[index] = true;
            true
        } else { false }
    }))
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
            if object_layout_contains(existing, layout) { return true; }
            if !object_layout_contains(layout, existing) { return false; }
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
        .filter(|(actual, _)| object_layout_contains(actual, layout))
        .map(|(_, closure)| *closure as *mut u8)
        .unwrap_or(std::ptr::null_mut()))
}

#[no_mangle]
/// # Safety
/// `owner` and `closure` are arena objects.
pub unsafe extern "C" fn thaw_object_register_describer(owner: *mut u8, closure: *mut u8) -> bool {
    if owner.is_null() || closure.is_null() || !thaw_arena::contains_allocation(owner as usize) {
        return false;
    }
    OBJECT_DESCRIBERS.with(|describers| {
        let old = describers.borrow_mut().insert(owner as usize, closure as usize).unwrap_or_default();
        thaw_arena::replace_reference(owner as usize, old, closure as usize);
        true
    })
}

#[no_mangle]
/// # Safety
/// `owner` is an arena object.
pub unsafe extern "C" fn thaw_object_describer(owner: *const u8) -> *mut u8 {
    if owner.is_null() { return std::ptr::null_mut(); }
    OBJECT_DESCRIBERS.with(|describers| describers.borrow().get(&(owner as usize))
        .map(|closure| *closure as *mut u8).unwrap_or(std::ptr::null_mut()))
}

fn clear_object_states() {
    OBJECT_STATES.with(|states| states.borrow_mut().clear());
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().clear());
    OBJECT_PROPERTY_FLAGS.with(|flags| flags.borrow_mut().clear());
    OBJECT_PROJECTORS.with(|projectors| projectors.borrow_mut().clear());
    OBJECT_DESCRIBERS.with(|describers| describers.borrow_mut().clear());
    OBJECT_FIELD_OFFSETS.with(|offsets| offsets.borrow_mut().clear());
    OBJECT_CLASS_IDENTITIES.with(|identities| identities.borrow_mut().clear());
}

fn prune_object_states() {
    if !thaw_arena::is_tracing() { clear_object_states(); return; }
    OBJECT_STATES.with(|states| states.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_ACCESSORS.with(|accessors| accessors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_PROPERTY_FLAGS.with(|flags| flags.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_PROJECTORS.with(|projectors| projectors.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_DESCRIBERS.with(|describers| describers.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
    OBJECT_FIELD_OFFSETS.with(|offsets| offsets.borrow_mut().retain(|pointer, _| !thaw_arena::was_reclaimed(*pointer)));
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
    fn accessor_names_preserve_embedded_nul() {
        clear_object_states();
        let object = 0_u8;
        let short_closure = 1_u8;
        let long_closure = 2_u8;
        let long_name = thaw_arena::owned_string("x\0y");
        assert!(!long_name.is_null());
        unsafe {
            let short = &short_closure as *const u8 as *mut u8;
            let long = &long_closure as *const u8 as *mut u8;
            assert!(thaw_object_set_accessor(&object, c"x".as_ptr(), short, false));
            assert!(thaw_object_accessor(&object, long_name, false).is_null());
            assert!(thaw_object_set_accessor(&object, long_name, long, false));
            assert_eq!(thaw_object_accessor(&object, c"x".as_ptr(), false), short);
            assert_eq!(thaw_object_accessor(&object, long_name, false), long);
            assert!(thaw_object_accessor(&object, long_name, true).is_null());
            clear_object_states();
            thaw_arena::destroy_string(long_name);
        }
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

#[cfg(test)]
mod full_native_own_key_tests {
    use super::*;

    #[test]
    fn rooted_key_array_keeps_its_arena_string_after_reset() {
        std::thread::spawn(|| {
            clear_object_states();
            thaw_arena::thaw_arena_enable_tracing();
            let owner = thaw_arena::thaw_arena_alloc(8, 8);
            assert!(!owner.is_null());
            let offsets = std::ffi::CString::new("0:61;").unwrap();
            let descriptor = std::ffi::CString::new("1:611:78").unwrap();
            assert!(unsafe { thaw_object_register_field_offsets(
                owner, offsets.as_ptr(), descriptor.as_ptr(),
            ) });
            let keys = thaw_object_full_own_keys(owner, false);
            assert!(!keys.is_null());
            assert_eq!(unsafe { keys.cast::<i64>().read() }, 1);
            let key = unsafe { keys.add(8).cast::<*const c_char>().read_unaligned() };
            assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(key) }.to_bytes(), b"a");
            let root = thaw_arena::ArenaRoot::new(keys as usize);
            thaw_arena::thaw_arena_reset();
            assert!(!thaw_arena::was_reclaimed(key as usize));
            assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(key) }.to_bytes(), b"a");
            drop(root);
            thaw_arena::thaw_arena_reset();
            assert!(thaw_arena::was_reclaimed(key as usize));
        }).join().unwrap();
    }
}
