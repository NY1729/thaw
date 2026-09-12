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
        "latin1" | "binary" | "ascii" => bytes.iter().map(|&b| b as char).collect(),
        // "utf8" / "utf-8" / anything else: lossy UTF-8 decode.
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_string(text: &str, encoding: &str) -> Vec<u8> {
    match encoding {
        "hex" => {
            let digits: Vec<u8> = text
                .bytes()
                .filter_map(|b| (b as char).to_digit(16).map(|d| d as u8))
                .collect();
            digits.chunks(2).map(|pair| (pair[0] << 4) | pair.get(1).copied().unwrap_or(0)).collect()
        }
        "base64" | "base64url" => {
            let cleaned: Vec<u8> = text
                .bytes()
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
        "latin1" | "binary" | "ascii" => text.chars().map(|c| c as u32 as u8).collect(),
        _ => text.as_bytes().to_vec(),
    }
}

unsafe fn read_byte_array(array: *const u8) -> Option<Vec<u8>> {
    let length = unsafe { native_array_length(array) }?;
    let mut bytes = Vec::with_capacity(length);
    for index in 0..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        bytes.push(slot as i64 as u8);
    }
    Some(bytes)
}

unsafe fn write_byte_array(bytes: &[u8]) -> *mut u8 {
    let output = thaw_arena::thaw_arena_alloc(8 + bytes.len() * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { output.cast::<u64>().write(bytes.len() as u64) };
    for (index, &byte) in bytes.iter().enumerate() {
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<f64>()
                .write_unaligned(f64::from(byte));
        }
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
/// `text` / `encoding` must be null or valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_bytes_from_string(
    text: *const c_char,
    encoding: *const c_char,
) -> *mut u8 {
    if text.is_null() {
        return std::ptr::null_mut();
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    let bytes = decode_string(&text, &encoding_str(encoding));
    unsafe { write_byte_array(&bytes) }
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
    let Some(bytes) = (unsafe { read_byte_array(array) }) else {
        return std::ptr::null_mut();
    };
    unsafe { write_byte_array(&bytes) }
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
    let Some(bytes) = (unsafe { read_byte_array(buf) }) else {
        return 0.0;
    };
    let Some(width) = accessor_width(width) else {
        return 0.0;
    };
    if !(offset.is_finite() && offset >= 0.0) {
        return 0.0;
    }
    let offset = offset as usize;
    if offset + width > bytes.len() {
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
    let Some(length) = (unsafe { native_array_length(buf) }) else {
        return offset;
    };
    let Some(width) = accessor_width(width) else {
        return offset;
    };
    if !(offset.is_finite() && offset >= 0.0) {
        return offset;
    }
    let offset = offset as usize;
    if offset + width > length {
        return (offset + width) as f64;
    }
    let encoded = encode_scalar(value, width, kind as u32);
    for index in 0..width {
        let byte = if le != 0.0 {
            encoded[index]
        } else {
            encoded[width - 1 - index]
        };
        unsafe {
            buf.add(8 + (offset + index) * 8)
                .cast::<f64>()
                .write_unaligned(f64::from(byte));
        }
    }
    (offset + width) as f64
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
    let src = unsafe { read_byte_array(source) }.unwrap_or_default();
    let Some(target_len) = (unsafe { native_array_length(target) }) else {
        return 0.0;
    };
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
    for index in 0..count {
        unsafe {
            target
                .add(8 + (target_start + index) * 8)
                .cast::<f64>()
                .write_unaligned(f64::from(src[source_start + index]));
        }
    }
    count as f64
}

#[no_mangle]
/// `buf.indexOf(needle, from)` / `lastIndexOf` -- a byte-subsequence
/// search. `needle` is already a byte buffer (thaw-hir decodes a string
/// / wraps a number first). Returns the first (or last, `last != 0`)
/// index at or after `from`, or `-1`. An empty needle returns `from`
/// clamped into the haystack (Node's rule). A negative / non-finite
/// `from` clamps to 0 for a forward search, to the last start for a
/// reverse one.
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
    let hay = unsafe { read_byte_array(haystack) }.unwrap_or_default();
    let nee = unsafe { read_byte_array(needle) }.unwrap_or_default();
    let clamp_from = |hi: usize| -> usize {
        if from.is_finite() && from >= 0.0 {
            (from as usize).min(hi)
        } else if from.is_finite() {
            0
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
    let start = if from.is_finite() && from >= 0.0 {
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
/// byte length, `len / 2` for `hex`, the decoded length for `base64`,
/// the character count for `latin1`.
///
/// # Safety
///
/// `text` / `encoding` must be null or valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_bytes_byte_length(
    text: *const c_char,
    encoding: *const c_char,
) -> f64 {
    if text.is_null() {
        return 0.0;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    decode_string(&text, &encoding_str(encoding)).len() as f64
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
        if let Some(bytes) = unsafe { read_byte_array(inner_buffer) } {
            result.extend(bytes);
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
    let left = unsafe { read_byte_array(a) }.unwrap_or_default();
    let right = unsafe { read_byte_array(b) }.unwrap_or_default();
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
    unsafe { write_byte_array(&vec![0_u8; size]) }
}
