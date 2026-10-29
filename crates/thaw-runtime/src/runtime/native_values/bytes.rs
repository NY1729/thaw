// Byte-buffer <-> string encode/decode for `Buffer.from(str, enc)` and
// `buf.toString(enc)`. A byte buffer is a native number array (`f64`
// element slots, each a `u8` value); these read/write that layout the
// same way `arrays.rs` does. `include!`d into `lib.rs` after `arrays.rs`,
// so `native_array_length` / `arena_c_string` / the arena are in scope.

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_decode_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn encode_bytes(bytes: &[u8], encoding: &str) -> String {
    match encoding {
        "hex" => {
            let mut out = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
                out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap());
            }
            out
        }
        "base64" | "base64url" => {
            let url = encoding == "base64url";
            let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for chunk in bytes.chunks(3) {
                let b = [
                    chunk[0],
                    *chunk.get(1).unwrap_or(&0),
                    *chunk.get(2).unwrap_or(&0),
                ];
                let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
                for i in 0..4 {
                    if i <= chunk.len() {
                        out.push(BASE64_ALPHABET[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
                    } else if !url {
                        out.push('=');
                    }
                }
            }
            if url {
                out = out.replace('+', "-").replace('/', "_");
            }
            out
        }
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => String::from_utf16_lossy(
            &bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect::<Vec<_>>(),
        ),
        "ascii" => bytes.iter().map(|&b| (b & 0x7f) as char).collect(),
        "latin1" | "binary" => bytes.iter().map(|&b| b as char).collect(),
        // "utf8" / "utf-8" / anything else: lossy UTF-8 decode.
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_string(text: &[u8], encoding: &str) -> Vec<u8> {
    match encoding {
        "hex" => {
            let mut bytes = Vec::with_capacity(text.len() / 2);
            for pair in text.as_chunks::<2>().0 {
                let (Some(high), Some(low)) = ((pair[0] as char).to_digit(16), (pair[1] as char).to_digit(16)) else {
                    break;
                };
                bytes.push(((high << 4) | low) as u8);
            }
            bytes
        }
        "base64" | "base64url" => {
            let cleaned: Vec<u8> = text
                .iter()
                .copied()
                .map(|b| match b {
                    b'-' => b'+',
                    b'_' => b'/',
                    other => other,
                })
                .filter_map(base64_decode_value)
                .collect();
            let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
            for chunk in cleaned.chunks(4) {
                if chunk.len() < 2 {
                    break;
                }
                let n = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |acc, (i, &v)| acc | (u32::from(v) << (18 - i * 6)));
                out.push((n >> 16) as u8);
                if chunk.len() >= 3 {
                    out.push((n >> 8) as u8);
                }
                if chunk.len() >= 4 {
                    out.push(n as u8);
                }
            }
            out
        }
        "utf16le" | "utf-16le" | "ucs2" | "ucs-2" => {
            wtf8_decode_utf16(text).into_iter().flat_map(u16::to_le_bytes).collect()
        }
        "latin1" | "binary" | "ascii" => wtf8_decode_utf16(text).into_iter().map(|unit| unit as u8).collect(),
        _ => wtf8_to_well_formed(text),
    }
}

// A byte buffer is a pointer to a `ByteView` record over a shared `ByteStore`
// (the `ArrayBuffer`): one `u8` per byte. The compiled Buffer value is the
// ordinary array handle cell whose "data" word points at this record, so
// `.length` (record word 0) and every `compile_array_data` unwrap keep working.
// `subarray`/`slice` create a second record over the same store.

/// The backing store: an `ArrayBuffer`'s bytes.
#[repr(C)]
struct ByteStore {
    data: *mut u8,
    len: usize,
    /// Reserved for a pinned JS `ArrayBuffer` wrapper.
    js: u64,
    _reserved: u64,
}

/// A view record. `len` is first so `[len]...` readers see the length; `tag` is
/// `BYTE_VIEW_MAGIC | flags`, which tells a record from an `f64`-slot array (whose
/// first element would have to be exactly that NaN payload).
#[repr(C)]
struct ByteView {
    len: usize,
    tag: u64,
    base: *mut u8,
    store: *mut ByteStore,
}

const BYTE_VIEW_MAGIC: u64 = 0x7FF8_5EED_B17E_B100;
const BYTE_VIEW_FLAG_MASK: u64 = 0xFF;
/// A `Buffer` (as opposed to a plain `Uint8Array`): `slice` shares.
const BYTE_VIEW_BUFFER: u64 = 1;
/// The whole backing store as an `ArrayBuffer` (`buf.buffer`): `slice` copies and
/// `Buffer.from` / `new Uint8Array` over it share.
const BYTE_VIEW_ARRAY_BUFFER: u64 = 2;
/// A `DataView` over a range of the backing store.
const BYTE_VIEW_DATAVIEW: u64 = 4;
/// An `ArrayBuffer` whose contents were moved out by `transfer()`.
const BYTE_VIEW_DETACHED: u64 = 8;

unsafe fn is_byte_view(ptr: *const u8) -> bool {
    !ptr.is_null() && (unsafe { ptr.add(8).cast::<u64>().read_unaligned() } & !BYTE_VIEW_FLAG_MASK) == BYTE_VIEW_MAGIC
}

/// `ptr` as a view record: a record passes through, an `f64`-slot array (a value
/// whose static type was erased on the way) is narrowed into a fresh one.
unsafe fn coerce_view(ptr: *const u8) -> *const ByteView {
    if ptr.is_null() || unsafe { is_byte_view(ptr) } {
        return ptr.cast();
    }
    unsafe { thaw_bytes_from_array(ptr) }.cast()
}

fn view_flags(view: &ByteView) -> u64 {
    view.tag & BYTE_VIEW_FLAG_MASK
}

unsafe fn alloc_byte_owner(len: usize, flags: u64) -> *mut u8 {
    let header = std::mem::size_of::<ByteView>() + std::mem::size_of::<ByteStore>();
    let Some(total) = header.checked_add(len) else {
        return std::ptr::null_mut();
    };
    let block = thaw_arena::thaw_arena_alloc(total.max(header + 1), 16);
    if block.is_null() {
        return block;
    }
    let view = block.cast::<ByteView>();
    let store = unsafe { block.add(std::mem::size_of::<ByteView>()) }.cast::<ByteStore>();
    let data = unsafe { block.add(header) };
    unsafe {
        data.write_bytes(0, len);
        store.write(ByteStore { data, len, js: 0, _reserved: 0 });
        view.write(ByteView { len, tag: BYTE_VIEW_MAGIC | flags, base: data, store });
    }
    block
}

unsafe fn new_byte_view(parent: *const u8, start: usize, len: usize) -> *mut u8 {
    let parent = unsafe { coerce_view(parent) };
    let block = thaw_arena::thaw_arena_alloc(std::mem::size_of::<ByteView>(), 8);
    if block.is_null() || parent.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let parent = &*parent;
        block.cast::<ByteView>().write(ByteView {
            len,
            tag: parent.tag,
            base: parent.base.add(start),
            store: parent.store,
        });
    }
    block
}

unsafe fn view_bytes<'a>(view: *const u8) -> Option<&'a [u8]> {
    let view = unsafe { coerce_view(view).as_ref() }?;
    Some(unsafe { std::slice::from_raw_parts(view.base, view.len) })
}

unsafe fn view_bytes_mut<'a>(view: *mut u8) -> Option<&'a mut [u8]> {
    let view = unsafe { coerce_view(view).as_ref() }?;
    Some(unsafe { std::slice::from_raw_parts_mut(view.base, view.len) })
}

unsafe fn read_byte_array(array: *const u8) -> Option<Vec<u8>> {
    unsafe { view_bytes(array) }.map(<[u8]>::to_vec)
}

unsafe fn write_byte_array(bytes: &[u8]) -> *mut u8 {
    let output = unsafe { alloc_byte_owner(bytes.len(), BYTE_VIEW_BUFFER) };
    if let Some(target) = unsafe { view_bytes_mut(output) } {
        target.copy_from_slice(bytes);
    }
    output
}

fn encoding_str(encoding: *const c_char) -> String {
    if encoding.is_null() {
        return "utf8".to_string();
    }
    unsafe { CStr::from_ptr(encoding) }
        .to_string_lossy()
        .to_ascii_lowercase()
}

#[no_mangle]
/// `buf.toString(encoding)` -- a native number array of `u8` values
/// rendered as a string. `encoding` may be null (defaults to `utf8`).
///
/// # Safety
///
/// `array` must point to a Thaw array of `f64` element slots; `encoding`
/// must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_bytes_to_string(
    array: *const u8,
    encoding: *const c_char,
) -> *const c_char {
    let Some(bytes) = (unsafe { read_byte_array(array) }) else {
        return std::ptr::null();
    };
    let text = encode_bytes(&bytes, &encoding_str(encoding));
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Buffer.from(str, encoding)` -- a string decoded into a native number
/// array of `u8` values. `encoding` may be null (defaults to `utf8`).
///
/// # Safety
///
/// `text` / `encoding` must be null or valid native or NUL-terminated strings.
pub unsafe extern "C" fn thaw_bytes_from_string(
    text: *const c_char,
    encoding: *const c_char,
) -> *mut u8 {
    if text.is_null() {
        return std::ptr::null_mut();
    }
    let bytes = decode_string(unsafe { CStr::from_ptr(text) }.to_bytes(), &encoding_str(encoding));
    unsafe { write_byte_array(&bytes) }
}

/// Decode the default base64 alphabet only as far as the destination can
/// hold complete chunks. Positions are UTF-16 code-unit offsets in `text`.
fn decode_set_from_base64(text: &[u16], max_length: usize) -> (usize, Vec<u8>) {
    if max_length == 0 {
        return (0, Vec::new());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4];
    let mut chunk_length = 0;
    let mut index = 0;
    let mut read = 0;
    while index < text.len() {
        // The five ASCII whitespace code units in SkipAsciiWhitespace.
        if matches!(text[index], 0x09 | 0x0a | 0x0c | 0x0d | 0x20) {
            index += 1;
            continue;
        }
        if text[index] == u16::from(b'=') && (chunk_length == 2 || chunk_length == 3) {
            let mut tail = index + 1;
            while tail < text.len() && matches!(text[tail], 0x09 | 0x0a | 0x0c | 0x0d | 0x20) {
                tail += 1;
            }
            if chunk_length == 2 && tail < text.len() && text[tail] == u16::from(b'=') {
                tail += 1;
            }
            while tail < text.len() && matches!(text[tail], 0x09 | 0x0a | 0x0c | 0x0d | 0x20) {
                tail += 1;
            }
            if tail == text.len() {
                let n = (u32::from(chunk[0]) << 18)
                    | (u32::from(chunk[1]) << 12)
                    | (u32::from(chunk[2]) << 6);
                bytes.push((n >> 16) as u8);
                if chunk_length == 3 {
                    bytes.push((n >> 8) as u8);
                }
                return (text.len(), bytes);
            }
        }
        let Some(value) = u8::try_from(text[index]).ok().and_then(base64_decode_value) else {
            // ponytail: Add setFrom SyntaxError/options validation with a separate decoder contract;
            // the generic Buffer decoder remains permissive.
            index += 1;
            continue;
        };
        index += 1;
        let remaining = max_length - bytes.len();
        if (remaining == 1 && chunk_length == 2)
            || (remaining == 2 && chunk_length == 3)
        {
            return (read, bytes);
        }
        chunk[chunk_length] = value;
        chunk_length += 1;
        if chunk_length == 4 {
            let n = (u32::from(chunk[0]) << 18)
                | (u32::from(chunk[1]) << 12)
                | (u32::from(chunk[2]) << 6)
                | u32::from(chunk[3]);
            bytes.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
            chunk_length = 0;
            read = index;
            if bytes.len() == max_length {
                return (read, bytes);
            }
        }
    }
    if chunk_length >= 2 {
        let n = (u32::from(chunk[0]) << 18)
            | (u32::from(chunk[1]) << 12)
            | (u32::from(chunk[2]) << 6);
        bytes.push((n >> 16) as u8);
        if chunk_length == 3 {
            bytes.push((n >> 8) as u8);
        }
    }
    (text.len(), bytes)
}

#[no_mangle]
/// `Uint8Array.prototype.setFromHex`/`setFromBase64`: decode `text` and
/// write its bytes into `array` from offset 0, up to the array's length.
/// Returns an arena object with adjacent `read` and `written` f64 fields.
///
/// # Safety
///
/// `array` must point to a Thaw array of `f64` element slots; `text` /
/// `encoding` must be null or a valid native or NUL-terminated string.
pub unsafe extern "C" fn thaw_bytes_set_from_string(
    array: *mut u8,
    text: *const c_char,
    encoding: *const c_char,
) -> *mut u8 {
    let result = thaw_arena::thaw_arena_alloc(16, 8);
    if result.is_null() {
        return result;
    }
    let (read, decoded) = if array.is_null() || text.is_null() {
        (0, Vec::new())
    } else if let Some(length) = unsafe { view_bytes(array) }.map(<[u8]>::len) {
        let source = unsafe { CStr::from_ptr(text) }.to_bytes();
        match encoding_str(encoding).as_str() {
            "base64" => decode_set_from_base64(&wtf8_decode_utf16(source), length),
            "hex" => {
                let mut decoded = decode_string(source, "hex");
                decoded.truncate(length);
                (decoded.len() * 2, decoded)
            }
            other => {
                let mut decoded = decode_string(source, other);
                decoded.truncate(length);
                (decoded.len(), decoded)
            }
        }
    } else {
        (0, Vec::new())
    };
    if let Some(target) = unsafe { view_bytes_mut(array) } {
        target[..decoded.len()].copy_from_slice(&decoded);
    }
    unsafe {
        result.cast::<f64>().write_unaligned(read as f64);
        result.add(8).cast::<f64>().write_unaligned(decoded.len() as f64);
    }
    result
}

#[no_mangle]
/// `Buffer.from(array)` for a `number[]` / byte buffer source -- a fresh
/// byte array holding each element truncated toward zero and taken mod
/// 256 (`300` -> `44`, `-1` -> `255`, `NaN` -> `0`), matching `Buffer`'s
/// own `ToUint8` element coercion. Always copies, so the result is
/// detached from the source array (also `Buffer.from(array)` semantics).
///
/// # Safety
///
/// `array` must point to a Thaw array of `f64` element slots.
pub unsafe extern "C" fn thaw_bytes_from_array(array: *const u8) -> *mut u8 {
    if unsafe { is_byte_view(array) } {
        return array.cast_mut();
    }
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    let output = unsafe { alloc_byte_owner(length, BYTE_VIEW_BUFFER) };
    if let Some(target) = unsafe { view_bytes_mut(output) } {
        for (index, byte) in target.iter_mut().enumerate() {
            let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
            *byte = slot as i64 as u8;
        }
    }
    output
}

#[no_mangle]
/// Widens a byte buffer into a fresh `[len][f64 slot...]` array buffer (the
/// layout every generic `number[]` operation reads). A copy.
///
/// # Safety
///
/// `buffer` must be null or a byte view record.
pub unsafe extern "C" fn thaw_bytes_to_array(buffer: *const u8) -> *mut u8 {
    if !buffer.is_null() && !unsafe { is_byte_view(buffer) } {
        return buffer.cast_mut();
    }
    let bytes = unsafe { view_bytes(buffer) }.unwrap_or_default();
    let output = thaw_arena::thaw_arena_alloc(8 + bytes.len() * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { output.cast::<u64>().write(bytes.len() as u64) };
    for (index, &byte) in bytes.iter().enumerate() {
        unsafe { output.add(8 + index * 8).cast::<f64>().write_unaligned(f64::from(byte)) };
    }
    output
}

#[no_mangle]
/// Narrows the first `min(len, array.len)` slots of an `f64` array buffer
/// back into an existing byte buffer, in place (`ToUint8` per element).
///
/// # Safety
///
/// `buffer` must be a byte view record, `array` an `f64`-slot array buffer.
pub unsafe extern "C" fn thaw_bytes_store_array(buffer: *mut u8, array: *const u8) {
    let (Some(target), Some(length)) =
        (unsafe { view_bytes_mut(buffer) }, unsafe { native_array_length(array) })
    else {
        return;
    };
    for (index, byte) in target.iter_mut().enumerate().take(length) {
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        *byte = slot as i64 as u8;
    }
}

unsafe fn map_buffer_list(
    list: *const u8,
    convert: unsafe extern "C" fn(*const u8) -> *mut u8,
) -> *mut u8 {
    let Some(count) = (unsafe { native_array_length(list) }) else {
        return std::ptr::null_mut();
    };
    let output = thaw_arena::thaw_arena_alloc(8 + count * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { output.cast::<u64>().write(count as u64) };
    for index in 0..count {
        let handle = unsafe { list.add(8 + index * 8).cast::<*const u8>().read() };
        let converted = if handle.is_null() {
            std::ptr::null_mut()
        } else {
            let data = unsafe { handle.cast::<*const u8>().read() };
            let converted = unsafe { convert(data) };
            let cell = thaw_arena::thaw_arena_alloc(16, 8);
            if !cell.is_null() {
                unsafe {
                    cell.cast::<*const u8>().write(converted);
                    cell.add(8).cast::<*const u8>().write(std::ptr::null());
                }
            }
            cell
        };
        unsafe { output.add(8 + index * 8).cast::<*mut u8>().write(converted) };
    }
    output
}

#[no_mangle]
/// A list of byte buffers as a list of `f64`-slot arrays (each element copied).
///
/// # Safety
///
/// `list` must be an array buffer of handles onto byte view records.
pub unsafe extern "C" fn thaw_bytes_widen_list(list: *const u8) -> *mut u8 {
    unsafe { map_buffer_list(list, thaw_bytes_to_array) }
}

#[no_mangle]
/// A list of `f64`-slot arrays as a list of byte buffers (each element copied).
///
/// # Safety
///
/// `list` must be an array buffer of handles onto `f64`-slot array buffers.
pub unsafe extern "C" fn thaw_bytes_narrow_list(list: *const u8) -> *mut u8 {
    unsafe { map_buffer_list(list, thaw_bytes_from_array) }
}

#[no_mangle]
/// Marks a fresh byte buffer as a plain `Uint8Array` (so `slice` copies).
///
/// # Safety
///
/// `buffer` must be null or a byte view record.
pub unsafe extern "C" fn thaw_bytes_as_u8(buffer: *mut u8) -> *mut u8 {
    if unsafe { is_byte_view(buffer) } {
        if let Some(view) = unsafe { buffer.cast::<ByteView>().as_mut() } {
            view.tag &= !BYTE_VIEW_BUFFER;
        }
    }
    buffer
}

#[no_mangle]
/// `Buffer.from(buffer)`: a fresh copy that is a `Buffer`.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_clone(buffer: *const u8) -> *mut u8 {
    let bytes = unsafe { view_bytes(buffer) }.unwrap_or_default();
    unsafe { write_byte_array(bytes) }
}

#[no_mangle]
/// `buf.subarray(start, end)` / Buffer `slice`: a new view over the same
/// store. Indexes follow `relative_array_index` (negative counts from the
/// end, clamped).
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_subarray(buffer: *const u8, start: f64, end: f64) -> *mut u8 {
    let buffer = unsafe { coerce_view(buffer) }.cast::<u8>();
    let Some(len) = (unsafe { view_bytes(buffer) }).map(<[u8]>::len) else {
        return std::ptr::null_mut();
    };
    let start = relative_array_index(start, len);
    let end = relative_array_index(end, len).max(start);
    unsafe { new_byte_view(buffer, start, end - start) }
}

#[no_mangle]
/// `buf.fill(value, start, end)` for an already-encoded `pattern` (repeated and
/// truncated to the range; an empty pattern zero-fills). Indexes follow
/// `relative_array_index`.
///
/// # Safety
///
/// `buffer` / `pattern` must be byte view records.
pub unsafe extern "C" fn thaw_bytes_fill(buffer: *mut u8, pattern: *const u8, start: f64, end: f64) {
    let Some(target) = (unsafe { view_bytes_mut(buffer) }) else {
        return;
    };
    let pattern = unsafe { view_bytes(pattern) }.unwrap_or_default().to_vec();
    let start = relative_array_index(start, target.len());
    let end = relative_array_index(end, target.len()).max(start);
    let range = &mut target[start..end];
    if pattern.is_empty() {
        range.fill(0);
    } else if let [byte] = pattern.as_slice() {
        range.fill(*byte);
    } else {
        for (slot, byte) in range.iter_mut().zip(pattern.iter().cycle()) {
            *slot = *byte;
        }
    }
}

#[no_mangle]
/// In-place `buf.reverse()` (`order` 0) or numeric `buf.sort()` (`order` 1).
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_reorder(buffer: *mut u8, order: f64) {
    if let Some(bytes) = unsafe { view_bytes_mut(buffer) } {
        if order == 0.0 {
            bytes.reverse();
        } else {
            bytes.sort_unstable();
        }
    }
}

#[no_mangle]
/// `buf.buffer`: an `ArrayBuffer`-flagged view of the whole backing store.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_buffer(buffer: *const u8) -> *mut u8 {
    let buffer = unsafe { coerce_view(buffer) };
    let Some(view) = (unsafe { buffer.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let store = unsafe { &*view.store };
    let block = thaw_arena::thaw_arena_alloc(std::mem::size_of::<ByteView>(), 8);
    if block.is_null() {
        return block;
    }
    unsafe {
        block.cast::<ByteView>().write(ByteView {
            len: store.len,
            tag: BYTE_VIEW_MAGIC | BYTE_VIEW_ARRAY_BUFFER,
            base: store.data,
            store: view.store,
        });
    }
    block
}

#[no_mangle]
/// `new Uint8Array(source, byteOffset, length)` (`kind` 0) and `Buffer.from(source,
/// byteOffset, length)` (`kind` 1): over an `ArrayBuffer` (`buf.buffer`) the result
/// shares its memory; over a `Buffer` / `Uint8Array` it is a copy. A `NaN` offset or
/// length means "omitted".
///
/// # Safety
///
/// `source` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_from_buffer_like(
    source: *const u8,
    byte_offset: f64,
    length: f64,
    kind: f64,
) -> *mut u8 {
    let source = unsafe { coerce_view(source) };
    let Some(view) = (unsafe { source.as_ref() }) else {
        return std::ptr::null_mut();
    };
    let flags = if kind != 0.0 { BYTE_VIEW_BUFFER } else { 0 };
    let result = if view_flags(view) & BYTE_VIEW_ARRAY_BUFFER != 0 {
        let available = view.len;
        let start = if byte_offset.is_nan() { 0 } else { (byte_offset.max(0.0) as usize).min(available) };
        let count = if length.is_nan() { available - start } else { (length.max(0.0) as usize).min(available - start) };
        unsafe { new_byte_view(source.cast(), start, count) }
    } else {
        let bytes = unsafe { view_bytes(source.cast()) }.unwrap_or_default();
        let start = if byte_offset.is_nan() { 0 } else { (byte_offset.max(0.0) as usize).min(bytes.len()) };
        let count = if length.is_nan() { bytes.len() - start } else { (length.max(0.0) as usize).min(bytes.len() - start) };
        let output = unsafe { alloc_byte_owner(count, flags) };
        if let Some(target) = unsafe { view_bytes_mut(output) } {
            target.copy_from_slice(&bytes[start..start + count]);
        }
        output
    };
    if let Some(view) = unsafe { result.cast::<ByteView>().as_mut() } {
        view.tag = BYTE_VIEW_MAGIC | flags;
    }
    result
}

#[no_mangle]
/// `slice`: a shared view for a `Buffer`, a copy for a plain `Uint8Array`.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_slice(buffer: *const u8, start: f64, end: f64) -> *mut u8 {
    let buffer_flag = unsafe { coerce_view(buffer).as_ref() }
        .is_some_and(|view| view_flags(view) & BYTE_VIEW_BUFFER != 0);
    if buffer_flag {
        unsafe { thaw_bytes_subarray(buffer, start, end) }
    } else {
        unsafe { thaw_bytes_slice_copy(buffer, start, end) }
    }
}

#[no_mangle]
/// `Uint8Array.prototype.slice`: a copy of the range.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_slice_copy(buffer: *const u8, start: f64, end: f64) -> *mut u8 {
    let Some(bytes) = (unsafe { view_bytes(buffer) }) else {
        return std::ptr::null_mut();
    };
    let start = relative_array_index(start, bytes.len());
    let end = relative_array_index(end, bytes.len()).max(start);
    let flags = unsafe { coerce_view(buffer).as_ref() }.map_or(0, view_flags);
    let output = unsafe { alloc_byte_owner(end - start, flags) };
    if let Some(target) = unsafe { view_bytes_mut(output) } {
        target.copy_from_slice(&bytes[start..end]);
    }
    output
}

/// Element width in bytes for a `thaw_bytes_read`/`write` `width`/`kind`
/// pair; `None` for an unsupported combination. The fixed-name
/// accessors (`readUInt16BE` &c, `bytes_numeric_accessor`) only ever
/// pass 1/2/4 (int) or 4/8 (float); `readUIntLE`/`readIntBE` &c's
/// variable-width, integer-only sibling
/// (`bytes_variable_width_accessor`) can pass any of 1-6 (Node's own
/// `byteLength` range -- 7-8 would need a real `BigInt` native type
/// Thaw doesn't have, so `readUIntLE`/`writeUIntLE` &c go no further
/// than 6, and this validates that even though `decode_scalar`/
/// `encode_scalar` below happen to also work for 7/8).
fn accessor_width(width: f64) -> Option<usize> {
    match width as u32 {
        1..=6 | 8 => Some(width as usize),
        _ => None,
    }
}

/// Reassembles `width` little-endian bytes (already byte-swapped by the
/// caller if the accessor was big-endian) into the number the accessor
/// `kind` (0 = unsigned int, 1 = signed int, 2 = float) names. Integer
/// widths other than the fixed accessors' 1/2/4/8 (i.e. 3/5/6, from
/// `readUIntLE`/`readIntBE` &c) decode via the same generic bit
/// pattern -- `buf` is already zero-padded to 8 bytes above `le.len()`,
/// so treating it as a `u64` directly gives the correct unsigned value
/// for any length up to 8, and sign-extending from the *actual* bit
/// width (not always 64) gives the correct signed value.
fn decode_scalar(le: &[u8], kind: u32) -> f64 {
    let mut buf = [0u8; 8];
    buf[..le.len()].copy_from_slice(le);
    match (kind, le.len()) {
        (2, 4) => f64::from(f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])),
        (2, 8) => f64::from_le_bytes(buf),
        (1, len @ 1..=8) => sign_extend_from_width(u64::from_le_bytes(buf), len) as f64,
        (_, 1..=8) => u64::from_le_bytes(buf) as f64,
        _ => 0.0,
    }
}

/// Sign-extends the low `width` bytes of `raw` (already zero-extended
/// to 64 bits) from that width's own sign bit out to a full `i64`.
fn sign_extend_from_width(raw: u64, width: usize) -> i64 {
    let shift = 64 - (width * 8) as u32;
    ((raw << shift) as i64) >> shift
}

/// Encodes `value` into `width` little-endian bytes per the accessor
/// `kind`. Integer kinds truncate toward zero then wrap mod 2^bits
/// (`Buffer`'s own coercion, including widths 3/5/6 from `writeUIntLE`/
/// `writeIntBE` &c -- the low `width` bytes of the same `i64`
/// truncation already used for 1/2/4/8); floats use IEEE-754.
fn encode_scalar(value: f64, width: usize, kind: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    match (kind, width) {
        (2, 4) => out[..4].copy_from_slice(&(value as f32).to_le_bytes()),
        (2, 8) => out = value.to_le_bytes(),
        (_, 1..=8) => {
            let truncated = (value as i64).to_le_bytes();
            out[..width].copy_from_slice(&truncated[..width]);
        }
        _ => {}
    }
    out
}

#[no_mangle]
/// `buf.readUInt16BE(offset)` / `buf.readInt8(offset)` /
/// `buf.readDoubleLE(offset)` &c. -- a fixed-width scalar out of the
/// native byte layout. `kind` is 0 (unsigned int), 1 (signed int), or 2
/// (float); `le` is 1 for little-endian. An out-of-range offset reads as
/// `0` (no `RangeError` across this boundary).
///
/// # Safety
///
/// `buf` must be null or point to a Thaw array of `f64` element slots.
pub unsafe extern "C" fn thaw_bytes_read(
    buf: *const u8,
    offset: f64,
    width: f64,
    kind: f64,
    le: f64,
) -> f64 {
    let Some(bytes) = (unsafe { view_bytes(buf) }) else {
        return 0.0;
    };
    let Some(width) = accessor_width(width) else {
        return 0.0;
    };
    if !(offset.is_finite() && offset >= 0.0) {
        return 0.0;
    }
    let offset = offset as usize;
    if offset.checked_add(width).is_none_or(|end| end > bytes.len()) {
        return 0.0;
    }
    let mut le_bytes = vec![0u8; width];
    for index in 0..width {
        le_bytes[index] = if le != 0.0 {
            bytes[offset + index]
        } else {
            bytes[offset + width - 1 - index]
        };
    }
    decode_scalar(&le_bytes, kind as u32)
}

#[no_mangle]
/// `buf.writeUInt16BE(value, offset)` &c. -- writes a fixed-width scalar
/// into the native byte layout in place and returns `offset + width`
/// (Node's contract). An out-of-range offset is a no-op. `kind` / `le`
/// as `thaw_bytes_read`.
///
/// # Safety
///
/// `buf` must be null or point to a writable Thaw array of `f64` slots.
pub unsafe extern "C" fn thaw_bytes_write(
    buf: *mut u8,
    offset: f64,
    value: f64,
    width: f64,
    kind: f64,
    le: f64,
) -> f64 {
    let Some(target) = (unsafe { view_bytes_mut(buf) }) else {
        return offset;
    };
    let length = target.len();
    let Some(width) = accessor_width(width) else {
        return offset;
    };
    if !(offset.is_finite() && offset >= 0.0) {
        return offset;
    }
    let offset = offset as usize;
    let Some(end) = offset.checked_add(width) else { return offset as f64; };
    if end > length { return offset as f64; }
    let encoded = encode_scalar(value, width, kind as u32);
    for index in 0..width {
        let byte = if le != 0.0 {
            encoded[index]
        } else {
            encoded[width - 1 - index]
        };
        target[offset + index] = byte;
    }
    end as f64
}

#[no_mangle]
/// `buf.readBigInt64LE(offset)` / `buf.readBigUInt64BE(offset)` &c. --
/// the full-precision 64-bit sibling of `thaw_bytes_read`: an `f64`
/// return can only exactly represent 53 bits of integer, so this
/// returns the raw bit pattern as a genuine `i64` instead of routing
/// through `decode_scalar`'s `f64` path. Signed (`BigInt64`) and
/// unsigned (`BigUInt64`) share the exact same bit pattern here --
/// Thaw's `HirType::I64` is always a signed 64-bit integer (no
/// separate unsigned native type), so a `BigUInt64` value `>= 2^63`
/// round-trips correctly through `Buffer` but displays as negative via
/// `.toString()`/`console.log`, a deliberate, documented practical-
/// subset limitation (see `docs/design/byte-buffers.md`) rather than a
/// silent miscalculation. Out-of-range offset reads as `0`, matching
/// `thaw_bytes_read`'s own convention.
///
/// # Safety
///
/// `buf` must be null or point to a Thaw array of `f64` element slots.
pub unsafe extern "C" fn thaw_bytes_read_i64(buf: *const u8, offset: f64, le: f64) -> i64 {
    let Some(bytes) = (unsafe { view_bytes(buf) }) else {
        return 0;
    };
    if !(offset.is_finite() && offset >= 0.0) {
        return 0;
    }
    let offset = offset as usize;
    if offset.checked_add(8).is_none_or(|end| end > bytes.len()) {
        return 0;
    }
    let mut le_bytes = [0u8; 8];
    for index in 0..8 {
        le_bytes[index] = if le != 0.0 {
            bytes[offset + index]
        } else {
            bytes[offset + 8 - 1 - index]
        };
    }
    i64::from_le_bytes(le_bytes)
}

#[no_mangle]
/// `buf.writeBigInt64LE(value, offset)` &c. -- the full-precision
/// 64-bit sibling of `thaw_bytes_write`; see `thaw_bytes_read_i64`'s
/// doc comment for why this takes/returns a real `i64` instead of
/// routing `value` through `encode_scalar`'s `f64` path. Returns
/// `offset + 8` (Node's contract); an out-of-range offset is a no-op.
///
/// # Safety
///
/// `buf` must be null or point to a writable Thaw array of `f64` slots.
pub unsafe extern "C" fn thaw_bytes_write_i64(
    buf: *mut u8,
    offset: f64,
    value: i64,
    le: f64,
) -> f64 {
    let Some(target) = (unsafe { view_bytes_mut(buf) }) else {
        return offset;
    };
    let length = target.len();
    if !(offset.is_finite() && offset >= 0.0) {
        return offset;
    }
    let offset = offset as usize;
    let Some(end) = offset.checked_add(8) else { return offset as f64; };
    if end > length { return offset as f64; }
    let encoded = value.to_le_bytes();
    for index in 0..8 {
        let byte = if le != 0.0 {
            encoded[index]
        } else {
            encoded[8 - 1 - index]
        };
        target[offset + index] = byte;
    }
    end as f64
}

#[no_mangle]
/// `source.copy(target, targetStart, sourceStart, sourceEnd)` -- blits
/// bytes into `target` in place, returning the count copied. Offsets are
/// clamped to their buffers; a `sourceEnd` of `-1` means "the source
/// length". `source` is read as a value; `target`'s `f64` slots are
/// written directly.
///
/// # Safety
///
/// `source` must be null or a Thaw `f64`-slot array; `target` must be
/// null or a writable one.
pub unsafe extern "C" fn thaw_bytes_copy(
    source: *const u8,
    target: *mut u8,
    target_start: f64,
    source_start: f64,
    source_end: f64,
) -> f64 {
    // `memmove` semantics: source and target may overlap (views of one store).
    let src = unsafe { read_byte_array(source) }.unwrap_or_default();
    let Some(target_bytes) = (unsafe { view_bytes_mut(target) }) else {
        return 0.0;
    };
    let target_len = target_bytes.len();
    let clamp = |value: f64, hi: usize| -> usize {
        if value.is_finite() && value >= 0.0 {
            (value as usize).min(hi)
        } else {
            0
        }
    };
    let target_start = clamp(target_start, target_len);
    let source_start = clamp(source_start, src.len());
    let source_end = if source_end.is_finite() && source_end >= 0.0 {
        (source_end as usize).min(src.len())
    } else {
        src.len()
    };
    if source_end <= source_start {
        return 0.0;
    }
    let count = (source_end - source_start).min(target_len - target_start);
    target_bytes[target_start..target_start + count]
        .copy_from_slice(&src[source_start..source_start + count]);
    count as f64
}

#[no_mangle]
/// `buf.indexOf(needle, from)` / `lastIndexOf` -- a byte-subsequence
/// search. `needle` is already a byte buffer (thaw-hir decodes a string
/// / wraps a number first). Returns the first (or last, `last != 0`)
/// index at or after `from`, or `-1`. An empty needle returns `from`
/// clamped into the haystack (Node's rule). A negative / non-finite
/// `from` clamps into the haystack for empty needles. `+Infinity` starts
/// beyond any nonempty forward match; `NaN` and `-Infinity` start at 0.
///
/// # Safety
///
/// `haystack` / `needle` must be null or Thaw `f64`-slot arrays.
pub unsafe extern "C" fn thaw_bytes_index_of(
    haystack: *const u8,
    needle: *const u8,
    from: f64,
    last: f64,
) -> f64 {
    let hay = unsafe { view_bytes(haystack) }.unwrap_or_default();
    let nee = unsafe { view_bytes(needle) }.unwrap_or_default();
    let clamp_from = |hi: usize| -> usize {
        if from.is_nan() || from <= 0.0 {
            0
        } else if from.is_finite() {
            (from as usize).min(hi)
        } else {
            hi
        }
    };
    if nee.is_empty() {
        return clamp_from(hay.len()) as f64;
    }
    if nee.len() > hay.len() {
        return -1.0;
    }
    let last_start = hay.len() - nee.len();
    if last != 0.0 {
        let start = clamp_from(last_start).min(last_start);
        for index in (0..=start).rev() {
            if hay[index..index + nee.len()] == nee[..] {
                return index as f64;
            }
        }
        return -1.0;
    }
    let start = if from.is_sign_positive() && !from.is_nan() {
        from as usize
    } else {
        0
    };
    if start > last_start {
        return -1.0;
    }
    for index in start..=last_start {
        if hay[index..index + nee.len()] == nee[..] {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// `Buffer.byteLength(string, encoding)` -- the number of bytes the
/// string occupies in `encoding` (null defaults to `utf8`): the UTF-8
/// byte length, UTF-16 length / 2 for `hex`, a padded-length estimate for
/// `base64`, and the UTF-16 code-unit count for `latin1`.
///
/// # Safety
///
/// `text` / `encoding` must be null or valid native or NUL-terminated strings.
pub unsafe extern "C" fn thaw_bytes_byte_length(
    text: *const c_char,
    encoding: *const c_char,
) -> f64 {
    if text.is_null() {
        return 0.0;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_bytes();
    match encoding_str(encoding).as_str() {
        "hex" => (wtf8_decode_utf16(text).len() / 2) as f64,
        "base64" | "base64url" => {
            let units = wtf8_decode_utf16(text);
            let padded = units.iter().rev().take(2).take_while(|&&unit| unit == b'=' as u16).count();
            ((units.len() - padded).saturating_mul(3) / 4) as f64
        }
        encoding => decode_string(text, encoding).len() as f64,
    }
}

#[no_mangle]
/// `Buffer.concat(list, totalLength)` -- flattens an array of byte
/// buffers into one fresh byte array, in order. A null / unreadable
/// entry contributes nothing (rather than faulting). `list` is the raw
/// outer `[len][elem...]` buffer; each element slot holds an inner array
/// *handle* (one word onto the inner `[len][elem...]` buffer), the same
/// nesting every `T[][]` uses. A non-negative `total` truncates or
/// zero-pads the result to exactly that many bytes (Node's optional
/// `totalLength`); a negative `total` means "no limit".
///
/// # Safety
///
/// `list` must point to a Thaw array whose element slots are array
/// handles onto `f64`-slot byte buffers.
pub unsafe extern "C" fn thaw_bytes_concat(list: *const u8, total: f64) -> *mut u8 {
    let Some(count) = (unsafe { native_array_length(list) }) else {
        return std::ptr::null_mut();
    };
    let mut result = Vec::new();
    for index in 0..count {
        let inner_handle = unsafe { list.add(8 + index * 8).cast::<*const u8>().read() };
        if inner_handle.is_null() {
            continue;
        }
        let inner_buffer = unsafe { inner_handle.cast::<*const u8>().read() };
        if let Some(bytes) = unsafe { view_bytes(inner_buffer) } {
            result.extend_from_slice(bytes);
        }
    }
    if total.is_finite() && total >= 0.0 {
        result.resize(total as usize, 0);
    }
    unsafe { write_byte_array(&result) }
}

#[no_mangle]
/// `a.equals(b)` -- byte-for-byte equality of two byte buffers. Both
/// pointers are raw `[len][elem...]` buffers (codegen unwraps the
/// handles). A null / unreadable side compares equal only to another
/// empty one.
///
/// # Safety
///
/// `a` / `b` must be null or point to Thaw arrays of `f64` element slots.
pub unsafe extern "C" fn thaw_bytes_equals(a: *const u8, b: *const u8) -> bool {
    let left = unsafe { view_bytes(a) }.unwrap_or_default();
    let right = unsafe { view_bytes(b) }.unwrap_or_default();
    left == right
}

#[no_mangle]
/// `Buffer.alloc(size)` -- a zero-filled byte buffer of `size` bytes
/// (clamped at 0; a fractional/negative size truncates like `Buffer`).
pub extern "C" fn thaw_bytes_alloc(size: f64) -> *mut u8 {
    let size = if size.is_finite() && size > 0.0 {
        size as usize
    } else {
        0
    };
    unsafe { alloc_byte_owner(size, BYTE_VIEW_BUFFER) }
}

#[no_mangle]
/// `Buffer.alloc(size, fill)` -- `size` bytes made of `pattern` repeated (a
/// truncated last repetition is kept). An empty or unreadable pattern leaves
/// the buffer zero-filled, as `Buffer.alloc(size, "")` does.
///
/// # Safety
///
/// `pattern` must be null or point to a Thaw byte array.
pub unsafe extern "C" fn thaw_bytes_alloc_fill(size: f64, pattern: *const u8) -> *mut u8 {
    let size = if size.is_finite() && size > 0.0 {
        size as usize
    } else {
        0
    };
    let pattern = unsafe { read_byte_array(pattern) }.unwrap_or_default();
    let bytes = if pattern.is_empty() {
        vec![0_u8; size]
    } else {
        pattern.iter().copied().cycle().take(size).collect()
    };
    unsafe { write_byte_array(&bytes) }
}

#[no_mangle]
/// `buf.byteOffset`: the position of `buffer` inside the buffer it was
/// viewed from (0 for an owner).
///
/// # Safety
///
/// `buffer` must be null or a Thaw `f64`-slot array.
pub unsafe extern "C" fn thaw_bytes_byte_offset(buffer: *const u8) -> f64 {
    let Some(view) = (unsafe { coerce_view(buffer).as_ref() }) else {
        return 0.0;
    };
    let store = unsafe { &*view.store };
    (view.base as usize - store.data as usize) as f64
}

#[no_mangle]
/// `buf.buffer.byteLength`: the size of the shared backing store.
///
/// # Safety
///
/// `buffer` must be null or a byte view record.
pub unsafe extern "C" fn thaw_bytes_store_length(buffer: *const u8) -> f64 {
    unsafe { coerce_view(buffer).as_ref() }.map_or(0.0, |view| unsafe { (*view.store).len } as f64)
}

#[no_mangle]
/// `new ArrayBuffer(len)`: a zeroed store viewed as an `ArrayBuffer`.
pub extern "C" fn thaw_arraybuffer_alloc(len: f64) -> *mut u8 {
    let len = if len.is_finite() && len > 0.0 { len as usize } else { 0 };
    unsafe { alloc_byte_owner(len, BYTE_VIEW_ARRAY_BUFFER) }
}

#[no_mangle]
/// `new DataView(buffer, byteOffset, byteLength)` over an `ArrayBuffer` / `buf.buffer`
/// view. A `NaN` offset or length means "omitted"; the caller range-checks.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_dataview_new(buffer: *const u8, offset: f64, len: f64) -> *mut u8 {
    let Some(source) = (unsafe { coerce_view(buffer).as_ref() }) else {
        return std::ptr::null_mut();
    };
    let start = if offset.is_nan() { 0 } else { (offset.max(0.0) as usize).min(source.len) };
    let count = if len.is_nan() { source.len - start } else { (len.max(0.0) as usize).min(source.len - start) };
    let view = unsafe { new_byte_view(buffer, start, count) };
    if let Some(view) = unsafe { view.cast::<ByteView>().as_mut() } {
        view.tag = BYTE_VIEW_MAGIC | BYTE_VIEW_DATAVIEW;
    }
    view
}

/// `DataView.prototype.get*`: `kind` is `1`/`2`/`4`/`8` (unsigned ints), `-1`/`-2`/`-4`
/// (signed ints) or `-5`/`-9` (`Float32` / `Float64`; the width is `|kind| - 1` for floats).
/// The caller range-checks `offset`; out of range reads as `0`.
///
/// # Safety
///
/// `view` must be a byte view record.
#[no_mangle]
pub unsafe extern "C" fn thaw_dataview_get(view: *const u8, kind: f64, offset: f64, little: f64) -> f64 {
    let Some(bytes) = (unsafe { view_bytes(view) }) else { return 0.0 };
    let (width, signed, float) = dataview_kind(kind);
    let Some(chunk) = dataview_range(bytes.len(), offset, width).map(|start| &bytes[start..start + width]) else {
        return 0.0;
    };
    let mut le = [0u8; 8];
    le[..width].copy_from_slice(chunk);
    if little == 0.0 {
        le[..width].reverse();
    }
    if float && width == 2 {
        return f16_bits_to_f64(u16::from_le_bytes([le[0], le[1]]));
    }
    decode_scalar(&le[..width], if float { 2 } else { u32::from(signed) })
}

/// `DataView.prototype.set*`; see `thaw_dataview_get` for `kind`.
///
/// # Safety
///
/// `view` must be a byte view record.
#[no_mangle]
pub unsafe extern "C" fn thaw_dataview_set(view: *mut u8, kind: f64, offset: f64, value: f64, little: f64) {
    let Some(bytes) = (unsafe { view_bytes_mut(view) }) else { return };
    let (width, _, float) = dataview_kind(kind);
    let Some(start) = dataview_range(bytes.len(), offset, width) else { return };
    let mut le = if float {
        if width == 2 {
            f64_to_f16_bits(value).to_le_bytes().to_vec()
        } else if width == 4 { (value as f32).to_le_bytes().to_vec() } else { value.to_le_bytes().to_vec() }
    } else {
        // ToInt/ToUint: truncate, then wrap modulo 2^64 (the low `width` bytes are what is kept).
        let truncated = value.trunc();
        let wrapped = if !value.is_finite() {
            0
        } else if truncated.abs() < 9.0e18 {
            truncated as i64 as u64
        } else {
            truncated.rem_euclid(18446744073709551616.0) as u64
        };
        wrapped.to_le_bytes()[..width].to_vec()
    };
    if little == 0.0 {
        le.reverse();
    }
    bytes[start..start + width].copy_from_slice(&le);
}

fn dataview_kind(kind: f64) -> (usize, bool, bool) {
    match kind as i32 {
        -3 => (2, true, true),
        -5 => (4, true, true),
        -9 => (8, true, true),
        k if k < 0 => (-k as usize, true, false),
        k => (k as usize, false, false),
    }
}

fn dataview_range(len: usize, offset: f64, width: usize) -> Option<usize> {
    (offset >= 0.0 && offset.is_finite()).then_some(offset as usize).filter(|start| start.checked_add(width).is_some_and(|end| end <= len))
}

#[no_mangle]
/// Console / type-test support: `field` 2 is whether an `ArrayBuffer` is detached, `field` 0 is the inspect kind (`0` Buffer / `Uint8Array`, `1`
/// `ArrayBuffer`, `2` `DataView`), `field` 1 the view length.
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_console_info(buffer: *const u8, field: f64) -> f64 {
    let Some(view) = (unsafe { coerce_view(buffer).as_ref() }) else { return 0.0 };
    if field == 2.0 {
        return f64::from(u8::from(view.tag & BYTE_VIEW_DETACHED != 0));
    }
    if field != 0.0 {
        return view.len as f64;
    }
    let flags = view_flags(view);
    if flags & BYTE_VIEW_DATAVIEW != 0 { 2.0 } else if flags & BYTE_VIEW_ARRAY_BUFFER != 0 { 1.0 } else { 0.0 }
}

#[no_mangle]
/// Console support: the bytes to list for `buffer` (a `DataView` shows its whole store).
///
/// # Safety
///
/// `buffer` must be a byte view record.
pub unsafe extern "C" fn thaw_bytes_console_target(buffer: *const u8) -> *mut u8 {
    let is_dataview = unsafe { coerce_view(buffer).as_ref() }.is_some_and(|view| view_flags(view) & BYTE_VIEW_DATAVIEW != 0);
    if is_dataview { unsafe { thaw_bytes_buffer(buffer) } } else { buffer.cast_mut() }
}

/// IEEE half precision, round to nearest even (Rust's `f16` is still unstable).
fn f64_to_f16_bits(value: f64) -> u16 {
    let sign = ((value.to_bits() >> 48) & 0x8000) as u16;
    let magnitude = value.abs();
    if value.is_nan() {
        return sign | 0x7e00;
    }
    if magnitude >= 65520.0 {
        return sign | 0x7c00;
    }
    let exponent = ((magnitude.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    if exponent < -14 {
        // Subnormal (or zero): a multiple of 2^-24; rounding up to 1024 lands on the smallest normal.
        return sign | (magnitude * 16777216.0).round_ties_even() as u16;
    }
    let mut mantissa = ((magnitude / 2f64.powi(exponent) - 1.0) * 1024.0).round_ties_even() as u16;
    let mut biased = (exponent + 15) as u16;
    if mantissa == 1024 {
        mantissa = 0;
        biased += 1;
    }
    if biased >= 31 {
        return sign | 0x7c00;
    }
    sign | (biased << 10) | mantissa
}

fn f16_bits_to_f64(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 31);
    let mantissa = f64::from(bits & 1023);
    sign * match exponent {
        0 => mantissa * 2f64.powi(-24),
        31 if mantissa == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        _ => (1.0 + mantissa / 1024.0) * 2f64.powi(exponent - 15),
    }
}

#[no_mangle]
/// `ab.transfer(newLength?)` / `ab.transferToFixedLength(newLength?)`: the contents move into a
/// fresh `ArrayBuffer` (a `NaN` length keeps the size) and `buffer` becomes detached.
///
/// # Safety
///
/// `buffer` must be an `ArrayBuffer` view record.
pub unsafe extern "C" fn thaw_arraybuffer_transfer(buffer: *mut u8, length: f64) -> *mut u8 {
    let Some(source) = (unsafe { coerce_view(buffer).cast_mut().as_mut() }) else {
        return std::ptr::null_mut();
    };
    let size = if length.is_nan() { source.len } else { length.max(0.0) as usize };
    let output = unsafe { alloc_byte_owner(size, BYTE_VIEW_ARRAY_BUFFER) };
    if let Some(target) = unsafe { view_bytes_mut(output) } {
        let copied = size.min(source.len);
        target[..copied].copy_from_slice(unsafe { std::slice::from_raw_parts(source.base, copied) });
    }
    source.len = 0;
    source.tag |= BYTE_VIEW_DETACHED;
    output
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn half_precision_conversion_rounds_like_ieee() {
        for (value, bits) in [
            (1.0, 0x3c00),
            (-2.0, 0xc000),
            (65504.0, 0x7bff),
            (65520.0, 0x7c00),
            (1.337, 0x3d59),
            (5.960464477539063e-8, 0x0001),
            (6.103515625e-5, 0x0400),
            (-0.0, 0x8000),
            (f64::INFINITY, 0x7c00),
        ] {
            assert_eq!(f64_to_f16_bits(value), bits, "{value}");
        }
        assert_eq!(f16_bits_to_f64(0x3d59), 1.3369140625);
        assert!(f16_bits_to_f64(f64_to_f16_bits(f64::NAN)).is_nan());
    }

    #[test]
    fn dataview_wraps_integers_and_checks_ranges() {
        unsafe {
            let buffer = thaw_arraybuffer_alloc(8.0);
            let view = thaw_dataview_new(buffer, 2.0, 4.0);
            thaw_dataview_set(view, -1.0, 0.0, -2.0, 1.0);
            thaw_dataview_set(view, -4.0, 0.0, 4294967295.0 + 3.0, 0.0);
            assert_eq!(thaw_dataview_get(view, 1.0, 0.0, 0.0), 0.0);
            assert_eq!(thaw_dataview_get(view, -4.0, 0.0, 0.0), 2.0);
            assert_eq!(thaw_dataview_get(view, -4.0, 1.0, 0.0), 0.0, "out of range reads 0");
            assert_eq!(thaw_bytes_console_info(view, 0.0), 2.0);
            assert_eq!(thaw_bytes_console_info(buffer, 0.0), 1.0);
        }
    }
}
