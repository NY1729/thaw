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
