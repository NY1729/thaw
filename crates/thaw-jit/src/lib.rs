//! Small runtime JIT for residual operations that Thaw cannot specialize AOT.
//!
//! This intentionally is not a JavaScript engine. The first tier only accepts
//! numeric binary operations and emits one W^X-protected native code stub per
//! symbol. More Dynamic IR instructions can be added when a real fallback site
//! needs them.

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::ptr;
use std::sync::{Mutex, OnceLock};

static INVALID_SYMBOL: &[u8] = b"invalid JIT symbol\0";
#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
static UNSUPPORTED_TARGET: &[u8] = b"JIT target is not supported\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static ALLOCATION_FAILED: &[u8] = b"failed to allocate JIT code\0";

#[repr(C)]
pub struct ThawJitResult {
    pub value: f64,
    pub error: *const c_char,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumericOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl NumericOp {
    fn parse(symbol: &str) -> Option<Self> {
        match symbol
            .split_once(':')
            .map_or(symbol, |(operation, _)| operation)
        {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(self) -> &'static [u8] {
        match self {
            Self::Add => &[0xf2, 0x0f, 0x58, 0xc1, 0xc3],
            Self::Subtract => &[0xf2, 0x0f, 0x5c, 0xc1, 0xc3],
            Self::Multiply => &[0xf2, 0x0f, 0x59, 0xc1, 0xc3],
            Self::Divide => &[0xf2, 0x0f, 0x5e, 0xc1, 0xc3],
        }
    }
}

struct Code(*mut libc::c_void);

unsafe impl Send for Code {}
unsafe impl Sync for Code {}

impl Drop for Code {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.0, page_size()) };
    }
}

fn page_size() -> usize {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as usize
    } else {
        4096
    }
}

fn cache() -> &'static Mutex<HashMap<String, Code>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Code>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile(symbol: &str, operation: NumericOp) -> Result<*mut libc::c_void, *const c_char> {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(code) = cache.get(symbol) {
        return Ok(code.0);
    }
    let size = page_size();
    let memory = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == libc::MAP_FAILED {
        return Err(ALLOCATION_FAILED.as_ptr().cast());
    }
    let bytes = operation.machine_code();
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), memory.cast(), bytes.len());
        if libc::mprotect(memory, size, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            libc::munmap(memory, size);
            return Err(ALLOCATION_FAILED.as_ptr().cast());
        }
    }
    cache.insert(symbol.to_owned(), Code(memory));
    Ok(memory)
}

#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
fn compile(_symbol: &str, _operation: NumericOp) -> Result<*mut libc::c_void, *const c_char> {
    Err(UNSUPPORTED_TARGET.as_ptr().cast())
}

/// Compiles `add:*`, `sub:*`, `mul:*`, or `div:*` on first use and executes it.
///
/// # Safety
///
/// `symbol` must point to a live NUL-terminated string for this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_jit_call_f64(
    symbol: *const c_char,
    left: f64,
    right: f64,
) -> ThawJitResult {
    let Some(symbol) = (!symbol.is_null())
        .then(|| CStr::from_ptr(symbol).to_str().ok())
        .flatten()
    else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    let Some(operation) = NumericOp::parse(symbol) else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    let code = match compile(symbol, operation) {
        Ok(code) => code,
        Err(error) => return ThawJitResult { value: 0.0, error },
    };
    let function = std::mem::transmute::<*mut libc::c_void, extern "C" fn(f64, f64) -> f64>(code);
    ThawJitResult {
        value: function(left, right),
        error: ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn specializes_numeric_operations_and_reuses_code() {
        for (symbol, expected) in [
            ("add:test", 42.0),
            ("sub:test", 0.0),
            ("mul:test", 441.0),
            ("div:test", 1.0),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = unsafe { thaw_jit_call_f64(symbol.as_ptr(), 21.0, 21.0) };
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
            let repeated = unsafe { thaw_jit_call_f64(symbol.as_ptr(), 20.0, 22.0) };
            assert!(repeated.error.is_null());
        }
    }
}
