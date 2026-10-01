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
