//! Small runtime JIT for residual operations that Thaw cannot specialize AOT.
//!
//! This intentionally is not a JavaScript engine. It accepts a compact numeric
//! expression IR and emits one W^X-protected native code stub per symbol.

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompareOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

impl NumericOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn opcode(self) -> u8 {
        match self {
            Self::Add => 0x58,
            Self::Subtract => 0x5c,
            Self::Multiply => 0x59,
            Self::Divide => 0x5e,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericValue {
    Left,
    Right,
    Constant(f64),
    Operation(NumericOp),
    Compare(CompareOp),
    Select,
}

struct NumericProgram(Vec<NumericValue>);

impl NumericProgram {
    fn parse(symbol: &str) -> Option<Self> {
        if let Some(encoded) = symbol.strip_prefix("expr:") {
            let encoded = encoded.split_once(':')?.0;
            let values = encoded
                .split(',')
                .map(|token| match token {
                    "x" => Some(NumericValue::Left),
                    "y" => Some(NumericValue::Right),
                    "+" => Some(NumericValue::Operation(NumericOp::Add)),
                    "-" => Some(NumericValue::Operation(NumericOp::Subtract)),
                    "*" => Some(NumericValue::Operation(NumericOp::Multiply)),
                    "/" => Some(NumericValue::Operation(NumericOp::Divide)),
                    "<" => Some(NumericValue::Compare(CompareOp::Less)),
                    "<=" => Some(NumericValue::Compare(CompareOp::LessEqual)),
                    ">" => Some(NumericValue::Compare(CompareOp::Greater)),
                    ">=" => Some(NumericValue::Compare(CompareOp::GreaterEqual)),
                    "==" => Some(NumericValue::Compare(CompareOp::Equal)),
                    "!=" => Some(NumericValue::Compare(CompareOp::NotEqual)),
                    "?" => Some(NumericValue::Select),
                    constant => constant
                        .strip_prefix('c')
                        .filter(|bits| bits.len() == 16)
                        .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                        .map(|bits| NumericValue::Constant(f64::from_bits(bits))),
                })
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty() && values.len() <= 128).then_some(Self(values))
        } else {
            let operation = symbol.split_once(':').map_or(symbol, |pair| pair.0);
            Some(Self(vec![
                NumericValue::Left,
                NumericValue::Right,
                NumericValue::Operation(NumericOp::parse(operation)?),
            ]))
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(&self) -> Option<Vec<u8>> {
        let mut code = Vec::with_capacity(self.0.len() * 12 + 8);
        // Preserve both arguments because expression temporaries use xmm0..xmm5.
        emit_move(&mut code, 6, 0);
        emit_move(&mut code, 7, 1);
        let mut depth = 0u8;
        for value in &self.0 {
            match value {
                NumericValue::Left | NumericValue::Right => {
                    if depth == 6 {
                        return None;
                    }
                    emit_move(
                        &mut code,
                        depth,
                        if *value == NumericValue::Left { 6 } else { 7 },
                    );
                    depth += 1;
                }
                NumericValue::Constant(value) => {
                    if depth == 6 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&value.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    depth += 1;
                }
                NumericValue::Operation(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        operation.opcode(),
                        0xc0 | (left << 3) | right,
                    ]);
                    depth -= 1;
                }
                NumericValue::Compare(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    emit_compare(&mut code, left, right, *operation);
                    depth -= 1;
                }
                NumericValue::Select => {
                    if depth < 3 {
                        return None;
                    }
                    let condition = depth - 3;
                    let consequent = depth - 2;
                    let alternate = depth - 1;
                    // Remove the sign bit before testing so both +0 and -0 are false;
                    // all other values, including NaN, retain JavaScript truthiness.
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                        0x74,
                        0x06,
                    ]);
                    emit_move(&mut code, condition, consequent);
                    code.extend_from_slice(&[0xeb, 0x04]);
                    emit_move(&mut code, condition, alternate);
                    depth -= 2;
                }
            }
        }
        (depth == 1).then(|| {
            code.push(0xc3);
            code
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_move(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[0x66, 0x0f, 0x28, 0xc0 | (destination << 3) | source]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_compare(code: &mut Vec<u8>, left: u8, right: u8, operation: CompareOp) {
    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (left << 3) | right]);
    let condition = match operation {
        CompareOp::Less => 0x92,
        CompareOp::LessEqual => 0x96,
        CompareOp::Greater => 0x97,
        CompareOp::GreaterEqual => 0x93,
        CompareOp::Equal => 0x94,
        CompareOp::NotEqual => 0x95,
    };
    code.extend_from_slice(&[0x0f, condition, 0xc0]);
    match operation {
        CompareOp::Less | CompareOp::LessEqual | CompareOp::Equal => {
            code.extend_from_slice(&[0x0f, 0x9b, 0xc2, 0x20, 0xd0]);
        }
        CompareOp::NotEqual => {
            code.extend_from_slice(&[0x0f, 0x9a, 0xc2, 0x08, 0xd0]);
        }
        CompareOp::Greater | CompareOp::GreaterEqual => {}
    }
    code.extend_from_slice(&[0x0f, 0xb6, 0xc0, 0xf2, 0x0f, 0x2a, 0xc0 | (left << 3)]);
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
fn compile(symbol: &str, program: &NumericProgram) -> Result<*mut libc::c_void, *const c_char> {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(code) = cache.get(symbol) {
        return Ok(code.0);
    }
    let bytes = program
        .machine_code()
        .ok_or_else(|| INVALID_SYMBOL.as_ptr().cast())?;
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
fn compile(_symbol: &str, _program: &NumericProgram) -> Result<*mut libc::c_void, *const c_char> {
    Err(UNSUPPORTED_TARGET.as_ptr().cast())
}

/// Compiles a validated numeric expression on first use and executes it.
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
    let Some(program) = NumericProgram::parse(symbol) else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    let code = match compile(symbol, &program) {
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

        let symbol = CString::new("expr:x,y,+,c4000000000000000,*:compound").unwrap();
        let result = unsafe { thaw_jit_call_f64(symbol.as_ptr(), 19.0, 2.0) };
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);

        let symbol =
            CString::new("expr:x,y,<,c4045000000000000,cc000000000000000,?:conditional").unwrap();
        let comparison = CString::new("expr:x,y,<:comparison").unwrap();
        assert_eq!(
            unsafe { thaw_jit_call_f64(comparison.as_ptr(), 2.0, 3.0) }.value,
            1.0
        );
        assert_eq!(
            unsafe { thaw_jit_call_f64(symbol.as_ptr(), 2.0, 3.0) }.value,
            42.0
        );
        assert_eq!(
            unsafe { thaw_jit_call_f64(symbol.as_ptr(), 3.0, 2.0) }.value,
            -2.0
        );

        for (operator, expected) in [("<", 0.0), ("==", 0.0), ("!=", 1.0)] {
            let symbol = CString::new(format!("expr:x,y,{operator}:nan")).unwrap();
            let result = unsafe { thaw_jit_call_f64(symbol.as_ptr(), f64::NAN, 1.0) };
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let symbol =
            CString::new("expr:x,c4045000000000000,cc000000000000000,?:truthiness").unwrap();
        assert_eq!(
            unsafe { thaw_jit_call_f64(symbol.as_ptr(), -0.0, 0.0) }.value,
            -2.0
        );
        assert_eq!(
            unsafe { thaw_jit_call_f64(symbol.as_ptr(), f64::NAN, 0.0) }.value,
            42.0
        );
    }
}
