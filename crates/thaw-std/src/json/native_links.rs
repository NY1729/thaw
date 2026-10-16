//! Links between a Json array and the typed native array a copy of it was written into.
//!
//! A Json value written through an `any` view into a typed native slot (`number[][]`,
//! `string[][]`, `boolean[][]`) is decoded into a fresh native record. Without a link the two
//! diverge; with one, every mutation of the Json array is mirrored into the native record and
//! every mutation of the record by typed code is pulled back into the Json array on its next
//! access, so both sides observe one array like in JS.
//!
//! The native array handle is stable (`handle -> {data, presence}`), only its data block is
//! swapped, so a link stays valid across typed `push`/`resize`. Only flat arrays of primitives
//! are linked: nested containers keep the plain copy.
//!
//! ponytail: pull compares the whole native array with a snapshot on each access of a linked
//! array (O(n)); keep it while links are rare, add a version word to the native header if not.

use super::*;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    F64,
    Bool,
    Str,
}

enum Target {
    /// `handle -> {data, presence}` of a flat `T[]`.
    Array { handle: usize, kind: Kind },
    /// A record of 8-byte field slots: `(key, offset, kind)` per field.
    Object {
        record: usize,
        fields: Vec<(Vec<u8>, usize, Kind)>,
    },
}

impl Target {
    fn address(&self) -> usize {
        match self {
            Target::Array { handle, .. } => *handle,
            Target::Object { record, .. } => *record,
        }
    }
}

struct Link {
    target: Target,
    snapshot: Vec<u64>,
}

thread_local! {
    static LINKS: RefCell<HashMap<usize, Vec<Link>>> = RefCell::new(HashMap::new());
    static LINK_COUNT: Cell<usize> = const { Cell::new(0) };
    static BUSY: Cell<bool> = const { Cell::new(false) };
    // Containers dropped while `LINKS` was borrowed (a pull replaced nested values).
    static DEFERRED_FORGET: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// One-time note (only when `THAW_WARN_COPY=1`) that a JavaScript-owned value written through an
/// `any` view into a typed native slot was stored as a copy: later changes on either side are not
/// shared.
fn note_view_copy() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if std::env::var_os("THAW_WARN_COPY").is_some_and(|value| value == "1") {
        ONCE.call_once(|| {
            eprintln!(
                "thaw: a JavaScript-owned object was copied into a typed native slot through an `any` view; \
                 the two sides no longer share it (declare the array as any[] to keep one object)"
            )
        });
    }
}

/// Called by compiled code when a JavaScript-owned value is stored as a plain copy.
#[no_mangle]
pub extern "C" fn thaw_json_note_view_copy() {
    note_view_copy();
}

fn active() -> bool {
    LINK_COUNT.with(Cell::get) != 0 && !BUSY.with(Cell::get)
}

unsafe fn read_word(pointer: *const u8, kind: Kind) -> u64 {
    let word = unsafe { (pointer as *const u64).read() };
    match kind {
        Kind::Bool => u64::from(word & 0xff != 0),
        _ => word,
    }
}

unsafe fn read_native(target: &Target) -> Vec<u64> {
    match target {
        Target::Array { handle, kind } => {
            let data = unsafe { (*handle as *const *const u8).read() };
            if data.is_null() {
                return Vec::new();
            }
            let length = unsafe { (data as *const u64).read() } as usize;
            (0..length)
                .map(|index| unsafe { read_word(data.add(8 + index * 8), *kind) })
                .collect()
        }
        Target::Object { record, fields } => fields
            .iter()
            .map(|(_, offset, kind)| unsafe {
                read_word((*record as *const u8).add(*offset), *kind)
            })
            .collect(),
    }
}

fn decode_word(word: u64, kind: Kind) -> Value {
    match kind {
        Kind::F64 => number_value(f64::from_bits(word)),
        Kind::Bool => Value::Bool(word != 0),
        Kind::Str => string_value(word as usize as *const c_char),
    }
}

fn string_bytes(value: &Value) -> &[u8] {
    match value {
        Value::String(text) => text.as_bytes(),
        Value::Wtf8(bytes) => bytes,
        _ => b"",
    }
}

/// The native word for a Json element; an unchanged string keeps its pointer, which typed code
/// may hold.
fn encode_word(item: &Value, kind: Kind, previous: Option<u64>) -> u64 {
    match kind {
        Kind::F64 => non_finite_number(item)
            .or_else(|| item.as_f64())
            .unwrap_or(0.0)
            .to_bits(),
        Kind::Bool => u64::from(matches!(item, Value::Bool(true))),
        Kind::Str => {
            let bytes = string_bytes(item);
            match previous {
                Some(old)
                    if unsafe {
                        thaw_arena::NativeStr::from_ptr(old as usize as *const c_char)
                    }
                    .to_bytes()
                        == bytes =>
                {
                    old
                }
                _ => thaw_arena::owned_string(bytes) as usize as u64,
            }
        }
    }
}

unsafe fn write_native(target: &Target, raw: &[u64]) -> bool {
    match target {
        Target::Array { handle, .. } => {
            let data = thaw_arena::thaw_arena_alloc(8 + raw.len() * 8, 8);
            if data.is_null() {
                return false;
            }
            unsafe {
                (data as *mut u64).write(raw.len() as u64);
                for (index, word) in raw.iter().enumerate() {
                    (data.add(8 + index * 8) as *mut u64).write(*word);
                }
                (*handle as *mut *mut u8).write(data);
                ((*handle + 8) as *mut *mut u8).write(std::ptr::null_mut());
            }
        }
        Target::Object { record, fields } => {
            for ((_, offset, _), word) in fields.iter().zip(raw) {
                unsafe { ((*record + offset) as *mut u64).write(*word) };
            }
        }
    }
    true
}

fn vec_at(address: usize) -> *mut Vec<Value> {
    unsafe { (*(address as *const SharedValue<Vec<Value>>)).get() }
}

fn map_at(address: usize) -> *mut indexmap::IndexMap<Vec<u8>, Value> {
    unsafe { (*(address as *const SharedValue<indexmap::IndexMap<Vec<u8>, Value>>)).get() }
}

/// Before any read or write of a Json container: adopt typed-side changes of a linked native copy.
pub(super) fn pull(address: usize) {
    if !active() {
        return;
    }
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let Some(entries) = links.get_mut(&address) else {
            return;
        };
        BUSY.with(|busy| busy.set(true));
        for link in entries.iter_mut() {
            let raw = unsafe { read_native(&link.target) };
            if raw == link.snapshot {
                continue;
            }
            match &link.target {
                Target::Array { kind, .. } => unsafe {
                    *vec_at(address) = raw.iter().map(|&word| decode_word(word, *kind)).collect();
                },
                Target::Object { fields, .. } => {
                    let map = unsafe { &mut *map_at(address) };
                    for ((key, _, kind), &word) in fields.iter().zip(&raw) {
                        map.insert(key.clone(), decode_word(word, *kind));
                    }
                }
            }
            link.snapshot = raw;
        }
        BUSY.with(|busy| busy.set(false));
    });
    run_deferred_forgets();
}

/// After a mutation of a Json container: mirror it into every linked native copy.
pub(super) fn push(address: usize) {
    if !active() {
        return;
    }
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let Some(entries) = links.get_mut(&address) else {
            return;
        };
        BUSY.with(|busy| busy.set(true));
        for link in entries.iter_mut() {
            let desired: Vec<u64> = match &link.target {
                Target::Array { kind, .. } => unsafe { &*vec_at(address) }
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        encode_word(item, *kind, link.snapshot.get(index).copied())
                    })
                    .collect(),
                Target::Object { fields, .. } => {
                    let map = unsafe { &*map_at(address) };
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, (key, _, kind))| match map.get(key) {
                            Some(item) => {
                                encode_word(item, *kind, link.snapshot.get(index).copied())
                            }
                            None => link.snapshot[index],
                        })
                        .collect()
                }
            };
            if desired != link.snapshot && unsafe { write_native(&link.target, &desired) } {
                link.snapshot = desired;
            }
        }
        BUSY.with(|busy| busy.set(false));
    });
    run_deferred_forgets();
}

/// The Json container is gone: its native copies continue as plain typed values.
pub(super) fn forget(address: usize) {
    if LINK_COUNT.with(Cell::get) == 0 {
        return;
    }
    let removed = LINKS.try_with(|links| match links.try_borrow_mut() {
        Ok(mut links) => {
            if let Some(entries) = links.remove(&address) {
                LINK_COUNT.with(|count| count.set(count.get() - entries.len()));
            }
            true
        }
        Err(_) => false,
    });
    if removed == Ok(false) {
        let _ = DEFERRED_FORGET.try_with(|pending| pending.borrow_mut().push(address));
    }
}

fn run_deferred_forgets() {
    let pending = DEFERRED_FORGET.with(|pending| std::mem::take(&mut *pending.borrow_mut()));
    for address in pending {
        forget(address);
    }
}

/// Drops the links whose native side the arena reclaimed. Also run first by the arena-owned root
/// cleanup: destroying a Json container pulls from its links, which must not read freed storage.
pub(super) fn reset(tracing: bool) {
    LINKS.with(|links| {
        let mut links = links.borrow_mut();
        let mut removed = 0;
        links.retain(|_, entries| {
            let before = entries.len();
            entries.retain(|link| tracing && !thaw_arena::was_reclaimed(link.target.address()));
            removed += before - entries.len();
            !entries.is_empty()
        });
        LINK_COUNT.with(|count| count.set(count.get() - removed));
    });
}

/// The live container a graph serial was handed out for.
fn origin_by_serial(serial: u64) -> Option<(usize, Value)> {
    GRAPH_ORIGIN_SERIALS.with(|table| {
        table
            .borrow()
            .1
            .iter()
            .find_map(|(address, (candidate, origin))| {
                if *candidate != serial {
                    return None;
                }
                match origin {
                    GraphOriginRef::Array(weak) => {
                        weak.upgrade().map(|rc| (*address, Value::Array(rc)))
                    }
                    GraphOriginRef::Object(weak) => {
                        weak.upgrade().map(|rc| (*address, Value::Object(rc)))
                    }
                }
            })
    })
}

fn add_link(address: usize, target: Target) {
    let snapshot = unsafe { read_native(&target) };
    thaw_arena::register_reset_hook(reset);
    LINKS.with(|links| {
        links
            .borrow_mut()
            .entry(address)
            .or_default()
            .push(Link { target, snapshot });
    });
    LINK_COUNT.with(|count| count.set(count.get() + 1));
}

/// Links the Json array that the JS shadow with this origin serial stands for to the typed
/// native array `handle` it was just copied into. `kind`: 0 number, 1 boolean, 2 string.
///
/// # Safety
/// `handle` must be null or a live native array handle whose elements match `kind`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_link_native_by_serial(serial: u64, handle: *mut u8, kind: u8) {
    let kind = match kind {
        0 => Kind::F64,
        1 => Kind::Bool,
        2 => Kind::Str,
        _ => return note_view_copy(),
    };
    if serial == 0 || handle.is_null() {
        return note_view_copy();
    }
    match origin_by_serial(serial) {
        Some((address, Value::Array(_))) => add_link(
            address,
            Target::Array {
                handle: handle as usize,
                kind,
            },
        ),
        _ => note_view_copy(),
    }
}

/// Links the Json object behind a JS shadow to the typed native record it was just copied into.
/// `layout` lists the record's fields as `key\u{1f}offset\u{1f}kind` separated by `\u{1e}`.
///
/// # Safety
/// `record` must be null or a live record matching `layout`; `layout` a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_link_native_object_by_serial(
    serial: u64,
    record: *mut u8,
    layout: *const c_char,
) {
    if serial == 0 || record.is_null() || layout.is_null() {
        return note_view_copy();
    }
    let Some((address, Value::Object(_))) = origin_by_serial(serial) else {
        return note_view_copy();
    };
    let layout = unsafe { std::ffi::CStr::from_ptr(layout) }.to_bytes();
    let mut fields = Vec::new();
    for entry in layout
        .split(|&byte| byte == 0x1e)
        .filter(|entry| !entry.is_empty())
    {
        let mut parts = entry.split(|&byte| byte == 0x1f);
        let (Some(key), Some(offset), Some(kind)) = (parts.next(), parts.next(), parts.next())
        else {
            return;
        };
        let Some(offset) = std::str::from_utf8(offset)
            .ok()
            .and_then(|text| text.parse::<usize>().ok())
        else {
            return;
        };
        let kind = match kind {
            b"0" => Kind::F64,
            b"1" => Kind::Bool,
            b"2" => Kind::Str,
            _ => return,
        };
        fields.push((key.to_vec(), offset, kind));
    }
    add_link(
        address,
        Target::Object {
            record: record as usize,
            fields,
        },
    );
}

fn linked_container(pointer: usize) -> Option<Value> {
    if LINK_COUNT.with(Cell::get) == 0 || pointer == 0 {
        return None;
    }
    let address = LINKS.with(|links| {
        links.borrow().iter().find_map(|(address, entries)| {
            entries
                .iter()
                .any(|link| link.target.address() == pointer)
                .then_some(*address)
        })
    })?;
    GRAPH_ORIGIN_SERIALS.with(|table| match table.borrow().1.get(&address)?.1 {
        GraphOriginRef::Array(ref weak) => weak.upgrade().map(Value::Array),
        GraphOriginRef::Object(ref weak) => weak.upgrade().map(Value::Object),
    })
}

/// Whether a Json container mirrors this native array handle or record.
#[no_mangle]
pub extern "C" fn thaw_json_has_native_link(pointer: *const u8) -> u8 {
    u8::from(linked_container(pointer as usize).is_some())
}

/// The Json container a linked native array/record is mirrored by (a fresh reference), or null.
#[no_mangle]
pub extern "C" fn thaw_json_linked_array(pointer: *const u8) -> *mut Value {
    linked_container(pointer as usize).map_or(std::ptr::null_mut(), leak)
}

/// Guard returned by `shared_array_ref_mut`: mirrors the mutation when it ends.
pub(super) struct ArrayMut<'a> {
    pub(super) vec: &'a mut Vec<Value>,
    pub(super) address: usize,
}

impl std::ops::Deref for ArrayMut<'_> {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> {
        self.vec
    }
}

impl std::ops::DerefMut for ArrayMut<'_> {
    fn deref_mut(&mut self) -> &mut Vec<Value> {
        self.vec
    }
}

impl Drop for ArrayMut<'_> {
    fn drop(&mut self) {
        push(self.address);
    }
}

/// Guard returned by `shared_object_ref_mut`: mirrors the mutation when it ends.
pub(super) struct ObjectMut<'a> {
    pub(super) map: &'a mut indexmap::IndexMap<Vec<u8>, Value>,
    pub(super) address: usize,
}

impl std::ops::Deref for ObjectMut<'_> {
    type Target = indexmap::IndexMap<Vec<u8>, Value>;
    fn deref(&self) -> &Self::Target {
        self.map
    }
}

impl std::ops::DerefMut for ObjectMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.map
    }
}

impl Drop for ObjectMut<'_> {
    fn drop(&mut self) {
        push(self.address);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_numbers(values: &[f64]) -> *mut u8 {
        let handle = thaw_arena::thaw_arena_alloc(16, 8);
        let target = Target::Array {
            handle: handle as usize,
            kind: Kind::F64,
        };
        let raw: Vec<u64> = values.iter().map(|value| value.to_bits()).collect();
        assert!(unsafe { write_native(&target, &raw) });
        handle
    }

    fn numbers(handle: *mut u8) -> Vec<f64> {
        let target = Target::Array {
            handle: handle as usize,
            kind: Kind::F64,
        };
        unsafe { read_native(&target) }
            .into_iter()
            .map(f64::from_bits)
            .collect()
    }

    #[test]
    fn json_array_and_native_copy_follow_each_other() {
        let handle = native_numbers(&[5.0, 6.0]);
        let array = Value::shared_array(vec![number_value(5.0), number_value(6.0)]);
        let Value::Array(items) = &array else {
            unreachable!()
        };
        add_link(
            Rc::as_ptr(items) as usize,
            Target::Array {
                handle: handle as usize,
                kind: Kind::F64,
            },
        );

        shared_array_ref_mut(items).push(number_value(7.0));
        assert_eq!(numbers(handle), [5.0, 6.0, 7.0]);

        // typed code grows the native array in place; the Json side adopts it on next access
        let target = Target::Array {
            handle: handle as usize,
            kind: Kind::F64,
        };
        let grown = [5.0f64, 6.0, 7.0, 9.0].map(f64::to_bits);
        assert!(unsafe { write_native(&target, &grown) });
        assert_eq!(shared_array_ref(items).len(), 4);
        assert_eq!(shared_array_ref(items)[3].as_f64(), Some(9.0));

        // dropping the Json array leaves the native copy as a plain typed array
        drop(array);
        assert_eq!(numbers(handle), [5.0, 6.0, 7.0, 9.0]);
        assert_eq!(LINK_COUNT.with(Cell::get), 0);
    }

    #[test]
    fn strings_keep_their_pointer_while_the_text_is_unchanged() {
        let first = thaw_arena::owned_string(b"x") as usize as u64;
        let handle = thaw_arena::thaw_arena_alloc(16, 8);
        let target = Target::Array {
            handle: handle as usize,
            kind: Kind::Str,
        };
        assert!(unsafe { write_native(&target, &[first]) });
        let array = Value::shared_array(vec![Value::String("x".into())]);
        let Value::Array(items) = &array else {
            unreachable!()
        };
        add_link(Rc::as_ptr(items) as usize, target);
        shared_array_ref_mut(items).push(Value::String("y".into()));
        let target = Target::Array {
            handle: handle as usize,
            kind: Kind::Str,
        };
        let raw = unsafe { read_native(&target) };
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[0], first);
        assert_eq!(
            unsafe { thaw_arena::NativeStr::from_ptr(raw[1] as usize as *const c_char) }.to_bytes(),
            b"y"
        );
    }

    #[test]
    fn json_object_and_native_record_follow_each_other() {
        let record = thaw_arena::thaw_arena_alloc(16, 8);
        unsafe {
            (record as *mut f64).write(2.0);
            (record.add(8) as *mut u64).write(1);
        }
        let object = Value::shared_object(indexmap::IndexMap::from([
            (b"n".to_vec(), number_value(2.0)),
            (b"ok".to_vec(), Value::Bool(true)),
        ]));
        let Value::Object(fields) = &object else {
            unreachable!()
        };
        add_link(
            Rc::as_ptr(fields) as usize,
            Target::Object {
                record: record as usize,
                fields: vec![
                    (b"n".to_vec(), 0, Kind::F64),
                    (b"ok".to_vec(), 8, Kind::Bool),
                ],
            },
        );
        shared_object_ref_mut(fields).insert(b"n".to_vec(), number_value(9.0));
        assert_eq!(unsafe { (record as *const f64).read() }, 9.0);
        unsafe { (record as *mut f64).write(10.0) };
        assert_eq!(
            shared_object_ref(fields)
                .get(b"n".as_slice())
                .and_then(Value::as_f64),
            Some(10.0)
        );
        drop(object);
        assert_eq!(LINK_COUNT.with(Cell::get), 0);
    }
}
