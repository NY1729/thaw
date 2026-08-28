// An arena-backed, insertion-order-preserving open-addressing hash table,
// backing both `Map` and `Set`. A `Set` is just this table with every
// value fixed at `0`.
//
// Three key kinds are supported: `F64` (`SameValueZero`-equal numbers),
// `Str` (content-hashed), and every other representable type (`Object`,
// `Array`, `Function`, `Map`, `Set`, ...), hashed and compared by
// reference identity -- their own pointer value, which is already a
// stable identity token for as long as the pointed-to allocation lives,
// exactly matching how JavaScript's own `SameValueZero` degenerates to
// `===` reference equality for non-primitive keys. The table's own arena
// pointer is its stable
// identity: growth reallocates the backing entries/buckets arrays and
// rewrites the header in place, exactly like a `Date`'s `timestamp` field
// mutates in place, so every reference to a `Map`/`Set` value keeps
// observing the same growing table.
//
// Entries live in a compact, append-only, insertion-ordered array;
// buckets are a separate open-addressed (linear probing) index from hash
// to entry position. Growth compacts out tombstoned (deleted) entries and
// rebuilds the bucket index from each surviving entry's already-computed
// hash, so the table never needs to re-hash a key.

#[repr(C)]
#[derive(Clone, Copy)]
struct MapEntry {
    state: u64,
    hash: u64,
    key: u64,
    value: u64,
}

const ENTRY_EMPTY: u64 = 0;
const ENTRY_LIVE: u64 = 1;
const ENTRY_TOMBSTONE: u64 = 2;

const BUCKET_EMPTY: i64 = -1;
const BUCKET_TOMBSTONE: i64 = -2;

const INITIAL_BUCKETS: u64 = 16;

#[repr(C)]
struct MapHeader {
    entries: *mut MapEntry,
    entries_len: u64,
    entries_cap: u64,
    live: u64,
    buckets: *mut i64,
    buckets_len: u64,
}

fn mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// Canonicalizes a numeric key per JavaScript's `SameValueZero`: `+0` and
/// `-0` collapse to one key, and `NaN` equals itself.
fn canonical_num_key(key: f64) -> u64 {
    if key.is_nan() {
        f64::NAN.to_bits()
    } else if key == 0.0 {
        0.0f64.to_bits()
    } else {
        key.to_bits()
    }
}

unsafe fn str_key_bytes<'a>(key: u64) -> &'a [u8] {
    unsafe { CStr::from_ptr(key as *const c_char) }.to_bytes()
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A key kind's hash and equality over the raw `u64` an entry stores for
/// it (an `f64` bit pattern for numeric keys, an arena string pointer for
/// string keys).
trait KeyKind {
    fn hash(key: u64) -> u64;
    fn eq(a: u64, b: u64) -> bool;
}

struct NumKey;
impl KeyKind for NumKey {
    fn hash(key: u64) -> u64 {
        mix64(key)
    }
    fn eq(a: u64, b: u64) -> bool {
        a == b
    }
}

struct StrKey;
impl KeyKind for StrKey {
    fn hash(key: u64) -> u64 {
        hash_bytes(unsafe { str_key_bytes(key) })
    }
    fn eq(a: u64, b: u64) -> bool {
        unsafe { str_key_bytes(a) == str_key_bytes(b) }
    }
}

fn arena_alloc_entries(capacity: u64) -> *mut MapEntry {
    let bytes = thaw_arena::thaw_arena_alloc(
        capacity as usize * std::mem::size_of::<MapEntry>(),
        std::mem::align_of::<MapEntry>(),
    );
    if bytes.is_null() {
        return std::ptr::null_mut();
    }
    let entries = bytes.cast::<MapEntry>();
    for index in 0..capacity {
        unsafe {
            entries.add(index as usize).write(MapEntry {
                state: ENTRY_EMPTY,
                hash: 0,
                key: 0,
                value: 0,
            });
        }
    }
    entries
}

fn arena_alloc_buckets(capacity: u64) -> *mut i64 {
    let bytes = thaw_arena::thaw_arena_alloc(
        capacity as usize * std::mem::size_of::<i64>(),
        std::mem::align_of::<i64>(),
    );
    if bytes.is_null() {
        return std::ptr::null_mut();
    }
    let buckets = bytes.cast::<i64>();
    for index in 0..capacity {
        unsafe { buckets.add(index as usize).write(BUCKET_EMPTY) };
    }
    buckets
}

unsafe fn header_of<'a>(map: *mut u8) -> &'a mut MapHeader {
    unsafe { &mut *map.cast::<MapHeader>() }
}

/// Finds `key`'s bucket. The second element of the pair is the matching
/// entry's index when the key is present; when absent, the first element
/// names an empty or tombstoned bucket a subsequent insert can reuse
/// (callers must `ensure_capacity_for_insert` first, since an insert must
/// never target a table with no free buckets).
unsafe fn find_bucket<K: KeyKind>(header: &MapHeader, key: u64, hash: u64) -> (usize, Option<usize>) {
    let mask = header.buckets_len - 1;
    let mut index = (hash & mask) as usize;
    let mut reusable: Option<usize> = None;
    loop {
        let bucket = unsafe { *header.buckets.add(index) };
        if bucket == BUCKET_EMPTY {
            return (reusable.unwrap_or(index), None);
        }
        if bucket == BUCKET_TOMBSTONE {
            reusable.get_or_insert(index);
        } else {
            let entry_index = bucket as usize;
            let entry = unsafe { *header.entries.add(entry_index) };
            if entry.state == ENTRY_LIVE && entry.hash == hash && K::eq(entry.key, key) {
                return (index, Some(entry_index));
            }
        }
        index = (index + 1) & mask as usize;
    }
}

/// Reallocates entries/buckets, compacting out tombstones and rebuilding
/// the bucket index from each surviving entry's already-known hash.
/// Returns `false` (leaving `header` unchanged) only on allocation
/// failure.
fn grow(header: &mut MapHeader) -> bool {
    let new_cap = (header.entries_cap.max(4)) * 2;
    let new_entries = arena_alloc_entries(new_cap);
    if new_entries.is_null() {
        return false;
    }
    let new_buckets_len = (new_cap * 2).next_power_of_two().max(INITIAL_BUCKETS);
    let new_buckets = arena_alloc_buckets(new_buckets_len);
    if new_buckets.is_null() {
        return false;
    }
    let mask = new_buckets_len - 1;
    let mut new_len = 0u64;
    for index in 0..header.entries_len {
        let entry = unsafe { *header.entries.add(index as usize) };
        if entry.state != ENTRY_LIVE {
            continue;
        }
        unsafe { new_entries.add(new_len as usize).write(entry) };
        let mut bucket_index = (entry.hash & mask) as usize;
        loop {
            if unsafe { *new_buckets.add(bucket_index) } == BUCKET_EMPTY {
                unsafe { new_buckets.add(bucket_index).write(new_len as i64) };
                break;
            }
            bucket_index = (bucket_index + 1) & mask as usize;
        }
        new_len += 1;
    }
    header.entries = new_entries;
    header.entries_len = new_len;
    header.entries_cap = new_cap;
    header.live = new_len;
    header.buckets = new_buckets;
    header.buckets_len = new_buckets_len;
    true
}

fn ensure_capacity_for_insert(header: &mut MapHeader) -> bool {
    let needs_growth = header.buckets_len == 0
        || header.entries_len >= header.entries_cap
        || header.entries_len * 10 >= header.buckets_len * 7;
    if needs_growth {
        grow(header)
    } else {
        true
    }
}

unsafe fn map_get<K: KeyKind>(map: *const u8, key: u64) -> Option<u64> {
    if map.is_null() {
        return None;
    }
    let header = unsafe { &*map.cast::<MapHeader>() };
    if header.buckets_len == 0 {
        return None;
    }
    let hash = K::hash(key);
    let (_, entry_index) = unsafe { find_bucket::<K>(header, key, hash) };
    entry_index.map(|index| unsafe { (*header.entries.add(index)).value })
}

unsafe fn map_has<K: KeyKind>(map: *const u8, key: u64) -> bool {
    unsafe { map_get::<K>(map, key) }.is_some()
}

/// Inserts or updates `key`. Returns `false` only on arena allocation
/// failure during growth.
unsafe fn map_set<K: KeyKind>(map: *mut u8, key: u64, value: u64) -> bool {
    if map.is_null() {
        return false;
    }
    let header = unsafe { header_of(map) };
    let hash = K::hash(key);
    if header.buckets_len != 0 {
        let (_, entry_index) = unsafe { find_bucket::<K>(header, key, hash) };
        if let Some(index) = entry_index {
            unsafe { (*header.entries.add(index)).value = value };
            return true;
        }
    }
    if !ensure_capacity_for_insert(header) {
        return false;
    }
    let (bucket_index, _) = unsafe { find_bucket::<K>(header, key, hash) };
    let entry_index = header.entries_len;
    unsafe {
        header.entries.add(entry_index as usize).write(MapEntry {
            state: ENTRY_LIVE,
            hash,
            key,
            value,
        });
        header.buckets.add(bucket_index).write(entry_index as i64);
    }
    header.entries_len += 1;
    header.live += 1;
    true
}

unsafe fn map_delete<K: KeyKind>(map: *mut u8, key: u64) -> bool {
    if map.is_null() {
        return false;
    }
    let header = unsafe { header_of(map) };
    if header.buckets_len == 0 {
        return false;
    }
    let hash = K::hash(key);
    let (bucket_index, entry_index) = unsafe { find_bucket::<K>(header, key, hash) };
    let Some(entry_index) = entry_index else {
        return false;
    };
    unsafe {
        (*header.entries.add(entry_index)).state = ENTRY_TOMBSTONE;
        header.buckets.add(bucket_index).write(BUCKET_TOMBSTONE);
    }
    header.live -= 1;
    true
}

#[no_mangle]
/// Allocates an empty `Map`/`Set` backing structure. The returned pointer
/// is null only on arena allocation failure.
///
/// # Safety
/// The returned pointer is valid until the next `thaw_arena_reset`.
pub unsafe extern "C" fn thaw_map_new() -> *mut u8 {
    let header_bytes = thaw_arena::thaw_arena_alloc(
        std::mem::size_of::<MapHeader>(),
        std::mem::align_of::<MapHeader>(),
    );
    if header_bytes.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        header_bytes.cast::<MapHeader>().write(MapHeader {
            entries: std::ptr::null_mut(),
            entries_len: 0,
            entries_cap: 0,
            live: 0,
            buckets: std::ptr::null_mut(),
            buckets_len: 0,
        });
    }
    header_bytes
}

#[no_mangle]
/// `Map.prototype.size`/`Set.prototype.size`. Returns `0` for a null map.
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_map_size(map: *const u8) -> f64 {
    if map.is_null() {
        return 0.0;
    }
    unsafe { &*map.cast::<MapHeader>() }.live as f64
}

#[no_mangle]
/// `Map.prototype.clear`/`Set.prototype.clear`.
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_map_clear(map: *mut u8) {
    if map.is_null() {
        return;
    }
    let header = unsafe { header_of(map) };
    header.entries = std::ptr::null_mut();
    header.entries_len = 0;
    header.entries_cap = 0;
    header.live = 0;
    header.buckets = std::ptr::null_mut();
    header.buckets_len = 0;
}

/// Builds a native array (`[i64 length][word0][word1]...]`, the same
/// layout `HirExpr::ArrayLit` itself produces) of one raw 64-bit word per
/// live entry, in insertion order, via `extract`. Reused for key
/// snapshots, value snapshots, and (with `extract` returning a small
/// arena-allocated 2-word tuple's address) entry-pair snapshots -- the
/// words are never reinterpreted here, so this same helper works
/// regardless of whether the caller means them as `f64` bits, a pointer,
/// or a packed boolean; the array's `HirType::Array(_)` element type
/// (known at HIR lowering time, not here) tells codegen how to read each
/// slot back.
fn map_snapshot(map: *const u8, extract: impl Fn(&MapEntry) -> u64) -> *mut u8 {
    let live = if map.is_null() {
        0
    } else {
        unsafe { &*map.cast::<MapHeader>() }.live
    };
    let output = thaw_arena::thaw_arena_alloc((live as usize + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { output.cast::<i64>().write(live as i64) };
    if map.is_null() {
        return output;
    }
    let header = unsafe { &*map.cast::<MapHeader>() };
    let mut written = 0u64;
    for index in 0..header.entries_len {
        let entry = unsafe { *header.entries.add(index as usize) };
        if entry.state != ENTRY_LIVE {
            continue;
        }
        unsafe {
            output
                .add(8 + written as usize * 8)
                .cast::<u64>()
                .write_unaligned(extract(&entry));
        }
        written += 1;
    }
    output
}

#[no_mangle]
/// `Map.prototype.keys`/`Set.prototype.keys`/`Set.prototype.values`
/// (`Set` has no separate key from its element, so both read this): the
/// live keys/elements in insertion order as a native array whose element
/// type the generated code already knows statically.
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_map_snapshot_keys(map: *const u8) -> *mut u8 {
    map_snapshot(map, |entry| entry.key)
}

#[no_mangle]
/// `Map.prototype.values`: the live values in insertion order.
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_map_snapshot_values(map: *const u8) -> *mut u8 {
    map_snapshot(map, |entry| entry.value)
}

/// Allocates a native 2-tuple (`[i64 length=2][first][second]`, the same
/// layout an `HirExpr::ArrayLit` of two elements produces) and returns its
/// address as a `u64`, or `0` only on arena allocation failure.
fn arena_pair(first: u64, second: u64) -> u64 {
    let pair = thaw_arena::thaw_arena_alloc(24, 8);
    if pair.is_null() {
        return 0;
    }
    unsafe {
        pair.cast::<i64>().write(2);
        pair.add(8).cast::<u64>().write_unaligned(first);
        pair.add(16).cast::<u64>().write_unaligned(second);
    }
    pair as u64
}

#[no_mangle]
/// `Map.prototype.entries`: a native array of `[key, value]` 2-tuples, one
/// per live entry in insertion order. Returns a null pointer only on
/// arena allocation failure.
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_map_snapshot_entries(map: *const u8) -> *mut u8 {
    map_snapshot(map, |entry| arena_pair(entry.key, entry.value))
}

#[no_mangle]
/// `Set.prototype.entries`: a native array of `[value, value]` 2-tuples,
/// matching the specification (a `Set` has no separate key from its
/// element, so both tuple slots repeat it).
///
/// # Safety
/// `map` must be null or a pointer returned by `thaw_map_new`.
pub unsafe extern "C" fn thaw_set_snapshot_entries(map: *const u8) -> *mut u8 {
    map_snapshot(map, |entry| arena_pair(entry.key, entry.key))
}

fn encode_num_key(key: f64) -> u64 {
    canonical_num_key(key)
}

// Reinterprets the string pointer's own bits as the entry's `u64` key
// slot; `StrKey`'s hash/eq dereference it as a C string.
fn encode_str_key(key: *const c_char) -> u64 {
    key as u64
}

// A reference-identity key (`Object`, `Array`, `Function`, `Map`, `Set`,
// ...) is just its own pointer bits, hashed and compared directly with no
// dereference -- `NumKey`'s hash/eq (a `mix64` scramble plus equality on
// the raw word) are exactly what that needs, since it never canonicalizes
// its input the way `encode_num_key` does for `SameValueZero`.
fn encode_ref_key(key: *const u8) -> u64 {
    key as u64
}

macro_rules! key_kind_functions {
    ($kind:ty, $key_ty:ty, $encode:ident, $has:ident, $set:ident, $delete:ident, $get_f64:ident, $get_bool:ident, $get_ptr:ident) => {
        #[no_mangle]
        /// # Safety
        /// `map` must be null or a pointer returned by `thaw_map_new`; a
        /// string key must be a valid NUL-terminated UTF-8 string.
        pub unsafe extern "C" fn $has(map: *const u8, key: $key_ty) -> u8 {
            unsafe { map_has::<$kind>(map, $encode(key)) as u8 }
        }

        #[no_mangle]
        /// Returns `0` only on arena allocation failure during growth.
        ///
        /// # Safety
        /// `map` must be a pointer returned by `thaw_map_new`; a string
        /// key must be a valid NUL-terminated UTF-8 string; `value` must
        /// be a bit pattern this key/value pair's codegen produced (see
        /// `__thaw_value_to_word` in `thaw-llvm`).
        pub unsafe extern "C" fn $set(map: *mut u8, key: $key_ty, value: u64) -> u8 {
            unsafe { map_set::<$kind>(map, $encode(key), value) as u8 }
        }

        #[no_mangle]
        /// Returns whether `key` was present (and thus removed).
        ///
        /// # Safety
        /// `map` must be null or a pointer returned by `thaw_map_new`; a
        /// string key must be a valid NUL-terminated UTF-8 string.
        pub unsafe extern "C" fn $delete(map: *mut u8, key: $key_ty) -> u8 {
            unsafe { map_delete::<$kind>(map, $encode(key)) as u8 }
        }

        #[no_mangle]
        /// Decodes the value word as an `f64`. Meaningless unless a
        /// matching `*_has` call first confirmed the key is present;
        /// chosen at HIR lowering time when the map's value type is `F64`.
        ///
        /// # Safety
        /// Same as the getter family above.
        pub unsafe extern "C" fn $get_f64(map: *const u8, key: $key_ty) -> f64 {
            f64::from_bits(unsafe { map_get::<$kind>(map, $encode(key)) }.unwrap_or(0))
        }

        #[no_mangle]
        /// Decodes the value word as a boolean; chosen at HIR lowering
        /// time when the map's value type is `Bool`/`Undefined`/`Null`.
        ///
        /// # Safety
        /// Same as the getter family above.
        pub unsafe extern "C" fn $get_bool(map: *const u8, key: $key_ty) -> u8 {
            (unsafe { map_get::<$kind>(map, $encode(key)) }.unwrap_or(0) != 0) as u8
        }

        #[no_mangle]
        /// Decodes the value word as a pointer; chosen at HIR lowering
        /// time for every other supported value type (`Str`, `Array`,
        /// `Object`, `Function`, `Promise`, `Map`, `Set`, ...).
        ///
        /// # Safety
        /// Same as the getter family above.
        pub unsafe extern "C" fn $get_ptr(map: *const u8, key: $key_ty) -> *mut u8 {
            unsafe { map_get::<$kind>(map, $encode(key)) }.unwrap_or(0) as *mut u8
        }
    };
}

key_kind_functions!(
    NumKey,
    f64,
    encode_num_key,
    thaw_map_num_has,
    thaw_map_num_set,
    thaw_map_num_delete,
    thaw_map_num_get_f64,
    thaw_map_num_get_bool,
    thaw_map_num_get_ptr
);

key_kind_functions!(
    StrKey,
    *const c_char,
    encode_str_key,
    thaw_map_str_has,
    thaw_map_str_set,
    thaw_map_str_delete,
    thaw_map_str_get_f64,
    thaw_map_str_get_bool,
    thaw_map_str_get_ptr
);

key_kind_functions!(
    NumKey,
    *const u8,
    encode_ref_key,
    thaw_map_ref_has,
    thaw_map_ref_set,
    thaw_map_ref_delete,
    thaw_map_ref_get_f64,
    thaw_map_ref_get_bool,
    thaw_map_ref_get_ptr
);

#[cfg(test)]
mod map_native_tests {
    use super::*;

    fn value(bits: f64) -> u64 {
        bits.to_bits()
    }

    #[test]
    fn num_keyed_round_trip() {
        let map = unsafe { thaw_map_new() };
        assert_eq!(unsafe { thaw_map_size(map) }, 0.0);
        assert_eq!(unsafe { thaw_map_num_has(map, 1.0) }, 0);
        assert_eq!(unsafe { thaw_map_num_set(map, 1.0, value(100.0)) }, 1);
        assert_eq!(unsafe { thaw_map_num_has(map, 1.0) }, 1);
        assert_eq!(unsafe { thaw_map_num_get_f64(map, 1.0) }, 100.0);
        assert_eq!(unsafe { thaw_map_size(map) }, 1.0);
        assert_eq!(unsafe { thaw_map_num_delete(map, 1.0) }, 1);
        assert_eq!(unsafe { thaw_map_num_delete(map, 1.0) }, 0);
        assert_eq!(unsafe { thaw_map_size(map) }, 0.0);
        assert_eq!(unsafe { thaw_map_num_has(map, 1.0) }, 0);
    }

    #[test]
    fn same_value_zero_collapses_signed_zero_and_nan() {
        let map = unsafe { thaw_map_new() };
        unsafe { thaw_map_num_set(map, 0.0, value(1.0)) };
        assert_eq!(unsafe { thaw_map_num_has(map, -0.0) }, 1);
        assert_eq!(unsafe { thaw_map_num_get_f64(map, -0.0) }, 1.0);
        unsafe { thaw_map_num_set(map, f64::NAN, value(2.0)) };
        assert_eq!(unsafe { thaw_map_num_has(map, f64::NAN) }, 1);
        assert_eq!(unsafe { thaw_map_num_get_f64(map, f64::NAN) }, 2.0);
        assert_eq!(unsafe { thaw_map_size(map) }, 2.0);
    }

    #[test]
    fn overwriting_a_key_does_not_grow_size() {
        let map = unsafe { thaw_map_new() };
        unsafe { thaw_map_num_set(map, 1.0, value(1.0)) };
        unsafe { thaw_map_num_set(map, 1.0, value(2.0)) };
        assert_eq!(unsafe { thaw_map_size(map) }, 1.0);
        assert_eq!(unsafe { thaw_map_num_get_f64(map, 1.0) }, 2.0);
    }

    #[test]
    fn str_keyed_round_trip() {
        let map = unsafe { thaw_map_new() };
        let a = std::ffi::CString::new("alpha").unwrap();
        let b = std::ffi::CString::new("beta").unwrap();
        unsafe { thaw_map_str_set(map, a.as_ptr(), value(1.0)) };
        unsafe { thaw_map_str_set(map, b.as_ptr(), value(2.0)) };
        assert_eq!(unsafe { thaw_map_str_get_f64(map, a.as_ptr()) }, 1.0);
        assert_eq!(unsafe { thaw_map_str_get_f64(map, b.as_ptr()) }, 2.0);
        // A distinct pointer with the same *content* is still the same key.
        let a_again = std::ffi::CString::new("alpha").unwrap();
        assert_eq!(unsafe { thaw_map_str_has(map, a_again.as_ptr()) }, 1);
        assert_eq!(unsafe { thaw_map_str_delete(map, a_again.as_ptr()) }, 1);
        assert_eq!(unsafe { thaw_map_size(map) }, 1.0);
    }

    #[test]
    fn ref_keyed_uses_pointer_identity_not_content() {
        let map = unsafe { thaw_map_new() };
        // Two distinct allocations that happen to hold identical bytes are
        // still two different keys -- unlike `Str`, this is by reference.
        let a = std::ffi::CString::new("same bytes").unwrap();
        let b = std::ffi::CString::new("same bytes").unwrap();
        assert_ne!(a.as_ptr(), b.as_ptr());
        unsafe { thaw_map_ref_set(map, a.as_ptr().cast::<u8>(), value(1.0)) };
        unsafe { thaw_map_ref_set(map, b.as_ptr().cast::<u8>(), value(2.0)) };
        assert_eq!(unsafe { thaw_map_size(map) }, 2.0);
        assert_eq!(unsafe { thaw_map_ref_get_f64(map, a.as_ptr().cast::<u8>()) }, 1.0);
        assert_eq!(unsafe { thaw_map_ref_get_f64(map, b.as_ptr().cast::<u8>()) }, 2.0);
        // The SAME pointer is of course still the same key.
        assert_eq!(unsafe { thaw_map_ref_has(map, a.as_ptr().cast::<u8>()) }, 1);
        assert_eq!(unsafe { thaw_map_ref_delete(map, a.as_ptr().cast::<u8>()) }, 1);
        assert_eq!(unsafe { thaw_map_size(map) }, 1.0);
    }

    #[test]
    fn survives_growth_and_preserves_insertion_order() {
        let map = unsafe { thaw_map_new() };
        for i in 0..500 {
            assert_eq!(unsafe { thaw_map_num_set(map, i as f64, value(i as f64 * 2.0)) }, 1);
        }
        assert_eq!(unsafe { thaw_map_size(map) }, 500.0);
        for i in 0..500 {
            assert_eq!(unsafe { thaw_map_num_has(map, i as f64) }, 1);
            assert_eq!(unsafe { thaw_map_num_get_f64(map, i as f64) }, i as f64 * 2.0);
        }
        // Delete every third entry, then confirm the survivors are intact
        // and still retrievable after further growth-triggering inserts.
        for i in (0..500).step_by(3) {
            assert_eq!(unsafe { thaw_map_num_delete(map, i as f64) }, 1);
        }
        for i in 500..800 {
            unsafe { thaw_map_num_set(map, i as f64, value(i as f64 * 2.0)) };
        }
        for i in 0..800 {
            let expected_present = i >= 500 || i % 3 != 0;
            assert_eq!(
                unsafe { thaw_map_num_has(map, i as f64) },
                expected_present as u8,
                "key {i}"
            );
            if expected_present {
                assert_eq!(unsafe { thaw_map_num_get_f64(map, i as f64) }, i as f64 * 2.0);
            }
        }

        // White-box: entries must still be in insertion order after the
        // compacting growth above, skipping tombstoned (deleted) ones.
        let header = unsafe { &*map.cast::<MapHeader>() };
        let mut previous_key: Option<u64> = None;
        let mut live_count = 0u64;
        for index in 0..header.entries_len {
            let entry = unsafe { *header.entries.add(index as usize) };
            if entry.state != ENTRY_LIVE {
                continue;
            }
            if let Some(previous) = previous_key {
                assert!(
                    f64::from_bits(previous) < f64::from_bits(entry.key),
                    "insertion order violated"
                );
            }
            previous_key = Some(entry.key);
            live_count += 1;
        }
        assert_eq!(live_count, header.live);
    }

    unsafe fn read_word_array(array: *mut u8) -> Vec<u64> {
        let length = unsafe { array.cast::<i64>().read() };
        (0..length)
            .map(|index| unsafe { array.add(8 + index as usize * 8).cast::<u64>().read_unaligned() })
            .collect()
    }

    #[test]
    fn snapshots_reflect_insertion_order_and_skip_deletions() {
        let map = unsafe { thaw_map_new() };
        for (key, value_bits) in [(1.0, 10.0), (2.0, 20.0), (3.0, 30.0)] {
            unsafe { thaw_map_num_set(map, key, value(value_bits)) };
        }
        unsafe { thaw_map_num_delete(map, 2.0) };

        let keys = unsafe { read_word_array(thaw_map_snapshot_keys(map)) };
        assert_eq!(
            keys.into_iter().map(f64::from_bits).collect::<Vec<_>>(),
            vec![1.0, 3.0]
        );

        let values = unsafe { read_word_array(thaw_map_snapshot_values(map)) };
        assert_eq!(
            values.into_iter().map(f64::from_bits).collect::<Vec<_>>(),
            vec![10.0, 30.0]
        );

        let entries = unsafe { read_word_array(thaw_map_snapshot_entries(map)) };
        assert_eq!(entries.len(), 2);
        let pairs: Vec<(f64, f64)> = entries
            .iter()
            .map(|&pointer| {
                let pair = pointer as *mut u8;
                let key = f64::from_bits(unsafe { pair.add(8).cast::<u64>().read() });
                let value = f64::from_bits(unsafe { pair.add(16).cast::<u64>().read() });
                (key, value)
            })
            .collect();
        assert_eq!(pairs, vec![(1.0, 10.0), (3.0, 30.0)]);
    }

    #[test]
    fn snapshots_of_null_or_empty_map_are_empty_arrays() {
        assert_eq!(unsafe { read_word_array(thaw_map_snapshot_keys(std::ptr::null())) }, Vec::<u64>::new());
        let map = unsafe { thaw_map_new() };
        assert_eq!(unsafe { read_word_array(thaw_map_snapshot_values(map)) }, Vec::<u64>::new());
    }
}
