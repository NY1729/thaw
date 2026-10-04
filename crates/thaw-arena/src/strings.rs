//! Length metadata preserves embedded NULs without changing the pointer ABI.
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::{Mutex, OnceLock};

thread_local! {
    static LENGTHS: RefCell<HashMap<usize, (usize, bool)>> = RefCell::new(HashMap::new());
}

fn owned_lengths() -> &'static Mutex<HashMap<usize, usize>> {
    // ponytail: one process-wide lock for owned lengths; shard only if allocation contention is measured.
    static OWNED: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    OWNED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A borrowed native string; unregistered pointers retain ordinary C semantics.
pub struct NativeStr<'a>(&'a [u8]);

impl<'a> NativeStr<'a> {
    /// # Safety
    /// `pointer` must reference a live registered string or a NUL-terminated C string.
    pub unsafe fn from_ptr(pointer: *const c_char) -> Self {
        let length = LENGTHS
            .with(|lengths| {
                lengths
                    .borrow()
                    .get(&(pointer as usize))
                    .map(|&(length, _)| length)
            })
            .or_else(|| owned_lengths().lock().unwrap().get(&(pointer as usize)).copied());
        Self(match length {
            Some(length) => unsafe { std::slice::from_raw_parts(pointer.cast(), length) },
            None => unsafe { CStr::from_ptr(pointer) }.to_bytes(),
        })
    }
    pub fn to_bytes(&self) -> &'a [u8] {
        self.0
    }
    pub fn to_str(&self) -> Result<&'a str, std::str::Utf8Error> {
        std::str::from_utf8(self.0)
    }
    pub fn to_string_lossy(&self) -> Cow<'a, str> {
        String::from_utf8_lossy(self.0)
    }
    pub fn as_ptr(&self) -> *const c_char {
        self.0.as_ptr().cast()
    }
}

/// # Safety
/// `pointer` must reference `length` initialized bytes followed by a NUL.
#[no_mangle]
pub unsafe extern "C" fn thaw_string_register(
    pointer: *const c_char,
    length: usize,
) -> *const c_char {
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), length) };
    if bytes.contains(&0) {
        LENGTHS.with(|lengths| {
            lengths
                .borrow_mut()
                .insert(pointer as usize, (length, false));
        });
    }
    pointer
}

/// # Safety
/// As for `thaw_string_register`, with process-lifetime storage.
#[no_mangle]
pub unsafe extern "C" fn thaw_string_register_literal(
    pointer: *const c_char,
    length: usize,
) -> *const c_char {
    LENGTHS.with(|lengths| {
        lengths
            .borrow_mut()
            .insert(pointer as usize, (length, true));
    });
    pointer
}

/// # Safety
/// `pointer` must reference a live native or C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_string_byte_length(pointer: *const c_char) -> usize {
    unsafe { NativeStr::from_ptr(pointer) }.to_bytes().len()
}

/// Copies bytes into invocation-managed native string storage.
pub fn arena_string(bytes: &[u8]) -> *mut c_char {
    let Some(size) = bytes.len().checked_add(1) else {
        return std::ptr::null_mut();
    };
    let pointer = super::thaw_arena_alloc(size, 1);
    if !pointer.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
            pointer.add(bytes.len()).write(0);
            thaw_string_register(pointer.cast(), bytes.len());
        }
    }
    pointer.cast()
}

/// Creates an owned native string, paired with `destroy_string`.
pub fn owned_string(bytes: impl AsRef<[u8]>) -> *mut c_char {
    let bytes = bytes.as_ref();
    if !bytes.contains(&0) {
        return CString::new(bytes).unwrap().into_raw();
    }
    let mut buffer = bytes.to_vec();
    buffer.push(0);
    let pointer = Box::into_raw(buffer.into_boxed_slice()) as *mut u8;
    owned_lengths().lock().unwrap().insert(pointer as usize, bytes.len());
    pointer.cast()
}

/// # Safety
/// `pointer` must be null or an owned native/C string, destroyed exactly once.
pub unsafe fn destroy_string(pointer: *mut c_char) {
    if pointer.is_null() {
        return;
    }
    if let Some(length) = owned_lengths().lock().unwrap().remove(&(pointer as usize)) {
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                pointer.cast::<u8>(),
                length + 1,
            )))
        };
    } else {
        unsafe { drop(CString::from_raw(pointer)) };
    }
}

pub(crate) fn reset_lengths(tracing: bool) {
    LENGTHS.with(|lengths| {
        lengths.borrow_mut().retain(|&pointer, &mut (_, literal)| {
            literal || (tracing && !super::was_reclaimed(pointer))
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wtf8_preserves_lone_surrogates_and_embedded_nul() {
        let units = [0xd800, 0, 0xdc00, 0xd83d, 0xde00];
        assert_eq!(wtf8_decode_utf16(&wtf8_encode_utf16(&units)), units);
        assert_eq!(wtf8_encode_utf16(&[0xd800]), [0xed, 0xa0, 0x80]);
    }

    #[test]
    fn utf16_replacement_preserves_units_and_substitution_patterns() {
        let value = [0xd800, b'a' as u16, 0xdc00];
        let replacement = [b'$' as u16, b'&' as u16, b'$' as u16, b'$' as u16];
        assert_eq!(utf16_replace(&value, &[b'a' as u16], &replacement, false), [0xd800, b'a' as u16, b'$' as u16, 0xdc00]);
        assert_eq!(utf16_replace(&[0xd800, 0xdc00], &[], &[b'-' as u16], true), [b'-' as u16, 0xd800, b'-' as u16, 0xdc00, b'-' as u16]);
    }

    #[test]
    fn case_segments_keep_context_and_lone_surrogates() {
        let units = [0xd800, 0x039f, 0x03a3, 0xdc00, 0x00df];
        assert_eq!(utf16_map_segments(&units, |text, output| output.extend(text.to_lowercase().encode_utf16())), [0xd800, 0x03bf, 0x03c2, 0xdc00, 0x00df]);
        assert_eq!(utf16_map_segments(&units, |text, output| output.extend(text.to_uppercase().encode_utf16())), [0xd800, 0x039f, 0x03a3, 0xdc00, 0x0053, 0x0053]);
    }

    #[test]
    fn native_and_c_strings_keep_distinct_lengths() {
        let pointer = owned_string(b"a\0b");
        assert_eq!(unsafe { NativeStr::from_ptr(pointer) }.to_bytes(), b"a\0b");
        assert_eq!(
            unsafe { NativeStr::from_ptr(c"abc".as_ptr()) }.to_bytes(),
            b"abc"
        );
        unsafe { destroy_string(pointer) };
    }

    #[test]
    fn owned_string_keeps_embedded_nul_across_threads() {
        let pointer = owned_string(b"a\0b") as usize;
        std::thread::spawn(move || {
            let pointer = pointer as *mut c_char;
            assert_eq!(unsafe { NativeStr::from_ptr(pointer) }.to_bytes(), b"a\0b");
            unsafe { destroy_string(pointer) };
        }).join().unwrap();
    }
}

// ---- WTF-8 <-> UTF-16 codec -------------------------------------------------
//
// Thaw native strings are WTF-8 with length metadata for embedded NULs:
// ordinary UTF-8, extended so
// a lone UTF-16 surrogate (which valid UTF-8 cannot represent) is encoded as
// its 3-byte WTF-8 sequence. `wtf8_decode_utf16` preserves lone surrogates
// where `std::str::from_utf8_lossy` would replace one with 3 U+FFFD (one per
// invalid byte of the 3-byte surrogate sequence).

fn wtf8_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// Decodes WTF-8 bytes to UTF-16 code units, preserving lone surrogates.
/// Malformed bytes decode as U+FFFD (one per stray byte).
pub fn wtf8_decode_utf16(bytes: &[u8]) -> Vec<u16> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        if first < 0x80 {
            units.push(u16::from(first));
            index += 1;
        } else if (0xC2..=0xDF).contains(&first) {
            if index + 1 < bytes.len() && wtf8_continuation(bytes[index + 1]) {
                let code =
                    ((u32::from(first) & 0x1F) << 6) | (u32::from(bytes[index + 1]) & 0x3F);
                units.push(code as u16);
                index += 2;
            } else {
                units.push(0xFFFD);
                index += 1;
            }
        } else if (0xE0..=0xEF).contains(&first) {
            if index + 2 < bytes.len()
                && wtf8_continuation(bytes[index + 1])
                && wtf8_continuation(bytes[index + 2])
            {
                let code = ((u32::from(first) & 0x0F) << 12)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 2]) & 0x3F);
                // 0xD800-0xDFFF is allowed here: a lone surrogate.
                units.push(code as u16);
                index += 3;
            } else {
                units.push(0xFFFD);
                index += 1;
            }
        } else if (0xF0..=0xF4).contains(&first) {
            if index + 3 < bytes.len()
                && wtf8_continuation(bytes[index + 1])
                && wtf8_continuation(bytes[index + 2])
                && wtf8_continuation(bytes[index + 3])
            {
                let code = ((u32::from(first) & 0x07) << 18)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 12)
                    | ((u32::from(bytes[index + 2]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 3]) & 0x3F);
                if (0x10000..=0x10FFFF).contains(&code) {
                    let code = code - 0x10000;
                    units.push(0xD800 + (code >> 10) as u16);
                    units.push(0xDC00 + (code & 0x3FF) as u16);
                    index += 4;
                } else {
                    units.push(0xFFFD);
                    index += 1;
                }
            } else {
                units.push(0xFFFD);
                index += 1;
            }
        } else {
            units.push(0xFFFD);
            index += 1;
        }
    }
    units
}

/// Appends the WTF-8 encoding of a single code point (allowing a lone
/// surrogate in 0xD800-0xDFFF as its 3-byte sequence).
pub fn wtf8_push_code_point(bytes: &mut Vec<u8>, code: u32) {
    if code < 0x80 {
        bytes.push(code as u8);
    } else if code < 0x800 {
        bytes.push(0xC0 | (code >> 6) as u8);
        bytes.push(0x80 | (code & 0x3F) as u8);
    } else if code < 0x10000 {
        bytes.push(0xE0 | (code >> 12) as u8);
        bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
        bytes.push(0x80 | (code & 0x3F) as u8);
    } else {
        bytes.push(0xF0 | (code >> 18) as u8);
        bytes.push(0x80 | ((code >> 12) & 0x3F) as u8);
        bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
        bytes.push(0x80 | (code & 0x3F) as u8);
    }
}

/// Encodes UTF-16 code units to WTF-8, combining a valid surrogate pair
/// into one astral code point and leaving a lone surrogate as a 3-byte
/// sequence.
pub fn wtf8_encode_utf16(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit)
            && index + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[index + 1])
        {
            let code = 0x10000
                + ((u32::from(unit) - 0xD800) << 10)
                + (u32::from(units[index + 1]) - 0xDC00);
            wtf8_push_code_point(&mut bytes, code);
            index += 2;
        } else {
            wtf8_push_code_point(&mut bytes, u32::from(unit));
            index += 1;
        }
    }
    bytes
}


/// Replaces UTF-16 string matches with ECMAScript string substitution patterns.
pub fn utf16_replace(value: &[u16], search: &[u16], replacement: &[u16], all: bool) -> Vec<u16> {
    let mut output = Vec::new();
    let mut cursor = 0;
    while cursor <= value.len() {
        let found = if search.is_empty() {
            Some(cursor)
        } else {
            value[cursor..].windows(search.len()).position(|part| part == search).map(|offset| cursor + offset)
        };
        let Some(start) = found else { break };
        let end = start + search.len();
        output.extend_from_slice(&value[cursor..start]);
        let mut index = 0;
        while index < replacement.len() {
            if replacement[index] == b'$' as u16 && index + 1 < replacement.len() {
                let fragment = match replacement[index + 1] {
                    x if x == b'$' as u16 => Some(&replacement[index..index + 1]),
                    x if x == b'&' as u16 => Some(&value[start..end]),
                    x if x == b'`' as u16 => Some(&value[..start]),
                    x if x == b'\'' as u16 => Some(&value[end..]),
                    _ => None,
                };
                if let Some(fragment) = fragment {
                    output.extend_from_slice(fragment);
                    index += 2;
                    continue;
                }
            }
            output.push(replacement[index]);
            index += 1;
        }
        cursor = end;
        if !all { break; }
        if search.is_empty() {
            if cursor == value.len() { break; }
            output.push(value[cursor]);
            cursor += 1;
        }
    }
    output.extend_from_slice(&value[cursor..]);
    output
}

/// Maps contiguous valid UTF-16 text while preserving lone surrogates unchanged.
pub fn utf16_map_segments(
    units: &[u16],
    mut map: impl FnMut(&str, &mut Vec<u16>),
) -> Vec<u16> {
    let mut output = Vec::with_capacity(units.len());
    let mut segment = String::new();
    for character in char::decode_utf16(units.iter().copied()) {
        match character {
            Ok(character) => segment.push(character),
            Err(error) => {
                map(&segment, &mut output);
                segment.clear();
                output.push(error.unpaired_surrogate());
            }
        }
    }
    map(&segment, &mut output);
    output
}
