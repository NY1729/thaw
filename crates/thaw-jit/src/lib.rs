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

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
#[link(name = "m")]
extern "C" {
    fn fmod(left: f64, right: f64) -> f64;
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn minimum(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left == right {
        return if left == 0.0 {
            f64::from_bits(left.to_bits() | right.to_bits())
        } else {
            left
        };
    }
    if left < right {
        left
    } else {
        right
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn maximum(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left == right {
        return if left == 0.0 {
            f64::from_bits(left.to_bits() & right.to_bits())
        } else {
            left
        };
    }
    if left > right {
        left
    } else {
        right
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn floor_number(value: f64) -> f64 {
    value.floor()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn ceil_number(value: f64) -> f64 {
    value.ceil()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn truncate_number(value: f64) -> f64 {
    value.trunc()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn round_number(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let rounded = (value + 0.5).floor();
    if rounded == 0.0 && value.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let mut modulo = value.trunc() % 4_294_967_296.0;
    if modulo < 0.0 {
        modulo += 4_294_967_296.0;
    }
    modulo as u32
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_and(left: f64, right: f64) -> f64 {
    (to_uint32(left) & to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_or(left: f64, right: f64) -> f64 {
    (to_uint32(left) | to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_xor(left: f64, right: f64) -> f64 {
    (to_uint32(left) ^ to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_left(left: f64, right: f64) -> f64 {
    (to_uint32(left) << (to_uint32(right) & 31)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_right(left: f64, right: f64) -> f64 {
    ((to_uint32(left) as i32) >> (to_uint32(right) & 31)) as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_right_unsigned(left: f64, right: f64) -> f64 {
    (to_uint32(left) >> (to_uint32(right) & 31)) as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_not(value: f64) -> f64 {
    (!to_uint32(value)) as i32 as f64
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryMath {
    Ceil,
    Floor,
    Round,
    SquareRoot,
    Truncate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BitwiseOp {
    And,
    Or,
    ShiftLeft,
    ShiftRight,
    ShiftRightUnsigned,
    Xor,
}

impl BitwiseOp {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64, f64) -> f64 {
        match self {
            Self::And => bit_and,
            Self::Or => bit_or,
            Self::ShiftLeft => shift_left,
            Self::ShiftRight => shift_right,
            Self::ShiftRightUnsigned => shift_right_unsigned,
            Self::Xor => bit_xor,
        }
    }
}

impl UnaryMath {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64) -> f64 {
        match self {
            Self::Ceil => ceil_number,
            Self::Floor => floor_number,
            Self::Round => round_number,
            Self::SquareRoot => unreachable!("square root emits SSE2 directly"),
            Self::Truncate => truncate_number,
        }
    }
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
    Argument(u8),
    Constant(f64),
    Operation(NumericOp),
    Compare(CompareOp),
    Bitwise(BitwiseOp),
    BitNot,
    Absolute,
    Negate,
    Maximum,
    Minimum,
    UnaryMath(UnaryMath),
    Remainder,
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
                    "x" => Some(NumericValue::Argument(0)),
                    "y" => Some(NumericValue::Argument(1)),
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
                    "band" => Some(NumericValue::Bitwise(BitwiseOp::And)),
                    "bor" => Some(NumericValue::Bitwise(BitwiseOp::Or)),
                    "bxor" => Some(NumericValue::Bitwise(BitwiseOp::Xor)),
                    "shl" => Some(NumericValue::Bitwise(BitwiseOp::ShiftLeft)),
                    "shr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRight)),
                    "ushr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRightUnsigned)),
                    "bnot" => Some(NumericValue::BitNot),
                    "abs" => Some(NumericValue::Absolute),
                    "neg" => Some(NumericValue::Negate),
                    "max" => Some(NumericValue::Maximum),
                    "min" => Some(NumericValue::Minimum),
                    "ceil" => Some(NumericValue::UnaryMath(UnaryMath::Ceil)),
                    "floor" => Some(NumericValue::UnaryMath(UnaryMath::Floor)),
                    "round" => Some(NumericValue::UnaryMath(UnaryMath::Round)),
                    "sqrt" => Some(NumericValue::UnaryMath(UnaryMath::SquareRoot)),
                    "trunc" => Some(NumericValue::UnaryMath(UnaryMath::Truncate)),
                    "%" => Some(NumericValue::Remainder),
                    "?" => Some(NumericValue::Select),
                    value => value
                        .strip_prefix('a')
                        .and_then(|index| index.parse::<u8>().ok())
                        .filter(|index| *index < 16)
                        .map(NumericValue::Argument)
                        .or_else(|| {
                            value
                                .strip_prefix('c')
                                .filter(|bits| bits.len() == 16)
                                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                                .map(|bits| NumericValue::Constant(f64::from_bits(bits)))
                        }),
                })
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty() && values.len() <= 128).then_some(Self(values))
        } else {
            let operation = symbol.split_once(':').map_or(symbol, |pair| pair.0);
            Some(Self(vec![
                NumericValue::Argument(0),
                NumericValue::Argument(1),
                NumericValue::Operation(NumericOp::parse(operation)?),
            ]))
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(&self) -> Option<Vec<u8>> {
        let mut code = Vec::with_capacity(self.0.len() * 12 + 8);
        let mut depth = 0u8;
        for value in &self.0 {
            match value {
                NumericValue::Argument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    depth += 1;
                }
                NumericValue::Constant(value) => {
                    if depth == 8 {
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
                NumericValue::Bitwise(operation) => {
                    if depth != 2 {
                        return None;
                    }
                    emit_call(&mut code, operation.function() as *const () as u64);
                    depth = 1;
                }
                NumericValue::BitNot => {
                    if depth != 1 {
                        return None;
                    }
                    emit_call(&mut code, bit_not as *const () as u64);
                }
                NumericValue::Absolute => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf0);
                }
                NumericValue::Negate => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf8);
                }
                NumericValue::Remainder => {
                    if depth != 2 {
                        return None;
                    }
                    emit_call(&mut code, fmod as *const () as u64);
                    depth = 1;
                }
                NumericValue::Minimum | NumericValue::Maximum => {
                    if depth != 2 {
                        return None;
                    }
                    let function = if matches!(value, NumericValue::Minimum) {
                        minimum
                    } else {
                        maximum
                    };
                    emit_call(&mut code, function as *const () as u64);
                    depth = 1;
                }
                NumericValue::UnaryMath(operation) => {
                    if depth == 0 {
                        return None;
                    }
                    if *operation == UnaryMath::SquareRoot {
                        let register = depth - 1;
                        code.extend_from_slice(&[
                            0xf2,
                            0x0f,
                            0x51,
                            0xc0 | (register << 3) | register,
                        ]);
                    } else {
                        if depth != 1 {
                            return None;
                        }
                        emit_call(&mut code, operation.function() as *const () as u64);
                    }
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

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_move(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[0x66, 0x0f, 0x28, 0xc0 | (destination << 3) | source]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_bit_operation(code: &mut Vec<u8>, value: u8, operation: u8) {
    code.extend_from_slice(&[
        0x66,
        0x48,
        0x0f,
        0x7e,
        0xc0 | (value << 3),
        0x48,
        0x0f,
        0xba,
        operation,
        0x3f,
        0x66,
        0x48,
        0x0f,
        0x6e,
        0xc0 | (value << 3),
    ]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_call(code: &mut Vec<u8>, function: u64) {
    code.extend_from_slice(&[0x48, 0x83, 0xec, 0x08, 0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0, 0x48, 0x83, 0xc4, 0x08]);
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
/// `symbol` must point to a live NUL-terminated string for this call. When
/// `arg_count` is nonzero, `args` must reference at least that many `f64`s.
#[no_mangle]
pub unsafe extern "C" fn thaw_jit_call_f64(
    symbol: *const c_char,
    args: *const f64,
    arg_count: usize,
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
    if program.required_args() > arg_count || (arg_count != 0 && args.is_null()) {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    }
    let code = match compile(symbol, &program) {
        Ok(code) => code,
        Err(error) => return ThawJitResult { value: 0.0, error },
    };
    let function = std::mem::transmute::<*mut libc::c_void, extern "C" fn(*const f64) -> f64>(code);
    ThawJitResult {
        value: function(args),
        error: ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn call(symbol: &CString, args: &[f64]) -> ThawJitResult {
        unsafe { thaw_jit_call_f64(symbol.as_ptr(), args.as_ptr(), args.len()) }
    }

    #[test]
    fn specializes_numeric_operations_and_reuses_code() {
        for (symbol, expected) in [
            ("add:test", 42.0),
            ("sub:test", 0.0),
            ("mul:test", 441.0),
            ("div:test", 1.0),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[21.0, 21.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
            let repeated = call(&symbol, &[20.0, 22.0]);
            assert!(repeated.error.is_null());
        }

        let symbol = CString::new("expr:x,y,+,c4000000000000000,*:compound").unwrap();
        let result = call(&symbol, &[19.0, 2.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);

        let symbol = CString::new("expr:a0,a1,+,a2,+:three_args").unwrap();
        let result = call(&symbol, &[10.0, 11.0, 21.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        assert!(!call(&symbol, &[10.0, 11.0]).error.is_null());

        let symbol =
            CString::new("expr:x,y,<,c4045000000000000,cc000000000000000,?:conditional").unwrap();
        let comparison = CString::new("expr:x,y,<:comparison").unwrap();
        assert_eq!(call(&comparison, &[2.0, 3.0]).value, 1.0);
        assert_eq!(call(&symbol, &[2.0, 3.0]).value, 42.0);
        assert_eq!(call(&symbol, &[3.0, 2.0]).value, -2.0);

        for (operator, expected) in [("<", 0.0), ("==", 0.0), ("!=", 1.0)] {
            let symbol = CString::new(format!("expr:x,y,{operator}:nan")).unwrap();
            let result = call(&symbol, &[f64::NAN, 1.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let symbol =
            CString::new("expr:x,c4045000000000000,cc000000000000000,?:truthiness").unwrap();
        assert_eq!(call(&symbol, &[-0.0]).value, -2.0);
        assert_eq!(call(&symbol, &[f64::NAN]).value, 42.0);

        let symbol = CString::new("expr:a0,neg:negate").unwrap();
        assert_eq!(call(&symbol, &[42.0]).value, -42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());

        let symbol = CString::new("expr:c4045000000000000:constant").unwrap();
        assert_eq!(call(&symbol, &[]).value, 42.0);

        let symbol = CString::new("expr:a0,a1,%:remainder").unwrap();
        assert_eq!(call(&symbol, &[5.5, 2.0]).value, 1.5);
        assert_eq!(call(&symbol, &[-5.5, 2.0]).value, -1.5);
        assert!(call(&symbol, &[f64::INFINITY, 2.0]).value.is_nan());

        let symbol = CString::new("expr:a0,abs:absolute").unwrap();
        assert_eq!(call(&symbol, &[-42.0]).value, 42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());
        assert!(call(&symbol, &[f64::NAN]).value.is_nan());

        let minimum = CString::new("expr:a0,a1,min:minimum").unwrap();
        let maximum = CString::new("expr:a0,a1,max:maximum").unwrap();
        assert_eq!(call(&minimum, &[42.0, 43.0]).value, 42.0);
        assert_eq!(call(&maximum, &[41.0, 42.0]).value, 42.0);
        assert!(call(&minimum, &[f64::NAN, 42.0]).value.is_nan());
        assert!(call(&maximum, &[42.0, f64::NAN]).value.is_nan());
        assert_eq!(
            call(&minimum, &[0.0, -0.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(
            call(&maximum, &[-0.0, 0.0]).value.to_bits(),
            0.0f64.to_bits()
        );

        for (operation, input, expected) in [
            ("floor", -1.2, -2.0),
            ("ceil", -1.2, -1.0),
            ("trunc", -1.2, -1.0),
            ("round", -1.5, -1.0),
            ("sqrt", 9.0, 3.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            assert_eq!(call(&symbol, &[input]).value, expected);
        }
        let round = CString::new("expr:a0,round:round_zero").unwrap();
        assert_eq!(call(&round, &[-0.5]).value.to_bits(), (-0.0f64).to_bits());
        let square_root = CString::new("expr:a0,sqrt:sqrt_nan").unwrap();
        assert!(call(&square_root, &[-1.0]).value.is_nan());

        for (expression, args, expected) in [
            ("a0,a1,bor", [4_294_967_297.0, 0.0], 1.0),
            ("a0,a1,band", [f64::NAN, 1.0], 0.0),
            ("a0,a1,bxor", [43.0, 1.0], 42.0),
            ("a0,a1,shl", [1.0, 33.0], 2.0),
            ("a0,a1,shr", [-4.0, 1.0], -2.0),
            ("a0,a1,ushr", [-1.0, 0.0], 4_294_967_295.0),
        ] {
            let symbol = CString::new(format!("expr:{expression}:bitwise")).unwrap();
            assert_eq!(call(&symbol, &args).value, expected);
        }
        let bit_not = CString::new("expr:a0,bnot:bit_not").unwrap();
        assert_eq!(call(&bit_not, &[0.0]).value, -1.0);
    }
}
