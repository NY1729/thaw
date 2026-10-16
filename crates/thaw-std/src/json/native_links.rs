//! Links between a Json array and the typed native array a copy of it was written into.
//!
//! A Json value written through an `any` view into a typed native slot (`number[][]`,
//! `string[][]`, `boolean[][]`) is decoded into a fresh native record. Without a link the two
//! diverge; with one, every mutation of the Json array is mirrored into the native record and
//! every mutation of the record by typed code is pulled back into the Json array on its next
//! access, so both sides observe one array like in JS.
//!
//! The native array handle is stable (`handle -> {data, presence}`), only its data block is
//! swapped, so a link stays valid across typed `push`/`resize`. Nested containers
//! (`number[][][]`, records with array/record fields) are linked recursively: the parent treats a
//! nested slot as a pointer word and every Json child has its own link to the matching native child.
//!
//! ponytail: pull compares the whole native array with a snapshot on each access of a linked
//! array (O(n)); keep it while links are rare, add a version word to the native header if not.

use super::*;
use std::collections::HashMap;

type Field = (Vec<u8>, usize, Kind);

/// What an 8-byte native slot holds; `Array`/`Record` slots hold a pointer to a nested container.
#[derive(Clone, PartialEq, Debug)]
enum Kind {
    F64,
    Bool,
    Str,
    Array(Rc<Kind>),
    Record(Rc<Vec<Field>>),
}

impl Kind {
    fn nested(&self) -> bool {
        matches!(self, Kind::Array(_) | Kind::Record(_))
    }
}

enum Target {
    /// `handle -> {data, presence}` of a flat `T[]`.
    Array { handle: usize, kind: Kind },
    /// A record of 8-byte field slots: `(key, offset, kind)` per field.
    Object { record: usize, fields: Vec<Field> },
}

/// The Json container a link mirrors (weak: the link must not keep it alive).
enum JsonRef {
    Array(std::rc::Weak<SharedValue<Vec<Value>>>),
    Object(std::rc::Weak<SharedValue<indexmap::IndexMap<Vec<u8>, Value>>>),
}

impl JsonRef {
    fn upgrade(&self) -> Option<Value> {
        match self {
            JsonRef::Array(weak) => weak.upgrade().map(Value::Array),
            JsonRef::Object(weak) => weak.upgrade().map(Value::Object),
        }
    }
}

fn json_ref(value: &Value) -> Option<(usize, JsonRef)> {
    match value {
        Value::Array(rc) => Some((Rc::as_ptr(rc) as usize, JsonRef::Array(Rc::downgrade(rc)))),
        Value::Object(rc) => Some((Rc::as_ptr(rc) as usize, JsonRef::Object(Rc::downgrade(rc)))),
        _ => None,
    }
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
    json: JsonRef,
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

unsafe fn read_word(pointer: *const u8, kind: &Kind) -> u64 {
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
                .map(|index| unsafe { read_word(data.add(8 + index * 8), kind) })
                .collect()
        }
        Target::Object { record, fields } => fields
            .iter()
            .map(|(_, offset, kind)| unsafe {
                read_word((*record as *const u8).add(*offset), kind)
            })
            .collect(),
    }
}

fn decode_word(word: u64, kind: &Kind) -> Value {
    match kind {
        Kind::F64 => number_value(f64::from_bits(word)),
        Kind::Bool => Value::Bool(word != 0),
        Kind::Str => string_value(word as usize as *const c_char),
        Kind::Array(_) | Kind::Record(_) => json_for_pointer(word, kind),
    }
}

/// The Json container for a nested native pointer: the one already linked to it (identity), or a
/// fresh one decoded from the native value and linked so later changes keep flowing.
fn json_for_pointer(word: u64, kind: &Kind) -> Value {
    if word == 0 {
        return Value::Null;
    }
    if let Some(existing) = container_for_target(word as usize) {
        return existing;
    }
    let pointer = word as usize;
    let (value, target) = match kind {
        Kind::Array(elem) => {
            let target = Target::Array {
                handle: pointer,
                kind: (**elem).clone(),
            };
            let items = unsafe { read_native(&target) }
                .into_iter()
                .map(|raw| decode_word(raw, elem))
                .collect();
            (Value::shared_array(items), target)
        }
        Kind::Record(fields) => {
            let target = Target::Object {
                record: pointer,
                fields: (**fields).clone(),
            };
            let map = fields
                .iter()
                .map(|(key, offset, kind)| {
                    let raw = unsafe { read_word((pointer as *const u8).add(*offset), kind) };
                    (key.clone(), decode_word(raw, kind))
                })
                .collect();
            (Value::shared_object(map), target)
        }
        _ => return Value::Null,
    };
    add_link(&value, target);
    value
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
fn encode_word(item: &Value, kind: &Kind, previous: Option<u64>) -> u64 {
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
        Kind::Array(_) | Kind::Record(_) => native_child(item, kind, previous),
    }
}

/// The native container behind a nested Json child: the one it is still linked to, else a new
/// native copy (linked, so both sides keep following each other).
fn native_child(item: &Value, kind: &Kind, previous: Option<u64>) -> u64 {
    let Some((address, _)) = json_ref(item) else {
        return previous.unwrap_or(0);
    };
    if let Some(old) = previous {
        if has_link_to(address, old as usize) {
            return old;
        }
    }
    match new_native_copy(item, kind) {
        Some(pointer) => {
            add_link(item, target_for(pointer as usize, kind));
            pointer
        }
        None => previous.unwrap_or(0),
    }
}

fn has_link_to(address: usize, pointer: usize) -> bool {
    LINKS.with(|links| {
        links
            .borrow()
            .get(&address)
            .is_some_and(|entries| entries.iter().any(|link| link.target.address() == pointer))
    })
}

fn target_for(pointer: usize, kind: &Kind) -> Target {
    match kind {
        Kind::Record(fields) => Target::Object {
            record: pointer,
            fields: (**fields).clone(),
        },
        Kind::Array(elem) => Target::Array {
            handle: pointer,
            kind: (**elem).clone(),
        },
        _ => Target::Array {
            handle: pointer,
            kind: Kind::F64,
        },
    }
}

/// ponytail: nesting deeper than this (or a cyclic Json value) keeps the plain copy.
const MAX_DEPTH: usize = 32;
thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A new arena copy of a Json container in the native layout `kind` describes.
fn new_native_copy(item: &Value, kind: &Kind) -> Option<u64> {
    if DEPTH.with(Cell::get) >= MAX_DEPTH {
        return None;
    }
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let copied = (|| match (kind, item) {
        (Kind::Array(elem), Value::Array(rc)) => {
            let words: Vec<u64> = unsafe { &*rc.get() }
                .iter()
                .map(|member| encode_word(member, elem, None))
                .collect();
            let handle = thaw_arena::thaw_arena_alloc(16, 8);
            if handle.is_null() {
                return None;
            }
            unsafe {
                (handle as *mut u64).write(0);
                (handle.add(8) as *mut u64).write(0);
            }
            let target = Target::Array {
                handle: handle as usize,
                kind: (**elem).clone(),
            };
            unsafe { write_native(&target, &words) }.then_some(handle as usize as u64)
        }
        (Kind::Record(fields), Value::Object(rc)) => {
            let size = fields
                .iter()
                .map(|(_, offset, _)| offset + 8)
                .max()
                .unwrap_or(8);
            let record = thaw_arena::thaw_arena_alloc(size, 8);
            if record.is_null() {
                return None;
            }
            let map = unsafe { &*rc.get() };
            let words: Vec<u64> = fields
                .iter()
                .map(|(key, _, field)| match map.get(key) {
                    Some(member) => encode_word(member, field, None),
                    None => default_word(field),
                })
                .collect();
            let target = Target::Object {
                record: record as usize,
                fields: (**fields).clone(),
            };
            unsafe { write_native(&target, &words) }.then_some(record as usize as u64)
        }
        _ => None,
    })();
    DEPTH.with(|depth| depth.set(depth.get() - 1));
    copied
}

/// A valid native value for a field the Json object does not have.
fn default_word(kind: &Kind) -> u64 {
    match kind {
        Kind::Str => thaw_arena::owned_string(b"") as usize as u64,
        Kind::Array(_) => new_native_copy(&Value::shared_array(Vec::new()), kind).unwrap_or(0),
        Kind::Record(_) => {
            new_native_copy(&Value::shared_object(indexmap::IndexMap::new()), kind).unwrap_or(0)
        }
        _ => 0,
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
///
/// The container's entries are taken out of `LINKS` while they are processed, so nested
/// `add_link`/`forget` calls (adopting or dropping nested containers) never meet a held borrow.
pub(super) fn pull(address: usize) {
    if !active() {
        return;
    }
    let Some(mut entries) = LINKS.with(|links| links.borrow_mut().remove(&address)) else {
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
                *vec_at(address) = raw.iter().map(|&word| decode_word(word, kind)).collect();
            },
            Target::Object { fields, .. } => {
                for ((key, _, kind), &word) in fields.iter().zip(&raw) {
                    let value = decode_word(word, kind);
                    unsafe { &mut *map_at(address) }.insert(key.clone(), value);
                }
            }
        }
        link.snapshot = raw;
    }
    BUSY.with(|busy| busy.set(false));
    restore(address, entries);
    run_deferred_forgets();
}

fn restore(address: usize, entries: Vec<Link>) {
    LINKS.with(|links| {
        links
            .borrow_mut()
            .entry(address)
            .or_default()
            .extend(entries)
    });
}

/// After a mutation of a Json container: mirror it into every linked native copy.
pub(super) fn push(address: usize) {
    if !active() {
        return;
    }
    let Some(mut entries) = LINKS.with(|links| links.borrow_mut().remove(&address)) else {
        return;
    };
    BUSY.with(|busy| busy.set(true));
    for link in entries.iter_mut() {
        let desired: Vec<u64> = match &link.target {
            Target::Array { kind, .. } => unsafe { &*vec_at(address) }
                .iter()
                .enumerate()
                .map(|(index, item)| encode_word(item, kind, link.snapshot.get(index).copied()))
                .collect(),
            Target::Object { fields, .. } => {
                let map = unsafe { &*map_at(address) };
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, (key, _, kind))| match map.get(key) {
                        Some(item) => encode_word(item, kind, link.snapshot.get(index).copied()),
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
    restore(address, entries);
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

fn add_link(container: &Value, target: Target) {
    let Some((address, json)) = json_ref(container) else {
        return;
    };
    let snapshot = unsafe { read_native(&target) };
    thaw_arena::register_reset_hook(reset);
    LINKS.with(|links| {
        links.borrow_mut().entry(address).or_default().push(Link {
            target,
            snapshot,
            json,
        });
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
        Some((_, container @ Value::Array(_))) => add_link(
            &container,
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
    let Some((_, container @ Value::Object(_))) = origin_by_serial(serial) else {
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
        &container,
        Target::Object {
            record: record as usize,
            fields,
        },
    );
}

/// Links a Json container tree to the nested typed native value it was just copied into.
/// `layout`: `n`/`b`/`s` for primitives, `A<elem>` for an array, `R<count>;` then per field
/// `<keylen>:<key><offset>:<field layout>` for a record (see `parse_kind`).
///
/// # Safety
/// `native` must be null or a live native array handle/record matching `layout`; `layout` a
/// NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_link_native_tree_by_serial(
    serial: u64,
    native: *mut u8,
    layout: *const c_char,
) {
    if serial == 0 || native.is_null() || layout.is_null() {
        return note_view_copy();
    }
    let Some((_, container)) = origin_by_serial(serial) else {
        return note_view_copy();
    };
    let layout = unsafe { std::ffi::CStr::from_ptr(layout) }.to_bytes();
    match parse_kind(layout) {
        Some((kind, rest)) if rest.is_empty() && link_tree(&container, &kind, native as usize) => {}
        _ => note_view_copy(),
    }
}

fn parse_number(text: &[u8], end: u8) -> Option<(usize, &[u8])> {
    let stop = text.iter().position(|&byte| byte == end)?;
    let value = std::str::from_utf8(&text[..stop]).ok()?.parse().ok()?;
    Some((value, &text[stop + 1..]))
}

fn parse_kind(text: &[u8]) -> Option<(Kind, &[u8])> {
    let (&first, rest) = text.split_first()?;
    match first {
        b'n' => Some((Kind::F64, rest)),
        b'b' => Some((Kind::Bool, rest)),
        b's' => Some((Kind::Str, rest)),
        b'A' => {
            let (elem, rest) = parse_kind(rest)?;
            Some((Kind::Array(Rc::new(elem)), rest))
        }
        b'R' => {
            let (count, mut rest) = parse_number(rest, b';')?;
            let mut fields = Vec::new();
            for _ in 0..count {
                let (key_len, after) = parse_number(rest, b':')?;
                let key = after.get(..key_len)?.to_vec();
                let (offset, after) = parse_number(&after[key_len..], b':')?;
                let (kind, after) = parse_kind(after)?;
                fields.push((key, offset, kind));
                rest = after;
            }
            Some((Kind::Record(Rc::new(fields)), rest))
        }
        _ => None,
    }
}

/// Links `container` and (recursively) its nested Json containers to the native value at
/// `pointer`; false if the shapes disagree.
fn link_tree(container: &Value, kind: &Kind, pointer: usize) -> bool {
    let target = match (kind, container) {
        (Kind::Array(elem), Value::Array(_)) => Target::Array {
            handle: pointer,
            kind: (**elem).clone(),
        },
        (Kind::Record(fields), Value::Object(_)) => Target::Object {
            record: pointer,
            fields: (**fields).clone(),
        },
        _ => return false,
    };
    let words = unsafe { read_native(&target) };
    match (&target, container) {
        (Target::Array { kind: elem, .. }, Value::Array(rc)) if elem.nested() => {
            for (member, &word) in unsafe { &*rc.get() }.iter().zip(&words) {
                if word != 0 {
                    link_tree(member, elem, word as usize);
                }
            }
        }
        (Target::Object { fields, .. }, Value::Object(rc)) => {
            let map = unsafe { &*rc.get() };
            for ((key, _, field), &word) in fields.iter().zip(&words) {
                if let (true, Some(member), true) = (field.nested(), map.get(key), word != 0) {
                    link_tree(member, field, word as usize);
                }
            }
        }
        _ => {}
    }
    add_link(container, target);
    true
}

/// The live Json container linked to the native array handle / record at `pointer`.
fn container_for_target(pointer: usize) -> Option<Value> {
    if LINK_COUNT.with(Cell::get) == 0 || pointer == 0 {
        return None;
    }
    LINKS.with(|links| {
        links.borrow().values().find_map(|entries| {
            entries
                .iter()
                .find(|link| link.target.address() == pointer)
                .and_then(|link| link.json.upgrade())
        })
    })
}

/// Whether a Json container mirrors this native array handle or record.
#[no_mangle]
pub extern "C" fn thaw_json_has_native_link(pointer: *const u8) -> u8 {
    u8::from(container_for_target(pointer as usize).is_some())
}

/// The Json container a linked native array/record is mirrored by (a fresh reference), or null.
#[no_mangle]
pub extern "C" fn thaw_json_linked_array(pointer: *const u8) -> *mut Value {
    container_for_target(pointer as usize).map_or(std::ptr::null_mut(), leak)
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
            &array,
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
        add_link(&array, target);
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
            &object,
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

    fn rows_kind() -> Kind {
        Kind::Array(Rc::new(Kind::Array(Rc::new(Kind::F64))))
    }

    /// A native `number[][]` with the given rows; returns the outer handle.
    fn native_rows(rows: &[&[f64]]) -> *mut u8 {
        let words: Vec<u64> = rows
            .iter()
            .map(|row| native_numbers(row) as usize as u64)
            .collect();
        let outer = thaw_arena::thaw_arena_alloc(16, 8);
        let target = Target::Array {
            handle: outer as usize,
            kind: Kind::Array(Rc::new(Kind::F64)),
        };
        assert!(unsafe { write_native(&target, &words) });
        outer
    }

    fn json_rows(rows: &[&[f64]]) -> Value {
        Value::shared_array(
            rows.iter()
                .map(|row| Value::shared_array(row.iter().map(|n| number_value(*n)).collect()))
                .collect(),
        )
    }

    fn native_row(outer: *mut u8, index: usize) -> *mut u8 {
        let target = Target::Array {
            handle: outer as usize,
            kind: Kind::Array(Rc::new(Kind::F64)),
        };
        let words = unsafe { read_native(&target) };
        words[index] as usize as *mut u8
    }

    #[test]
    fn nested_json_arrays_and_native_rows_follow_each_other() {
        let outer = native_rows(&[&[1.0, 2.0], &[3.0]]);
        let json = json_rows(&[&[1.0, 2.0], &[3.0]]);
        assert!(link_tree(&json, &rows_kind(), outer as usize));
        let Value::Array(rows) = &json else {
            unreachable!()
        };
        let Value::Array(first) = shared_array_ref(rows)[0].clone() else {
            unreachable!()
        };

        // Json inner push reaches the native row
        shared_array_ref_mut(&first).push(number_value(9.0));
        assert_eq!(numbers(native_row(outer, 0)), [1.0, 2.0, 9.0]);

        // a typed write to the native row is seen through the Json row
        let row = Target::Array {
            handle: native_row(outer, 1) as usize,
            kind: Kind::F64,
        };
        assert!(unsafe { write_native(&row, &[3.0f64, 4.0].map(f64::to_bits)) });
        let Value::Array(second) = shared_array_ref(rows)[1].clone() else {
            unreachable!()
        };
        assert_eq!(shared_array_ref(&second).len(), 2);

        // pushing a brand-new Json row builds a linked native row
        let added = Value::shared_array(vec![number_value(7.0)]);
        shared_array_ref_mut(rows).push(added.clone());
        assert_eq!(numbers(native_row(outer, 2)), [7.0]);
        let Value::Array(added_rc) = &added else {
            unreachable!()
        };
        shared_array_ref_mut(added_rc).push(number_value(8.0));
        assert_eq!(numbers(native_row(outer, 2)), [7.0, 8.0]);

        // typed code replaces a whole row: the outer Json array adopts a linked Json row
        let replacement = native_numbers(&[5.0]);
        let slots = Target::Array {
            handle: outer as usize,
            kind: Kind::Array(Rc::new(Kind::F64)),
        };
        let mut words = unsafe { read_native(&slots) };
        words[0] = replacement as usize as u64;
        assert!(unsafe { write_native(&slots, &words) });
        let Value::Array(adopted) = shared_array_ref(rows)[0].clone() else {
            unreachable!()
        };
        assert_eq!(shared_array_ref(&adopted).len(), 1);
        shared_array_ref_mut(&adopted).push(number_value(6.0));
        assert_eq!(numbers(replacement), [5.0, 6.0]);
    }

    #[test]
    fn parse_kind_reads_nested_layouts() {
        let (kind, rest) = parse_kind(b"R2;1:n0:n4:tags8:As").unwrap();
        assert!(rest.is_empty());
        let Kind::Record(fields) = kind else {
            panic!("record")
        };
        assert_eq!(fields[0], (b"n".to_vec(), 0, Kind::F64));
        assert_eq!(
            fields[1],
            (b"tags".to_vec(), 8, Kind::Array(Rc::new(Kind::Str)))
        );
        assert!(parse_kind(b"R1;3:ab").is_none());
    }
}
