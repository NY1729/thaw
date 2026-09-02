//! Small runtime JIT for residual operations that Thaw cannot specialize AOT.
//!
//! This intentionally is not a JavaScript engine. It accepts a compact numeric
//! expression IR and emits one W^X-protected native code stub per symbol.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

struct JitGlobals {
    slots: [OnceLock<AtomicU64>; 16],
    callable_entries: Mutex<HashMap<(u8, String), u64>>,
}

static INVALID_SYMBOL: &[u8] = b"invalid JIT symbol\0";
const ABSENT_STATUS: *const c_char = ptr::dangling();
const NULL_STATUS: *const c_char = 2usize as *const c_char;
const ARRAY_RESULT_TAG: u64 = 1;
const DYNAMIC_NUMBER_TAG: u64 = 1;
const DYNAMIC_STRING_TAG: u64 = 2;
const DYNAMIC_BOOLEAN_TAG: u64 = 3;
const DYNAMIC_NUMBER_ARRAY_TAG: u64 = 4;
const DYNAMIC_BOOLEAN_ARRAY_TAG: u64 = 5;
const DYNAMIC_STRING_ARRAY_TAG: u64 = 6;
const DYNAMIC_NUMBER_DICTIONARY_TAG: u64 = 7;
const DYNAMIC_BOOLEAN_DICTIONARY_TAG: u64 = 8;
const DYNAMIC_STRING_DICTIONARY_TAG: u64 = 9;
const DYNAMIC_OBJECT_TAG: u64 = 10;
const DYNAMIC_TUPLE_TAG: u64 = 11;
#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
static UNSUPPORTED_TARGET: &[u8] = b"JIT target is not supported\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static ALLOCATION_FAILED: &[u8] = b"failed to allocate JIT code\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_REPEAT_COUNT: &[u8] = b"invalid string repeat count\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_FIXED_DIGITS: &[u8] = b"toFixed() digits argument must be between 0 and 100\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_PRECISION: &[u8] = b"toPrecision() argument must be between 1 and 100\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_RADIX: &[u8] = b"toString() radix argument must be between 2 and 36\0";
static INVALID_EXPONENTIAL_DIGITS: &[u8] =
    b"toExponential() digits argument must be between 0 and 100\0";
static INVALID_NORMALIZATION_FORM: &[u8] = b"invalid Unicode normalization form\0";
static INVALID_ARRAY_WITH_INDEX: &[u8] = b"Invalid index for Array.prototype.with\0";
static INVALID_CODE_POINT: &[u8] = b"Invalid code point\0";
static EMPTY_REDUCE: &[u8] = b"Reduce of empty array with no initial value\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static VALUE_NOT_CALLABLE: &[u8] = b"value is not a function\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_DYNAMIC_VALUE: &[u8] = b"invalid dynamic JIT value\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static TRUE_THROW: &[u8] = b"true\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static FALSE_THROW: &[u8] = b"false\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static COMMA: &[u8] = b",\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static OBJECT_THROW: &[u8] = b"[object Object]\0";

pub type ArenaAlloc = unsafe extern "C" fn(usize, usize) -> *mut u8;
pub type NumberToString = unsafe extern "C" fn(f64) -> *const c_char;
pub type StringToNumber = unsafe extern "C" fn(*const c_char) -> f64;
pub type ParseFloat = unsafe extern "C" fn(*const c_char) -> f64;
pub type ParseInt = unsafe extern "C" fn(*const c_char, f64) -> f64;
pub type NumberFormat = unsafe extern "C" fn(u8, f64, f64) -> *const c_char;
pub type ArraySearch = unsafe extern "C" fn(u8, *const u8, f64, f64) -> f64;
pub type ArrayFormat = unsafe extern "C" fn(u8, *const u8, *const c_char) -> *const c_char;
pub type StringNormalize = unsafe extern "C" fn(*const c_char, *const c_char) -> *const c_char;
pub type StringSplit = unsafe extern "C" fn(*const c_char, *const c_char, f64) -> *mut u8;
pub type StringToArray = unsafe extern "C" fn(*const c_char) -> *mut u8;
pub type ArraySlice = unsafe extern "C" fn(*const u8, usize, f64, f64) -> *mut u8;
pub type ArrayConcat = unsafe extern "C" fn(*const u8, *const u8, usize) -> *mut u8;
pub type ArrayAppend = unsafe extern "C" fn(u8, *const u8, f64) -> *mut u8;
pub type ArrayToReversed = unsafe extern "C" fn(*const u8, usize) -> *mut u8;
pub type ArrayToSorted = unsafe extern "C" fn(u8, *const u8) -> *mut u8;
pub type ArrayReverse = unsafe extern "C" fn(*mut u8, usize) -> *mut u8;
pub type ArraySort = unsafe extern "C" fn(u8, *mut u8) -> *mut u8;
pub type ArrayFill = unsafe extern "C" fn(u8, *mut u8, f64, f64, f64) -> *mut u8;
pub type ArrayCopyWithin = unsafe extern "C" fn(*mut u8, usize, f64, f64, f64) -> *mut u8;
pub type ArrayPush = unsafe extern "C" fn(u8, *mut *mut u8, f64) -> f64;
pub type ArrayRemove = unsafe extern "C" fn(u8, *mut *mut u8, *mut f64) -> i8;
pub type ArraySplice = unsafe extern "C" fn(*mut *mut u8, f64, f64, *const u8) -> *mut u8;
pub type ArraySet = unsafe extern "C" fn(u8, *mut *mut u8, f64, f64) -> i8;
pub type ArrayWith = unsafe extern "C" fn(u8, *const u8, f64, f64) -> *mut u8;
pub type NumberSource = unsafe extern "C" fn() -> f64;
pub type DictionaryGet = unsafe extern "C" fn(u8, *mut libc::c_void, *const c_char) -> f64;
pub type DictionaryMutate = unsafe extern "C" fn(u8, *mut libc::c_void, *const c_char, f64) -> f64;
pub type DictionaryQuery = unsafe extern "C" fn(u8, *mut libc::c_void, *const c_char) -> f64;

thread_local! {
    static ARENA_ALLOC: Cell<Option<ArenaAlloc>> = const { Cell::new(None) };
    static NUMBER_TO_STRING: Cell<Option<NumberToString>> = const { Cell::new(None) };
    static STRING_TO_NUMBER: Cell<Option<StringToNumber>> = const { Cell::new(None) };
    static PARSE_FLOAT: Cell<Option<ParseFloat>> = const { Cell::new(None) };
    static PARSE_INT: Cell<Option<ParseInt>> = const { Cell::new(None) };
    static NUMBER_FORMAT: Cell<Option<NumberFormat>> = const { Cell::new(None) };
    static ARRAY_SEARCH: Cell<Option<ArraySearch>> = const { Cell::new(None) };
    static ARRAY_FORMAT: Cell<Option<ArrayFormat>> = const { Cell::new(None) };
    static STRING_NORMALIZE: Cell<Option<StringNormalize>> = const { Cell::new(None) };
    static STRING_SPLIT: Cell<Option<StringSplit>> = const { Cell::new(None) };
    static STRING_TO_ARRAY: Cell<Option<StringToArray>> = const { Cell::new(None) };
    static STRING_FROM_CHAR_CODE: Cell<Option<NumberToString>> = const { Cell::new(None) };
    static STRING_FROM_CODE_POINT: Cell<Option<NumberToString>> = const { Cell::new(None) };
    static ARRAY_SLICE: Cell<Option<ArraySlice>> = const { Cell::new(None) };
    static ARRAY_CONCAT: Cell<Option<ArrayConcat>> = const { Cell::new(None) };
    static ARRAY_APPEND: Cell<Option<ArrayAppend>> = const { Cell::new(None) };
    static ARRAY_TO_REVERSED: Cell<Option<ArrayToReversed>> = const { Cell::new(None) };
    static ARRAY_TO_SORTED: Cell<Option<ArrayToSorted>> = const { Cell::new(None) };
    static ARRAY_REVERSE: Cell<Option<ArrayReverse>> = const { Cell::new(None) };
    static ARRAY_SORT: Cell<Option<ArraySort>> = const { Cell::new(None) };
    static ARRAY_FILL: Cell<Option<ArrayFill>> = const { Cell::new(None) };
    static ARRAY_COPY_WITHIN: Cell<Option<ArrayCopyWithin>> = const { Cell::new(None) };
    static ARRAY_PUSH: Cell<Option<ArrayPush>> = const { Cell::new(None) };
    static ARRAY_UNSHIFT: Cell<Option<ArrayPush>> = const { Cell::new(None) };
    static ARRAY_REMOVE: Cell<Option<ArrayRemove>> = const { Cell::new(None) };
    static ARRAY_SPLICE: Cell<Option<ArraySplice>> = const { Cell::new(None) };
    static ARRAY_SET: Cell<Option<ArraySet>> = const { Cell::new(None) };
    static ARRAY_WITH: Cell<Option<ArrayWith>> = const { Cell::new(None) };
    static MATH_RANDOM: Cell<Option<NumberSource>> = const { Cell::new(None) };
    static DATE_NOW: Cell<Option<NumberSource>> = const { Cell::new(None) };
    static PERFORMANCE_NOW: Cell<Option<NumberSource>> = const { Cell::new(None) };
    static PROCESS_PID: Cell<Option<NumberSource>> = const { Cell::new(None) };
    static PROCESS_PPID: Cell<Option<NumberSource>> = const { Cell::new(None) };
    static DICTIONARY_GET: Cell<Option<DictionaryGet>> = const { Cell::new(None) };
    static DICTIONARY_MUTATE: Cell<Option<DictionaryMutate>> = const { Cell::new(None) };
    static DICTIONARY_QUERY: Cell<Option<DictionaryQuery>> = const { Cell::new(None) };
    static CALL_ERROR: Cell<*const c_char> = const { Cell::new(ptr::null()) };
    static CALL_PRESENT: Cell<bool> = const { Cell::new(true) };
    static CALL_ABSENCE: Cell<u8> = const { Cell::new(1) };
    static JIT_GLOBALS: Cell<*const JitGlobals> = const { Cell::new(ptr::null()) };
    static DYNAMIC_VALUES: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn global_get(index: f64) -> f64 {
    if !index.is_finite() || index.fract() != 0.0 || !(0.0..16.0).contains(&index) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let index = index as usize;
    let globals = JIT_GLOBALS.with(Cell::get);
    if globals.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    f64::from_bits(
        unsafe { &*globals }.slots[index]
            .get_or_init(|| AtomicU64::new(0))
            .load(Ordering::Relaxed),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn global_set(index: f64, value: f64) -> f64 {
    if !index.is_finite() || index.fract() != 0.0 || !(0.0..16.0).contains(&index) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let index = index as usize;
    let globals = JIT_GLOBALS.with(Cell::get);
    if globals.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    unsafe { &*globals }.slots[index]
        .get_or_init(|| AtomicU64::new(0))
        .store(value.to_bits(), Ordering::Relaxed);
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn global_init(index: f64, initial: f64) -> f64 {
    if !index.is_finite() || index.fract() != 0.0 || !(0.0..16.0).contains(&index) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let globals = JIT_GLOBALS.with(Cell::get);
    if globals.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    f64::from_bits(
        unsafe { &*globals }.slots[index as usize]
            .get_or_init(|| AtomicU64::new(initial.to_bits()))
            .load(Ordering::Relaxed),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn callable_entry_get(key: f64, table: f64) -> f64 {
    if !table.is_finite() || table.fract() != 0.0 || !(0.0..=u8::MAX as f64).contains(&table) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return -1.0;
    }
    let key = key.to_bits() as usize as *const c_char;
    let globals = JIT_GLOBALS.with(Cell::get);
    if key.is_null() || globals.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return -1.0;
    }
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy();
    unsafe { &*globals }
        .callable_entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&(table as u8, key.into_owned()))
        .map_or(-1.0, |selection| *selection as f64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn callable_entry_set(key: f64, table: f64, selection: f64) -> f64 {
    if !selection.is_finite()
        || selection.fract() != 0.0
        || !(0.0..=u64::MAX as f64).contains(&selection)
    {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let globals = JIT_GLOBALS.with(Cell::get);
    let key = key.to_bits() as usize as *const c_char;
    if globals.is_null()
        || key.is_null()
        || !table.is_finite()
        || table.fract() != 0.0
        || !(0.0..=u8::MAX as f64).contains(&table)
    {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let key = unsafe { CStr::from_ptr(key) }
        .to_string_lossy()
        .into_owned();
    unsafe { &*globals }
        .callable_entries
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert((table as u8, key), selection as u64);
    selection
}

extern "C" fn dictionary_get(value: f64, key: f64, kind: u8) -> f64 {
    let Some(get) = DICTIONARY_GET.with(Cell::get) else {
        return 0.0;
    };
    unsafe {
        get(
            kind,
            value.to_bits() as usize as *mut libc::c_void,
            key.to_bits() as usize as *const c_char,
        )
    }
}

extern "C" fn number_dictionary_get(value: f64, key: f64) -> f64 {
    dictionary_get(value, key, 0)
}

extern "C" fn bool_dictionary_get(value: f64, key: f64) -> f64 {
    dictionary_get(value, key, 1)
}

extern "C" fn string_dictionary_get(value: f64, key: f64) -> f64 {
    dictionary_get(value, key, 2)
}

extern "C" fn dictionary_mutate(object: f64, key: f64, value: f64, kind: u8) -> f64 {
    let Some(mutate) = DICTIONARY_MUTATE.with(Cell::get) else {
        return 0.0;
    };
    unsafe {
        mutate(
            kind,
            object.to_bits() as usize as *mut libc::c_void,
            key.to_bits() as usize as *const c_char,
            value,
        )
    }
}

extern "C" fn number_dictionary_set(object: f64, key: f64, value: f64) -> f64 {
    dictionary_mutate(object, key, value, 0)
}

extern "C" fn bool_dictionary_set(object: f64, key: f64, value: f64) -> f64 {
    dictionary_mutate(object, key, value, 1)
}

extern "C" fn boolean_not(value: f64) -> f64 {
    f64::from(value == 0.0 || value.is_nan())
}

extern "C" fn string_dictionary_set(object: f64, key: f64, value: f64) -> f64 {
    dictionary_mutate(object, key, value, 2)
}

extern "C" fn dictionary_delete(object: f64, key: f64) -> f64 {
    dictionary_mutate(object, key, 0.0, 3)
}

extern "C" fn dictionary_query(object: f64, key: f64, operation: u8) -> f64 {
    let Some(query) = DICTIONARY_QUERY.with(Cell::get) else {
        return 0.0;
    };
    unsafe {
        query(
            operation,
            object.to_bits() as usize as *mut libc::c_void,
            key.to_bits() as usize as *const c_char,
        )
    }
}

extern "C" fn dictionary_has_own(object: f64, key: f64) -> f64 {
    dictionary_query(object, key, 0)
}

extern "C" fn dictionary_in(key: f64, object: f64) -> f64 {
    dictionary_query(object, key, 0)
}

extern "C" fn dictionary_keys(object: f64) -> f64 {
    dictionary_query(object, 0.0, 1)
}

extern "C" fn number_dictionary_values(object: f64) -> f64 {
    dictionary_query(object, 0.0, 2)
}

extern "C" fn bool_dictionary_values(object: f64) -> f64 {
    dictionary_query(object, 0.0, 3)
}

extern "C" fn string_dictionary_values(object: f64) -> f64 {
    dictionary_query(object, 0.0, 4)
}

extern "C" fn number_dictionary_entries(object: f64) -> f64 {
    dictionary_query(object, 0.0, 5)
}

extern "C" fn bool_dictionary_entries(object: f64) -> f64 {
    dictionary_query(object, 0.0, 6)
}

extern "C" fn string_dictionary_entries(object: f64) -> f64 {
    dictionary_query(object, 0.0, 7)
}

extern "C" fn dictionary_from_number_entries(entries: f64) -> f64 {
    dictionary_query(entries, 0.0, 8)
}

extern "C" fn dictionary_from_bool_entries(entries: f64) -> f64 {
    dictionary_query(entries, 0.0, 9)
}

extern "C" fn dictionary_from_string_entries(entries: f64) -> f64 {
    dictionary_query(entries, 0.0, 10)
}

extern "C" fn dictionary_assign(target: f64, source: f64) -> f64 {
    dictionary_query(target, source, 11)
}

extern "C" fn empty_dictionary() -> f64 {
    dictionary_query(0.0, 0.0, 12)
}

extern "C" fn dictionary_length(object: f64) -> f64 {
    dictionary_query(object, 0.0, 13)
}

extern "C" fn dictionary_key_at(object: f64, index: f64) -> f64 {
    dictionary_query(object, index, 14)
}

static STRING_CONSTANTS: OnceLock<Mutex<HashMap<String, CString>>> = OnceLock::new();

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
fn array_extreme(value: f64, is_min: bool) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut result = if is_min {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    };
    for index in 0..length {
        let value = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        result = if is_min {
            minimum(result, value)
        } else {
            maximum(result, value)
        };
    }
    result
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_min(value: f64) -> f64 {
    array_extreme(value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_max(value: f64) -> f64 {
    array_extreme(value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_hypot(value: f64) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut result = 0.0f64;
    for index in 0..length {
        result = result.hypot(unsafe { array.add(8 + index * 8).cast::<f64>().read() });
    }
    result
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_reduce(
    value: f64,
    initial: Option<f64>,
    from_right: bool,
    operation: impl Fn(f64, f64) -> f64,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if from_right {
        let (mut accumulator, end) = match initial {
            Some(initial) => (initial, length),
            None if length != 0 => (
                unsafe { array.add(8 + (length - 1) * 8).cast::<f64>().read() },
                length - 1,
            ),
            None => {
                CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
                return 0.0;
            }
        };
        for index in (0..end).rev() {
            accumulator = operation(accumulator, unsafe {
                array.add(8 + index * 8).cast::<f64>().read()
            });
        }
        return accumulator;
    }
    let (mut accumulator, start) = match initial {
        Some(initial) => (initial, 0),
        None if length != 0 => (unsafe { array.add(8).cast::<f64>().read() }, 1),
        None => {
            CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
            return 0.0;
        }
    };
    for index in start..length {
        accumulator = operation(accumulator, unsafe {
            array.add(8 + index * 8).cast::<f64>().read()
        });
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_reducers {
    ($forward:ident, $first:ident, $reverse:ident, $last:ident, $operation:expr) => {
        extern "C" fn $forward(value: f64, accumulator: f64) -> f64 {
            number_array_reduce(value, Some(accumulator), false, $operation)
        }
        extern "C" fn $first(value: f64, _unused: f64) -> f64 {
            number_array_reduce(value, None, false, $operation)
        }
        extern "C" fn $reverse(value: f64, accumulator: f64) -> f64 {
            number_array_reduce(value, Some(accumulator), true, $operation)
        }
        extern "C" fn $last(value: f64, _unused: f64) -> f64 {
            number_array_reduce(value, None, true, $operation)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_add,
    number_array_reduce_add_first,
    number_array_reduce_add_right,
    number_array_reduce_add_last,
    |left, right| left + right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_subtract,
    number_array_reduce_subtract_first,
    number_array_reduce_subtract_right,
    number_array_reduce_subtract_last,
    |left, right| left - right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_multiply,
    number_array_reduce_multiply_first,
    number_array_reduce_multiply_right,
    number_array_reduce_multiply_last,
    |left, right| left * right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_divide,
    number_array_reduce_divide_first,
    number_array_reduce_divide_right,
    number_array_reduce_divide_last,
    |left, right| left / right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_remainder,
    number_array_reduce_remainder_first,
    number_array_reduce_remainder_right,
    number_array_reduce_remainder_last,
    |left, right| left % right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_power,
    number_array_reduce_power_first,
    number_array_reduce_power_right,
    number_array_reduce_power_last,
    |left, right| power(left, right)
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_minimum,
    number_array_reduce_minimum_first,
    number_array_reduce_minimum_right,
    number_array_reduce_minimum_last,
    |left, right| minimum(left, right)
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_maximum,
    number_array_reduce_maximum_first,
    number_array_reduce_maximum_right,
    number_array_reduce_maximum_last,
    |left, right| maximum(left, right)
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_quantify(
    value: f64,
    operand: f64,
    every: bool,
    predicate: impl Fn(f64, f64) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    for index in 0..length {
        let matches = predicate(
            unsafe { array.add(8 + index * 8).cast::<f64>().read() },
            operand,
        );
        if matches != every {
            return f64::from(!every);
        }
    }
    f64::from(every)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_quantifiers {
    ($some:ident, $every:ident, $predicate:expr) => {
        extern "C" fn $some(value: f64, operand: f64) -> f64 {
            number_array_quantify(value, operand, false, $predicate)
        }
        extern "C" fn $every(value: f64, operand: f64) -> f64 {
            number_array_quantify(value, operand, true, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_lt,
    number_array_every_lt,
    |left, right| left < right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_lte,
    number_array_every_lte,
    |left, right| left <= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_gt,
    number_array_every_gt,
    |left, right| left > right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_gte,
    number_array_every_gte,
    |left, right| left >= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_eq,
    number_array_every_eq,
    |left, right| left == right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_ne,
    number_array_every_ne,
    |left, right| left != right
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_find(
    value: f64,
    operand: f64,
    mode: u8,
    predicate: impl Fn(f64, f64) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let matches = |index: usize| {
        predicate(
            unsafe { array.add(8 + index * 8).cast::<f64>().read() },
            operand,
        )
    };
    let index = if mode >= 2 {
        (0..length).rev().find(|index| matches(*index))
    } else {
        (0..length).find(|index| matches(*index))
    };
    match (index, mode % 2) {
        (Some(index), 0) => unsafe { array.add(8 + index * 8).cast::<f64>().read() },
        (Some(index), 1) => index as f64,
        (None, 0) => {
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        (None, 1) => -1.0,
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_finders {
    ($find:ident, $find_index:ident, $find_last:ident, $find_last_index:ident, $predicate:expr) => {
        extern "C" fn $find(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 0, $predicate)
        }
        extern "C" fn $find_index(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 1, $predicate)
        }
        extern "C" fn $find_last(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 2, $predicate)
        }
        extern "C" fn $find_last_index(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 3, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_lt,
    number_array_find_index_lt,
    number_array_find_last_lt,
    number_array_find_last_index_lt,
    |left, right| left < right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_lte,
    number_array_find_index_lte,
    number_array_find_last_lte,
    number_array_find_last_index_lte,
    |left, right| left <= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_gt,
    number_array_find_index_gt,
    number_array_find_last_gt,
    number_array_find_last_index_gt,
    |left, right| left > right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_gte,
    number_array_find_index_gte,
    number_array_find_last_gte,
    number_array_find_last_index_gte,
    |left, right| left >= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_eq,
    number_array_find_index_eq,
    number_array_find_last_eq,
    number_array_find_last_index_eq,
    |left, right| left == right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_ne,
    number_array_find_index_ne,
    number_array_find_last_ne,
    number_array_find_last_index_ne,
    |left, right| left != right
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_filter(value: f64, operand: f64, predicate: impl Fn(f64, f64) -> bool) -> f64 {
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    let mut selected = 0;
    for index in 0..length {
        let value = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        if predicate(value, operand) {
            unsafe { output.add(8 + selected * 8).cast::<f64>().write(value) };
            selected += 1;
        }
    }
    unsafe { output.cast::<u64>().write(selected as u64) };
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_filters {
    ($name:ident, $predicate:expr) => {
        extern "C" fn $name(value: f64, operand: f64) -> f64 {
            number_array_filter(value, operand, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_lt, |left, right| left < right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_lte, |left, right| left <= right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_gt, |left, right| left > right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_gte, |left, right| left >= right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_eq, |left, right| left == right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_ne, |left, right| left != right);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_scan(
    value: f64,
    kind: u8,
    mode: u8,
    matches: impl Fn(*const u8, usize) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    if mode < 2 {
        return f64::from(if mode == 1 {
            (0..length).all(|index| matches(array, index))
        } else {
            (0..length).any(|index| matches(array, index))
        });
    }
    if mode < 6 {
        let index = if mode >= 4 {
            (0..length).rev().find(|index| matches(array, *index))
        } else {
            (0..length).find(|index| matches(array, *index))
        };
        return match (index, mode % 2) {
            (Some(index), 0) => unsafe { array_element(array, index, kind) },
            (Some(index), 1) => index as f64,
            (None, 0) => {
                CALL_PRESENT.with(|present| present.set(false));
                0.0
            }
            (None, 1) => -1.0,
            _ => unreachable!(),
        };
    }
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    let mut selected = 0;
    for index in 0..length {
        if matches(array, index) {
            unsafe {
                output
                    .add(8 + selected * 8)
                    .cast::<u64>()
                    .write_unaligned(array.add(8 + index * 8).cast::<u64>().read_unaligned())
            };
            selected += 1;
        }
    }
    unsafe { output.cast::<u64>().write(selected as u64) };
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_truthy(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    primitive_array_scan(value, kind, mode, |array, index| unsafe {
        let slot = array.add(8 + index * 8);
        match kind {
            0 => {
                let value = slot.cast::<f64>().read_unaligned();
                value != 0.0 && !value.is_nan()
            }
            1 => slot.read() != 0,
            2 => {
                let value = slot.cast::<*const c_char>().read_unaligned();
                !value.is_null() && !CStr::from_ptr(value).to_bytes().is_empty()
            }
            _ => false,
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_truthy(value: f64, mode: f64) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let kind = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let result = primitive_array_truthy(f64::from_bits(array.payload), f64::from(kind * 8 + mode));
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_scan_result(array_tag: u64, mode: u8, result: f64) -> f64 {
    match mode {
        2 | 4 if CALL_PRESENT.with(Cell::get) => dynamic_from_parts(
            match array_tag {
                DYNAMIC_NUMBER_ARRAY_TAG => DYNAMIC_NUMBER_TAG,
                DYNAMIC_BOOLEAN_ARRAY_TAG => DYNAMIC_BOOLEAN_TAG,
                DYNAMIC_STRING_ARRAY_TAG => DYNAMIC_STRING_TAG,
                _ => unreachable!(),
            } as f64,
            result,
        ),
        6 => dynamic_array_result(array_tag, result),
        _ => result,
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_compare(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 64;
    let mode = encoded / 8 % 8;
    let operation = encoded % 8;
    if !(1..=2).contains(&kind) || operation > 5 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |array, index| unsafe {
        let element = array_element(array, index, kind);
        let ordering = if kind == 1 {
            element.total_cmp(&operand)
        } else {
            string_compare(element, operand).total_cmp(&0.0)
        };
        match operation {
            0 => ordering.is_lt(),
            1 => !ordering.is_gt(),
            2 => ordering.is_gt(),
            3 => !ordering.is_lt(),
            4 => ordering.is_eq(),
            5 => !ordering.is_eq(),
            _ => false,
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_compare(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let mode = encoded / 8;
    let operation = encoded % 8;
    let Some((array, operand)) =
        dynamic_primitive(value, None).zip(dynamic_primitive(operand, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if operand.tag > DYNAMIC_BOOLEAN_TAG || mode > 6 || operation > 7 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = primitive_array_scan(f64::from_bits(array.payload), kind, mode, |data, index| {
        let element = unsafe { array_element(data, index, kind) };
        let same = || match element_tag {
            DYNAMIC_NUMBER_TAG => element == f64::from_bits(operand.payload),
            DYNAMIC_BOOLEAN_TAG => (element != 0.0) == (operand.payload != 0),
            DYNAMIC_STRING_TAG => {
                string_same_value(element, f64::from_bits(operand.payload)) != 0.0
            }
            _ => unreachable!(),
        };
        let number = |tag, payload| match tag {
            DYNAMIC_NUMBER_TAG => f64::from_bits(payload),
            DYNAMIC_BOOLEAN_TAG => f64::from(payload != 0),
            DYNAMIC_STRING_TAG => string_to_number(f64::from_bits(payload)),
            _ => f64::NAN,
        };
        let element_payload = if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(element != 0.0)
        } else {
            element.to_bits()
        };
        match operation {
            0..=3 => {
                if element_tag == DYNAMIC_STRING_TAG && operand.tag == DYNAMIC_STRING_TAG {
                    let ordering = string_compare(element, f64::from_bits(operand.payload));
                    match operation {
                        0 => ordering < 0.0,
                        1 => ordering <= 0.0,
                        2 => ordering > 0.0,
                        _ => ordering >= 0.0,
                    }
                } else {
                    let left = number(element_tag, element_payload);
                    let right = number(operand.tag, operand.payload);
                    match operation {
                        0 => left < right,
                        1 => left <= right,
                        2 => left > right,
                        _ => left >= right,
                    }
                }
            }
            4 => {
                element_tag == operand.tag && same()
                    || element_tag != operand.tag
                        && number(element_tag, element_payload)
                            == number(operand.tag, operand.payload)
            }
            5 => {
                !(element_tag == operand.tag && same()
                    || element_tag != operand.tag
                        && number(element_tag, element_payload)
                            == number(operand.tag, operand.payload))
            }
            6 => element_tag == operand.tag && same(),
            7 => element_tag != operand.tag || !same(),
            _ => false,
        }
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_map(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let operation = encoded % 8;
    if !(1..=2).contains(&kind) || !matches!((kind, operation), (1, 0 | 1) | (2, 0 | 2..=7)) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<u64>().read_unaligned() };
        let mapped = match operation {
            0 => slot,
            1 => u64::from(slot == 0),
            2 => string_to_lower_case(f64::from_bits(slot)).to_bits(),
            3 => string_to_upper_case(f64::from_bits(slot)).to_bits(),
            4 => string_trim(f64::from_bits(slot)).to_bits(),
            5 => string_trim_start(f64::from_bits(slot)).to_bits(),
            6 => string_trim_end(f64::from_bits(slot)).to_bits(),
            7 => string_length(f64::from_bits(slot)).to_bits(),
            _ => unreachable!(),
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_convert(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<u64>().read_unaligned() };
        let value = match source {
            0 | 2 => f64::from_bits(slot),
            1 => f64::from(u8::from(slot != 0)),
            _ => unreachable!(),
        };
        let mapped = match (source, target) {
            (_, 0) if source != 2 => value,
            (2, 0) => string_to_number(value),
            (0, 1) => f64::from(u8::from(value != 0.0 && !value.is_nan())),
            (1, 1) => value,
            (2, 1) => string_truthy(value),
            (0, 2) => number_to_string(value),
            (1, 2) => boolean_to_string(value),
            (2, 2) => value,
            _ => unreachable!(),
        };
        let mapped = if target == 1 {
            u64::from(mapped != 0.0)
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_convert(value: f64, target: f64) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let source = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    primitive_array_convert(
        f64::from_bits(array.payload),
        f64::from(source * 4 + target as u8),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_map_identity(value: f64) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let source = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let result = primitive_array_convert(
        f64::from_bits(array.payload),
        f64::from(source * 4 + source),
    );
    dynamic_array_result(array.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_map(value: f64, operation: impl Fn(f64, f64) -> f64) -> f64 {
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        let mapped = operation(element, index as f64);
        unsafe { output.add(8 + index * 8).cast::<f64>().write(mapped) };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
type JitCallback = extern "C" fn(*const f64) -> f64;

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile_jit_callback(callback: f64) -> Option<(JitCallback, usize)> {
    let Some(callback) = (unsafe { string_argument(callback) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let symbol = format!("expr:{callback}:array-callback");
    let Some(program) = NumericProgram::parse(&symbol) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let required_args = program.required_args();
    if required_args > 16 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    let code = match compile(&symbol, &program) {
        Ok((code, _)) => code,
        Err(error) => {
            CALL_ERROR.with(|slot| slot.set(error));
            return None;
        }
    };
    Some((
        unsafe { std::mem::transmute::<*mut libc::c_void, JitCallback>(code) },
        required_args,
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn capture_arguments(value: f64, builtins: usize, required: usize) -> Option<Vec<f64>> {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    if builtins + length > 16 || required > builtins + length {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    Some(
        (0..length)
            .map(|index| unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() })
            .collect(),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn call_jit_callback(callback: JitCallback, builtins: &[f64], captures: &[f64]) -> f64 {
    let mut arguments = [0.0; 16];
    arguments[..builtins.len()].copy_from_slice(builtins);
    arguments[builtins.len()..builtins.len() + captures.len()].copy_from_slice(captures);
    callback(arguments.as_ptr())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_jit_map_impl(
    value: f64,
    callback: f64,
    encoded: f64,
    captures: Option<f64>,
) -> f64 {
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 3, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 3 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(array, index, source) };
        let mapped = call_jit_callback(callback, &[element, index as f64, value], &captures);
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map(value: f64, callback: f64, encoded: f64) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_map_impl(
    value: f64,
    callback: f64,
    target: f64,
    captures: Option<f64>,
) -> f64 {
    let target = target as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (source, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((data, length))) = (ARENA_ALLOC.with(Cell::get), unsafe {
        array_data(f64::from_bits(array.payload))
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(data, index, source) };
        let mapped = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map(value: f64, callback: f64, target: f64) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    target: f64,
) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_scan_impl(value: f64, callback: f64, mode: f64, captures: Option<f64>) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let result = primitive_array_scan(f64::from_bits(array.payload), kind, mode, |data, index| {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        result != 0.0 && !result.is_nan()
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan(value: f64, callback: f64, mode: f64) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    mode: f64,
) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan(value: f64, callback: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if required > 3 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = callback([element, index as f64, value].as_ptr());
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let Some(captures) = capture_arguments(captures, 3, required) else {
        return 0.0;
    };
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = call_jit_callback(callback, &[element, index as f64, value], &captures);
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    has_initial: bool,
    captures: Option<f64>,
) -> f64 {
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 4, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 4 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if !has_initial && length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let mut accumulator = if has_initial {
        initial
    } else {
        let index = if from_right { length - 1 } else { 0 };
        unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() }
    };
    let mut apply = |index| {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        accumulator = call_jit_callback(
            callback,
            &[accumulator, element, index as f64, value],
            &captures,
        );
    };
    if from_right {
        for index in (0..if has_initial { length } else { length - 1 }).rev() {
            apply(index);
        }
    } else {
        for index in if has_initial { 0 } else { 1 }..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64) -> f64 {
            number_array_jit_reduce(value, initial, callback, $from_right, $has_initial, None)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_initial, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_first, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_initial, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_last, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_captured_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64, captures: f64) -> f64 {
            number_array_jit_reduce(
                value,
                initial,
                callback,
                $from_right,
                $has_initial,
                Some(captures),
            )
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_initial_captured, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_first_captured, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_initial_captured, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_last_captured, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 6, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 6 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut accumulator = initial;
    let mut apply = |index| {
        let element = unsafe { array_element(data, index, kind) };
        accumulator = call_jit_callback(
            callback,
            &[
                accumulator,
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
    };
    if from_right {
        for index in (0..length).rev() {
            apply(index);
        }
    } else {
        for index in 0..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce_unseeded(
    value: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 7, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 7 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let first = if from_right { length - 1 } else { 0 };
    let first_payload = unsafe { array_element(data, first, kind) };
    let mut accumulator = DynamicPrimitive {
        tag: element_tag,
        payload: if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(first_payload != 0.0)
        } else {
            first_payload.to_bits()
        },
    };
    let mut apply = |index| -> Option<()> {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                accumulator.tag as f64,
                f64::from_bits(accumulator.payload),
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let result = dynamic_primitive(result, None)?;
        accumulator = DynamicPrimitive {
            tag: result.tag,
            payload: result.payload,
        };
        Some(())
    };
    if from_right {
        for index in (0..length - 1).rev() {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    } else {
        for index in 1..length {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    }
    arena_dynamic(accumulator.tag, accumulator.payload)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_index_map(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let reverse = encoded >= 8;
    let operation = encoded % 8;
    number_array_map(value, |element, index| {
        let (left, right) = if reverse {
            (index, element)
        } else {
            (element, index)
        };
        match operation {
            0 => left + right,
            1 => left - right,
            2 => left * right,
            3 => left / right,
            4 => left % right,
            5 => power(left, right),
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                0.0
            }
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_select_map(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let operation = encoded % 8;
    let mode = encoded / 8;
    if operation > 5 || mode > 3 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    number_array_map(value, |element, _| {
        let (left, right) = if mode & 1 == 0 {
            (element, operand)
        } else {
            (operand, element)
        };
        let condition = match operation {
            0 => left < right,
            1 => left <= right,
            2 => left > right,
            3 => left >= right,
            4 => left == right,
            5 => left != right,
            _ => unreachable!(),
        };
        if condition == (mode & 2 == 0) {
            element
        } else {
            operand
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_branch_map(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u16;
    let operation = (encoded & 7) as u8;
    let reverse = encoded & 8 != 0;
    let true_branch = ((encoded >> 4) & 15) as u8;
    let false_branch = ((encoded >> 8) & 15) as u8;
    if operation > 5 || true_branch > 13 || false_branch > 13 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let branch = |element: f64, mode: u8| {
        let (operation, left, right) = if mode < 8 {
            (mode.saturating_sub(2), element, operand)
        } else {
            (mode - 8, operand, element)
        };
        match mode {
            0 => element,
            1 => operand,
            _ => match operation {
                0 => left + right,
                1 => left - right,
                2 => left * right,
                3 => left / right,
                4 => left % right,
                5 => power(left, right),
                _ => unreachable!(),
            },
        }
    };
    number_array_map(value, |element, _| {
        let (left, right) = if reverse {
            (operand, element)
        } else {
            (element, operand)
        };
        let condition = match operation {
            0 => left < right,
            1 => left <= right,
            2 => left > right,
            3 => left >= right,
            4 => left == right,
            5 => left != right,
            _ => unreachable!(),
        };
        branch(element, if condition { true_branch } else { false_branch })
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_maps {
    ($forward:ident, $reverse:ident, $operation:expr) => {
        extern "C" fn $forward(value: f64, operand: f64) -> f64 {
            number_array_map(value, |element, _| $operation(element, operand))
        }
        extern "C" fn $reverse(value: f64, operand: f64) -> f64 {
            number_array_map(value, |element, _| $operation(operand, element))
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_add,
    number_array_map_add_reverse,
    |left, right| left + right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_subtract,
    number_array_map_subtract_reverse,
    |left, right| left - right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_multiply,
    number_array_map_multiply_reverse,
    |left, right| left * right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_divide,
    number_array_map_divide_reverse,
    |left, right| left / right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_remainder,
    number_array_map_remainder_reverse,
    |left, right| left % right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_power,
    number_array_map_power_reverse,
    |left, right| power(left, right)
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_negate(value: f64) -> f64 {
    number_array_map(value, |element, _| -element)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_absolute(value: f64) -> f64 {
    number_array_map(value, |element, _| element.abs())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn square_root_number(value: f64) -> f64 {
    value.sqrt()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_math(value: f64, operation: f64) -> f64 {
    let function = match operation as u8 {
        0 => acos_number,
        1 => acosh_number,
        2 => asin_number,
        3 => asinh_number,
        4 => atan_number,
        5 => atanh_number,
        6 => cbrt_number,
        7 => ceil_number,
        8 => clz32_number,
        9 => cos_number,
        10 => cosh_number,
        11 => exp_number,
        12 => expm1_number,
        13 => floor_number,
        14 => fround_number,
        15 => log_number,
        16 => log1p_number,
        17 => log2_number,
        18 => log10_number,
        19 => round_number,
        20 => sign_number,
        21 => sin_number,
        22 => sinh_number,
        23 => square_root_number,
        24 => tan_number,
        25 => tanh_number,
        26 => truncate_number,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
    };
    number_array_map(value, |element, _| function(element))
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
macro_rules! unary_math_helpers {
    ($($function:ident => $method:ident),+ $(,)?) => {
        $(extern "C" fn $function(value: f64) -> f64 { value.$method() })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unary_math_helpers! {
    acos_number => acos,
    acosh_number => acosh,
    asin_number => asin,
    asinh_number => asinh,
    atan_number => atan,
    atanh_number => atanh,
    cbrt_number => cbrt,
    cos_number => cos,
    cosh_number => cosh,
    exp_number => exp,
    expm1_number => exp_m1,
    log_number => ln,
    log1p_number => ln_1p,
    log2_number => log2,
    log10_number => log10,
    sin_number => sin,
    sinh_number => sinh,
    tan_number => tan,
    tanh_number => tanh,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn sign_number(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        value
    } else if value.is_sign_positive() {
        1.0
    } else {
        -1.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn atan2_number(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn hypot_number(left: f64, right: f64) -> f64 {
    left.hypot(right)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fround_number(value: f64) -> f64 {
    value as f32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_nan(value: f64) -> f64 {
    f64::from(u8::from(value.is_nan()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_finite(value: f64) -> f64 {
    f64::from(u8::from(value.is_finite()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_integer(value: f64) -> f64 {
    f64::from(u8::from(value.is_finite() && value.fract() == 0.0))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_safe_integer(value: f64) -> f64 {
    f64::from(u8::from(
        value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0,
    ))
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
extern "C" fn clz32_number(value: f64) -> f64 {
    to_uint32(value).leading_zeros() as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn imul_number(left: f64, right: f64) -> f64 {
    to_uint32(left).wrapping_mul(to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_argument(value: f64) -> Option<String> {
    let pointer = value.to_bits() as usize as *const c_char;
    (!pointer.is_null()).then(|| CStr::from_ptr(pointer).to_string_lossy().into_owned())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_length(value: f64) -> f64 {
    unsafe {
        string_argument(value)
            .map(|value| value.encode_utf16().count() as f64)
            .unwrap_or(0.0)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_data(value: f64) -> Option<(*const u8, usize)> {
    let bits = value.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        let data = (bits & !ARRAY_RESULT_TAG) as usize as *const u8;
        return (!data.is_null())
            .then(|| (data, unsafe { data.cast::<i64>().read() }.max(0) as usize));
    }
    let handle = bits as usize as *const *const u8;
    if handle.is_null() {
        return None;
    }
    let data = unsafe { handle.read() };
    if data.is_null() {
        return None;
    }
    Some((data, unsafe { data.cast::<i64>().read() }.max(0) as usize))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_result(pointer: *mut u8) -> f64 {
    f64::from_bits(pointer as usize as u64 | ARRAY_RESULT_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn empty_array() -> f64 {
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let array = unsafe { allocate(8, 8) };
    if array.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { array.cast::<u64>().write(0) };
    array_result(array)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_length(value: f64) -> f64 {
    unsafe { array_data(value) }.map_or(0.0, |(_, length)| length as f64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_value(value: f64) -> f64 {
    match unsafe { array_data(value) } {
        Some((data, _)) => f64::from_bits(data as usize as u64),
        None => {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            0.0
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn mutable_array_handle(value: f64) -> f64 {
    if value.to_bits() & ARRAY_RESULT_TAG == 0 {
        return value;
    }
    let (Some(allocate), Some((data, _))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let handle = unsafe {
        allocate(
            std::mem::size_of::<*mut u8>(),
            std::mem::align_of::<*mut u8>(),
        )
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { handle.cast::<*const u8>().write(data) };
    f64::from_bits(handle as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_array(value: f64) -> f64 {
    let Some(convert) = STRING_TO_ARRAY.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value.to_bits() as usize as *const c_char) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_from_char_code(value: f64) -> f64 {
    let Some(convert) = STRING_FROM_CHAR_CODE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_from_code_point(value: f64) -> f64 {
    let Some(convert) = STRING_FROM_CODE_POINT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_CODE_POINT.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn is_array(_: f64) -> f64 {
    1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn is_not_array(_: f64) -> f64 {
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn absent_value() -> f64 {
    CALL_ABSENCE.with(|absence| absence.set(1));
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn null_value() -> f64 {
    CALL_ABSENCE.with(|absence| absence.set(2));
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn preserve_absent_value() -> f64 {
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn missing_callable() -> f64 {
    CALL_ERROR.with(|error| error.set(VALUE_NOT_CALLABLE.as_ptr().cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn take_present() -> f64 {
    f64::from(CALL_PRESENT.with(|present| present.replace(true)))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_at(value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let index = if index < 0.0 {
        length as f64 + index
    } else {
        index
    };
    if !index.is_finite() || index < 0.0 || index >= length as f64 {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe { array_element(data, index as usize, kind) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_element(data: *const u8, index: usize, kind: u8) -> f64 {
    let slot = unsafe { data.add(8 + index * 8) };
    match kind {
        0 => unsafe { slot.cast::<f64>().read_unaligned() },
        1 => f64::from(unsafe { slot.read() } != 0),
        2 => f64::from_bits(unsafe { slot.cast::<usize>().read_unaligned() } as u64),
        _ => 0.0,
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_get(value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index >= length as f64 {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe { array_element(data, index as usize, kind) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 0) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 1) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 2) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_at(value: f64, index: f64) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, tag) = match dynamic.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = unsafe { array_at(f64::from_bits(dynamic.payload), index, kind) };
    if !CALL_PRESENT.with(Cell::get) {
        0.0
    } else {
        dynamic_from_parts(tag as f64, value)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 0) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 1) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 2) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(search), Some((data, _))) =
        (ARRAY_SEARCH.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    unsafe { search(operation, data, needle, from_index) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_format(operation: u8, value: f64, separator: f64) -> f64 {
    let (Some(format), Some((data, _))) =
        (ARRAY_FORMAT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe {
        format(
            operation,
            data,
            separator.to_bits() as usize as *const c_char,
        )
    };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_join(value: f64, separator: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            let operation = match dynamic.tag {
                DYNAMIC_NUMBER_ARRAY_TAG => 0,
                DYNAMIC_BOOLEAN_ARRAY_TAG => 2,
                DYNAMIC_STRING_ARRAY_TAG => 1,
                _ => {
                    CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                    return 0.0;
                }
            };
            array_format(operation, f64::from_bits(dynamic.payload), separator)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_result(tag: u64, result: f64) -> f64 {
    let bits = result.to_bits();
    if bits & ARRAY_RESULT_TAG == 0 {
        return 0.0;
    }
    let Some(handle) = ARENA_ALLOC.with(|allocator| {
        allocator.get().map(|alloc| unsafe {
            alloc(std::mem::size_of::<usize>(), std::mem::align_of::<usize>())
        })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        handle
            .cast::<usize>()
            .write(bits as usize & !ARRAY_RESULT_TAG as usize)
    };
    dynamic_from_parts(tag as f64, f64::from_bits(handle as usize as u64))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_slice(value: f64, start: f64, end: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if !matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
            ) {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
            let result = array_slice(f64::from_bits(dynamic.payload), start, end);
            dynamic_array_result(dynamic.tag, result)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_slice(value: f64, start: f64, end: f64) -> f64 {
    let (Some(slice), Some((data, _))) =
        (ARRAY_SLICE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { slice(data, 8, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_concat(left: f64, right: f64) -> f64 {
    let (Some(concat), Some((left, _)), Some((right, _))) = (
        ARRAY_CONCAT.with(Cell::get),
        unsafe { array_data(left) },
        unsafe { array_data(right) },
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { concat(left, right, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_concat(left: f64, right: f64) -> f64 {
    let (Some(left), Some(right)) = (
        dynamic_primitive(left, None),
        dynamic_primitive(right, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if left.tag != right.tag
        || !matches!(
            left.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = array_concat(f64::from_bits(left.payload), f64::from_bits(right.payload));
    dynamic_array_result(left.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_append(operation: u8, array: f64, value: f64) -> f64 {
    let (Some(append), Some((array, _))) =
        (ARRAY_APPEND.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { append(operation, array, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_append_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_append($operation, array, value)
        }
    };
}

array_append_fn!(number_array_append, 0);
array_append_fn!(string_array_append, 1);
array_append_fn!(bool_array_append, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_append(array: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_append(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_reversed(value: f64) -> f64 {
    let (Some(reverse), Some((data, _))) = (ARRAY_TO_REVERSED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_reverse(value: f64) -> f64 {
    let (Some(reverse), Some((data, _))) =
        (ARRAY_REVERSE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data.cast_mut(), 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_reverse(value: f64, reverse: extern "C" fn(f64) -> f64) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(
        dynamic.tag,
        DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
    ) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = reverse(f64::from_bits(dynamic.payload));
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_reversed(value: f64) -> f64 {
    dynamic_array_reverse(value, array_to_reversed)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_reverse_in_place(value: f64) -> f64 {
    dynamic_array_reverse(value, array_reverse)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_to_sorted(operation: u8, value: f64) -> f64 {
    let (Some(sort), Some((data, _))) = (ARRAY_TO_SORTED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(0, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(1, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(2, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_ascending(value: f64) -> f64 {
    array_to_sorted(3, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(4, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(5, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_sort(operation: u8, value: f64) -> f64 {
    let (Some(sort), Some((data, _))) = (ARRAY_SORT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data.cast_mut()) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_sort_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64) -> f64 {
            array_sort($operation, value)
        }
    };
}

array_sort_fn!(number_array_sort, 0);
array_sort_fn!(string_array_sort, 1);
array_sort_fn!(bool_array_sort, 2);
array_sort_fn!(number_array_sort_ascending, 3);
array_sort_fn!(number_array_sort_descending, 4);
array_sort_fn!(string_array_sort_descending, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_sort(value: f64, in_place: bool) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match dynamic.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_STRING_ARRAY_TAG => 1,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = f64::from_bits(dynamic.payload);
    let result = if in_place {
        array_sort(operation, value)
    } else {
        array_to_sorted(operation, value)
    };
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_sorted(value: f64) -> f64 {
    dynamic_array_sort(value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_sort_in_place(value: f64) -> f64 {
    dynamic_array_sort(value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_fill(operation: u8, array: f64, value: f64, start: f64, end: f64) -> f64 {
    let (Some(fill), Some((data, _))) = (ARRAY_FILL.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { fill(operation, data.cast_mut(), value, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_fill_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64, start: f64, end: f64) -> f64 {
            array_fill($operation, array, value, start, end)
        }
    };
}

array_fill_fn!(number_array_fill, 0);
array_fill_fn!(string_array_fill, 1);
array_fill_fn!(bool_array_fill, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_fill(array: f64, value: f64, start: f64, end: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_fill(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
            start,
            end,
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let (Some(copy), Some((data, _))) = (ARRAY_COPY_WITHIN.with(Cell::get), unsafe {
        array_data(array)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { copy(data.cast_mut(), 8, target, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(
        array.tag,
        DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
    ) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    dynamic_array_result(
        array.tag,
        array_copy_within(f64::from_bits(array.payload), target, start, end),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_with(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_with(
            operation,
            f64::from_bits(array.payload),
            index,
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_push(operation: u8, array: f64, value: f64) -> f64 {
    let Some(push) = ARRAY_PUSH.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { push(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
}

macro_rules! array_push_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_push($operation, array, value)
        }
    };
}

array_push_fn!(number_array_push, 0);
array_push_fn!(string_array_push, 1);
array_push_fn!(bool_array_push, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_insert(array: f64, value: f64, unshift: bool) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let array = f64::from_bits(array.payload);
    let value = f64::from_bits(value.payload);
    if unshift {
        array_unshift(operation, array, value)
    } else {
        array_push(operation, array, value)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_push(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_unshift(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_unshift(operation: u8, array: f64, value: f64) -> f64 {
    let Some(unshift) = ARRAY_UNSHIFT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { unshift(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_set(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let Some(set) = ARRAY_SET.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0
        || unsafe { set(operation, bits as usize as *mut *mut u8, index, value) } != 1
    {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        value
    }
}

macro_rules! array_set_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_set($operation, array, index, value)
        }
    };
}

array_set_fn!(number_array_set, 0);
array_set_fn!(string_array_set, 1);
array_set_fn!(bool_array_set, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_set(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(dynamic_value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, dynamic_value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    array_set(
        operation,
        f64::from_bits(array.payload),
        index,
        f64::from_bits(dynamic_value.payload),
    );
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_source(source: &'static std::thread::LocalKey<Cell<Option<NumberSource>>>) -> f64 {
    let Some(source) = source.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    unsafe { source() }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn math_random() -> f64 {
    number_source(&MATH_RANDOM)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn date_now() -> f64 {
    number_source(&DATE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn performance_now() -> f64 {
    number_source(&PERFORMANCE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_pid() -> f64 {
    number_source(&PROCESS_PID)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_ppid() -> f64 {
    number_source(&PROCESS_PPID)
}

macro_rules! array_unshift_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_unshift($operation, array, value)
        }
    };
}

array_unshift_fn!(number_array_unshift, 0);
array_unshift_fn!(string_array_unshift, 1);
array_unshift_fn!(bool_array_unshift, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_remove(operation: u8, array: f64) -> f64 {
    let Some(remove) = ARRAY_REMOVE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let mut value = 0.0;
    match unsafe { remove(operation, bits as usize as *mut *mut u8, &mut value) } {
        1 => value,
        0 => {
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        _ => {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            0.0
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_splice(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    let Some((inserts, _)) = (unsafe { array_data(inserts) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let removed = unsafe { splice(bits as usize as *mut *mut u8, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(removed)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_spliced(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let (Some((array, _)), Some((inserts, _))) =
        (unsafe { array_data(array) }, unsafe { array_data(inserts) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut output = array.cast_mut();
    let removed = unsafe { splice(&mut output, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(output)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_splice(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
    copy: bool,
) -> f64 {
    let (Some(array), Some(inserts)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(inserts, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if array.tag != inserts.tag
        || !matches!(
            array.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let splice = if copy { array_to_spliced } else { array_splice };
    dynamic_array_result(
        array.tag,
        splice(
            f64::from_bits(array.payload),
            start,
            delete_count,
            f64::from_bits(inserts.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_splice_in_place(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_spliced(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, true)
}

macro_rules! array_remove_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64) -> f64 {
            array_remove($operation, array)
        }
    };
}

array_remove_fn!(number_array_pop, 0);
array_remove_fn!(string_array_pop, 1);
array_remove_fn!(bool_array_pop, 2);
array_remove_fn!(number_array_shift, 3);
array_remove_fn!(string_array_shift, 4);
array_remove_fn!(bool_array_shift, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_remove(array: f64, shift: bool) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (operation, tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (1, DYNAMIC_STRING_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (2, DYNAMIC_BOOLEAN_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = array_remove(
        operation + u8::from(shift) * 3,
        f64::from_bits(array.payload),
    );
    if CALL_PRESENT.with(Cell::get) {
        dynamic_from_parts(tag as f64, value)
    } else {
        0.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_pop(array: f64) -> f64 {
    dynamic_array_remove(array, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_shift(array: f64) -> f64 {
    dynamic_array_remove(array, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_with(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let (Some(replace), Some((data, _))) =
        (ARRAY_WITH.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { replace(operation, data, index, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_ARRAY_WITH_INDEX.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_with_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_with($operation, array, index, value)
        }
    };
}

array_with_fn!(number_array_with, 0);
array_with_fn!(string_array_with, 1);
array_with_fn!(bool_array_with, 2);

macro_rules! array_format_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, separator: f64) -> f64 {
            array_format($operation, value, separator)
        }
    };
}

array_format_fn!(number_array_join, 0);
array_format_fn!(string_array_join, 1);
array_format_fn!(bool_array_join, 2);

macro_rules! array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            array_search($operation, value, needle, from_index)
        }
    };
}

array_search_fn!(number_array_index_of, 0);
array_search_fn!(number_array_includes, 1);
array_search_fn!(string_array_index_of, 2);
array_search_fn!(string_array_includes, 3);
array_search_fn!(bool_array_index_of, 4);
array_search_fn!(bool_array_includes, 5);
array_search_fn!(number_array_last_index_of, 6);
array_search_fn!(string_array_last_index_of, 7);
array_search_fn!(bool_array_last_index_of, 8);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(value), Some(needle)) = (
        dynamic_primitive(value, None),
        dynamic_primitive(needle, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let search = match (value.tag, needle.tag, operation) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 0..=1) => operation,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 0..=1) => operation + 2,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 0..=1) => operation + 4,
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 2) => 6,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 2) => 7,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 2) => 8,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    array_search(
        search,
        f64::from_bits(value.payload),
        f64::from_bits(needle.payload),
        from_index,
    )
}

macro_rules! dynamic_array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            dynamic_array_search($operation, value, needle, from_index)
        }
    };
}

dynamic_array_search_fn!(dynamic_array_index_of, 0);
dynamic_array_search_fn!(dynamic_array_includes, 1);
dynamic_array_search_fn!(dynamic_array_last_index_of, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_truthy(value: f64) -> f64 {
    let value = value.to_bits() as usize as *const c_char;
    unsafe { (!value.is_null() && !CStr::from_ptr(value).to_bytes().is_empty()) as u8 as f64 }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_char_code_at(value: f64, index: f64) -> f64 {
    if index.is_infinite() {
        return f64::NAN;
    }
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    if index < 0.0 || index > usize::MAX as f64 {
        return f64::NAN;
    }
    unsafe {
        string_argument(value)
            .and_then(|value| value.encode_utf16().nth(index as usize))
            .map_or(f64::NAN, f64::from)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_char_at(value: f64, index: f64) -> f64 {
    let code = string_char_code_at(value, index);
    if code.is_nan() {
        arena_string(String::new())
    } else {
        arena_string(String::from_utf16_lossy(&[code as u16]))
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return f64::from_bits(0);
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let index = if index < 0.0 {
        units.len() as f64 + index
    } else {
        index
    };
    let Some(unit) = (index.is_finite() && index >= 0.0)
        .then(|| units.get(index as usize))
        .flatten()
    else {
        CALL_PRESENT.with(|present| present.set(false));
        return f64::from_bits(0);
    };
    arena_string(String::from_utf16_lossy(&[*unit]))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_code_point_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let Some(&first) = (index.is_finite() && index >= 0.0)
        .then(|| units.get(index as usize))
        .flatten()
    else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let Some(&second) = units.get(index as usize + 1) else {
        return f64::from(first);
    };
    if (0xd800..=0xdbff).contains(&first) && (0xdc00..=0xdfff).contains(&second) {
        f64::from(0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00)
    } else {
        f64::from(first)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_compare(left: f64, right: f64) -> f64 {
    unsafe {
        let Some(left) = string_argument(left) else {
            return 0.0;
        };
        let Some(right) = string_argument(right) else {
            return 0.0;
        };
        match left.encode_utf16().cmp(right.encode_utf16()) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(
        (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits(),
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(unsafe {
        string_argument(left) == string_argument(right)
    }))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn reference_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(left.to_bits() == right.to_bits()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_number(_: f64) -> f64 {
    f64::from_bits(c"number".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_boolean(_: f64) -> f64 {
    f64::from_bits(c"boolean".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_string(_: f64) -> f64 {
    f64::from_bits(c"string".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_object(_: f64) -> f64 {
    f64::from_bits(c"object".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_is_well_formed(_: f64) -> f64 {
    1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn clamped_string_position(position: f64, length: usize) -> usize {
    if position.is_nan() || position == f64::NEG_INFINITY {
        0
    } else if position == f64::INFINITY {
        length
    } else {
        position.trunc().clamp(0.0, length as f64) as usize
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn positioned_string_search(value: f64, search: f64, position: f64, kind: u8) -> f64 {
    let Some(value) = string_argument(value) else {
        return -1.0;
    };
    let Some(search) = string_argument(search) else {
        return -1.0;
    };
    let value = value.encode_utf16().collect::<Vec<_>>();
    let search = search.encode_utf16().collect::<Vec<_>>();
    let position = clamped_string_position(position, value.len());
    match kind {
        0 => f64::from(value.get(position..position.saturating_add(search.len())) == Some(&search)),
        1 => f64::from(
            search.len() <= position
                && value.get(position - search.len()..position) == Some(&search),
        ),
        2 | 3 => {
            if search.is_empty() {
                return if kind == 2 { 1.0 } else { position as f64 };
            }
            let found = value[position..]
                .windows(search.len())
                .position(|window| window == search)
                .map(|index| position + index);
            if kind == 2 {
                f64::from(found.is_some())
            } else {
                found.map_or(-1.0, |index| index as f64)
            }
        }
        4 => {
            if search.is_empty() {
                return position as f64;
            }
            if search.len() > value.len() {
                return -1.0;
            }
            let start = position.min(value.len() - search.len());
            (0..=start)
                .rev()
                .find(|index| value.get(*index..*index + search.len()) == Some(&search))
                .map_or(-1.0, |index| index as f64)
        }
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_index_of(value: f64, search: f64) -> f64 {
    unsafe { positioned_string_search(value, search, 0.0, 3) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_last_index_of(value: f64, search: f64) -> f64 {
    unsafe { positioned_string_search(value, search, f64::INFINITY, 4) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! positioned_searches {
    ($($function:ident, $kind:expr);+ $(;)?) => {
        $(extern "C" fn $function(value: f64, search: f64, position: f64) -> f64 {
            unsafe { positioned_string_search(value, search, position, $kind) }
        })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
positioned_searches! {
    string_starts_with_at, 0;
    string_ends_with_at, 1;
    string_includes_at, 2;
    string_index_of_at, 3;
    string_last_index_of_at, 4;
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_starts_with(value: f64, search: f64) -> f64 {
    string_starts_with_at(value, search, 0.0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_ends_with(value: f64, search: f64) -> f64 {
    string_ends_with_at(value, search, f64::INFINITY)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_includes(value: f64, search: f64) -> f64 {
    string_includes_at(value, search, 0.0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn arena_string(value: String) -> f64 {
    let size = value.len() + 1;
    let Some(output) =
        ARENA_ALLOC.with(|allocator| allocator.get().map(|alloc| unsafe { alloc(size, 1) }))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return f64::from_bits(0);
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return f64::from_bits(0);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(value.as_ptr(), output, value.len());
        output.add(value.len()).write(0);
    }
    f64::from_bits(output as usize as u64)
}

#[repr(C)]
struct DynamicPrimitive {
    tag: u64,
    payload: u64,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn arena_dynamic(tag: u64, payload: u64) -> f64 {
    let Some(output) = ARENA_ALLOC.with(|allocator| {
        allocator.get().map(|alloc| unsafe {
            alloc(
                std::mem::size_of::<DynamicPrimitive>(),
                std::mem::align_of::<DynamicPrimitive>(),
            )
        })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let Some(output) = (!output.is_null()).then(|| output.cast::<DynamicPrimitive>()) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    unsafe { output.write(DynamicPrimitive { tag, payload }) };
    DYNAMIC_VALUES.with(|values| {
        values.borrow_mut().insert(output as usize);
    });
    f64::from_bits(output as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_number(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_NUMBER_TAG, value.to_bits())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_string(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_STRING_TAG, value.to_bits())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_boolean(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_BOOLEAN_TAG, u64::from(value != 0.0))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn tag_aggregate(value: f64, tag: u64) -> f64 {
    let value = if matches!(tag, DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG) {
        mutable_array_handle(value)
    } else {
        value
    };
    if CALL_ERROR.with(Cell::get).is_null() {
        arena_dynamic(tag, value.to_bits())
    } else {
        0.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! aggregate_tagger {
    ($name:ident, $tag:expr) => {
        extern "C" fn $name(value: f64) -> f64 {
            tag_aggregate(value, $tag)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_number_array, DYNAMIC_NUMBER_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_boolean_array, DYNAMIC_BOOLEAN_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_string_array, DYNAMIC_STRING_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_number_dictionary, DYNAMIC_NUMBER_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_boolean_dictionary, DYNAMIC_BOOLEAN_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_string_dictionary, DYNAMIC_STRING_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_object, DYNAMIC_OBJECT_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_tuple, DYNAMIC_TUPLE_TAG);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_from_parts(tag: f64, payload: f64) -> f64 {
    let tag = tag as u64;
    if !(DYNAMIC_NUMBER_TAG..=DYNAMIC_TUPLE_TAG).contains(&tag) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    arena_dynamic(
        tag,
        if tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(payload != 0.0)
        } else {
            payload.to_bits()
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_primitive(value: f64, expected: Option<u64>) -> Option<&'static DynamicPrimitive> {
    let pointer = value.to_bits() as usize as *const DynamicPrimitive;
    if !DYNAMIC_VALUES.with(|values| values.borrow().contains(&(pointer as usize))) {
        return None;
    }
    let dynamic = unsafe { pointer.as_ref() }?;
    (DYNAMIC_NUMBER_TAG..=DYNAMIC_TUPLE_TAG)
        .contains(&dynamic.tag)
        .then_some(dynamic)
        .filter(|dynamic| expected.is_none_or(|tag| dynamic.tag == tag))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_tag(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| dynamic.tag as f64,
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_dynamic(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            f64::from_bits(match dynamic.tag {
                DYNAMIC_NUMBER_TAG => c"number".as_ptr(),
                DYNAMIC_STRING_TAG => c"string".as_ptr(),
                DYNAMIC_BOOLEAN_TAG => c"boolean".as_ptr(),
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => c"object".as_ptr(),
                _ => unreachable!(),
            } as usize as u64)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_to_boolean(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => {
                let value = f64::from_bits(dynamic.payload);
                f64::from(value != 0.0 && !value.is_nan())
            }
            DYNAMIC_STRING_TAG => f64::from(unsafe {
                (dynamic.payload as usize as *const c_char)
                    .as_ref()
                    .is_some_and(|value| *value != 0)
            }),
            DYNAMIC_BOOLEAN_TAG => f64::from(dynamic.payload != 0),
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => 1.0,
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_is_array(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            f64::from(matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG | DYNAMIC_TUPLE_TAG
            ))
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean(value: f64) -> f64 {
    dynamic_primitive(value, Some(DYNAMIC_BOOLEAN_TAG)).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| dynamic.payload as f64,
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_BOOLEAN_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_array(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG | DYNAMIC_TUPLE_TAG
            ) {
                f64::from_bits(dynamic.payload)
            } else {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                0.0
            }
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_BOOLEAN_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_dictionary(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_DICTIONARY_TAG..=DYNAMIC_STRING_DICTIONARY_TAG
            ) {
                f64::from_bits(dynamic.payload)
            } else {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                0.0
            }
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_object(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_OBJECT_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_tuple(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_TUPLE_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_number_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe { pointer.add(offset as usize).cast::<f64>().read() }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_boolean_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    f64::from(unsafe { pointer.add(offset as usize).read() != 0 })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_string_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    f64::from_bits(unsafe { pointer.add(offset as usize).cast::<usize>().read() } as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn object_optional_field(object: f64, offset: f64, kind: u8) -> f64 {
    object_tagged_field(object, offset, kind, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn object_tagged_field(object: f64, offset: f64, kind: u8, present_tag: u8) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 || offset.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let tag = unsafe { pointer.add(offset as usize).read() };
    if tag != present_tag {
        if present_tag == 0 {
            CALL_ABSENCE.with(|absence| absence.set(if tag == 1 { 2 } else { 1 }));
        }
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    match kind {
        0 => object_number_field(object, offset + 8.0),
        1 => object_boolean_field(object, offset + 1.0),
        2 => object_string_field(object, offset + 8.0),
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! optional_object_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, offset: f64) -> f64 {
            object_optional_field(object, offset, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! nullish_object_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, offset: f64) -> f64 {
            object_tagged_field(object, offset, $kind, 0)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn optional_tuple_field(tuple: f64, index: f64, kind: u8) -> f64 {
    tagged_tuple_field(tuple, index, kind, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn tagged_tuple_field(tuple: f64, index: f64, kind: u8, present_tag: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let field = unsafe { data.add(8 + index as usize * 16) };
    if unsafe { field.read() } != present_tag {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe {
        match kind {
            0 => field.add(8).cast::<f64>().read(),
            1 => f64::from(field.add(1).read() != 0),
            2 => f64::from_bits(field.add(8).cast::<usize>().read() as u64),
            _ => unreachable!(),
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! optional_tuple_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, index: f64) -> f64 {
            optional_tuple_field(tuple, index, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! nullish_tuple_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, index: f64) -> f64 {
            tagged_tuple_field(tuple, index, $kind, 0)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_object_new(size: f64) -> f64 {
    if !size.is_finite() || size <= 0.0 || size.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let Some(output) = ARENA_ALLOC.with(|allocator| {
        allocator
            .get()
            .map(|allocate| unsafe { allocate(size as usize, 8) })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.write_bytes(0, size as usize) };
    f64::from_bits(output as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_object_set(object: f64, value: f64, offset: f64, kind: u8) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 || offset.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let field = pointer.add(offset as usize);
        match kind {
            0 => field.cast::<f64>().write(value),
            1 => field.write(u8::from(value != 0.0)),
            2 => field.cast::<usize>().write(value.to_bits() as usize),
            3 => field.write(value as u8),
            _ => unreachable!(),
        }
    }
    object
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tagged_object_number_update(object: f64, offset: f64, mode: f64) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null()
        || !offset.is_finite()
        || offset < 0.0
        || offset.fract() != 0.0
        || !mode.is_finite()
        || mode < 0.0
        || mode.fract() != 0.0
        || mode >= 12.0
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let mode = mode as u8;
    let semantic = mode / 4;
    let tag = unsafe { pointer.add(offset as usize).read() };
    let present = match semantic {
        0 | 1 => tag == 1,
        2 => tag == 0,
        _ => unreachable!(),
    };
    let old = if present {
        unsafe { pointer.add(offset as usize + 8).cast::<f64>().read() }
    } else {
        match (semantic, tag) {
            (0, 0) | (2, 2) => f64::NAN,
            (1, 0) | (2, 1) => 0.0,
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    };
    let new = if mode % 4 >= 2 { old - 1.0 } else { old + 1.0 };
    unsafe {
        pointer.add(offset as usize + 8).cast::<f64>().write(new);
        pointer
            .add(offset as usize)
            .write(if semantic == 2 { 0 } else { 1 });
    }
    if mode.is_multiple_of(2) {
        new
    } else {
        old
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tagged_object_number_assign(object: f64, right: f64, offset: f64, mode: f64) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null()
        || !offset.is_finite()
        || offset < 0.0
        || offset.fract() != 0.0
        || !mode.is_finite()
        || mode < 0.0
        || mode.fract() != 0.0
        || mode >= 36.0
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let mode = mode as u8;
    let semantic = mode / 12;
    let operation = mode % 12;
    let tag = unsafe { pointer.add(offset as usize).read() };
    let present = match semantic {
        0 | 1 => tag == 1,
        2 => tag == 0,
        _ => unreachable!(),
    };
    let left = if present {
        unsafe { pointer.add(offset as usize + 8).cast::<f64>().read() }
    } else {
        match (semantic, tag) {
            (0, 0) | (2, 2) => f64::NAN,
            (1, 0) | (2, 1) => 0.0,
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    };
    let value = match operation {
        0 => left + right,
        1 => left - right,
        2 => left * right,
        3 => left / right,
        4 => unsafe { fmod(left, right) },
        5 => shift_left(left, right),
        6 => shift_right(left, right),
        7 => shift_right_unsigned(left, right),
        8 => bit_or(left, right),
        9 => bit_xor(left, right),
        10 => bit_and(left, right),
        11 => power(left, right),
        _ => unreachable!(),
    };
    unsafe {
        pointer.add(offset as usize + 8).cast::<f64>().write(value);
        pointer
            .add(offset as usize)
            .write(if semantic == 2 { 0 } else { 1 });
    }
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_object_setter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, value: f64, offset: f64) -> f64 {
            fixed_object_set(object, value, offset, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_number, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_boolean, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_string, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_byte, 3);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_tuple_new(length: f64) -> f64 {
    fixed_tuple_new_with_stride(length, 8)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_wide_tuple_new(length: f64) -> f64 {
    fixed_tuple_new_with_stride(length, 16)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_tuple_new_with_stride(length: f64, stride: usize) -> f64 {
    if !length.is_finite() || length < 0.0 || length.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let Some(size) = (length as usize)
        .checked_mul(stride)
        .and_then(|size| size.checked_add(8))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let Some(output) =
        ARENA_ALLOC.with(|allocator| allocator.get().map(|allocate| unsafe { allocate(size, 8) }))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        output.write_bytes(0, size);
        output.cast::<u64>().write(length as u64);
    }
    mutable_array_handle(array_result(output))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_wide_tuple_set(tuple: f64, value: f64, index: f64, kind: u8, mode: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let field = data.cast_mut().add(8 + index as usize * 16);
        let payload = if mode == 1 {
            field.write(1);
            field.add(if kind == 1 { 1 } else { 8 })
        } else if mode == 2 {
            field.add(if kind == 1 { 1 } else { 8 })
        } else {
            field
        };
        match kind {
            0 => payload.cast::<f64>().write(value),
            1 => payload.write(u8::from(value != 0.0)),
            2 => payload.cast::<usize>().write(value.to_bits() as usize),
            3 => payload.write(value as u8),
            _ => unreachable!(),
        }
    }
    tuple
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_wide_tuple_setter {
    ($name:ident, $kind:expr, $mode:expr) => {
        extern "C" fn $name(tuple: f64, value: f64, index: f64) -> f64 {
            fixed_wide_tuple_set(tuple, value, index, $kind, $mode)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_number, 0, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_boolean, 1, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_pointer, 2, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_byte, 3, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_number, 0, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_boolean, 1, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_pointer, 2, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_number, 0, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_boolean, 1, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_pointer, 2, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_tuple_set(tuple: f64, value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let element = data.cast_mut().add(8 + index as usize * 8);
        match kind {
            0 => element.cast::<f64>().write(value),
            1 => element.write(u8::from(value != 0.0)),
            2 => element.cast::<usize>().write(value.to_bits() as usize),
            _ => unreachable!(),
        }
    }
    tuple
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_tuple_setter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, value: f64, index: f64) -> f64 {
            fixed_tuple_set(tuple, value, index, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_number, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_boolean, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_pointer, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn untag_dynamic(value: f64, expected: u64) -> f64 {
    dynamic_primitive(value, Some(expected)).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| f64::from_bits(dynamic.payload),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_string(value: f64) -> f64 {
    let Some(format) = NUMBER_TO_STRING.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::from_bits(0);
    };
    let value = unsafe { format(value) };
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        f64::from_bits(0)
    } else {
        f64::from_bits(value as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn boolean_to_string(value: f64) -> f64 {
    arena_string(if value != 0.0 { "true" } else { "false" }.into())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_to_string(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => number_to_string(f64::from_bits(dynamic.payload)),
            DYNAMIC_STRING_TAG => f64::from_bits(dynamic.payload),
            DYNAMIC_BOOLEAN_TAG => boolean_to_string(dynamic.payload as f64),
            DYNAMIC_NUMBER_ARRAY_TAG => array_format(
                0,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_BOOLEAN_ARRAY_TAG => array_format(
                2,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_STRING_ARRAY_TAG => array_format(
                1,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_NUMBER_DICTIONARY_TAG..=DYNAMIC_TUPLE_TAG => {
                arena_string("[object Object]".into())
            }
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_number(value: f64) -> f64 {
    let Some(parse) = STRING_TO_NUMBER.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_to_number(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => f64::from_bits(dynamic.payload),
            DYNAMIC_STRING_TAG => string_to_number(f64::from_bits(dynamic.payload)),
            DYNAMIC_BOOLEAN_TAG => dynamic.payload as f64,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => {
                string_to_number(dynamic_to_string(value))
            }
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_add(left: f64, right: f64) -> f64 {
    let Some((left_value, right_value)) =
        dynamic_primitive(left, None).zip(dynamic_primitive(right, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(left_value.tag, DYNAMIC_NUMBER_TAG | DYNAMIC_BOOLEAN_TAG)
        || !matches!(right_value.tag, DYNAMIC_NUMBER_TAG | DYNAMIC_BOOLEAN_TAG)
    {
        let value = string_concat(dynamic_to_string(left), dynamic_to_string(right));
        arena_dynamic(DYNAMIC_STRING_TAG, value.to_bits())
    } else {
        let value = dynamic_to_number(left) + dynamic_to_number(right);
        arena_dynamic(DYNAMIC_NUMBER_TAG, value.to_bits())
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_compare(left: f64, right: f64, operation: u8) -> f64 {
    let Some((left_value, right_value)) =
        dynamic_primitive(left, None).zip(dynamic_primitive(right, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let strict_equal = || {
        if left_value.tag != right_value.tag {
            return false;
        }
        match left_value.tag {
            DYNAMIC_NUMBER_TAG => {
                f64::from_bits(left_value.payload) == f64::from_bits(right_value.payload)
            }
            DYNAMIC_STRING_TAG => {
                string_same_value(
                    f64::from_bits(left_value.payload),
                    f64::from_bits(right_value.payload),
                ) != 0.0
            }
            DYNAMIC_BOOLEAN_TAG => left_value.payload == right_value.payload,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => {
                left_value.payload == right_value.payload
            }
            _ => unreachable!(),
        }
    };
    let equal = || {
        strict_equal()
            || (left_value.tag != right_value.tag
                && !(left_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG
                    && right_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG)
                && dynamic_to_number(left) == dynamic_to_number(right))
    };
    let result = match operation {
        0..=3 => {
            let left_string =
                left_value.tag == DYNAMIC_STRING_TAG || left_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG;
            let right_string = right_value.tag == DYNAMIC_STRING_TAG
                || right_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG;
            if left_string && right_string {
                let ordering = string_compare(dynamic_to_string(left), dynamic_to_string(right));
                match operation {
                    0 => ordering < 0.0,
                    1 => ordering <= 0.0,
                    2 => ordering > 0.0,
                    3 => ordering >= 0.0,
                    _ => unreachable!(),
                }
            } else {
                let left = dynamic_to_number(left);
                let right = dynamic_to_number(right);
                match operation {
                    0 => left < right,
                    1 => left <= right,
                    2 => left > right,
                    3 => left >= right,
                    _ => unreachable!(),
                }
            }
        }
        4 => equal(),
        5 => !equal(),
        6 => strict_equal(),
        7 => !strict_equal(),
        _ => unreachable!(),
    };
    f64::from(result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! dynamic_compare_functions {
    ($($name:ident => $operation:literal),+ $(,)?) => {
        $(extern "C" fn $name(left: f64, right: f64) -> f64 {
            dynamic_compare(left, right, $operation)
        })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
dynamic_compare_functions!(
    dynamic_less => 0,
    dynamic_less_equal => 1,
    dynamic_greater => 2,
    dynamic_greater_equal => 3,
    dynamic_equal => 4,
    dynamic_not_equal => 5,
    dynamic_strict_equal => 6,
    dynamic_strict_not_equal => 7,
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn parse_float(value: f64) -> f64 {
    let Some(parse) = PARSE_FLOAT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn parse_int(value: f64, radix: f64) -> f64 {
    let Some(parse) = PARSE_INT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value, radix) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn formatted_number(operation: u8, value: f64, argument: f64) -> f64 {
    let Some(format) = NUMBER_FORMAT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::from_bits(0);
    };
    let value = unsafe { format(operation, value, argument) };
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        f64::from_bits(0)
    } else {
        f64::from_bits(value as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_fixed(value: f64, digits: f64) -> f64 {
    let digits = if digits.is_nan() { 0.0 } else { digits.trunc() };
    if !(0.0..=100.0).contains(&digits) {
        CALL_ERROR.with(|error| error.set(INVALID_FIXED_DIGITS.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(0, value, digits)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_precision(value: f64, precision: f64) -> f64 {
    let precision = if precision.is_nan() {
        0.0
    } else {
        precision.trunc()
    };
    if !(1.0..=100.0).contains(&precision) {
        CALL_ERROR.with(|error| error.set(INVALID_PRECISION.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(1, value, precision)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_radix_string(value: f64, radix: f64) -> f64 {
    let radix = if radix.is_nan() { 0.0 } else { radix.trunc() };
    if !(2.0..=36.0).contains(&radix) {
        CALL_ERROR.with(|error| error.set(INVALID_RADIX.as_ptr().cast()));
        return f64::from_bits(0);
    }
    if radix == 10.0 {
        return number_to_string(value);
    }
    formatted_number(2, value, radix)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_exponential(value: f64, digits: f64) -> f64 {
    let digits = if digits.is_nan() { 0.0 } else { digits.trunc() };
    if !(0.0..=100.0).contains(&digits) {
        CALL_ERROR.with(|error| error.set(INVALID_EXPONENTIAL_DIGITS.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(3, value, digits)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_exponential_shortest(value: f64) -> f64 {
    formatted_number(3, value, -1.0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_lower_case(value: f64) -> f64 {
    unsafe {
        string_argument(value).map_or(f64::from_bits(0), |value| {
            arena_string(value.to_lowercase())
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_upper_case(value: f64) -> f64 {
    unsafe {
        string_argument(value).map_or(f64::from_bits(0), |value| {
            arena_string(value.to_uppercase())
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn is_javascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}' | '\u{000b}' | '\u{000c}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
                | '\u{000a}'
                | '\u{000d}'
                | '\u{2028}'
                | '\u{2029}'
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn trim_string(value: f64, start: bool, end: bool) -> f64 {
    let Some(value) = string_argument(value) else {
        return f64::from_bits(0);
    };
    let value = if start {
        value.trim_start_matches(is_javascript_whitespace)
    } else {
        value.as_str()
    };
    let value = if end {
        value.trim_end_matches(is_javascript_whitespace)
    } else {
        value
    };
    arena_string(value.to_owned())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim(value: f64) -> f64 {
    unsafe { trim_string(value, true, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim_start(value: f64) -> f64 {
    unsafe { trim_string(value, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim_end(value: f64) -> f64 {
    unsafe { trim_string(value, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_repeat(value: f64, count: f64) -> f64 {
    unsafe {
        let Some(value) = string_argument(value) else {
            return f64::from_bits(0);
        };
        let count = if count.is_nan() || count == 0.0 {
            0
        } else if !count.is_finite() || count < 0.0 {
            CALL_ERROR.with(|error| error.set(INVALID_REPEAT_COUNT.as_ptr().cast()));
            return f64::from_bits(0);
        } else {
            count.trunc() as usize
        };
        if value.len().checked_mul(count).is_none() {
            CALL_ERROR.with(|error| error.set(INVALID_REPEAT_COUNT.as_ptr().cast()));
            return f64::from_bits(0);
        }
        arena_string(value.repeat(count))
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_normalize(value: f64, form: f64) -> f64 {
    let Some(normalize) = STRING_NORMALIZE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let normalized = unsafe {
        normalize(
            value.to_bits() as usize as *const c_char,
            form.to_bits() as usize as *const c_char,
        )
    };
    if normalized.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_NORMALIZATION_FORM.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(normalized as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_split(value: f64, separator: f64, limit: f64) -> f64 {
    let Some(split) = STRING_SPLIT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe {
        split(
            value.to_bits() as usize as *const c_char,
            separator.to_bits() as usize as *const c_char,
            limit,
        )
    };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn normalize_string_index(index: f64, length: f64, negative_from_end: bool) -> usize {
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    if negative_from_end && index < 0.0 {
        (length + index).max(0.0) as usize
    } else {
        index.clamp(0.0, length) as usize
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_range(
    value: f64,
    start: f64,
    end: f64,
    negative_from_end: bool,
    swap: bool,
) -> f64 {
    let Some(value) = string_argument(value) else {
        return f64::from_bits(0);
    };
    let value = value.encode_utf16().collect::<Vec<_>>();
    let length = value.len() as f64;
    let mut start = normalize_string_index(start, length, negative_from_end);
    let mut end = normalize_string_index(end, length, negative_from_end);
    if swap && start > end {
        std::mem::swap(&mut start, &mut end);
    }
    arena_string(String::from_utf16_lossy(&value[start..end.max(start)]))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_slice(value: f64, start: f64) -> f64 {
    unsafe { string_range(value, start, f64::INFINITY, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_substring(value: f64, start: f64) -> f64 {
    unsafe { string_range(value, start, f64::INFINITY, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_slice_range(value: f64, start: f64, end: f64) -> f64 {
    unsafe { string_range(value, start, end, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_substring_range(value: f64, start: f64, end: f64) -> f64 {
    unsafe { string_range(value, start, end, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_pad(value: f64, target_length: f64, pad: f64, at_start: bool) -> f64 {
    let (Some(value), Some(pad)) = (string_argument(value), string_argument(pad)) else {
        return f64::from_bits(0);
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
    let target_length = if target_length.is_finite() && target_length > 0.0 {
        target_length as usize
    } else {
        0
    };
    let pad = pad.encode_utf16().collect::<Vec<_>>();
    if target_length <= units.len() || pad.is_empty() {
        return arena_string(value);
    }
    let needed = target_length - units.len();
    let filler = pad.into_iter().cycle().take(needed);
    let combined = if at_start {
        filler.chain(units).collect::<Vec<_>>()
    } else {
        units.into_iter().chain(filler).collect()
    };
    arena_string(String::from_utf16_lossy(&combined))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_pad_start(value: f64, target_length: f64, pad: f64) -> f64 {
    unsafe { string_pad(value, target_length, pad, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_pad_end(value: f64, target_length: f64, pad: f64) -> f64 {
    unsafe { string_pad(value, target_length, pad, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_replace(value: f64, search: f64, replacement: f64, all: bool) -> f64 {
    let (Some(value), Some(search), Some(replacement)) = (
        string_argument(value),
        string_argument(search),
        string_argument(replacement),
    ) else {
        return f64::from_bits(0);
    };
    arena_string(if all {
        value.replace(&search, &replacement)
    } else {
        value.replacen(&search, &replacement, 1)
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_replace_first(value: f64, search: f64, replacement: f64) -> f64 {
    unsafe { string_replace(value, search, replacement, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_replace_all(value: f64, search: f64, replacement: f64) -> f64 {
    unsafe { string_replace(value, search, replacement, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_concat(left: f64, right: f64) -> f64 {
    unsafe {
        let left = left.to_bits() as usize as *const c_char;
        let right = right.to_bits() as usize as *const c_char;
        if left.is_null() || right.is_null() {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return f64::from_bits(0);
        }
        let left = CStr::from_ptr(left).to_bytes();
        let right = CStr::from_ptr(right).to_bytes();
        let Some(size) = left
            .len()
            .checked_add(right.len())
            .and_then(|length| length.checked_add(1))
        else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        };
        let Some(output) =
            ARENA_ALLOC.with(|allocator| allocator.get().map(|alloc| alloc(size, 1)))
        else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        };
        if output.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        }
        std::ptr::copy_nonoverlapping(left.as_ptr(), output, left.len());
        std::ptr::copy_nonoverlapping(right.as_ptr(), output.add(left.len()), right.len());
        output.add(size - 1).write(0);
        f64::from_bits(output as usize as u64)
    }
}

fn intern_string(encoded: &str) -> Option<*const c_char> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let value = CString::new(bytes).ok()?;
    let mut constants = STRING_CONSTANTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    let value = constants.entry(encoded.to_owned()).or_insert(value);
    Some(value.as_ptr())
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

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn is_odd_integer(value: f64) -> bool {
    value.is_finite()
        && value.trunc() == value
        && value.abs() < 9_007_199_254_740_992.0
        && value.abs() % 2.0 == 1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn power(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.is_nan() {
        return f64::NAN;
    }
    let odd = is_odd_integer(exponent);
    if base.is_infinite() {
        if base.is_sign_positive() {
            return if exponent > 0.0 { f64::INFINITY } else { 0.0 };
        }
        return if exponent > 0.0 {
            if odd {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        } else if odd {
            -0.0
        } else {
            0.0
        };
    }
    if base == 0.0 {
        return if exponent > 0.0 {
            if base.is_sign_negative() && odd {
                -0.0
            } else {
                0.0
            }
        } else if base.is_sign_negative() && odd {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if exponent.is_infinite() {
        return match base.abs().partial_cmp(&1.0) {
            Some(std::cmp::Ordering::Greater) if exponent.is_sign_positive() => f64::INFINITY,
            Some(std::cmp::Ordering::Greater) => 0.0,
            Some(std::cmp::Ordering::Less) if exponent.is_sign_positive() => 0.0,
            Some(std::cmp::Ordering::Less) => f64::INFINITY,
            _ => f64::NAN,
        };
    }
    if base < 0.0 && exponent.trunc() != exponent {
        return f64::NAN;
    }
    base.powf(exponent)
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
#[repr(u8)]
enum CompareOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

impl CompareOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "lt" => Some(Self::Less),
            "lte" => Some(Self::LessEqual),
            "gt" => Some(Self::Greater),
            "gte" => Some(Self::GreaterEqual),
            "eq" => Some(Self::Equal),
            "ne" => Some(Self::NotEqual),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum UnaryMath {
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Cbrt,
    Ceil,
    Clz32,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Fround,
    Log,
    Log1p,
    Log2,
    Log10,
    Round,
    Sign,
    Sin,
    Sinh,
    SquareRoot,
    Tan,
    Tanh,
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
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "acos" => Some(Self::Acos),
            "acosh" => Some(Self::Acosh),
            "asin" => Some(Self::Asin),
            "asinh" => Some(Self::Asinh),
            "atan" => Some(Self::Atan),
            "atanh" => Some(Self::Atanh),
            "cbrt" => Some(Self::Cbrt),
            "ceil" => Some(Self::Ceil),
            "clz32" => Some(Self::Clz32),
            "cos" => Some(Self::Cos),
            "cosh" => Some(Self::Cosh),
            "exp" => Some(Self::Exp),
            "expm1" => Some(Self::Expm1),
            "floor" => Some(Self::Floor),
            "fround" => Some(Self::Fround),
            "log" => Some(Self::Log),
            "log1p" => Some(Self::Log1p),
            "log2" => Some(Self::Log2),
            "log10" => Some(Self::Log10),
            "round" => Some(Self::Round),
            "sign" => Some(Self::Sign),
            "sin" => Some(Self::Sin),
            "sinh" => Some(Self::Sinh),
            "sqrt" => Some(Self::SquareRoot),
            "tan" => Some(Self::Tan),
            "tanh" => Some(Self::Tanh),
            "trunc" => Some(Self::Truncate),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64) -> f64 {
        match self {
            Self::Acos => acos_number,
            Self::Acosh => acosh_number,
            Self::Asin => asin_number,
            Self::Asinh => asinh_number,
            Self::Atan => atan_number,
            Self::Atanh => atanh_number,
            Self::Cbrt => cbrt_number,
            Self::Ceil => ceil_number,
            Self::Clz32 => clz32_number,
            Self::Cos => cos_number,
            Self::Cosh => cosh_number,
            Self::Exp => exp_number,
            Self::Expm1 => expm1_number,
            Self::Floor => floor_number,
            Self::Fround => fround_number,
            Self::Log => log_number,
            Self::Log1p => log1p_number,
            Self::Log2 => log2_number,
            Self::Log10 => log10_number,
            Self::Round => round_number,
            Self::Sign => sign_number,
            Self::Sin => sin_number,
            Self::Sinh => sinh_number,
            Self::SquareRoot => unreachable!("square root emits SSE2 directly"),
            Self::Tan => tan_number,
            Self::Tanh => tanh_number,
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
enum NumericReduceOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
    Minimum,
    Maximum,
}

impl NumericReduceOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            "rem" => Some(Self::Remainder),
            "pow" => Some(Self::Power),
            "min" => Some(Self::Minimum),
            "max" => Some(Self::Maximum),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericValue {
    Argument(u8),
    DynamicArgument(u8),
    Constant(f64),
    Operation(NumericOp),
    Compare(CompareOp),
    Bitwise(BitwiseOp),
    BitNot,
    Absolute,
    Negate,
    Maximum,
    Minimum,
    Atan2,
    Hypot,
    Imul,
    IsFinite,
    IsInteger,
    IsNaN,
    IsSafeInteger,
    NumberSameValue,
    StringSameValue,
    ReferenceSameValue,
    TypeOfNumber,
    TypeOfBoolean,
    TypeOfString,
    TypeOfObject,
    TypeOfDynamic,
    DynamicToBoolean,
    StringCompare,
    StringCharAt,
    StringCharCodeAt,
    StringAt,
    StringCodePointAt,
    StringConcat,
    NumberToString,
    BooleanToString,
    StringToNumber,
    DynamicToString,
    DynamicToNumber,
    DynamicAdd,
    DynamicCompare(u8),
    TagNumber,
    TagString,
    TagBoolean,
    TagAggregate(u8),
    DynamicTag,
    UntagNumber,
    UntagString,
    UntagBoolean,
    UntagNumberArray,
    UntagBooleanArray,
    UntagStringArray,
    UntagArray,
    UntagNumberDictionary,
    UntagBooleanDictionary,
    UntagStringDictionary,
    UntagDictionary,
    UntagObject,
    UntagTuple,
    ObjectField(u8, u16),
    OptionalObjectField(u8, u16),
    OptionalTupleField(u8, u16),
    NullishObjectField(u8, u16),
    NullishTupleField(u8, u16),
    FixedObjectNew(u16),
    FixedObjectSet(u8, u16),
    TaggedObjectNumberUpdate(u8, u16),
    TaggedObjectNumberAssign(u8, u16),
    FixedTupleNew(u16),
    FixedTupleSet(u8, u16),
    FixedWideTupleNew(u16),
    FixedWideTupleSet(u8, u16, u8),
    ExcludeNumber,
    ExcludeString,
    ExcludeBoolean,
    ExcludeArray,
    ExcludeObject,
    GlobalGet,
    GlobalInit,
    GlobalSet,
    CallableEntryGet,
    CallableEntrySet,
    ParseFloat,
    ParseInt,
    NumberToFixed,
    NumberToPrecision,
    NumberToRadixString,
    NumberToExponential,
    NumberToExponentialShortest,
    StringConstant(*const c_char),
    StringEndsWith,
    StringEndsWithAt,
    StringIncludes,
    StringIncludesAt,
    StringIsWellFormed,
    StringIndexOf,
    StringIndexOfAt,
    StringLastIndexOf,
    StringLastIndexOfAt,
    StringLength,
    ArrayLength,
    IsArray,
    IsNotArray,
    DynamicIsArray,
    NumberArrayAt,
    BoolArrayAt,
    StringArrayAt,
    DynamicArrayAt,
    NumberArrayGet,
    BoolArrayGet,
    StringArrayGet,
    NumberDictionaryGet,
    BoolDictionaryGet,
    StringDictionaryGet,
    NumberDictionarySet,
    NumberDictionaryPostSet,
    StringDictionarySet,
    BoolDictionarySet,
    DictionaryDelete,
    DictionaryHasOwn,
    DictionaryIn,
    DictionaryKeys,
    NumberDictionaryValues,
    BoolDictionaryValues,
    StringDictionaryValues,
    NumberDictionaryEntries,
    BoolDictionaryEntries,
    StringDictionaryEntries,
    DictionaryFromNumberEntries,
    DictionaryFromBoolEntries,
    DictionaryFromStringEntries,
    DictionaryAssign,
    DictionaryLength,
    DictionaryKeyAt,
    EmptyDictionary,
    DictionaryAppend(u8),
    DictionaryStaticAppend(u8, *const c_char),
    NumberArrayIncludes,
    BoolArrayIncludes,
    StringArrayIncludes,
    DynamicArrayIncludes,
    NumberArrayIndexOf,
    BoolArrayIndexOf,
    StringArrayIndexOf,
    DynamicArrayIndexOf,
    NumberArrayLastIndexOf,
    BoolArrayLastIndexOf,
    StringArrayLastIndexOf,
    DynamicArrayLastIndexOf,
    NumberArrayJoin,
    BoolArrayJoin,
    StringArrayJoin,
    DynamicArrayJoin,
    DynamicArraySlice,
    ArraySlice,
    DynamicArrayConcat,
    ArrayConcat,
    NumberArrayAppend,
    StringArrayAppend,
    BoolArrayAppend,
    DynamicArrayAppend,
    ArrayToReversed,
    ArrayReverse,
    DynamicArrayToReversed,
    DynamicArrayReverse,
    NumberArrayToSorted,
    StringArrayToSorted,
    BoolArrayToSorted,
    DynamicArrayToSorted,
    NumberArraySort,
    NumberArrayToSortedBy(bool),
    NumberArraySortBy(bool),
    StringArrayToSortedDescending,
    StringArraySortDescending,
    StringArraySort,
    BoolArraySort,
    DynamicArraySort,
    NumberArrayFill,
    StringArrayFill,
    BoolArrayFill,
    DynamicArrayFill,
    ArrayCopyWithin,
    DynamicArrayCopyWithin,
    DynamicArrayWith,
    NumberArrayPush,
    StringArrayPush,
    BoolArrayPush,
    DynamicArrayPush,
    NumberArrayUnshift,
    StringArrayUnshift,
    BoolArrayUnshift,
    DynamicArrayUnshift,
    NumberArraySet,
    NumberArrayPostSet,
    StringArraySet,
    BoolArraySet,
    DynamicArraySet,
    AggregateLocalSet(u8, u8),
    AggregateLocalArrayInsert(u8, u8, bool),
    ArrayValue,
    MutableArrayHandle,
    EmptyArray,
    NumberArrayMin,
    NumberArrayMax,
    NumberArrayHypot,
    NumberArrayReduce(NumericReduceOp, bool, bool),
    NumberArrayJitReduce(bool, bool, bool),
    NumberArrayQuantifier(CompareOp, bool),
    NumberArrayFind(CompareOp, u8),
    NumberArrayFilter(CompareOp),
    PrimitiveArrayTruthy(u8, u8),
    DynamicArrayTruthy(u8),
    PrimitiveArrayCompare(u8, CompareOp, u8),
    DynamicArrayCompare(u8, u8),
    PrimitiveArrayMap(u8, u8),
    PrimitiveArrayConvert(u8, u8),
    DynamicArrayConvert(u8),
    DynamicArrayMapIdentity,
    DynamicArrayJitMap(u8, bool),
    DynamicArrayJitScan(u8, bool),
    DynamicArrayJitReduce(bool, bool, bool),
    NumberArrayMap(NumericReduceOp, bool),
    PrimitiveArrayJitMap(u8, u8, bool),
    PrimitiveArrayJitScan(u8, u8, bool),
    NumberArrayIndexMap(NumericReduceOp, bool),
    NumberArraySelectMap(CompareOp, u8),
    NumberArrayBranchMap(u16),
    NumberArrayUnaryMap(bool),
    NumberArrayMathMap(UnaryMath),
    NumberArrayPop,
    StringArrayPop,
    BoolArrayPop,
    DynamicArrayPop,
    NumberArrayShift,
    StringArrayShift,
    BoolArrayShift,
    DynamicArrayShift,
    ArraySplice,
    ArrayToSpliced,
    DynamicArraySplice,
    DynamicArrayToSpliced,
    NumberArrayWith,
    StringArrayWith,
    BoolArrayWith,
    StringTruthy,
    StringPadEnd,
    StringPadStart,
    StringStartsWith,
    StringStartsWithAt,
    StringRepeat,
    StringNormalize,
    StringSplit,
    StringToArray,
    StringFromCharCode,
    StringFromCodePoint,
    StringReplace,
    StringReplaceAll,
    StringSlice,
    StringSliceRange,
    StringSubstring,
    StringSubstringRange,
    StringToLowerCase,
    StringToWellFormed,
    StringToUpperCase,
    StringTrim,
    StringTrimEnd,
    StringTrimStart,
    Power,
    UnaryMath(UnaryMath),
    Remainder,
    Select,
    ShortCircuit(bool),
    ConditionalStart,
    PresentConditionalStart,
    ConditionalAlternate,
    ShortCircuitEnd,
    Absent,
    Null,
    PreserveAbsent,
    AsBoolean,
    BooleanNot,
    StrictMismatch(bool),
    Drop,
    DropUnder,
    Duplicate,
    DuplicatePair,
    LocalGet(u8),
    LocalSet(u8),
    LoopStart,
    LoopWhile,
    LoopContinuePoint,
    LoopBreak,
    LoopContinue,
    LoopEnd,
    GuardStart,
    GuardAlternate,
    GuardEnd,
    SwitchStart,
    SwitchCaseStart,
    SwitchCaseBody,
    SwitchDefault,
    SwitchBreak,
    SwitchEnd,
    TryStart,
    TaggedTryStart,
    CatchStart,
    TryEnd,
    ResultStart,
    ResultReturn(u8),
    ResultEnd,
    Throw,
    TaggedThrow(u8),
    CheckError,
    UncaughtNumberThrow,
    UncaughtBooleanThrow,
    UncaughtStringThrow,
    UncaughtNumberArrayThrow,
    UncaughtBooleanArrayThrow,
    UncaughtStringArrayThrow,
    UncaughtDictionaryThrow,
    EarlyReturn,
    MathRandom,
    DateNow,
    PerformanceNow,
    ProcessPid,
    ProcessPpid,
    MissingCallable,
    Recur(u8),
}

struct NumericProgram(Vec<NumericValue>);

impl NumericProgram {
    fn returns_tagged_array(&self) -> bool {
        matches!(
            self.0.last(),
            Some(
                NumericValue::StringSplit
                    | NumericValue::StringToArray
                    | NumericValue::ArraySlice
                    | NumericValue::ArrayConcat
                    | NumericValue::NumberArrayAppend
                    | NumericValue::StringArrayAppend
                    | NumericValue::BoolArrayAppend
                    | NumericValue::ArrayToReversed
                    | NumericValue::ArrayReverse
                    | NumericValue::NumberArrayToSorted
                    | NumericValue::StringArrayToSorted
                    | NumericValue::BoolArrayToSorted
                    | NumericValue::NumberArraySort
                    | NumericValue::StringArraySort
                    | NumericValue::BoolArraySort
                    | NumericValue::PrimitiveArrayJitMap(_, _, _)
                    | NumericValue::PrimitiveArrayJitScan(_, 6, _)
                    | NumericValue::StringArrayToSortedDescending
                    | NumericValue::StringArraySortDescending
                    | NumericValue::NumberArrayFill
                    | NumericValue::StringArrayFill
                    | NumericValue::BoolArrayFill
                    | NumericValue::ArrayCopyWithin
                    | NumericValue::ArraySplice
                    | NumericValue::ArrayToSpliced
                    | NumericValue::ArrayValue
                    | NumericValue::NumberArrayWith
                    | NumericValue::StringArrayWith
                    | NumericValue::BoolArrayWith
            )
        )
    }

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
                    "atan2" => Some(NumericValue::Atan2),
                    "hypot" => Some(NumericValue::Hypot),
                    "imul" => Some(NumericValue::Imul),
                    "isfinite" => Some(NumericValue::IsFinite),
                    "isinteger" => Some(NumericValue::IsInteger),
                    "isnan" => Some(NumericValue::IsNaN),
                    "issafeinteger" => Some(NumericValue::IsSafeInteger),
                    "numsame" => Some(NumericValue::NumberSameValue),
                    "strsame" => Some(NumericValue::StringSameValue),
                    "refsame" => Some(NumericValue::ReferenceSameValue),
                    "typeofnumber" => Some(NumericValue::TypeOfNumber),
                    "typeofboolean" => Some(NumericValue::TypeOfBoolean),
                    "typeofstring" => Some(NumericValue::TypeOfString),
                    "typeofobject" => Some(NumericValue::TypeOfObject),
                    "typeofdynamic" => Some(NumericValue::TypeOfDynamic),
                    "dynbool" => Some(NumericValue::DynamicToBoolean),
                    "strcmp" => Some(NumericValue::StringCompare),
                    "charat" => Some(NumericValue::StringCharAt),
                    "charcodeat" => Some(NumericValue::StringCharCodeAt),
                    "at" => Some(NumericValue::StringAt),
                    "codepointat" => Some(NumericValue::StringCodePointAt),
                    "concat" => Some(NumericValue::StringConcat),
                    "numstr" => Some(NumericValue::NumberToString),
                    "boolstr" => Some(NumericValue::BooleanToString),
                    "strnum" => Some(NumericValue::StringToNumber),
                    "dynstr" => Some(NumericValue::DynamicToString),
                    "dynnum" => Some(NumericValue::DynamicToNumber),
                    "dynadd" => Some(NumericValue::DynamicAdd),
                    "dynlt" => Some(NumericValue::DynamicCompare(0)),
                    "dynlte" => Some(NumericValue::DynamicCompare(1)),
                    "dyngt" => Some(NumericValue::DynamicCompare(2)),
                    "dyngte" => Some(NumericValue::DynamicCompare(3)),
                    "dyneq" => Some(NumericValue::DynamicCompare(4)),
                    "dynne" => Some(NumericValue::DynamicCompare(5)),
                    "dynseq" => Some(NumericValue::DynamicCompare(6)),
                    "dynsne" => Some(NumericValue::DynamicCompare(7)),
                    "tagnum" => Some(NumericValue::TagNumber),
                    "tagstr" => Some(NumericValue::TagString),
                    "tagbool" => Some(NumericValue::TagBoolean),
                    "tagrn" => Some(NumericValue::TagAggregate(0)),
                    "tagrb" => Some(NumericValue::TagAggregate(1)),
                    "tagrs" => Some(NumericValue::TagAggregate(2)),
                    "tagdn" => Some(NumericValue::TagAggregate(3)),
                    "tagdb" => Some(NumericValue::TagAggregate(4)),
                    "tagds" => Some(NumericValue::TagAggregate(5)),
                    "tagobject" => Some(NumericValue::TagAggregate(6)),
                    "tagtuple" => Some(NumericValue::TagAggregate(7)),
                    "tagkind" => Some(NumericValue::DynamicTag),
                    "untagnum" => Some(NumericValue::UntagNumber),
                    "untagstr" => Some(NumericValue::UntagString),
                    "untagbool" => Some(NumericValue::UntagBoolean),
                    "untagrn" => Some(NumericValue::UntagNumberArray),
                    "untagrb" => Some(NumericValue::UntagBooleanArray),
                    "untagrs" => Some(NumericValue::UntagStringArray),
                    "untagarray" | "untagarrayn" | "untagarrayb" | "untagarrays" => {
                        Some(NumericValue::UntagArray)
                    }
                    "untagdn" => Some(NumericValue::UntagNumberDictionary),
                    "untagdb" => Some(NumericValue::UntagBooleanDictionary),
                    "untagds" => Some(NumericValue::UntagStringDictionary),
                    "untagdictionary" => Some(NumericValue::UntagDictionary),
                    "untagobject" => Some(NumericValue::UntagObject),
                    "untagtuple" => Some(NumericValue::UntagTuple),
                    "notnum" => Some(NumericValue::ExcludeNumber),
                    "notstr" => Some(NumericValue::ExcludeString),
                    "notbool" => Some(NumericValue::ExcludeBoolean),
                    "notarray" => Some(NumericValue::ExcludeArray),
                    "notobject" => Some(NumericValue::ExcludeObject),
                    "globalget" => Some(NumericValue::GlobalGet),
                    "globalinit" => Some(NumericValue::GlobalInit),
                    "globalset" => Some(NumericValue::GlobalSet),
                    "callableget" => Some(NumericValue::CallableEntryGet),
                    "callableset" => Some(NumericValue::CallableEntrySet),
                    "parsefloat" => Some(NumericValue::ParseFloat),
                    "parseint" => Some(NumericValue::ParseInt),
                    "tofixed" => Some(NumericValue::NumberToFixed),
                    "toprecision" => Some(NumericValue::NumberToPrecision),
                    "toradix" => Some(NumericValue::NumberToRadixString),
                    "toexponential" => Some(NumericValue::NumberToExponential),
                    "toexponential0" => Some(NumericValue::NumberToExponentialShortest),
                    "endswith" => Some(NumericValue::StringEndsWith),
                    "endswith2" => Some(NumericValue::StringEndsWithAt),
                    "includes" => Some(NumericValue::StringIncludes),
                    "includes2" => Some(NumericValue::StringIncludesAt),
                    "iswellformed" => Some(NumericValue::StringIsWellFormed),
                    "indexof" => Some(NumericValue::StringIndexOf),
                    "indexof2" => Some(NumericValue::StringIndexOfAt),
                    "lastindexof" => Some(NumericValue::StringLastIndexOf),
                    "lastindexof2" => Some(NumericValue::StringLastIndexOfAt),
                    "strlen" => Some(NumericValue::StringLength),
                    "arraylen" => Some(NumericValue::ArrayLength),
                    "isarray" => Some(NumericValue::IsArray),
                    "isnotarray" => Some(NumericValue::IsNotArray),
                    "dynisarray" => Some(NumericValue::DynamicIsArray),
                    "rnat" => Some(NumericValue::NumberArrayAt),
                    "rbat" => Some(NumericValue::BoolArrayAt),
                    "rsat" => Some(NumericValue::StringArrayAt),
                    "dynarrayat" => Some(NumericValue::DynamicArrayAt),
                    "rnget" => Some(NumericValue::NumberArrayGet),
                    "rbget" => Some(NumericValue::BoolArrayGet),
                    "rsget" => Some(NumericValue::StringArrayGet),
                    "raget" => Some(NumericValue::StringArrayGet),
                    "roget" => Some(NumericValue::StringArrayGet),
                    "ragetrn" | "ragetrb" | "ragetrs" | "rogetdn" | "rogetdb" | "rogetds" => {
                        Some(NumericValue::StringArrayGet)
                    }
                    "dnget" => Some(NumericValue::NumberDictionaryGet),
                    "dbget" => Some(NumericValue::BoolDictionaryGet),
                    "dsget" => Some(NumericValue::StringDictionaryGet),
                    "dnset" => Some(NumericValue::NumberDictionarySet),
                    "dnpostset" => Some(NumericValue::NumberDictionaryPostSet),
                    "dsset" => Some(NumericValue::StringDictionarySet),
                    "dbset" => Some(NumericValue::BoolDictionarySet),
                    "ddelete" => Some(NumericValue::DictionaryDelete),
                    "dhasown" => Some(NumericValue::DictionaryHasOwn),
                    "din" => Some(NumericValue::DictionaryIn),
                    "dkeys" => Some(NumericValue::DictionaryKeys),
                    "dnvalues" => Some(NumericValue::NumberDictionaryValues),
                    "dbvalues" => Some(NumericValue::BoolDictionaryValues),
                    "dsvalues" => Some(NumericValue::StringDictionaryValues),
                    "dnentries" => Some(NumericValue::NumberDictionaryEntries),
                    "dbentries" => Some(NumericValue::BoolDictionaryEntries),
                    "dsentries" => Some(NumericValue::StringDictionaryEntries),
                    "dnfromentries" => Some(NumericValue::DictionaryFromNumberEntries),
                    "dbfromentries" => Some(NumericValue::DictionaryFromBoolEntries),
                    "dsfromentries" => Some(NumericValue::DictionaryFromStringEntries),
                    "dassign" => Some(NumericValue::DictionaryAssign),
                    "dlen" => Some(NumericValue::DictionaryLength),
                    "dkeyat" => Some(NumericValue::DictionaryKeyAt),
                    "dnempty" | "dbempty" | "dsempty" => Some(NumericValue::EmptyDictionary),
                    "dnappend" => Some(NumericValue::DictionaryAppend(0)),
                    "dbappend" => Some(NumericValue::DictionaryAppend(1)),
                    "dsappend" => Some(NumericValue::DictionaryAppend(2)),
                    "rnincludes" => Some(NumericValue::NumberArrayIncludes),
                    "rbincludes" => Some(NumericValue::BoolArrayIncludes),
                    "rsincludes" => Some(NumericValue::StringArrayIncludes),
                    "dynarrayincludes" => Some(NumericValue::DynamicArrayIncludes),
                    "rnindexof" => Some(NumericValue::NumberArrayIndexOf),
                    "rbindexof" => Some(NumericValue::BoolArrayIndexOf),
                    "rsindexof" => Some(NumericValue::StringArrayIndexOf),
                    "dynarrayindexof" => Some(NumericValue::DynamicArrayIndexOf),
                    "rnlastindexof" => Some(NumericValue::NumberArrayLastIndexOf),
                    "rblastindexof" => Some(NumericValue::BoolArrayLastIndexOf),
                    "rslastindexof" => Some(NumericValue::StringArrayLastIndexOf),
                    "dynarraylastindexof" => Some(NumericValue::DynamicArrayLastIndexOf),
                    "rnjoin" => Some(NumericValue::NumberArrayJoin),
                    "rbjoin" => Some(NumericValue::BoolArrayJoin),
                    "rsjoin" => Some(NumericValue::StringArrayJoin),
                    "dynarrayjoin" => Some(NumericValue::DynamicArrayJoin),
                    "dynarrayslice" => Some(NumericValue::DynamicArraySlice),
                    "arrayslice" => Some(NumericValue::ArraySlice),
                    "dynarrayconcat" => Some(NumericValue::DynamicArrayConcat),
                    "arrayconcat" => Some(NumericValue::ArrayConcat),
                    "rnappend" => Some(NumericValue::NumberArrayAppend),
                    "rsappend" => Some(NumericValue::StringArrayAppend),
                    "rbappend" => Some(NumericValue::BoolArrayAppend),
                    "dynarrayappend" => Some(NumericValue::DynamicArrayAppend),
                    "captureappend" => Some(NumericValue::NumberArrayAppend),
                    "arrayreversed" => Some(NumericValue::ArrayToReversed),
                    "arrayreverse" => Some(NumericValue::ArrayReverse),
                    "dynarrayreversed" => Some(NumericValue::DynamicArrayToReversed),
                    "dynarrayreverse" => Some(NumericValue::DynamicArrayReverse),
                    "rnsorted" => Some(NumericValue::NumberArrayToSorted),
                    "rssorted" => Some(NumericValue::StringArrayToSorted),
                    "rbsorted" => Some(NumericValue::BoolArrayToSorted),
                    "dynarraysorted" => Some(NumericValue::DynamicArrayToSorted),
                    "rnsort" => Some(NumericValue::NumberArraySort),
                    "rnsortedasc" => Some(NumericValue::NumberArrayToSortedBy(false)),
                    "rnsorteddesc" => Some(NumericValue::NumberArrayToSortedBy(true)),
                    "rnsortasc" => Some(NumericValue::NumberArraySortBy(false)),
                    "rnsortdesc" => Some(NumericValue::NumberArraySortBy(true)),
                    "rssorteddesc" => Some(NumericValue::StringArrayToSortedDescending),
                    "rssortdesc" => Some(NumericValue::StringArraySortDescending),
                    "rssort" => Some(NumericValue::StringArraySort),
                    "rbsort" => Some(NumericValue::BoolArraySort),
                    "dynarraysort" => Some(NumericValue::DynamicArraySort),
                    "rnfill" => Some(NumericValue::NumberArrayFill),
                    "rsfill" => Some(NumericValue::StringArrayFill),
                    "rbfill" => Some(NumericValue::BoolArrayFill),
                    "dynarrayfill" => Some(NumericValue::DynamicArrayFill),
                    "arraycopywithin" => Some(NumericValue::ArrayCopyWithin),
                    "dynarraycopywithin" => Some(NumericValue::DynamicArrayCopyWithin),
                    "dynarraywith" => Some(NumericValue::DynamicArrayWith),
                    "arraysplice" => Some(NumericValue::ArraySplice),
                    "arraytospliced" => Some(NumericValue::ArrayToSpliced),
                    "dynarraysplice" => Some(NumericValue::DynamicArraySplice),
                    "dynarraytospliced" => Some(NumericValue::DynamicArrayToSpliced),
                    "rnpush" => Some(NumericValue::NumberArrayPush),
                    "rspush" => Some(NumericValue::StringArrayPush),
                    "rbpush" => Some(NumericValue::BoolArrayPush),
                    "dynarraypush" => Some(NumericValue::DynamicArrayPush),
                    "rnunshift" => Some(NumericValue::NumberArrayUnshift),
                    "rsunshift" => Some(NumericValue::StringArrayUnshift),
                    "rbunshift" => Some(NumericValue::BoolArrayUnshift),
                    "dynarrayunshift" => Some(NumericValue::DynamicArrayUnshift),
                    "rnset" => Some(NumericValue::NumberArraySet),
                    "rnpostset" => Some(NumericValue::NumberArrayPostSet),
                    "rsset" => Some(NumericValue::StringArraySet),
                    "rbset" => Some(NumericValue::BoolArraySet),
                    "dynarrayset" => Some(NumericValue::DynamicArraySet),
                    "dynarraysometruthy" => Some(NumericValue::DynamicArrayTruthy(0)),
                    "dynarrayeverytruthy" => Some(NumericValue::DynamicArrayTruthy(1)),
                    "dynarrayfindtruthy" => Some(NumericValue::DynamicArrayTruthy(2)),
                    "dynarrayfindindextruthy" => Some(NumericValue::DynamicArrayTruthy(3)),
                    "dynarrayfindlasttruthy" => Some(NumericValue::DynamicArrayTruthy(4)),
                    "dynarrayfindlastindextruthy" => Some(NumericValue::DynamicArrayTruthy(5)),
                    "dynarrayfiltertruthy" => Some(NumericValue::DynamicArrayTruthy(6)),
                    "dynarraymaptonumber" => Some(NumericValue::DynamicArrayConvert(0)),
                    "dynarraymaptoboolean" => Some(NumericValue::DynamicArrayConvert(1)),
                    "dynarraymaptostring" => Some(NumericValue::DynamicArrayConvert(2)),
                    "dynarraymapidentity" => Some(NumericValue::DynamicArrayMapIdentity),
                    _ if token.starts_with("dynarraymapjit") => {
                        let (target, captured) = match token.strip_prefix("dynarraymapjit")? {
                            "n" => (0, false),
                            "b" => (1, false),
                            "s" => (2, false),
                            "nc" => (0, true),
                            "bc" => (1, true),
                            "sc" => (2, true),
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayJitMap(target, captured))
                    }
                    "dynarrayreducejit" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, false, false))
                    }
                    "dynarrayreducejit0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, false, false))
                    }
                    "dynarrayreducerightjit" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, true, false))
                    }
                    "dynarrayreducerightjit0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, true, false))
                    }
                    "dynarrayreducejitc" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, false, true))
                    }
                    "dynarrayreducejitc0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, false, true))
                    }
                    "dynarrayreducerightjitc" => {
                        Some(NumericValue::DynamicArrayJitReduce(true, true, true))
                    }
                    "dynarrayreducerightjitc0" => {
                        Some(NumericValue::DynamicArrayJitReduce(false, true, true))
                    }
                    _ if token.starts_with("dynarray") && token.contains("jit") => {
                        let suffix = token.strip_prefix("dynarray")?;
                        let (mode, captured) = match suffix {
                            "somejit" => (0, false),
                            "everyjit" => (1, false),
                            "findjit" => (2, false),
                            "findindexjit" => (3, false),
                            "findlastjit" => (4, false),
                            "findlastindexjit" => (5, false),
                            "filterjit" => (6, false),
                            "somejitc" => (0, true),
                            "everyjitc" => (1, true),
                            "findjitc" => (2, true),
                            "findindexjitc" => (3, true),
                            "findlastjitc" => (4, true),
                            "findlastindexjitc" => (5, true),
                            "filterjitc" => (6, true),
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayJitScan(mode, captured))
                    }
                    _ if token.strip_prefix("dynarray").is_some_and(|operation| {
                        [
                            "findlastindex",
                            "findlast",
                            "findindex",
                            "filter",
                            "every",
                            "some",
                            "find",
                        ]
                        .iter()
                        .any(|method| operation.starts_with(method))
                    }) =>
                    {
                        let operation = token.strip_prefix("dynarray")?;
                        let (operation, mode) = [
                            ("findlastindex", 5),
                            ("findlast", 4),
                            ("findindex", 3),
                            ("filter", 6),
                            ("every", 1),
                            ("some", 0),
                            ("find", 2),
                        ]
                        .into_iter()
                        .find_map(|(method, mode)| {
                            operation
                                .strip_prefix(method)
                                .map(|operation| (operation, mode))
                        })?;
                        let operation = match operation {
                            "lt" => 0,
                            "lte" => 1,
                            "gt" => 2,
                            "gte" => 3,
                            "eq" => 4,
                            "ne" => 5,
                            "seq" => 6,
                            "sne" => 7,
                            _ => return None,
                        };
                        Some(NumericValue::DynamicArrayCompare(operation, mode))
                    }
                    "arrayvalue" => Some(NumericValue::ArrayValue),
                    "arrayhandle" => Some(NumericValue::MutableArrayHandle),
                    "arrayempty" => Some(NumericValue::EmptyArray),
                    "rnmin" => Some(NumericValue::NumberArrayMin),
                    "rnmax" => Some(NumericValue::NumberArrayMax),
                    "rnhypot" => Some(NumericValue::NumberArrayHypot),
                    "rnreducejit" => Some(NumericValue::NumberArrayJitReduce(true, false, false)),
                    "rnreducejit0" => Some(NumericValue::NumberArrayJitReduce(false, false, false)),
                    "rnreducerightjit" => {
                        Some(NumericValue::NumberArrayJitReduce(true, true, false))
                    }
                    "rnreducerightjit0" => {
                        Some(NumericValue::NumberArrayJitReduce(false, true, false))
                    }
                    "rnreducejitc" => Some(NumericValue::NumberArrayJitReduce(true, false, true)),
                    "rnreducejitc0" => Some(NumericValue::NumberArrayJitReduce(false, false, true)),
                    "rnreducerightjitc" => {
                        Some(NumericValue::NumberArrayJitReduce(true, true, true))
                    }
                    "rnreducerightjitc0" => {
                        Some(NumericValue::NumberArrayJitReduce(false, true, true))
                    }
                    "rnpop" => Some(NumericValue::NumberArrayPop),
                    "rspop" => Some(NumericValue::StringArrayPop),
                    "rbpop" => Some(NumericValue::BoolArrayPop),
                    "dynarraypop" => Some(NumericValue::DynamicArrayPop),
                    "rnshift" => Some(NumericValue::NumberArrayShift),
                    "rsshift" => Some(NumericValue::StringArrayShift),
                    "rbshift" => Some(NumericValue::BoolArrayShift),
                    "dynarrayshift" => Some(NumericValue::DynamicArrayShift),
                    "drop" => Some(NumericValue::Drop),
                    "nip" => Some(NumericValue::DropUnder),
                    "dup" => Some(NumericValue::Duplicate),
                    "dup2" => Some(NumericValue::DuplicatePair),
                    "loop" => Some(NumericValue::LoopStart),
                    "while" => Some(NumericValue::LoopWhile),
                    "looptail" => Some(NumericValue::LoopContinuePoint),
                    "break" => Some(NumericValue::LoopBreak),
                    "continue" => Some(NumericValue::LoopContinue),
                    "loopend" => Some(NumericValue::LoopEnd),
                    "guard" => Some(NumericValue::GuardStart),
                    "guardelse" => Some(NumericValue::GuardAlternate),
                    "guardend" => Some(NumericValue::GuardEnd),
                    "switch" => Some(NumericValue::SwitchStart),
                    "case" => Some(NumericValue::SwitchCaseStart),
                    "casebody" => Some(NumericValue::SwitchCaseBody),
                    "default" => Some(NumericValue::SwitchDefault),
                    "switchbreak" => Some(NumericValue::SwitchBreak),
                    "switchend" => Some(NumericValue::SwitchEnd),
                    "trystart" => Some(NumericValue::TryStart),
                    "trystarttag" => Some(NumericValue::TaggedTryStart),
                    "catch" => Some(NumericValue::CatchStart),
                    "tryend" => Some(NumericValue::TryEnd),
                    "resultstart" => Some(NumericValue::ResultStart),
                    "resultreturn" => Some(NumericValue::ResultReturn(0)),
                    value if value.starts_with("resultreturn") => value
                        .strip_prefix("resultreturn")?
                        .parse::<u8>()
                        .ok()
                        .filter(|count| (1..=8).contains(count))
                        .map(NumericValue::ResultReturn),
                    "resultend" => Some(NumericValue::ResultEnd),
                    "throw" => Some(NumericValue::Throw),
                    value if value.starts_with("throwtag") => value
                        .strip_prefix("throwtag")?
                        .parse::<u8>()
                        .ok()
                        .filter(|tag| *tag <= 8)
                        .map(NumericValue::TaggedThrow),
                    "checkerror" => Some(NumericValue::CheckError),
                    "throwoutn" => Some(NumericValue::UncaughtNumberThrow),
                    "throwoutb" => Some(NumericValue::UncaughtBooleanThrow),
                    "throwouts" => Some(NumericValue::UncaughtStringThrow),
                    "throwoutrn" => Some(NumericValue::UncaughtNumberArrayThrow),
                    "throwoutrb" => Some(NumericValue::UncaughtBooleanArrayThrow),
                    "throwoutrs" => Some(NumericValue::UncaughtStringArrayThrow),
                    "throwoutd" => Some(NumericValue::UncaughtDictionaryThrow),
                    "return" => Some(NumericValue::EarlyReturn),
                    "random" => Some(NumericValue::MathRandom),
                    "datenow" => Some(NumericValue::DateNow),
                    "performancenow" => Some(NumericValue::PerformanceNow),
                    "processpid" => Some(NumericValue::ProcessPid),
                    "processppid" => Some(NumericValue::ProcessPpid),
                    "missingcalln" | "missingcallb" | "missingcalls" | "missingcalldyn"
                    | "missingcalla" | "missingcalld" => Some(NumericValue::MissingCallable),
                    "rnwith" => Some(NumericValue::NumberArrayWith),
                    "rswith" => Some(NumericValue::StringArrayWith),
                    "rbwith" => Some(NumericValue::BoolArrayWith),
                    "strbool" => Some(NumericValue::StringTruthy),
                    "padend" => Some(NumericValue::StringPadEnd),
                    "padstart" => Some(NumericValue::StringPadStart),
                    "startswith" => Some(NumericValue::StringStartsWith),
                    "startswith2" => Some(NumericValue::StringStartsWithAt),
                    "repeat" => Some(NumericValue::StringRepeat),
                    "normalize" => Some(NumericValue::StringNormalize),
                    "split" => Some(NumericValue::StringSplit),
                    "strarray" => Some(NumericValue::StringToArray),
                    "fromcharcode" => Some(NumericValue::StringFromCharCode),
                    "fromcodepoint" => Some(NumericValue::StringFromCodePoint),
                    "replace" => Some(NumericValue::StringReplace),
                    "replaceall" => Some(NumericValue::StringReplaceAll),
                    "slice" => Some(NumericValue::StringSlice),
                    "slice2" => Some(NumericValue::StringSliceRange),
                    "substring" => Some(NumericValue::StringSubstring),
                    "substring2" => Some(NumericValue::StringSubstringRange),
                    "tolowercase" => Some(NumericValue::StringToLowerCase),
                    "towellformed" => Some(NumericValue::StringToWellFormed),
                    "touppercase" => Some(NumericValue::StringToUpperCase),
                    "trim" => Some(NumericValue::StringTrim),
                    "trimend" => Some(NumericValue::StringTrimEnd),
                    "trimstart" => Some(NumericValue::StringTrimStart),
                    "pow" => Some(NumericValue::Power),
                    "%" => Some(NumericValue::Remainder),
                    "?" => Some(NumericValue::Select),
                    "&&" => Some(NumericValue::ShortCircuit(true)),
                    "||" => Some(NumericValue::ShortCircuit(false)),
                    "if" => Some(NumericValue::ConditionalStart),
                    "ifpresent" => Some(NumericValue::PresentConditionalStart),
                    "else" => Some(NumericValue::ConditionalAlternate),
                    "end" => Some(NumericValue::ShortCircuitEnd),
                    "absentn" | "absentb" | "absents" | "absentdyn" | "absenta" | "absentd" => {
                        Some(NumericValue::Absent)
                    }
                    "keepabsentn" | "keepabsentb" | "keepabsents" | "keepabsenta" => {
                        Some(NumericValue::PreserveAbsent)
                    }
                    "nulln" | "nullb" | "nulls" | "nulla" | "nulld" => Some(NumericValue::Null),
                    "asbool" => Some(NumericValue::AsBoolean),
                    "boolnot" => Some(NumericValue::BooleanNot),
                    "strictfalse" => Some(NumericValue::StrictMismatch(false)),
                    "stricttrue" => Some(NumericValue::StrictMismatch(true)),
                    value => UnaryMath::parse(value)
                        .map(NumericValue::UnaryMath)
                        .or_else(|| {
                            value.strip_prefix("rnreduce").and_then(|operation| {
                                let (operation, from_right) = operation
                                    .strip_prefix("right")
                                    .map_or((operation, false), |operation| (operation, true));
                                let (operation, has_initial) = operation
                                    .strip_suffix('0')
                                    .map_or((operation, true), |operation| (operation, false));
                                NumericReduceOp::parse(operation).map(|operation| {
                                    NumericValue::NumberArrayReduce(
                                        operation,
                                        has_initial,
                                        from_right,
                                    )
                                })
                            })
                        })
                        .or_else(|| {
                            let (operation, every) = value
                                .strip_prefix("rnsome")
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnevery")
                                        .map(|operation| (operation, true))
                                })?;
                            let operation = match operation {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayQuantifier(operation, every))
                        })
                        .or_else(|| {
                            let (operation, mode) = value
                                .strip_prefix("rnfindlastindex")
                                .map(|operation| (operation, 3))
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnfindlast")
                                        .map(|operation| (operation, 2))
                                })
                                .or_else(|| {
                                    value
                                        .strip_prefix("rnfindindex")
                                        .map(|operation| (operation, 1))
                                })
                                .or_else(|| {
                                    value.strip_prefix("rnfind").map(|operation| (operation, 0))
                                })?;
                            let operation = match operation {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayFind(operation, mode))
                        })
                        .or_else(|| {
                            let operation = match value.strip_prefix("rnfilter")? {
                                "lt" => CompareOp::Less,
                                "lte" => CompareOp::LessEqual,
                                "gt" => CompareOp::Greater,
                                "gte" => CompareOp::GreaterEqual,
                                "eq" => CompareOp::Equal,
                                "ne" => CompareOp::NotEqual,
                                _ => return None,
                            };
                            Some(NumericValue::NumberArrayFilter(operation))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rn")
                                .map(|operation| (0, operation))
                                .or_else(|| {
                                    value.strip_prefix("rb").map(|operation| (1, operation))
                                })
                                .or_else(|| {
                                    value.strip_prefix("rs").map(|operation| (2, operation))
                                })?;
                            let mode = match operation {
                                "sometruthy" => 0,
                                "everytruthy" => 1,
                                "findtruthy" => 2,
                                "findindextruthy" => 3,
                                "findlasttruthy" => 4,
                                "findlastindextruthy" => 5,
                                "filtertruthy" => 6,
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayTruthy(kind, mode))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rb")
                                .map(|operation| (1, operation))
                                .or_else(|| {
                                    value.strip_prefix("rs").map(|operation| (2, operation))
                                })?;
                            let (operation, mode) = operation
                                .strip_prefix("some")
                                .map(|operation| (operation, 0))
                                .or_else(|| {
                                    operation
                                        .strip_prefix("every")
                                        .map(|operation| (operation, 1))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findlastindex")
                                        .map(|operation| (operation, 5))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findlast")
                                        .map(|operation| (operation, 4))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("findindex")
                                        .map(|operation| (operation, 3))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("find")
                                        .map(|operation| (operation, 2))
                                })
                                .or_else(|| {
                                    operation
                                        .strip_prefix("filter")
                                        .map(|operation| (operation, 6))
                                })?;
                            Some(NumericValue::PrimitiveArrayCompare(
                                kind,
                                CompareOp::parse(operation)?,
                                mode,
                            ))
                        })
                        .or_else(|| {
                            let (kind, operation) = value
                                .strip_prefix("rbmap")
                                .map(|operation| (1, operation))
                                .or_else(|| {
                                    value.strip_prefix("rsmap").map(|operation| (2, operation))
                                })?;
                            let operation = match operation {
                                "identity" => 0,
                                "not" if kind == 1 => 1,
                                "tolowercase" if kind == 2 => 2,
                                "touppercase" if kind == 2 => 3,
                                "trim" if kind == 2 => 4,
                                "trimstart" if kind == 2 => 5,
                                "trimend" if kind == 2 => 6,
                                "length" if kind == 2 => 7,
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayMap(kind, operation))
                        })
                        .or_else(|| {
                            let (source, target) = value
                                .strip_prefix("rnmapto")
                                .map(|target| (0, target))
                                .or_else(|| value.strip_prefix("rbmapto").map(|target| (1, target)))
                                .or_else(|| {
                                    value.strip_prefix("rsmapto").map(|target| (2, target))
                                })?;
                            Some(NumericValue::PrimitiveArrayConvert(
                                source,
                                match target {
                                    "number" => 0,
                                    "boolean" => 1,
                                    "string" => 2,
                                    _ => return None,
                                },
                            ))
                        })
                        .or_else(|| {
                            let encoded =
                                u16::from_str_radix(value.strip_prefix("rnmapbranch")?, 16).ok()?;
                            let operation = encoded & 7;
                            let true_branch = (encoded >> 4) & 15;
                            let false_branch = (encoded >> 8) & 15;
                            (operation <= 5 && true_branch <= 13 && false_branch <= 13)
                                .then_some(NumericValue::NumberArrayBranchMap(encoded))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("rnmapselect")?;
                            let (operation, mode) = encoded.split_at(encoded.len().checked_sub(1)?);
                            Some(NumericValue::NumberArraySelectMap(
                                CompareOp::parse(operation)?,
                                mode.parse::<u8>().ok().filter(|mode| *mode < 4)?,
                            ))
                        })
                        .or_else(|| {
                            let operation = value.strip_prefix("rnmapindex")?;
                            let (operation, reverse) = NumericReduceOp::parse(operation)
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    NumericReduceOp::parse(operation.strip_prefix('r')?)
                                        .map(|operation| (operation, true))
                                })?;
                            (!matches!(
                                operation,
                                NumericReduceOp::Minimum | NumericReduceOp::Maximum
                            ))
                            .then_some(NumericValue::NumberArrayIndexMap(operation, reverse))
                        })
                        .or_else(|| {
                            Some(NumericValue::NumberArrayUnaryMap(match value {
                                "rnmapneg" => false,
                                "rnmapabs" => true,
                                _ => return None,
                            }))
                        })
                        .or(match value {
                            "rnmapjit" => Some(NumericValue::PrimitiveArrayJitMap(0, 0, false)),
                            "rnmapjitc" => Some(NumericValue::PrimitiveArrayJitMap(0, 0, true)),
                            _ => None,
                        })
                        .or_else(|| {
                            let (source, suffix) = ["rn", "rb", "rs"].iter().enumerate().find_map(
                                |(source, prefix)| {
                                    value
                                        .strip_prefix(prefix)
                                        .map(|suffix| (source as u8, suffix))
                                },
                            )?;
                            let suffix = suffix.strip_prefix("mapjit")?;
                            let (target, captured) = match suffix {
                                "n" => (0, false),
                                "b" => (1, false),
                                "s" => (2, false),
                                "nc" => (0, true),
                                "bc" => (1, true),
                                "sc" => (2, true),
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayJitMap(source, target, captured))
                        })
                        .or_else(|| {
                            let (kind, suffix) = ["rn", "rb", "rs"].iter().enumerate().find_map(
                                |(kind, prefix)| {
                                    value
                                        .strip_prefix(prefix)
                                        .map(|suffix| (kind as u8, suffix))
                                },
                            )?;
                            let (mode, captured) = match suffix {
                                "somejit" => (0, false),
                                "everyjit" => (1, false),
                                "findjit" => (2, false),
                                "findindexjit" => (3, false),
                                "findlastjit" => (4, false),
                                "findlastindexjit" => (5, false),
                                "filterjit" => (6, false),
                                "somejitc" => (0, true),
                                "everyjitc" => (1, true),
                                "findjitc" => (2, true),
                                "findindexjitc" => (3, true),
                                "findlastjitc" => (4, true),
                                "findlastindexjitc" => (5, true),
                                "filterjitc" => (6, true),
                                _ => return None,
                            };
                            Some(NumericValue::PrimitiveArrayJitScan(kind, mode, captured))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("rnmap")
                                .and_then(UnaryMath::parse)
                                .map(NumericValue::NumberArrayMathMap)
                        })
                        .or_else(|| {
                            let operation = value.strip_prefix("rnmap")?;
                            let (operation, reverse) = NumericReduceOp::parse(operation)
                                .map(|operation| (operation, false))
                                .or_else(|| {
                                    NumericReduceOp::parse(operation.strip_prefix('r')?)
                                        .map(|operation| (operation, true))
                                })?;
                            (!matches!(
                                operation,
                                NumericReduceOp::Minimum | NumericReduceOp::Maximum
                            ))
                            .then_some(NumericValue::NumberArrayMap(operation, reverse))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("recur")?;
                            matches!(encoded.as_bytes().first(), Some(b'n' | b'b' | b's'))
                                .then_some(encoded.get(1..)?)?
                                .parse::<u8>()
                                .ok()
                                .filter(|arity| (1..=8).contains(arity))
                                .map(NumericValue::Recur)
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix('u')?;
                            let digits = encoded.bytes().take_while(u8::is_ascii_digit).count();
                            let (index, kinds) = encoded.split_at(digits);
                            (!kinds.is_empty()
                                && kinds.bytes().all(|kind| {
                                    matches!(
                                        kind,
                                        b'n' | b'b'
                                            | b's'
                                            | b'N'
                                            | b'B'
                                            | b'S'
                                            | b'D'
                                            | b'E'
                                            | b'F'
                                            | b'O'
                                            | b'T'
                                            | b'X'
                                            | b'Y'
                                            | b'Z'
                                    )
                                }))
                            .then_some(index)?
                            .parse::<u8>()
                            .ok()
                            .filter(|index| *index < 15)
                            .map(NumericValue::DynamicArgument)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("objnew")?
                                .parse::<u16>()
                                .ok()
                                .filter(|size| *size > 0)
                                .map(NumericValue::FixedObjectNew)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("tupneww")?
                                .parse::<u16>()
                                .ok()
                                .map(NumericValue::FixedWideTupleNew)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("tupnew")?
                                .parse::<u16>()
                                .ok()
                                .map(NumericValue::FixedTupleNew)
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupnull")?;
                            let (kind, index) = if let Some(index) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, index)
                            } else {
                                let (kind, index) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    index,
                                )
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::NullishTupleField(kind, index))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupopt")?;
                            let (kind, index) = if let Some(index) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, index)
                            } else {
                                let (kind, index) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    index,
                                )
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::OptionalTupleField(kind, index))
                        })
                        .or_else(|| {
                            let (mode, encoded) = value
                                .strip_prefix("tupsetnull")
                                .map(|encoded| (2, encoded))
                                .or_else(|| {
                                    value.strip_prefix("tupsetopt").map(|encoded| (1, encoded))
                                })
                                .or_else(|| {
                                    value.strip_prefix("tupsetw").map(|encoded| (0, encoded))
                                })?;
                            let (kind, index) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "p" | "o" => 2,
                                "u" => 3,
                                _ => return None,
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::FixedWideTupleSet(kind, index, mode))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("tupset")?;
                            let (kind, index) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "p" | "o" => 2,
                                _ => return None,
                            };
                            index
                                .parse::<u16>()
                                .ok()
                                .map(|index| NumericValue::FixedTupleSet(kind, index))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objca")?;
                            let semantic = match encoded.as_bytes().first()? {
                                b'o' => 0,
                                b'l' => 1,
                                b'n' => 2,
                                _ => return None,
                            };
                            let operation = match encoded.as_bytes().get(1)? {
                                b'a' => 0,
                                b's' => 1,
                                b'm' => 2,
                                b'd' => 3,
                                b'r' => 4,
                                b'l' => 5,
                                b'h' => 6,
                                b'u' => 7,
                                b'o' => 8,
                                b'x' => 9,
                                b'b' => 10,
                                b'p' => 11,
                                _ => return None,
                            };
                            encoded.get(2..)?.parse::<u16>().ok().map(|offset| {
                                NumericValue::TaggedObjectNumberAssign(
                                    semantic * 12 + operation,
                                    offset,
                                )
                            })
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objup")?;
                            let semantic = match encoded.as_bytes().first()? {
                                b'o' => 0,
                                b'l' => 1,
                                b'n' => 2,
                                _ => return None,
                            };
                            let decrement = match encoded.as_bytes().get(1)? {
                                b'i' => 0,
                                b'd' => 2,
                                _ => return None,
                            };
                            let postfix = match encoded.as_bytes().get(2)? {
                                b'p' => 0,
                                b'o' => 1,
                                _ => return None,
                            };
                            encoded.get(3..)?.parse::<u16>().ok().map(|offset| {
                                NumericValue::TaggedObjectNumberUpdate(
                                    semantic * 4 + decrement + postfix,
                                    offset,
                                )
                            })
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objset")?;
                            let (kind, offset) = encoded.split_at(1);
                            let kind = match kind {
                                "n" => 0,
                                "b" => 1,
                                "s" | "a" | "o" => 2,
                                "u" => 3,
                                _ => return None,
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::FixedObjectSet(kind, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objnull")?;
                            let (kind, offset) = if let Some(offset) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::NullishObjectField(kind, offset))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("objnullablen")?
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::OptionalObjectField(0, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("objopt")?;
                            let (kind, offset) = if let Some(offset) = encoded
                                .strip_prefix("rn")
                                .or_else(|| encoded.strip_prefix("rb"))
                                .or_else(|| encoded.strip_prefix("rs"))
                                .or_else(|| encoded.strip_prefix("dn"))
                                .or_else(|| encoded.strip_prefix("db"))
                                .or_else(|| encoded.strip_prefix("ds"))
                            {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "a" | "o" | "t" | "d" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::OptionalObjectField(kind, offset))
                        })
                        .or_else(|| {
                            let encoded = value.strip_prefix("obj")?;
                            let (kind, offset) = if let Some(offset) = encoded.strip_prefix("rn") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("rb") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("rs") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("dn") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("db") {
                                (2, offset)
                            } else if let Some(offset) = encoded.strip_prefix("ds") {
                                (2, offset)
                            } else {
                                let (kind, offset) = encoded.split_at(1);
                                (
                                    match kind {
                                        "n" => 0,
                                        "b" => 1,
                                        "s" | "o" | "t" => 2,
                                        _ => return None,
                                    },
                                    offset,
                                )
                            };
                            offset
                                .parse::<u16>()
                                .ok()
                                .map(|offset| NumericValue::ObjectField(kind, offset))
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('a')
                                .or_else(|| value.strip_prefix('b'))
                                .or_else(|| value.strip_prefix('s'))
                                .or_else(|| value.strip_prefix("rn"))
                                .or_else(|| value.strip_prefix("rb"))
                                .or_else(|| value.strip_prefix("rs"))
                                .or_else(|| value.strip_prefix("dn"))
                                .or_else(|| value.strip_prefix("db"))
                                .or_else(|| value.strip_prefix("ds"))
                                .or_else(|| value.strip_prefix("en"))
                                .or_else(|| value.strip_prefix("eb"))
                                .or_else(|| value.strip_prefix("es"))
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 16)
                                .map(NumericValue::Argument)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("setl")
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 8)
                                .map(NumericValue::LocalSet)
                        })
                        .or_else(|| {
                            ["dnput", "dbput", "dsput"].iter().enumerate().find_map(
                                |(kind, prefix)| {
                                    value.strip_prefix(prefix).and_then(|key| {
                                        intern_string(key).map(|key| {
                                            NumericValue::DictionaryStaticAppend(kind as u8, key)
                                        })
                                    })
                                },
                            )
                        })
                        .or_else(|| {
                            let (kind, index) =
                                ["rnlset", "rblset", "rslset", "dnlset", "dblset", "dslset"]
                                    .iter()
                                    .enumerate()
                                    .find_map(|(kind, prefix)| {
                                        value.strip_prefix(prefix).map(|index| (kind as u8, index))
                                    })?;
                            index
                                .parse::<u8>()
                                .ok()
                                .filter(|index| *index < 8)
                                .map(|index| NumericValue::AggregateLocalSet(kind, index))
                        })
                        .or_else(|| {
                            let (kind, index, unshift) = ["rn", "rb", "rs"]
                                .iter()
                                .enumerate()
                                .find_map(|(kind, prefix)| {
                                    value
                                        .strip_prefix(&format!("{prefix}lpush"))
                                        .map(|index| (kind as u8, index, false))
                                        .or_else(|| {
                                            value
                                                .strip_prefix(&format!("{prefix}lunshift"))
                                                .map(|index| (kind as u8, index, true))
                                        })
                                })?;
                            index
                                .parse::<u8>()
                                .ok()
                                .filter(|index| *index < 8)
                                .map(|index| {
                                    NumericValue::AggregateLocalArrayInsert(kind, index, unshift)
                                })
                        })
                        .or_else(|| {
                            value
                                .strip_prefix("rnl")
                                .or_else(|| value.strip_prefix("rbl"))
                                .or_else(|| value.strip_prefix("rsl"))
                                .or_else(|| value.strip_prefix("dnl"))
                                .or_else(|| value.strip_prefix("dbl"))
                                .or_else(|| value.strip_prefix("dsl"))
                                .or_else(|| value.strip_prefix("ln"))
                                .or_else(|| value.strip_prefix("lb"))
                                .or_else(|| value.strip_prefix("ls"))
                                .or_else(|| value.strip_prefix("ld"))
                                .or_else(|| value.strip_prefix('l'))
                                .and_then(|index| index.parse::<u8>().ok())
                                .filter(|index| *index < 8)
                                .map(NumericValue::LocalGet)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('t')
                                .and_then(intern_string)
                                .map(NumericValue::StringConstant)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('c')
                                .filter(|bits| bits.len() == 16)
                                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                                .map(|bits| NumericValue::Constant(f64::from_bits(bits)))
                        }),
                })
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty() && values.len() <= 256).then_some(Self(values))
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
        let mut branches = Vec::new();
        let mut loops = Vec::new();
        let mut guards = Vec::new();
        let mut switches = Vec::new();
        let mut tries = Vec::new();
        let mut catches = Vec::new();
        let mut results = Vec::new();
        for value in &self.0 {
            match value {
                NumericValue::Argument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    depth += 1;
                }
                NumericValue::DynamicArgument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        0x10,
                        0x47 | ((depth + 1) << 3),
                        (index + 1) * 8,
                    ]);
                    emit_binary_call(&mut code, dynamic_from_parts as *const () as u64, depth);
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
                NumericValue::StringConstant(value) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*value as usize as u64).to_le_bytes());
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
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        operation.function() as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::BitNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, bit_not as *const () as u64, depth - 1);
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
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, fmod as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Minimum | NumericValue::Maximum => {
                    if depth < 2 {
                        return None;
                    }
                    let function = if matches!(value, NumericValue::Minimum) {
                        minimum
                    } else {
                        maximum
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Power => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, power as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Atan2 | NumericValue::Hypot | NumericValue::Imul => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::Atan2 => atan2_number,
                        NumericValue::Hypot => hypot_number,
                        NumericValue::Imul => imul_number,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::IsFinite
                | NumericValue::IsInteger
                | NumericValue::IsNaN
                | NumericValue::IsSafeInteger => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::IsFinite => number_is_finite,
                        NumericValue::IsInteger => number_is_integer,
                        NumericValue::IsNaN => number_is_nan,
                        NumericValue::IsSafeInteger => number_is_safe_integer,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringCompare => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_compare as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberSameValue
                | NumericValue::StringSameValue
                | NumericValue::ReferenceSameValue => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberSameValue => number_same_value,
                        NumericValue::StringSameValue => string_same_value,
                        NumericValue::ReferenceSameValue => reference_same_value,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TypeOfNumber
                | NumericValue::TypeOfBoolean
                | NumericValue::TypeOfString
                | NumericValue::TypeOfObject
                | NumericValue::TypeOfDynamic => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::TypeOfNumber => type_of_number,
                        NumericValue::TypeOfBoolean => type_of_boolean,
                        NumericValue::TypeOfString => type_of_string,
                        NumericValue::TypeOfObject => type_of_object,
                        NumericValue::TypeOfDynamic => type_of_dynamic,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::DynamicToBoolean => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_to_boolean as *const () as u64, depth - 1);
                }
                NumericValue::DynamicAdd | NumericValue::DynamicCompare(_) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DynamicAdd => dynamic_add,
                        NumericValue::DynamicCompare(operation) => [
                            dynamic_less,
                            dynamic_less_equal,
                            dynamic_greater,
                            dynamic_greater_equal,
                            dynamic_equal,
                            dynamic_not_equal,
                            dynamic_strict_equal,
                            dynamic_strict_not_equal,
                        ][usize::from(*operation)],
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringCharAt
                | NumericValue::StringCharCodeAt
                | NumericValue::StringAt
                | NumericValue::StringCodePointAt => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringCharAt => string_char_at,
                        NumericValue::StringCharCodeAt => string_char_code_at,
                        NumericValue::StringAt => string_at,
                        NumericValue::StringCodePointAt => string_code_point_at,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberToString
                | NumericValue::BooleanToString
                | NumericValue::StringToNumber
                | NumericValue::DynamicToString
                | NumericValue::DynamicToNumber
                | NumericValue::TagNumber
                | NumericValue::TagString
                | NumericValue::TagBoolean
                | NumericValue::TagAggregate(_)
                | NumericValue::DynamicTag
                | NumericValue::UntagNumber
                | NumericValue::UntagString
                | NumericValue::UntagBoolean
                | NumericValue::UntagNumberArray
                | NumericValue::UntagBooleanArray
                | NumericValue::UntagStringArray
                | NumericValue::UntagArray
                | NumericValue::UntagNumberDictionary
                | NumericValue::UntagBooleanDictionary
                | NumericValue::UntagStringDictionary
                | NumericValue::UntagDictionary
                | NumericValue::UntagObject
                | NumericValue::UntagTuple
                | NumericValue::ParseFloat
                | NumericValue::NumberToExponentialShortest => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberToString => number_to_string,
                        NumericValue::BooleanToString => boolean_to_string,
                        NumericValue::StringToNumber => string_to_number,
                        NumericValue::DynamicToString => dynamic_to_string,
                        NumericValue::DynamicToNumber => dynamic_to_number,
                        NumericValue::TagNumber => tag_number,
                        NumericValue::TagString => tag_string,
                        NumericValue::TagBoolean => tag_boolean,
                        NumericValue::TagAggregate(kind) => [
                            tag_number_array,
                            tag_boolean_array,
                            tag_string_array,
                            tag_number_dictionary,
                            tag_boolean_dictionary,
                            tag_string_dictionary,
                            tag_object,
                            tag_tuple,
                        ][*kind as usize],
                        NumericValue::DynamicTag => dynamic_tag,
                        NumericValue::UntagNumber => untag_number,
                        NumericValue::UntagString => untag_string,
                        NumericValue::UntagBoolean => untag_boolean,
                        NumericValue::UntagNumberArray => untag_number_array,
                        NumericValue::UntagBooleanArray => untag_boolean_array,
                        NumericValue::UntagStringArray => untag_string_array,
                        NumericValue::UntagArray => untag_array,
                        NumericValue::UntagNumberDictionary => untag_number_dictionary,
                        NumericValue::UntagBooleanDictionary => untag_boolean_dictionary,
                        NumericValue::UntagStringDictionary => untag_string_dictionary,
                        NumericValue::UntagDictionary => untag_dictionary,
                        NumericValue::UntagObject => untag_object,
                        NumericValue::UntagTuple => untag_tuple,
                        NumericValue::ParseFloat => parse_float,
                        NumericValue::NumberToExponentialShortest => number_to_exponential_shortest,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::ObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_number_field,
                        object_boolean_field,
                        object_string_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_optional_number_field,
                        object_optional_boolean_field,
                        object_optional_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_nullish_number_field,
                        object_nullish_boolean_field,
                        object_nullish_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        optional_tuple_number_field,
                        optional_tuple_boolean_field,
                        optional_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        nullish_tuple_number_field,
                        nullish_tuple_boolean_field,
                        nullish_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::FixedObjectNew(size) => {
                    if depth == 8 {
                        return None;
                    }
                    let size = f64::from(*size);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&size.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_object_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedObjectSet(kind, offset) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_object_set_number,
                        fixed_object_set_boolean,
                        fixed_object_set_string,
                        fixed_object_set_byte,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TaggedObjectNumberUpdate(mode, offset) => {
                    if depth == 0 || depth > 6 {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_ternary_call(
                        &mut code,
                        tagged_object_number_update as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::TaggedObjectNumberAssign(mode, offset) => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_quaternary_call(
                        &mut code,
                        tagged_object_number_assign as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::FixedTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedWideTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_wide_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedTupleSet(kind, index) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_tuple_set_number,
                        fixed_tuple_set_boolean,
                        fixed_tuple_set_pointer,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::FixedWideTupleSet(kind, index, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let functions = if *mode == 1 {
                        [
                            fixed_wide_tuple_set_optional_number,
                            fixed_wide_tuple_set_optional_boolean,
                            fixed_wide_tuple_set_optional_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else if *mode == 2 {
                        [
                            fixed_wide_tuple_set_nullish_number,
                            fixed_wide_tuple_set_nullish_boolean,
                            fixed_wide_tuple_set_nullish_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else {
                        [
                            fixed_wide_tuple_set_number,
                            fixed_wide_tuple_set_boolean,
                            fixed_wide_tuple_set_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    };
                    emit_ternary_call(
                        &mut code,
                        functions[usize::from(*kind)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::ExcludeNumber
                | NumericValue::ExcludeString
                | NumericValue::ExcludeBoolean
                | NumericValue::ExcludeArray
                | NumericValue::ExcludeObject => {}
                NumericValue::GlobalGet => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, global_get as *const () as u64, depth - 1);
                }
                NumericValue::GlobalSet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_set as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::GlobalInit => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_init as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntryGet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, callable_entry_get as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntrySet => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, callable_entry_set as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::ParseInt
                | NumericValue::NumberToFixed
                | NumericValue::NumberToPrecision
                | NumericValue::NumberToRadixString
                | NumericValue::NumberToExponential => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::ParseInt => parse_int,
                        NumericValue::NumberToFixed => number_to_fixed,
                        NumericValue::NumberToPrecision => number_to_precision,
                        NumericValue::NumberToRadixString => number_to_radix_string,
                        NumericValue::NumberToExponential => number_to_exponential,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringStartsWith
                | NumericValue::StringEndsWith
                | NumericValue::StringIncludes
                | NumericValue::StringIndexOf
                | NumericValue::StringLastIndexOf
                | NumericValue::StringRepeat
                | NumericValue::StringNormalize
                | NumericValue::StringSlice
                | NumericValue::StringSubstring => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringStartsWith => string_starts_with,
                        NumericValue::StringEndsWith => string_ends_with,
                        NumericValue::StringIncludes => string_includes,
                        NumericValue::StringIndexOf => string_index_of,
                        NumericValue::StringLastIndexOf => string_last_index_of,
                        NumericValue::StringRepeat => string_repeat,
                        NumericValue::StringNormalize => string_normalize,
                        NumericValue::StringSlice => string_slice,
                        NumericValue::StringSubstring => string_substring,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringSliceRange
                | NumericValue::StringSubstringRange
                | NumericValue::StringPadStart
                | NumericValue::StringPadEnd
                | NumericValue::StringStartsWithAt
                | NumericValue::StringEndsWithAt
                | NumericValue::StringIncludesAt
                | NumericValue::StringIndexOfAt
                | NumericValue::StringLastIndexOfAt
                | NumericValue::StringReplace
                | NumericValue::StringReplaceAll => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringSliceRange => string_slice_range,
                        NumericValue::StringSubstringRange => string_substring_range,
                        NumericValue::StringPadStart => string_pad_start,
                        NumericValue::StringPadEnd => string_pad_end,
                        NumericValue::StringStartsWithAt => string_starts_with_at,
                        NumericValue::StringEndsWithAt => string_ends_with_at,
                        NumericValue::StringIncludesAt => string_includes_at,
                        NumericValue::StringIndexOfAt => string_index_of_at,
                        NumericValue::StringLastIndexOfAt => string_last_index_of_at,
                        NumericValue::StringReplace => string_replace_first,
                        NumericValue::StringReplaceAll => string_replace_all,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringSplit => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, string_split as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_to_array as *const () as u64, depth - 1);
                }
                NumericValue::StringFromCharCode | NumericValue::StringFromCodePoint => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringFromCharCode => string_from_char_code,
                        NumericValue::StringFromCodePoint => string_from_code_point,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMin | NumericValue::NumberArrayMax => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayMin => array_min,
                        NumericValue::NumberArrayMax => array_max,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayHypot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_hypot as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayReduce(operation, has_initial, from_right) => {
                    if depth < 2 {
                        return None;
                    }
                    type ReduceFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [ReduceFn; 4] = match operation {
                        NumericReduceOp::Add => [
                            number_array_reduce_add,
                            number_array_reduce_add_first,
                            number_array_reduce_add_right,
                            number_array_reduce_add_last,
                        ],
                        NumericReduceOp::Subtract => [
                            number_array_reduce_subtract,
                            number_array_reduce_subtract_first,
                            number_array_reduce_subtract_right,
                            number_array_reduce_subtract_last,
                        ],
                        NumericReduceOp::Multiply => [
                            number_array_reduce_multiply,
                            number_array_reduce_multiply_first,
                            number_array_reduce_multiply_right,
                            number_array_reduce_multiply_last,
                        ],
                        NumericReduceOp::Divide => [
                            number_array_reduce_divide,
                            number_array_reduce_divide_first,
                            number_array_reduce_divide_right,
                            number_array_reduce_divide_last,
                        ],
                        NumericReduceOp::Remainder => [
                            number_array_reduce_remainder,
                            number_array_reduce_remainder_first,
                            number_array_reduce_remainder_right,
                            number_array_reduce_remainder_last,
                        ],
                        NumericReduceOp::Power => [
                            number_array_reduce_power,
                            number_array_reduce_power_first,
                            number_array_reduce_power_right,
                            number_array_reduce_power_last,
                        ],
                        NumericReduceOp::Minimum => [
                            number_array_reduce_minimum,
                            number_array_reduce_minimum_first,
                            number_array_reduce_minimum_right,
                            number_array_reduce_minimum_last,
                        ],
                        NumericReduceOp::Maximum => [
                            number_array_reduce_maximum,
                            number_array_reduce_maximum_first,
                            number_array_reduce_maximum_right,
                            number_array_reduce_maximum_last,
                        ],
                    };
                    let function = functions[*from_right as usize * 2 + usize::from(!*has_initial)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial_captured,
                            (false, false) => number_array_jit_reduce_first_captured,
                            (true, true) => number_array_jit_reduce_right_initial_captured,
                            (false, true) => number_array_jit_reduce_right_last_captured,
                        };
                        emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial,
                            (false, false) => number_array_jit_reduce_first,
                            (true, true) => number_array_jit_reduce_right_initial,
                            (false, true) => number_array_jit_reduce_right_last,
                        };
                        emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayQuantifier(operation, every) => {
                    if depth < 2 {
                        return None;
                    }
                    type QuantifierFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [QuantifierFn; 2] = match operation {
                        CompareOp::Less => [number_array_some_lt, number_array_every_lt],
                        CompareOp::LessEqual => [number_array_some_lte, number_array_every_lte],
                        CompareOp::Greater => [number_array_some_gt, number_array_every_gt],
                        CompareOp::GreaterEqual => [number_array_some_gte, number_array_every_gte],
                        CompareOp::Equal => [number_array_some_eq, number_array_every_eq],
                        CompareOp::NotEqual => [number_array_some_ne, number_array_every_ne],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*every)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFind(operation, mode) => {
                    if depth < 2 {
                        return None;
                    }
                    type FindFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [FindFn; 4] = match operation {
                        CompareOp::Less => [
                            number_array_find_lt,
                            number_array_find_index_lt,
                            number_array_find_last_lt,
                            number_array_find_last_index_lt,
                        ],
                        CompareOp::LessEqual => [
                            number_array_find_lte,
                            number_array_find_index_lte,
                            number_array_find_last_lte,
                            number_array_find_last_index_lte,
                        ],
                        CompareOp::Greater => [
                            number_array_find_gt,
                            number_array_find_index_gt,
                            number_array_find_last_gt,
                            number_array_find_last_index_gt,
                        ],
                        CompareOp::GreaterEqual => [
                            number_array_find_gte,
                            number_array_find_index_gte,
                            number_array_find_last_gte,
                            number_array_find_last_index_gte,
                        ],
                        CompareOp::Equal => [
                            number_array_find_eq,
                            number_array_find_index_eq,
                            number_array_find_last_eq,
                            number_array_find_last_index_eq,
                        ],
                        CompareOp::NotEqual => [
                            number_array_find_ne,
                            number_array_find_index_ne,
                            number_array_find_last_ne,
                            number_array_find_last_index_ne,
                        ],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[*mode as usize] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFilter(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match operation {
                        CompareOp::Less => number_array_filter_lt,
                        CompareOp::LessEqual => number_array_filter_lte,
                        CompareOp::Greater => number_array_filter_gt,
                        CompareOp::GreaterEqual => number_array_filter_gte,
                        CompareOp::Equal => number_array_filter_eq,
                        CompareOp::NotEqual => number_array_filter_ne,
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayTruthy(kind, mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayTruthy(mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayCompare(kind, operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 64 + mode * 8 + *operation as u8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        primitive_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::DynamicArrayCompare(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(mode * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayMap(kind, operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayConvert(source, target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayConvert(target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayMapIdentity => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        dynamic_array_map_identity as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayJitMap(target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitScan(mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded_captured
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded_captured
                            } else if *from_right {
                                dynamic_array_jit_reduce_right_captured
                            } else {
                                dynamic_array_jit_reduce_left_captured
                            } as *const () as u64,
                            depth - 4,
                        );
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded
                            } else if *from_right {
                                dynamic_array_jit_reduce_right
                            } else {
                                dynamic_array_jit_reduce_left
                            } as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayMap(operation, reverse) => {
                    if depth < 2 {
                        return None;
                    }
                    let functions = match operation {
                        NumericReduceOp::Add => {
                            [number_array_map_add, number_array_map_add_reverse]
                        }
                        NumericReduceOp::Subtract => {
                            [number_array_map_subtract, number_array_map_subtract_reverse]
                        }
                        NumericReduceOp::Multiply => {
                            [number_array_map_multiply, number_array_map_multiply_reverse]
                        }
                        NumericReduceOp::Divide => {
                            [number_array_map_divide, number_array_map_divide_reverse]
                        }
                        NumericReduceOp::Remainder => [
                            number_array_map_remainder,
                            number_array_map_remainder_reverse,
                        ],
                        NumericReduceOp::Power => {
                            [number_array_map_power, number_array_map_power_reverse]
                        }
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*reverse)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayJitMap(source, target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::PrimitiveArrayJitScan(kind, mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::NumberArrayIndexMap(operation, reverse) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let operation = match operation {
                        NumericReduceOp::Add => 0,
                        NumericReduceOp::Subtract => 1,
                        NumericReduceOp::Multiply => 2,
                        NumericReduceOp::Divide => 3,
                        NumericReduceOp::Remainder => 4,
                        NumericReduceOp::Power => 5,
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    } + if *reverse { 8 } else { 0 };
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(operation).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_index_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::NumberArraySelectMap(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(*operation as u8 + mode * 8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_select_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayBranchMap(encoded) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*encoded).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_branch_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayUnaryMap(absolute) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *absolute {
                        number_array_map_absolute
                    } else {
                        number_array_map_negate
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMathMap(operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*operation as u8).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_map_math as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::ArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, array_slice as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::DynamicArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_slice as *const () as u64,
                        depth - 3,
                    );
                    depth -= 2;
                }
                NumericValue::ArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, array_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DynamicArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        dynamic_array_concat as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayAppend
                | NumericValue::StringArrayAppend
                | NumericValue::BoolArrayAppend
                | NumericValue::DynamicArrayAppend => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAppend => number_array_append,
                        NumericValue::StringArrayAppend => string_array_append,
                        NumericValue::BoolArrayAppend => bool_array_append,
                        NumericValue::DynamicArrayAppend => dynamic_array_append,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::ArrayToReversed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_to_reversed as *const () as u64, depth - 1);
                }
                NumericValue::ArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_reverse as *const () as u64, depth - 1);
                }
                NumericValue::DynamicArrayToReversed | NumericValue::DynamicArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayReverse {
                        dynamic_array_reverse_in_place
                    } else {
                        dynamic_array_to_reversed
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSorted
                | NumericValue::StringArrayToSorted
                | NumericValue::BoolArrayToSorted
                | NumericValue::DynamicArrayToSorted => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayToSorted => number_array_to_sorted,
                        NumericValue::StringArrayToSorted => string_array_to_sorted,
                        NumericValue::BoolArrayToSorted => bool_array_to_sorted,
                        NumericValue::DynamicArrayToSorted => dynamic_array_to_sorted,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArraySort
                | NumericValue::StringArraySort
                | NumericValue::BoolArraySort
                | NumericValue::DynamicArraySort => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySort => number_array_sort,
                        NumericValue::StringArraySort => string_array_sort,
                        NumericValue::BoolArraySort => bool_array_sort,
                        NumericValue::DynamicArraySort => dynamic_array_sort_in_place,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSortedBy(descending)
                | NumericValue::NumberArraySortBy(descending) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match (value, descending) {
                        (NumericValue::NumberArrayToSortedBy(_), false) => {
                            number_array_to_sorted_ascending
                        }
                        (NumericValue::NumberArrayToSortedBy(_), true) => {
                            number_array_to_sorted_descending
                        }
                        (NumericValue::NumberArraySortBy(_), false) => number_array_sort_ascending,
                        (NumericValue::NumberArraySortBy(_), true) => number_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringArrayToSortedDescending
                | NumericValue::StringArraySortDescending => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringArrayToSortedDescending => {
                            string_array_to_sorted_descending
                        }
                        NumericValue::StringArraySortDescending => string_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayFill
                | NumericValue::StringArrayFill
                | NumericValue::BoolArrayFill
                | NumericValue::DynamicArrayFill => {
                    if depth < 4 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayFill => number_array_fill,
                        NumericValue::StringArrayFill => string_array_fill,
                        NumericValue::BoolArrayFill => bool_array_fill,
                        NumericValue::DynamicArrayFill => dynamic_array_fill,
                        _ => unreachable!(),
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayCopyWithin | NumericValue::DynamicArrayCopyWithin => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayCopyWithin {
                        dynamic_array_copy_within
                    } else {
                        array_copy_within
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArraySplice => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(&mut code, array_splice as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(
                        &mut code,
                        array_to_spliced as *const () as u64,
                        depth - 4,
                    );
                    depth -= 3;
                }
                NumericValue::DynamicArraySplice | NumericValue::DynamicArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayToSpliced {
                        dynamic_array_to_spliced
                    } else {
                        dynamic_array_splice_in_place
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::NumberArrayPush
                | NumericValue::StringArrayPush
                | NumericValue::BoolArrayPush
                | NumericValue::DynamicArrayPush => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPush => number_array_push,
                        NumericValue::StringArrayPush => string_array_push,
                        NumericValue::BoolArrayPush => bool_array_push,
                        NumericValue::DynamicArrayPush => dynamic_array_push,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayUnshift
                | NumericValue::StringArrayUnshift
                | NumericValue::BoolArrayUnshift
                | NumericValue::DynamicArrayUnshift => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayUnshift => number_array_unshift,
                        NumericValue::StringArrayUnshift => string_array_unshift,
                        NumericValue::BoolArrayUnshift => bool_array_unshift,
                        NumericValue::DynamicArrayUnshift => dynamic_array_unshift,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArraySet
                | NumericValue::StringArraySet
                | NumericValue::BoolArraySet
                | NumericValue::DynamicArraySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySet => number_array_set,
                        NumericValue::StringArraySet => string_array_set,
                        NumericValue::BoolArraySet => bool_array_set,
                        NumericValue::DynamicArraySet => dynamic_array_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::AggregateLocalSet(kind, index) => {
                    if depth < 2 || *index >= depth - 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_array_set,
                        1 => bool_array_set,
                        2 => string_array_set,
                        3 => number_dictionary_set,
                        4 => bool_dictionary_set,
                        5 => string_dictionary_set,
                        _ => return None,
                    };
                    let destination = depth - 2;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    emit_move(&mut code, 2, destination + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                    depth -= 1;
                }
                NumericValue::AggregateLocalArrayInsert(kind, index, unshift) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    let function = match (kind, unshift) {
                        (0, false) => number_array_push,
                        (1, false) => bool_array_push,
                        (2, false) => string_array_push,
                        (0, true) => number_array_unshift,
                        (1, true) => bool_array_unshift,
                        (2, true) => string_array_unshift,
                        _ => return None,
                    };
                    let destination = depth - 1;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                }
                NumericValue::NumberArrayPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(number_array_set as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::ArrayValue => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_value as *const () as u64, depth - 1);
                }
                NumericValue::MutableArrayHandle => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        mutable_array_handle as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::EmptyArray => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_array as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayPop
                | NumericValue::StringArrayPop
                | NumericValue::BoolArrayPop
                | NumericValue::DynamicArrayPop
                | NumericValue::NumberArrayShift
                | NumericValue::StringArrayShift
                | NumericValue::BoolArrayShift
                | NumericValue::DynamicArrayShift => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPop => number_array_pop,
                        NumericValue::StringArrayPop => string_array_pop,
                        NumericValue::BoolArrayPop => bool_array_pop,
                        NumericValue::DynamicArrayPop => dynamic_array_pop,
                        NumericValue::NumberArrayShift => number_array_shift,
                        NumericValue::StringArrayShift => string_array_shift,
                        NumericValue::BoolArrayShift => bool_array_shift,
                        NumericValue::DynamicArrayShift => dynamic_array_shift,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::Drop => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                NumericValue::DropUnder => {
                    if depth < 2 {
                        return None;
                    }
                    emit_move(&mut code, depth - 2, depth - 1);
                    depth -= 1;
                }
                NumericValue::Duplicate => {
                    if !(1..=7).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 1);
                    depth += 1;
                }
                NumericValue::DuplicatePair => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 2);
                    emit_move(&mut code, depth + 1, depth - 1);
                    depth += 2;
                }
                NumericValue::LocalGet(index) => {
                    if *index >= depth || depth == 8 {
                        return None;
                    }
                    emit_move(&mut code, depth, *index);
                    depth += 1;
                }
                NumericValue::LocalSet(index) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    emit_move(&mut code, *index, depth - 1);
                    depth -= 1;
                }
                NumericValue::LoopStart => loops.push(LoopPatch {
                    start: code.len(),
                    continue_target: None,
                    base_depth: depth,
                    condition_exits: Vec::new(),
                    continues: Vec::new(),
                    breaks: Vec::new(),
                }),
                NumericValue::LoopWhile => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    loop_patch
                        .condition_exits
                        .extend([parity, emit_near_jump(&mut code, 0x84)]);
                    depth -= 1;
                }
                NumericValue::LoopContinuePoint => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth || loop_patch.continue_target.is_some() {
                        return None;
                    }
                    for jump in loop_patch.continues.drain(..) {
                        patch_near_jump(&mut code, jump)?;
                    }
                    loop_patch.continue_target = Some(code.len());
                }
                NumericValue::LoopBreak => {
                    let loop_patch = loops.last_mut()?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    loop_patch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::LoopContinue => {
                    let loop_patch = loops.last_mut()?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    if let Some(target) = loop_patch.continue_target {
                        emit_backward_jump(&mut code, target)?;
                    } else {
                        loop_patch
                            .continues
                            .push(emit_unconditional_jump(&mut code));
                    }
                }
                NumericValue::LoopEnd => {
                    let loop_patch = loops.pop()?;
                    if depth != loop_patch.base_depth
                        || loop_patch.continue_target.is_none()
                        || !loop_patch.continues.is_empty()
                    {
                        return None;
                    }
                    emit_backward_jump(&mut code, loop_patch.start)?;
                    for exit in loop_patch
                        .condition_exits
                        .into_iter()
                        .chain(loop_patch.breaks)
                    {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::GuardStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    guards.push((depth, vec![parity, zero], Vec::new(), false));
                }
                NumericValue::GuardAlternate => {
                    let (base_depth, false_exits, end_exits, has_alternate) = guards.last_mut()?;
                    if *has_alternate || depth != *base_depth {
                        return None;
                    }
                    end_exits.push(emit_unconditional_jump(&mut code));
                    for exit in false_exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    *has_alternate = true;
                }
                NumericValue::GuardEnd => {
                    let (base_depth, false_exits, end_exits, _) = guards.pop()?;
                    if depth != base_depth {
                        return None;
                    }
                    for exit in false_exits.into_iter().chain(end_exits) {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::SwitchStart => {
                    if depth == 0 {
                        return None;
                    }
                    switches.push(SwitchPatch {
                        base_depth: depth,
                        next_case: Vec::new(),
                        fallthrough: None,
                        breaks: Vec::new(),
                        has_case: false,
                        has_default: false,
                        default_body: None,
                    });
                }
                NumericValue::SwitchCaseStart => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    if switch.has_case {
                        switch.fallthrough = Some(emit_unconditional_jump(&mut code));
                    }
                    for next in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, next)?;
                    }
                    switch.has_case = true;
                }
                NumericValue::SwitchCaseBody => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let mismatch = emit_near_jump(&mut code, 0x84);
                    switch.next_case.extend([parity, mismatch]);
                    if let Some(fallthrough) = switch.fallthrough.take() {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    depth -= 1;
                }
                NumericValue::SwitchDefault => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth || switch.has_default {
                        return None;
                    }
                    let fallthrough = switch.has_case.then(|| emit_unconditional_jump(&mut code));
                    for mismatch in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, mismatch)?;
                    }
                    switch.next_case.push(emit_unconditional_jump(&mut code));
                    let body = code.len();
                    if let Some(fallthrough) = fallthrough {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    switch.has_default = true;
                    switch.default_body = Some(body);
                }
                NumericValue::SwitchBreak => {
                    let switch = switches.last_mut()?;
                    if depth < switch.base_depth {
                        return None;
                    }
                    switch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::SwitchEnd => {
                    let switch = switches.pop()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    let unmatched = switch.default_body.unwrap_or(code.len());
                    for exit in switch.next_case {
                        patch_jump_to(&mut code, exit, unmatched)?;
                    }
                    for exit in switch.breaks {
                        patch_near_jump(&mut code, exit)?;
                    }
                    depth -= 1;
                }
                NumericValue::TryStart | NumericValue::TaggedTryStart => tries.push(TryPatch {
                    base_depth: depth,
                    throws: Vec::new(),
                    tagged: matches!(value, NumericValue::TaggedTryStart),
                }),
                NumericValue::Throw => {
                    let exception = tries.last_mut()?;
                    if exception.tagged || depth <= exception.base_depth {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth, depth - 1);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::TaggedThrow(tag) => {
                    let exception = tries.last_mut()?;
                    if !exception.tagged
                        || depth <= exception.base_depth
                        || exception.base_depth >= 7
                    {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth + 1, depth - 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*tag).to_bits().to_le_bytes());
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x6e,
                        0xc0 | (exception.base_depth << 3),
                    ]);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::CheckError => {
                    let exception = tries.last_mut()?;
                    if exception.tagged && exception.base_depth >= 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_call_error as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0, 0x66, 0x48, 0x0f, 0x7e, 0xc0]);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0x85, 0xc0]);
                    let no_error = emit_near_jump(&mut code, 0x84);
                    let value_slot = exception.base_depth + u8::from(exception.tagged);
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (value_slot << 3)]);
                    if exception.tagged {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&2.0_f64.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | (exception.base_depth << 3),
                        ]);
                    }
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    patch_near_jump(&mut code, no_error)?;
                }
                NumericValue::UncaughtNumberThrow
                | NumericValue::UncaughtBooleanThrow
                | NumericValue::UncaughtStringThrow
                | NumericValue::UncaughtNumberArrayThrow
                | NumericValue::UncaughtBooleanArrayThrow
                | NumericValue::UncaughtStringArrayThrow
                | NumericValue::UncaughtDictionaryThrow => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::UncaughtNumberThrow => uncaught_number_throw,
                        NumericValue::UncaughtBooleanThrow => uncaught_boolean_throw,
                        NumericValue::UncaughtStringThrow => uncaught_string_throw,
                        NumericValue::UncaughtNumberArrayThrow => uncaught_number_array_throw,
                        NumericValue::UncaughtBooleanArrayThrow => uncaught_boolean_array_throw,
                        NumericValue::UncaughtStringArrayThrow => uncaught_string_array_throw,
                        NumericValue::UncaughtDictionaryThrow => uncaught_dictionary_throw,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::CatchStart => {
                    let exception = tries.pop()?;
                    if depth < exception.base_depth || exception.throws.is_empty() {
                        return None;
                    }
                    let result_depth = depth - exception.base_depth;
                    let normal_exit = emit_unconditional_jump(&mut code);
                    for jump in exception.throws {
                        patch_near_jump(&mut code, jump)?;
                    }
                    catches.push((exception.base_depth, normal_exit, result_depth));
                    depth = exception.base_depth + if exception.tagged { 2 } else { 1 };
                }
                NumericValue::TryEnd => {
                    let (base_depth, normal_exit, result_depth) = catches.pop()?;
                    if depth != base_depth + result_depth {
                        return None;
                    }
                    patch_near_jump(&mut code, normal_exit)?;
                }
                NumericValue::ResultStart => results.push(ResultPatch {
                    base_depth: depth,
                    result_depth: None,
                    exits: Vec::new(),
                }),
                NumericValue::ResultReturn(count) => {
                    let continuation_depth = loops
                        .last()
                        .map(|loop_patch| loop_patch.base_depth)
                        .into_iter()
                        .chain(guards.last().map(|guard| guard.0))
                        .max();
                    let result = results.last_mut()?;
                    if depth <= result.base_depth {
                        return None;
                    }
                    let result_depth = if *count == 0 {
                        depth - result.base_depth
                    } else {
                        *count
                    };
                    if depth < result.base_depth + result_depth {
                        return None;
                    }
                    if result
                        .result_depth
                        .replace(result_depth)
                        .is_some_and(|expected| expected != result_depth)
                    {
                        return None;
                    }
                    let source = depth - result_depth;
                    for offset in 0..result_depth {
                        emit_move(&mut code, result.base_depth + offset, source + offset);
                    }
                    result.exits.push(emit_unconditional_jump(&mut code));
                    depth = continuation_depth
                        .unwrap_or(result.base_depth)
                        .max(result.base_depth);
                }
                NumericValue::ResultEnd => {
                    let result = results.pop()?;
                    if depth != result.base_depth + result.result_depth? {
                        return None;
                    }
                    for exit in result.exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::EarlyReturn => {
                    let loop_patch = loops.last()?;
                    if depth <= loop_patch.base_depth {
                        return None;
                    }
                    emit_move(&mut code, 0, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::MathRandom
                | NumericValue::DateNow
                | NumericValue::PerformanceNow
                | NumericValue::ProcessPid
                | NumericValue::ProcessPpid
                | NumericValue::MissingCallable
                | NumericValue::Absent
                | NumericValue::Null
                | NumericValue::PreserveAbsent => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    let function = match value {
                        NumericValue::MathRandom => math_random,
                        NumericValue::DateNow => date_now,
                        NumericValue::PerformanceNow => performance_now,
                        NumericValue::ProcessPid => process_pid,
                        NumericValue::ProcessPpid => process_ppid,
                        NumericValue::MissingCallable => missing_callable,
                        NumericValue::Absent => absent_value,
                        NumericValue::Null => null_value,
                        NumericValue::PreserveAbsent => preserve_absent_value,
                        _ => unreachable!(),
                    };
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayWith
                | NumericValue::StringArrayWith
                | NumericValue::BoolArrayWith
                | NumericValue::DynamicArrayWith => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayWith => number_array_with,
                        NumericValue::StringArrayWith => string_array_with,
                        NumericValue::BoolArrayWith => bool_array_with,
                        NumericValue::DynamicArrayWith => dynamic_array_with,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToLowerCase
                | NumericValue::StringToUpperCase
                | NumericValue::StringTrim
                | NumericValue::StringTrimStart
                | NumericValue::StringTrimEnd => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringToLowerCase => string_to_lower_case,
                        NumericValue::StringToUpperCase => string_to_upper_case,
                        NumericValue::StringTrim => string_trim,
                        NumericValue::StringTrimStart => string_trim_start,
                        NumericValue::StringTrimEnd => string_trim_end,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_length as *const () as u64, depth - 1);
                }
                NumericValue::ArrayLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_length as *const () as u64, depth - 1);
                }
                NumericValue::IsArray | NumericValue::IsNotArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        if *value == NumericValue::IsArray {
                            is_array
                        } else {
                            is_not_array
                        } as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicIsArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_is_array as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayAt
                | NumericValue::BoolArrayAt
                | NumericValue::StringArrayAt
                | NumericValue::DynamicArrayAt
                | NumericValue::NumberArrayGet
                | NumericValue::BoolArrayGet
                | NumericValue::StringArrayGet => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAt => number_array_at,
                        NumericValue::BoolArrayAt => bool_array_at,
                        NumericValue::StringArrayAt => string_array_at,
                        NumericValue::DynamicArrayAt => dynamic_array_at,
                        NumericValue::NumberArrayGet => number_array_get,
                        NumericValue::BoolArrayGet => bool_array_get,
                        NumericValue::StringArrayGet => string_array_get,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryGet
                | NumericValue::BoolDictionaryGet
                | NumericValue::StringDictionaryGet
                | NumericValue::DictionaryDelete
                | NumericValue::DictionaryHasOwn
                | NumericValue::DictionaryIn
                | NumericValue::DictionaryAssign => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionaryGet => number_dictionary_get,
                        NumericValue::BoolDictionaryGet => bool_dictionary_get,
                        NumericValue::StringDictionaryGet => string_dictionary_get,
                        NumericValue::DictionaryDelete => dictionary_delete,
                        NumericValue::DictionaryHasOwn => dictionary_has_own,
                        NumericValue::DictionaryIn => dictionary_in,
                        NumericValue::DictionaryAssign => dictionary_assign,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryKeys
                | NumericValue::NumberDictionaryValues
                | NumericValue::BoolDictionaryValues
                | NumericValue::StringDictionaryValues
                | NumericValue::NumberDictionaryEntries
                | NumericValue::BoolDictionaryEntries
                | NumericValue::StringDictionaryEntries
                | NumericValue::DictionaryFromNumberEntries
                | NumericValue::DictionaryFromBoolEntries
                | NumericValue::DictionaryFromStringEntries => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DictionaryKeys => dictionary_keys,
                        NumericValue::NumberDictionaryValues => number_dictionary_values,
                        NumericValue::BoolDictionaryValues => bool_dictionary_values,
                        NumericValue::StringDictionaryValues => string_dictionary_values,
                        NumericValue::NumberDictionaryEntries => number_dictionary_entries,
                        NumericValue::BoolDictionaryEntries => bool_dictionary_entries,
                        NumericValue::StringDictionaryEntries => string_dictionary_entries,
                        NumericValue::DictionaryFromNumberEntries => dictionary_from_number_entries,
                        NumericValue::DictionaryFromBoolEntries => dictionary_from_bool_entries,
                        NumericValue::DictionaryFromStringEntries => dictionary_from_string_entries,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberDictionarySet
                | NumericValue::StringDictionarySet
                | NumericValue::BoolDictionarySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionarySet => number_dictionary_set,
                        NumericValue::StringDictionarySet => string_dictionary_set,
                        NumericValue::BoolDictionarySet => bool_dictionary_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::EmptyDictionary => {
                    if depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_dictionary as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::DictionaryLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dictionary_length as *const () as u64, depth - 1);
                }
                NumericValue::DictionaryKeyAt => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, dictionary_key_at as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryAppend(kind) => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 3;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 1, object + 1);
                    emit_move(&mut code, 2, object + 2);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 2;
                }
                NumericValue::DictionaryStaticAppend(kind, key) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 2;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 2, object + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*key as usize as u64).to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc8]);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &(number_dictionary_set as *const () as u64).to_le_bytes(),
                    );
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::NumberArrayJoin
                | NumericValue::BoolArrayJoin
                | NumericValue::StringArrayJoin
                | NumericValue::DynamicArrayJoin => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayJoin => number_array_join,
                        NumericValue::BoolArrayJoin => bool_array_join,
                        NumericValue::StringArrayJoin => string_array_join,
                        NumericValue::DynamicArrayJoin => dynamic_array_join,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayIncludes
                | NumericValue::BoolArrayIncludes
                | NumericValue::StringArrayIncludes
                | NumericValue::DynamicArrayIncludes
                | NumericValue::NumberArrayIndexOf
                | NumericValue::BoolArrayIndexOf
                | NumericValue::StringArrayIndexOf
                | NumericValue::DynamicArrayIndexOf
                | NumericValue::NumberArrayLastIndexOf
                | NumericValue::BoolArrayLastIndexOf
                | NumericValue::StringArrayLastIndexOf
                | NumericValue::DynamicArrayLastIndexOf => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayIncludes => number_array_includes,
                        NumericValue::BoolArrayIncludes => bool_array_includes,
                        NumericValue::StringArrayIncludes => string_array_includes,
                        NumericValue::DynamicArrayIncludes => dynamic_array_includes,
                        NumericValue::NumberArrayIndexOf => number_array_index_of,
                        NumericValue::BoolArrayIndexOf => bool_array_index_of,
                        NumericValue::StringArrayIndexOf => string_array_index_of,
                        NumericValue::DynamicArrayIndexOf => dynamic_array_index_of,
                        NumericValue::NumberArrayLastIndexOf => number_array_last_index_of,
                        NumericValue::BoolArrayLastIndexOf => bool_array_last_index_of,
                        NumericValue::StringArrayLastIndexOf => string_array_last_index_of,
                        NumericValue::DynamicArrayLastIndexOf => dynamic_array_last_index_of,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringTruthy => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_truthy as *const () as u64, depth - 1);
                }
                NumericValue::StringIsWellFormed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        string_is_well_formed as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::StringToWellFormed => {
                    if depth == 0 {
                        return None;
                    }
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
                        emit_unary_call(
                            &mut code,
                            operation.function() as *const () as u64,
                            depth - 1,
                        );
                    }
                }
                NumericValue::Select => {
                    if depth < 3 {
                        return None;
                    }
                    let condition = depth - 3;
                    let consequent = depth - 2;
                    let alternate = depth - 1;
                    // JavaScript ToBoolean treats NaN and both signed zeroes as false.
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                        0x7a,
                        0x13,
                    ]);
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
                NumericValue::ShortCircuit(and) => {
                    if depth < 2 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let truth = emit_near_jump(&mut code, if *and { 0x84 } else { 0x85 });
                    let mut exits = vec![truth];
                    if *and {
                        exits.push(parity);
                    } else {
                        patch_near_jump(&mut code, parity)?;
                    }
                    depth -= 2;
                    branches.push((depth, exits, false, Some(1)));
                }
                NumericValue::ConditionalStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    branches.push((depth, vec![parity, zero], true, None));
                }
                NumericValue::PresentConditionalStart => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_present as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (depth << 3) | depth]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (depth << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let absent = emit_near_jump(&mut code, 0x84);
                    branches.push((depth - 1, vec![parity, absent], true, None));
                }
                NumericValue::ConditionalAlternate => {
                    let (base_depth, exits, awaits_alternate, result_depth) =
                        branches.last_mut()?;
                    if !*awaits_alternate || depth <= *base_depth {
                        return None;
                    }
                    *result_depth = Some(depth - *base_depth);
                    let end = emit_unconditional_jump(&mut code);
                    for exit in exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    exits.push(end);
                    *awaits_alternate = false;
                    depth = *base_depth;
                }
                NumericValue::ShortCircuitEnd => {
                    let (base_depth, exits, awaits_alternate, result_depth) = branches.pop()?;
                    if awaits_alternate || depth != base_depth + result_depth? {
                        return None;
                    }
                    for exit in exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::AsBoolean => {
                    if depth == 0 {
                        return None;
                    }
                }
                NumericValue::BooleanNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, boolean_not as *const () as u64, depth - 1);
                }
                NumericValue::StrictMismatch(result) => {
                    if depth < 2 {
                        return None;
                    }
                    let destination = depth - 2;
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(u8::from(*result)).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (destination << 3)]);
                    depth -= 1;
                }
                NumericValue::Recur(arity) => {
                    if depth < *arity {
                        return None;
                    }
                    let base = depth - arity;
                    emit_recursive_call(&mut code, base, *arity)?;
                    depth = base + 1;
                }
            }
        }
        (depth == 1
            && branches.is_empty()
            && loops.is_empty()
            && guards.is_empty()
            && switches.is_empty()
            && results.is_empty())
        .then(|| {
            code.push(0xc3);
            code
        })
    }

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                NumericValue::DynamicArgument(index) => Some(*index as usize + 2),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct LoopPatch {
    start: usize,
    continue_target: Option<usize>,
    base_depth: u8,
    condition_exits: Vec<usize>,
    continues: Vec<usize>,
    breaks: Vec<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct SwitchPatch {
    base_depth: u8,
    next_case: Vec<usize>,
    fallthrough: Option<usize>,
    breaks: Vec<usize>,
    has_case: bool,
    has_default: bool,
    default_body: Option<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct TryPatch {
    base_depth: u8,
    throws: Vec<usize>,
    tagged: bool,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
struct ResultPatch {
    base_depth: u8,
    result_depth: Option<u8>,
    exits: Vec<usize>,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn take_call_error() -> f64 {
    let error = CALL_ERROR.with(|error| error.replace(ptr::null()));
    f64::from_bits(error as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_string_throw(value: f64) -> f64 {
    CALL_ERROR.with(|error| error.set(value.to_bits() as usize as *const c_char));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_number_throw(value: f64) -> f64 {
    let error = NUMBER_TO_STRING
        .with(Cell::get)
        .map(|format| unsafe { format(value) })
        .unwrap_or_else(|| INVALID_SYMBOL.as_ptr().cast());
    CALL_ERROR.with(|slot| slot.set(error));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_boolean_throw(value: f64) -> f64 {
    let error = if value == 0.0 {
        FALSE_THROW.as_ptr()
    } else {
        TRUE_THROW.as_ptr()
    };
    CALL_ERROR.with(|slot| slot.set(error.cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn uncaught_array_throw(value: f64, operation: u8) -> f64 {
    let error = array_format(
        operation,
        value,
        f64::from_bits(COMMA.as_ptr() as usize as u64),
    );
    if error != 0.0 {
        CALL_ERROR.with(|slot| slot.set(error.to_bits() as usize as *const c_char));
    }
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_number_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_string_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_boolean_array_throw(value: f64) -> f64 {
    uncaught_array_throw(value, 2)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn uncaught_dictionary_throw(_: f64) -> f64 {
    CALL_ERROR.with(|slot| slot.set(OBJECT_THROW.as_ptr().cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_move(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[0x66, 0x0f, 0x28, 0xc0 | (destination << 3) | source]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_recursive_call(code: &mut Vec<u8>, base: u8, arity: u8) -> Option<()> {
    const FRAME_SIZE: u32 = 136;
    code.extend_from_slice(&[0x48, 0x81, 0xec]);
    code.extend_from_slice(&FRAME_SIZE.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x89, 0xbc, 0x24, 0x80, 0, 0, 0]);
    for register in 0..base {
        code.extend_from_slice(&[
            0xf2,
            0x0f,
            0x11,
            0x44 | (register << 3),
            0x24,
            64 + register * 8,
        ]);
    }
    for index in 0..arity {
        let register = base + index;
        code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (register << 3), 0x24, index * 8]);
    }
    code.extend_from_slice(&[0x48, 0x89, 0xe7]);
    code.push(0xe8);
    let next = code.len().checked_add(4)?;
    code.extend_from_slice(&i32::try_from(next).ok()?.wrapping_neg().to_le_bytes());
    emit_move(code, base, 0);
    for register in 0..base {
        code.extend_from_slice(&[
            0xf2,
            0x0f,
            0x10,
            0x44 | (register << 3),
            0x24,
            64 + register * 8,
        ]);
    }
    code.extend_from_slice(&[0x48, 0x8b, 0xbc, 0x24, 0x80, 0, 0, 0]);
    code.extend_from_slice(&[0x48, 0x81, 0xc4]);
    code.extend_from_slice(&FRAME_SIZE.to_le_bytes());
    Some(())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_near_jump(code: &mut Vec<u8>, condition: u8) -> usize {
    code.extend_from_slice(&[0x0f, condition]);
    let displacement = code.len();
    code.extend_from_slice(&0i32.to_le_bytes());
    displacement
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unconditional_jump(code: &mut Vec<u8>) -> usize {
    code.push(0xe9);
    let displacement = code.len();
    code.extend_from_slice(&0i32.to_le_bytes());
    displacement
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_backward_jump(code: &mut Vec<u8>, target: usize) -> Option<()> {
    code.push(0xe9);
    let next = code.len().checked_add(4)?;
    let distance = i32::try_from(target as isize - next as isize).ok()?;
    code.extend_from_slice(&distance.to_le_bytes());
    Some(())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn patch_near_jump(code: &mut [u8], displacement: usize) -> Option<()> {
    patch_jump_to(code, displacement, code.len())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn patch_jump_to(code: &mut [u8], displacement: usize, target: usize) -> Option<()> {
    let next = displacement.checked_add(4)?;
    let distance = i32::try_from(target as isize - next as isize).ok()?;
    code[displacement..displacement + 4].copy_from_slice(&distance.to_le_bytes());
    Some(())
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
fn emit_spill(code: &mut Vec<u8>, registers: u8) {
    code.extend_from_slice(&[0x48, 0x83, 0xec, 0x48]);
    // The generated function keeps its argument-array base in caller-saved
    // RDI, so every native helper call must preserve it as well as live XMMs.
    code.extend_from_slice(&[0x48, 0x89, 0x7c, 0x24, 0x40]);
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (register << 3), 0x24, register * 8]);
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_restore(code: &mut Vec<u8>, registers: u8) {
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x44 | (register << 3), 0x24, register * 8]);
    }
    code.extend_from_slice(&[0x48, 0x8b, 0x7c, 0x24, 0x40]);
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x48]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unary_call(code: &mut Vec<u8>, function: u64, value: u8) {
    emit_spill(code, value);
    emit_move(code, 0, value);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, value, 0);
    emit_restore(code, value);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_binary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_ternary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    emit_move(code, 2, left + 2);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_quaternary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    emit_move(code, 2, left + 2);
    emit_move(code, 3, left + 3);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
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

struct Code {
    memory: *mut libc::c_void,
    globals: *const JitGlobals,
}

unsafe impl Send for Code {}
unsafe impl Sync for Code {}

impl Drop for Code {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.memory, page_size()) };
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

fn globals_cache() -> &'static Mutex<HashMap<String, Box<JitGlobals>>> {
    static GLOBALS: OnceLock<Mutex<HashMap<String, Box<JitGlobals>>>> = OnceLock::new();
    GLOBALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn global_namespace(symbol: &str) -> &str {
    if let Some((qualified, _)) = symbol.rsplit_once("::") {
        return qualified
            .rsplit_once(':')
            .map_or(qualified, |(_, module)| module);
    }
    let label = symbol.rsplit_once(':').map_or(symbol, |(_, label)| label);
    label
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile(
    symbol: &str,
    program: &NumericProgram,
) -> Result<(*mut libc::c_void, *const JitGlobals), *const c_char> {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(code) = cache.get(symbol) {
        return Ok((code.memory, code.globals));
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
    let globals_ptr = {
        let mut globals = globals_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        globals
            .entry(global_namespace(symbol).to_owned())
            .or_insert_with(|| {
                Box::new(JitGlobals {
                    slots: std::array::from_fn(|_| OnceLock::new()),
                    callable_entries: Mutex::new(HashMap::new()),
                })
            })
            .as_ref() as *const JitGlobals
    };
    cache.insert(
        symbol.to_owned(),
        Code {
            memory,
            globals: globals_ptr,
        },
    );
    Ok((memory, globals_ptr))
}

#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
fn compile(
    _symbol: &str,
    _program: &NumericProgram,
) -> Result<(*mut libc::c_void, *const JitGlobals), *const c_char> {
    Err(UNSUPPORTED_TARGET.as_ptr().cast())
}

/// Compiles a validated numeric expression on first use and executes it.
///
/// # Safety
///
/// `symbol` must point to a live NUL-terminated string for this call. When
/// `arg_count` is nonzero, `args` must reference at least that many `f64`s.
/// `arena_alloc`, when supplied for a string-returning program, must return a
/// writable allocation of the requested size and alignment. `number_to_string`
/// must return an arena-backed NUL-terminated string when numeric coercion is
/// used. String parser callbacks must accept live NUL-terminated strings, and
/// zero-argument number callbacks, when supplied, must be safe to call for the
/// duration of this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_jit_call_f64(
    symbol: *const c_char,
    args: *const f64,
    arg_count: usize,
    arena_alloc: Option<ArenaAlloc>,
    number_to_string: Option<NumberToString>,
    string_to_number: Option<StringToNumber>,
    parse_float: Option<ParseFloat>,
    parse_int: Option<ParseInt>,
    number_format: Option<NumberFormat>,
    array_search: Option<ArraySearch>,
    array_format: Option<ArrayFormat>,
    string_normalize: Option<StringNormalize>,
    string_split: Option<StringSplit>,
    array_slice: Option<ArraySlice>,
    array_concat: Option<ArrayConcat>,
    array_append: Option<ArrayAppend>,
    array_to_reversed: Option<ArrayToReversed>,
    array_to_sorted: Option<ArrayToSorted>,
    array_reverse: Option<ArrayReverse>,
    array_sort: Option<ArraySort>,
    array_fill: Option<ArrayFill>,
    array_copy_within: Option<ArrayCopyWithin>,
    array_push: Option<ArrayPush>,
    array_unshift: Option<ArrayPush>,
    array_remove: Option<ArrayRemove>,
    array_splice: Option<ArraySplice>,
    array_set: Option<ArraySet>,
    array_with: Option<ArrayWith>,
    math_random: Option<NumberSource>,
    date_now: Option<NumberSource>,
    performance_now: Option<NumberSource>,
    process_pid: Option<NumberSource>,
    process_ppid: Option<NumberSource>,
    string_to_array: Option<StringToArray>,
    string_from_char_code: Option<NumberToString>,
    string_from_code_point: Option<NumberToString>,
    dictionary_get: Option<DictionaryGet>,
    dictionary_mutate: Option<DictionaryMutate>,
    dictionary_query: Option<DictionaryQuery>,
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
    let (code, globals) = match compile(symbol, &program) {
        Ok(compiled) => compiled,
        Err(error) => return ThawJitResult { value: 0.0, error },
    };
    let function = std::mem::transmute::<*mut libc::c_void, extern "C" fn(*const f64) -> f64>(code);
    let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(arena_alloc));
    let previous_formatter = NUMBER_TO_STRING.with(|formatter| formatter.replace(number_to_string));
    let previous_parser = STRING_TO_NUMBER.with(|parser| parser.replace(string_to_number));
    let previous_parse_float = PARSE_FLOAT.with(|parser| parser.replace(parse_float));
    let previous_parse_int = PARSE_INT.with(|parser| parser.replace(parse_int));
    let previous_number_format = NUMBER_FORMAT.with(|format| format.replace(number_format));
    let previous_array_search = ARRAY_SEARCH.with(|search| search.replace(array_search));
    let previous_array_format = ARRAY_FORMAT.with(|format| format.replace(array_format));
    let previous_string_normalize =
        STRING_NORMALIZE.with(|normalize| normalize.replace(string_normalize));
    let previous_string_split = STRING_SPLIT.with(|split| split.replace(string_split));
    let previous_string_to_array = STRING_TO_ARRAY.with(|convert| convert.replace(string_to_array));
    let previous_string_from_char_code =
        STRING_FROM_CHAR_CODE.with(|convert| convert.replace(string_from_char_code));
    let previous_string_from_code_point =
        STRING_FROM_CODE_POINT.with(|convert| convert.replace(string_from_code_point));
    let previous_array_slice = ARRAY_SLICE.with(|slice| slice.replace(array_slice));
    let previous_array_concat = ARRAY_CONCAT.with(|concat| concat.replace(array_concat));
    let previous_array_append = ARRAY_APPEND.with(|append| append.replace(array_append));
    let previous_array_to_reversed =
        ARRAY_TO_REVERSED.with(|reverse| reverse.replace(array_to_reversed));
    let previous_array_to_sorted = ARRAY_TO_SORTED.with(|sort| sort.replace(array_to_sorted));
    let previous_array_reverse = ARRAY_REVERSE.with(|reverse| reverse.replace(array_reverse));
    let previous_array_sort = ARRAY_SORT.with(|sort| sort.replace(array_sort));
    let previous_array_fill = ARRAY_FILL.with(|fill| fill.replace(array_fill));
    let previous_array_copy_within = ARRAY_COPY_WITHIN.with(|copy| copy.replace(array_copy_within));
    let previous_array_push = ARRAY_PUSH.with(|push| push.replace(array_push));
    let previous_array_unshift = ARRAY_UNSHIFT.with(|unshift| unshift.replace(array_unshift));
    let previous_array_remove = ARRAY_REMOVE.with(|remove| remove.replace(array_remove));
    let previous_array_splice = ARRAY_SPLICE.with(|splice| splice.replace(array_splice));
    let previous_array_set = ARRAY_SET.with(|set| set.replace(array_set));
    let previous_array_with = ARRAY_WITH.with(|replace| replace.replace(array_with));
    let previous_math_random = MATH_RANDOM.with(|random| random.replace(math_random));
    let previous_date_now = DATE_NOW.with(|now| now.replace(date_now));
    let previous_performance_now = PERFORMANCE_NOW.with(|now| now.replace(performance_now));
    let previous_process_pid = PROCESS_PID.with(|pid| pid.replace(process_pid));
    let previous_process_ppid = PROCESS_PPID.with(|ppid| ppid.replace(process_ppid));
    let previous_dictionary_get = DICTIONARY_GET.with(|get| get.replace(dictionary_get));
    let previous_dictionary_mutate =
        DICTIONARY_MUTATE.with(|mutate| mutate.replace(dictionary_mutate));
    let previous_dictionary_query = DICTIONARY_QUERY.with(|query| query.replace(dictionary_query));
    let previous_error = CALL_ERROR.with(|error| error.replace(ptr::null()));
    let previous_present = CALL_PRESENT.with(|present| present.replace(true));
    let previous_absence = CALL_ABSENCE.with(|absence| absence.replace(1));
    let previous_globals = JIT_GLOBALS.with(|slot| slot.replace(globals));
    let previous_dynamic_values =
        DYNAMIC_VALUES.with(|values| std::mem::take(&mut *values.borrow_mut()));
    let mut value = function(args);
    if program.returns_tagged_array() && value.to_bits() & ARRAY_RESULT_TAG != 0 {
        value = f64::from_bits(value.to_bits() & !ARRAY_RESULT_TAG);
    }
    let error = CALL_ERROR.with(|error| error.replace(previous_error));
    let present = CALL_PRESENT.with(|state| state.replace(previous_present));
    let absence = CALL_ABSENCE.with(|state| state.replace(previous_absence));
    JIT_GLOBALS.with(|slot| slot.set(previous_globals));
    DYNAMIC_VALUES.with(|values| *values.borrow_mut() = previous_dynamic_values);
    ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
    NUMBER_TO_STRING.with(|formatter| formatter.set(previous_formatter));
    STRING_TO_NUMBER.with(|parser| parser.set(previous_parser));
    PARSE_FLOAT.with(|parser| parser.set(previous_parse_float));
    PARSE_INT.with(|parser| parser.set(previous_parse_int));
    NUMBER_FORMAT.with(|format| format.set(previous_number_format));
    ARRAY_SEARCH.with(|search| search.set(previous_array_search));
    ARRAY_FORMAT.with(|format| format.set(previous_array_format));
    STRING_NORMALIZE.with(|normalize| normalize.set(previous_string_normalize));
    STRING_SPLIT.with(|split| split.set(previous_string_split));
    STRING_TO_ARRAY.with(|convert| convert.set(previous_string_to_array));
    STRING_FROM_CHAR_CODE.with(|convert| convert.set(previous_string_from_char_code));
    STRING_FROM_CODE_POINT.with(|convert| convert.set(previous_string_from_code_point));
    ARRAY_SLICE.with(|slice| slice.set(previous_array_slice));
    ARRAY_CONCAT.with(|concat| concat.set(previous_array_concat));
    ARRAY_APPEND.with(|append| append.set(previous_array_append));
    ARRAY_TO_REVERSED.with(|reverse| reverse.set(previous_array_to_reversed));
    ARRAY_TO_SORTED.with(|sort| sort.set(previous_array_to_sorted));
    ARRAY_REVERSE.with(|reverse| reverse.set(previous_array_reverse));
    ARRAY_SORT.with(|sort| sort.set(previous_array_sort));
    ARRAY_FILL.with(|fill| fill.set(previous_array_fill));
    ARRAY_COPY_WITHIN.with(|copy| copy.set(previous_array_copy_within));
    ARRAY_PUSH.with(|push| push.set(previous_array_push));
    ARRAY_UNSHIFT.with(|unshift| unshift.set(previous_array_unshift));
    ARRAY_REMOVE.with(|remove| remove.set(previous_array_remove));
    ARRAY_SPLICE.with(|splice| splice.set(previous_array_splice));
    ARRAY_SET.with(|set| set.set(previous_array_set));
    ARRAY_WITH.with(|replace| replace.set(previous_array_with));
    MATH_RANDOM.with(|random| random.set(previous_math_random));
    DATE_NOW.with(|now| now.set(previous_date_now));
    PERFORMANCE_NOW.with(|now| now.set(previous_performance_now));
    PROCESS_PID.with(|pid| pid.set(previous_process_pid));
    PROCESS_PPID.with(|ppid| ppid.set(previous_process_ppid));
    DICTIONARY_GET.with(|get| get.set(previous_dictionary_get));
    DICTIONARY_MUTATE.with(|mutate| mutate.set(previous_dictionary_mutate));
    DICTIONARY_QUERY.with(|query| query.set(previous_dictionary_query));
    if !error.is_null() {
        return ThawJitResult { value: 0.0, error };
    }
    if !present {
        return ThawJitResult {
            value: 0.0,
            error: if absence == 2 {
                NULL_STATUS
            } else {
                ABSENT_STATUS
            },
        };
    }
    ThawJitResult {
        value,
        error: ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn call(symbol: &CString, args: &[f64]) -> ThawJitResult {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn format_number_string(_: f64) -> *const c_char {
            c"42".as_ptr()
        }
        unsafe extern "C" fn parse_string(_: *const c_char) -> f64 {
            42.0
        }
        unsafe extern "C" fn random() -> f64 {
            0.25
        }
        unsafe extern "C" fn wall_now() -> f64 {
            1_000.0
        }
        unsafe extern "C" fn monotonic_now() -> f64 {
            10.0
        }
        unsafe extern "C" fn pid() -> f64 {
            123.0
        }
        unsafe extern "C" fn ppid() -> f64 {
            12.0
        }
        unsafe extern "C" fn parse_float(value: *const c_char) -> f64 {
            match unsafe { CStr::from_ptr(value) }.to_bytes() {
                b"  -12.5px" => -12.5,
                _ => 42.0,
            }
        }
        unsafe extern "C" fn parse_int(value: *const c_char, radix: f64) -> f64 {
            match (unsafe { CStr::from_ptr(value) }.to_bytes(), radix) {
                (b"11", 2.0) => 3.0,
                _ => 42.0,
            }
        }
        unsafe extern "C" fn format_method(
            operation: u8,
            value: f64,
            argument: f64,
        ) -> *const c_char {
            let text = match (operation, value, argument) {
                (0, 12.5, 1.0) => "12.5",
                (1, 12.5, 3.0) => "12.5",
                (2, 255.0, 16.0) => "ff",
                (3, 12.5, 1.0) => "1.3e+1",
                (3, 12.5, -1.0) => "1.25e+1",
                _ => return ptr::null(),
            };
            let output = unsafe { libc::malloc(text.len() + 1).cast::<u8>() };
            unsafe {
                ptr::copy_nonoverlapping(text.as_ptr(), output, text.len());
                output.add(text.len()).write(0);
            }
            output.cast()
        }
        unsafe extern "C" fn search_array(
            operation: u8,
            array: *const u8,
            needle: f64,
            from_index: f64,
        ) -> f64 {
            assert!(!array.is_null());
            match (operation, needle, from_index) {
                (0, 20.0, 0.0) => 1.0,
                (1, 20.0, 0.0) => 1.0,
                (6, 20.0, f64::INFINITY) => 1.0,
                _ => -1.0,
            }
        }
        unsafe extern "C" fn format_array(
            operation: u8,
            array: *const u8,
            separator: *const c_char,
        ) -> *const c_char {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            match unsafe { CStr::from_ptr(separator) }.to_bytes() {
                b"|" => c"10|20|30".as_ptr(),
                b"," => c"10,20,30".as_ptr(),
                separator => panic!("unexpected separator {separator:?}"),
            }
        }
        unsafe extern "C" fn normalize_string(
            value: *const c_char,
            form: *const c_char,
        ) -> *const c_char {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), "é".as_bytes());
            match unsafe { CStr::from_ptr(form) }.to_bytes() {
                b"NFC" => c"é".as_ptr(),
                _ => ptr::null(),
            }
        }
        unsafe extern "C" fn split_string(
            value: *const c_char,
            separator: *const c_char,
            limit: f64,
        ) -> *mut u8 {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), b"a,b");
            assert_eq!(unsafe { CStr::from_ptr(separator) }.to_bytes(), b",");
            assert_eq!(limit, 1.0);
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<*const c_char>().write(c"a".as_ptr());
            }
            output
        }
        unsafe extern "C" fn convert_string(value: *const c_char) -> *mut u8 {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), "😀".as_bytes());
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<*const c_char>().write(c"😀".as_ptr());
            }
            output
        }
        unsafe extern "C" fn from_char_code(value: f64) -> *const c_char {
            if value == 65.0 {
                c"A".as_ptr()
            } else {
                ptr::null()
            }
        }
        unsafe extern "C" fn from_code_point(value: f64) -> *const c_char {
            if value == 0x1f600 as f64 {
                c"😀".as_ptr()
            } else {
                ptr::null()
            }
        }
        unsafe extern "C" fn slice_array(
            array: *const u8,
            element_width: usize,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            assert_eq!((start, end), (1.0, 2.0));
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<f64>().write(20.0);
            }
            output
        }
        unsafe extern "C" fn reverse_array(array: *const u8, element_width: usize) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [30.0_f64, 20.0, 10.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe extern "C" fn reverse_array_in_place(
            array: *mut u8,
            element_width: usize,
        ) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            array
        }
        unsafe extern "C" fn concatenate_arrays(
            left: *const u8,
            right: *const u8,
            element_width: usize,
        ) -> *mut u8 {
            assert!(!left.is_null() && !right.is_null());
            assert_eq!(element_width, 8);
            let left_len = unsafe { left.cast::<u64>().read() } as usize;
            let right_len = unsafe { right.cast::<u64>().read() } as usize;
            let output = unsafe { libc::malloc(8 + (left_len + right_len) * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write((left_len + right_len) as u64);
                std::ptr::copy_nonoverlapping(left.add(8), output.add(8), left_len * 8);
                std::ptr::copy_nonoverlapping(
                    right.add(8),
                    output.add(8 + left_len * 8),
                    right_len * 8,
                );
            }
            output
        }
        unsafe extern "C" fn append_array_value(
            operation: u8,
            array: *const u8,
            value: f64,
        ) -> *mut u8 {
            assert_eq!(operation, 0);
            let length = unsafe { array.cast::<u64>().read() } as usize;
            let output = unsafe { libc::malloc(8 + (length + 1) * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write((length + 1) as u64);
                std::ptr::copy_nonoverlapping(array.add(8), output.add(8), length * 8);
                output.add(8 + length * 8).cast::<f64>().write(value);
            }
            output
        }
        unsafe extern "C" fn sort_array(operation: u8, array: *const u8) -> *mut u8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [10.0_f64, 20.0, 30.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe extern "C" fn sort_array_in_place(operation: u8, array: *mut u8) -> *mut u8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn fill_array_in_place(
            operation: u8,
            array: *mut u8,
            value: f64,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert_eq!((operation, value, start, end), (0, 20.0, 1.0, 2.0));
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn copy_array_within(
            array: *mut u8,
            element_width: usize,
            target: f64,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert_eq!((element_width, target, start, end), (8, 0.0, 1.0, 2.0));
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn push_array_value(
            operation: u8,
            array: *mut *mut u8,
            value: f64,
        ) -> f64 {
            assert_eq!((operation, value), (0, 20.0));
            assert!(!array.is_null());
            4.0
        }
        unsafe extern "C" fn remove_array_value(
            operation: u8,
            array: *mut *mut u8,
            out_value: *mut f64,
        ) -> i8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null() && !out_value.is_null());
            unsafe { out_value.write(20.0) };
            1
        }
        unsafe extern "C" fn replace_array(
            operation: u8,
            array: *const u8,
            index: f64,
            value: f64,
        ) -> *mut u8 {
            assert_eq!((operation, index, value), (0, 1.0, 99.0));
            assert!(!array.is_null());
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [10.0_f64, 99.0, 30.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe {
            thaw_jit_call_f64(
                symbol.as_ptr(),
                args.as_ptr(),
                args.len(),
                Some(allocate),
                Some(format_number_string),
                Some(parse_string),
                Some(parse_float),
                Some(parse_int),
                Some(format_method),
                Some(search_array),
                Some(format_array),
                Some(normalize_string),
                Some(split_string),
                Some(slice_array),
                Some(concatenate_arrays),
                Some(append_array_value),
                Some(reverse_array),
                Some(sort_array),
                Some(reverse_array_in_place),
                Some(sort_array_in_place),
                Some(fill_array_in_place),
                Some(copy_array_within),
                Some(push_array_value),
                Some(push_array_value),
                Some(remove_array_value),
                None,
                None,
                Some(replace_array),
                Some(random),
                Some(wall_now),
                Some(monotonic_now),
                Some(pid),
                Some(ppid),
                Some(convert_string),
                Some(from_char_code),
                Some(from_code_point),
                None,
                None,
                None,
            )
        }
    }

    #[test]
    fn carries_tagged_number_and_string_primitives_in_one_jit_slot() {
        let boxed_number = CString::new("expr:a0,tagnum:boxed-number").unwrap();
        let boxed = call(&boxed_number, &[20.0]);
        assert!(boxed.error.is_null());
        let boxed = unsafe { &*(boxed.value.to_bits() as usize as *const DynamicPrimitive) };
        assert_eq!(
            (boxed.tag, f64::from_bits(boxed.payload)),
            (DYNAMIC_NUMBER_TAG, 20.0)
        );

        let number_tag = CString::new("expr:a0,tagnum,tagkind:number-tag").unwrap();
        let result = call(&number_tag, &[20.0]);
        assert!(
            result.error.is_null(),
            "{}",
            unsafe { CStr::from_ptr(result.error) }.to_string_lossy()
        );
        assert_eq!(result.value, DYNAMIC_NUMBER_TAG as f64);

        for (symbol, expected) in [
            ("expr:a0,tagnum,typeofdynamic:number-type", "number"),
            ("expr:t78,tagstr,typeofdynamic:string-type", "string"),
            ("expr:b0,tagbool,typeofdynamic:boolean-type", "boolean"),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[20.0]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }

        let number = CString::new("expr:a0,tagnum,untagnum:number").unwrap();
        let result = call(&number, &[20.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 20.0);

        let string = CString::new("expr:t68656c6c6f,tagstr,untagstr:string").unwrap();
        let result = call(&string, &[]);
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char).to_bytes() },
            b"hello"
        );
        let boolean = CString::new("expr:b0,tagbool,notnum,notstr,untagbool:boolean").unwrap();
        let result = call(&boolean, &[1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let dynamic_argument = CString::new("expr:u0nbs,untagbool:dynamic-argument").unwrap();
        let result = call(&dynamic_argument, &[3.0, f64::from_bits(1)]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
        for tag in DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_DICTIONARY_TAG {
            let result = call(
                &CString::new("expr:u0NBSD,typeofdynamic:aggregate-type").unwrap(),
                &[tag as f64, 0.0],
            );
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                "object"
            );
        }
        for (symbol, expected) in [
            (
                "expr:arrayempty,tagrn,tagkind:number-array-tag",
                DYNAMIC_NUMBER_ARRAY_TAG,
            ),
            (
                "expr:arrayempty,tagrb,tagkind:boolean-array-tag",
                DYNAMIC_BOOLEAN_ARRAY_TAG,
            ),
            (
                "expr:arrayempty,tagrs,tagkind:string-array-tag",
                DYNAMIC_STRING_ARRAY_TAG,
            ),
            (
                "expr:dnempty,tagdn,tagkind:number-dictionary-tag",
                DYNAMIC_NUMBER_DICTIONARY_TAG,
            ),
            (
                "expr:dbempty,tagdb,tagkind:boolean-dictionary-tag",
                DYNAMIC_BOOLEAN_DICTIONARY_TAG,
            ),
            (
                "expr:dsempty,tagds,tagkind:string-dictionary-tag",
                DYNAMIC_STRING_DICTIONARY_TAG,
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected as f64, "{symbol}");
        }

        for (symbol, expected) in [
            ("expr:c0000000000000000,tagnum,dynbool:false-number", 0.0),
            ("expr:c7ff8000000000000,tagnum,dynbool:nan", 0.0),
            ("expr:c3ff0000000000000,tagnum,dynbool:true-number", 1.0),
            ("expr:t,tagstr,dynbool:false-string", 0.0),
            ("expr:t78,tagstr,dynbool:true-string", 1.0),
            ("expr:c0000000000000000,tagbool,dynbool:false-boolean", 0.0),
            ("expr:c3ff0000000000000,tagbool,dynbool:true-boolean", 1.0),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected, "{symbol}");
        }

        let result = call(
            &CString::new("expr:t3432,tagstr,dynnum:dynamic-number").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        let result = call(
            &CString::new("expr:c4000000000000000,tagnum,dynstr:dynamic-string").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "42"
        );

        let result = call(
            &CString::new("expr:t61,tagstr,t62,tagstr,dynadd,untagstr:dynamic-string-add").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "ab"
        );
        for (symbol, expected) in [
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyneq:loose-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dynlte:less-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyngt:greater",
                0.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyngte:greater-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dynne:not-equal",
                0.0,
            ),
            (
                "expr:c4000000000000000,tagnum,t32,tagstr,dynseq:strict-equal",
                0.0,
            ),
            (
                "expr:c4000000000000000,tagnum,t32,tagstr,dynsne:strict-not-equal",
                1.0,
            ),
            ("expr:t61,tagstr,t62,tagstr,dynlt:string-less", 1.0),
            (
                "expr:c3ff0000000000000,tagbool,c3ff0000000000000,tagnum,dyneq:boolean-number",
                1.0,
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected, "{symbol}");
        }

        let mismatch = CString::new("expr:t68656c6c6f,tagstr,untagnum:mismatch").unwrap();
        assert!(!call(&mismatch, &[]).error.is_null());

        let malformed = CString::new("expr:a0,untagnum:malformed").unwrap();
        assert!(!call(&malformed, &[42.0]).error.is_null());
    }

    #[test]
    fn short_circuit_skips_unselected_native_calls() {
        let one = format!("c{:016x}", 1.0f64.to_bits());
        let zero = format!("c{:016x}", 0.0f64.to_bits());
        let invalid_digits = format!("c{:016x}", 101.0f64.to_bits());
        let selected = CString::new(format!(
            "expr:{one},dup,asbool,||,{one},{invalid_digits},tofixed,end:short-or"
        ))
        .unwrap();
        let result = call(&selected, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let rejected = CString::new(format!(
            "expr:{zero},dup,asbool,||,{one},{invalid_digits},tofixed,end:run-or"
        ))
        .unwrap();
        assert!(!call(&rejected, &[]).error.is_null());

        let consequent = CString::new(format!(
            "expr:{one},if,{one},else,{one},{invalid_digits},tofixed,end:conditional-true"
        ))
        .unwrap();
        let result = call(&consequent, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let alternate = CString::new(format!(
            "expr:{zero},if,{one},{invalid_digits},tofixed,else,{one},end:conditional-false"
        ))
        .unwrap();
        let result = call(&alternate, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let two = format!("c{:016x}", 2.0f64.to_bits());
        let three = format!("c{:016x}", 3.0f64.to_bits());
        let four = format!("c{:016x}", 4.0f64.to_bits());
        for (condition, expected) in [(&one, 3.0), (&zero, 7.0)] {
            let multiple = CString::new(format!(
                "expr:{condition},if,{one},{two},else,{three},{four},end,+:conditional-multiple"
            ))
            .unwrap();
            let result = call(&multiple, &[]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let five = format!("c{:016x}", 5.0f64.to_bits());
        let ten = format!("c{:016x}", 10.0f64.to_bits());
        let twenty = format!("c{:016x}", 20.0f64.to_bits());
        let thirty = format!("c{:016x}", 30.0f64.to_bits());
        let forty = format!("c{:016x}", 40.0f64.to_bits());
        let try_values = CString::new(format!(
            "expr:trystart,a0,asbool,if,{five},throw,{zero},else,{zero},end,drop,{zero},{ten},{twenty},catch,{thirty},{forty},tryend,+,nip:try-multiple"
        ))
        .unwrap();
        for (throws, expected) in [(0.0, 30.0), (1.0, 70.0)] {
            let result = call(&try_values, &[throws]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let early_values = CString::new(format!(
            "expr:resultstart,a0,asbool,guard,{ten},{twenty},resultreturn,guardend,{thirty},{forty},resultend,+:early-multiple"
        ))
        .unwrap();
        for (early, expected) in [(0.0, 70.0), (1.0, 30.0)] {
            let result = call(&early_values, &[early]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let nested_loop_values = CString::new(format!(
            "expr:{zero},resultstart,{zero},loop,ln1,{one},<,while,a0,asbool,guard,{ten},{twenty},resultreturn2,guardend,looptail,ln1,{one},+,setl1,loopend,drop,{thirty},{forty},resultend,+,nip:nested-loop-early-multiple"
        ))
        .unwrap();
        for (early, expected) in [(0.0, 70.0), (1.0, 30.0)] {
            let result = call(&nested_loop_values, &[early]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let optional_present = CString::new(format!(
            "expr:{one},if,{one},else,absentn,end:optional-present"
        ))
        .unwrap();
        assert!(call(&optional_present, &[]).error.is_null());
        let optional_absent = CString::new(format!(
            "expr:{zero},if,{one},else,absentn,end:optional-absent"
        ))
        .unwrap();
        assert_eq!(call(&optional_absent, &[]).error, ABSENT_STATUS);

        let present_coalesce = CString::new(format!(
            "expr:{one},ifpresent,else,{one},{invalid_digits},tofixed,end:present-coalesce"
        ))
        .unwrap();
        let result = call(&present_coalesce, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
        let absent_coalesce = CString::new(format!(
            "expr:absentn,ifpresent,else,{one},end:absent-coalesce"
        ))
        .unwrap();
        let result = call(&absent_coalesce, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
    }

    #[test]
    fn recursively_calls_the_compiled_expression() {
        let factorial = CString::new(
            "expr:a0,c3ff0000000000000,<=,if,c3ff0000000000000,else,a0,a0,c3ff0000000000000,-,recurn1,*,end:factorial",
        )
        .unwrap();
        let result = call(&factorial, &[6.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 720.0);

        let gcd =
            CString::new("expr:a1,c0000000000000000,==,if,a0,else,a1,a0,a1,%,recurn2,end:gcd")
                .unwrap();
        let result = call(&gcd, &[1071.0, 462.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 21.0);
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
        assert_eq!(call(&symbol, &[f64::NAN]).value, -2.0);

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

        for (operation, input, expected) in [
            ("acos", 1.0, 0.0),
            ("acosh", 1.0, 0.0),
            ("asin", 0.0, 0.0),
            ("asinh", 0.0, 0.0),
            ("atan", 0.0, 0.0),
            ("atanh", 0.0, 0.0),
            ("cbrt", 8.0, 2.0),
            ("cos", 0.0, 1.0),
            ("cosh", 0.0, 1.0),
            ("exp", 0.0, 1.0),
            ("expm1", 0.0, 0.0),
            ("log", 1.0, 0.0),
            ("log1p", 0.0, 0.0),
            ("log2", 8.0, 3.0),
            ("log10", 100.0, 2.0),
            ("sign", -8.0, -1.0),
            ("sin", 0.0, 0.0),
            ("sinh", 0.0, 0.0),
            ("tan", 0.0, 0.0),
            ("tanh", 0.0, 0.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[input]);
            assert!(result.error.is_null());
            assert!((result.value - expected).abs() < 1e-12);
        }
        let sign = CString::new("expr:a0,sign:sign_special").unwrap();
        assert_eq!(call(&sign, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        assert!(call(&sign, &[f64::NAN]).value.is_nan());
        let atan2 = CString::new("expr:a0,a1,atan2:atan2").unwrap();
        assert!((call(&atan2, &[1.0, 1.0]).value - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        let hypot = CString::new("expr:a0,a1,hypot:hypot").unwrap();
        assert_eq!(call(&hypot, &[3.0, 4.0]).value, 5.0);
        let nested_unary = CString::new("expr:a0,a1,sin,+:nested_unary").unwrap();
        assert!((call(&nested_unary, &[2.0, 1.0]).value - (2.0 + 1.0f64.sin())).abs() < 1e-12);
        let nested_binary = CString::new("expr:a0,a1,a2,hypot,+:nested_binary").unwrap();
        assert_eq!(call(&nested_binary, &[7.0, 3.0, 4.0]).value, 12.0);
        let clz32 = CString::new("expr:a0,clz32:clz32").unwrap();
        assert_eq!(call(&clz32, &[0.0]).value, 32.0);
        assert_eq!(call(&clz32, &[1.0]).value, 31.0);
        assert_eq!(call(&clz32, &[-1.0]).value, 0.0);
        let fround = CString::new("expr:a0,fround:fround").unwrap();
        assert_eq!(call(&fround, &[1.337]).value, 1.337f64 as f32 as f64);
        assert_eq!(call(&fround, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        let imul = CString::new("expr:a0,a1,imul:imul").unwrap();
        assert_eq!(call(&imul, &[4_294_967_295.0, 5.0]).value, -5.0);
        let text = CString::new("a😀").unwrap();
        let text_argument = f64::from_bits(text.as_ptr() as usize as u64);
        let string_length = CString::new("expr:s0,strlen:string_length").unwrap();
        assert_eq!(call(&string_length, &[text_argument]).value, 3.0);
        for (operation, index, expected) in [
            ("charcodeat", 0.0, 97.0),
            ("charcodeat", 1.0, 55_357.0),
            ("charcodeat", 2.0, 56_832.0),
            ("charcodeat", -1.0, f64::NAN),
        ] {
            let character = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let actual = call(&character, &[text_argument, index]).value;
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual, expected);
            }
        }
        for (index, expected) in [(0.0, "a"), (1.0, "�"), (-1.0, "")] {
            let character = CString::new("expr:s0,a1,charat:charat").unwrap();
            let result = call(&character, &[text_argument, index]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let at = CString::new("expr:s0,a1,at:at").unwrap();
        let result = call(&at, &[text_argument, -1.0]);
        assert!(result.error.is_null());
        let value = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(value) }.to_str().unwrap(), "�");
        unsafe { libc::free(value.cast()) };
        assert_eq!(call(&at, &[text_argument, 3.0]).error, ABSENT_STATUS);
        let code_point = CString::new("expr:s0,a1,codepointat:codepointat").unwrap();
        let result = call(&code_point, &[text_argument, 1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 128_512.0);
        assert_eq!(
            call(&code_point, &[text_argument, 3.0]).error,
            ABSENT_STATUS
        );
        let supplementary = CString::new("𐀀").unwrap();
        let bmp = CString::new("\u{e000}").unwrap();
        let string_less =
            CString::new("expr:s0,s1,strcmp,c0000000000000000,<:string_utf16_comparison").unwrap();
        assert_eq!(
            call(
                &string_less,
                &[
                    f64::from_bits(supplementary.as_ptr() as usize as u64),
                    f64::from_bits(bmp.as_ptr() as usize as u64),
                ],
            )
            .value,
            1.0
        );
        let value = CString::new("prefix").unwrap();
        for (operation, search, expected) in [
            ("startswith", "pre", 1.0),
            ("endswith", "fix", 1.0),
            ("includes", "ref", 1.0),
            ("includes", "xyz", 0.0),
        ] {
            let search = CString::new(search).unwrap();
            let predicate = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &predicate,
                    &[
                        f64::from_bits(value.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        let positioned = CString::new("😀abc😀").unwrap();
        for (operation, search, position, expected) in [
            ("startswith2", "abc", 2.0, 1.0),
            ("endswith2", "😀", 7.0, 1.0),
            ("includes2", "😀", 1.0, 1.0),
            ("indexof2", "😀", 1.0, 5.0),
            ("lastindexof2", "😀", 4.0, 0.0),
            ("indexof2", "", f64::INFINITY, 7.0),
        ] {
            let search = CString::new(search).unwrap();
            let search_at = CString::new(format!("expr:s0,s1,a2,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &search_at,
                    &[
                        f64::from_bits(positioned.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                        position,
                    ],
                )
                .value,
                expected
            );
        }
        let indexed = CString::new("😀a😀").unwrap();
        let emoji = CString::new("😀").unwrap();
        for (operation, expected) in [("indexof", 0.0), ("lastindexof", 3.0)] {
            let index = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &index,
                    &[
                        f64::from_bits(indexed.as_ptr() as usize as u64),
                        f64::from_bits(emoji.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        for (operation, expected) in [("tolowercase", "straße"), ("touppercase", "STRASSE")] {
            let input = CString::new("Straße").unwrap();
            let convert = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(&convert, &[f64::from_bits(input.as_ptr() as usize as u64)]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let well_formed = CString::new("expr:s0,iswellformed:iswellformed").unwrap();
        assert_eq!(call(&well_formed, &[text_argument]).value, 1.0);
        let preserve = CString::new("expr:s0,towellformed:towellformed").unwrap();
        assert_eq!(call(&preserve, &[text_argument]).value, text_argument);
        let whitespace = CString::new("\u{feff}  value\u{3000}").unwrap();
        for (operation, expected) in [
            ("trim", "value"),
            ("trimstart", "value\u{3000}"),
            ("trimend", "\u{feff}  value"),
        ] {
            let trim = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(
                &trim,
                &[f64::from_bits(whitespace.as_ptr() as usize as u64)],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let repeated = CString::new("ab").unwrap();
        let repeat = CString::new("expr:s0,a1,repeat:repeat").unwrap();
        let result = call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), 2.9],
        );
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "abab");
        unsafe { libc::free(result.cast()) };
        assert!(!call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), -1.0],
        )
        .error
        .is_null());
        let sliced = CString::new("😀abcd").unwrap();
        for (operation, start, expected) in [
            ("slice", -2.0, "cd"),
            ("slice", 2.0, "abcd"),
            ("substring", -2.0, "😀abcd"),
            ("substring", 4.9, "cd"),
        ] {
            let suffix = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let result = call(
                &suffix,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        for (operation, start, end, expected) in [
            ("slice2", -4.0, -1.0, "abc"),
            ("slice2", 5.0, 2.0, ""),
            ("substring2", 5.0, 2.0, "abc"),
            ("substring2", f64::NAN, 2.9, "😀"),
        ] {
            let range = CString::new(format!("expr:s0,a1,a2,{operation}:{operation}")).unwrap();
            let result = call(
                &range,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start, end],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let padded = CString::new("😀").unwrap();
        let padding = CString::new("ab").unwrap();
        for (operation, expected) in [("padstart", "a😀"), ("padend", "😀a")] {
            let pad = CString::new(format!("expr:s0,a1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &pad,
                &[
                    f64::from_bits(padded.as_ptr() as usize as u64),
                    3.0,
                    f64::from_bits(padding.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let replace_value = CString::new("aba").unwrap();
        for (operation, search, replacement, expected) in [
            ("replace", "a", "x", "xba"),
            ("replaceall", "a", "x", "xbx"),
            ("replaceall", "", "-", "-a-b-a-"),
        ] {
            let search = CString::new(search).unwrap();
            let replacement = CString::new(replacement).unwrap();
            let replace = CString::new(format!("expr:s0,s1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &replace,
                &[
                    f64::from_bits(replace_value.as_ptr() as usize as u64),
                    f64::from_bits(search.as_ptr() as usize as u64),
                    f64::from_bits(replacement.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let combined = CString::new("expr:s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,s0,t726566,includes,s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,?:combined_string_predicates").unwrap();
        assert_eq!(
            call(&combined, &[f64::from_bits(value.as_ptr() as usize as u64)]).value,
            1.0
        );
        let name = CString::new("世界").unwrap();
        let concatenate = CString::new("expr:t686920,s0,concat:concatenate").unwrap();
        let result = call(
            &concatenate,
            &[f64::from_bits(name.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "hi 世界"
        );
        unsafe { libc::free(result.cast()) };
        let constructors = CString::new(
            "expr:c4050400000000000,fromcharcode,c40ff600000000000,fromcodepoint,concat:constructors",
        )
        .unwrap();
        let result = call(&constructors, &[]);
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "A😀"
        );
        let invalid_code_point =
            CString::new("expr:c4131000000000000,fromcodepoint:invalid_code_point").unwrap();
        assert!(!call(&invalid_code_point, &[]).error.is_null());
        let argument = [f64::from_bits(name.as_ptr() as usize as u64)];
        let missing_allocator = unsafe {
            thaw_jit_call_f64(
                concatenate.as_ptr(),
                argument.as_ptr(),
                1,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        };
        assert!(!missing_allocator.error.is_null());

        let decomposed = CString::new("é").unwrap();
        let normalize = CString::new("expr:s0,t4e4643,normalize:normalize").unwrap();
        let result = call(
            &normalize,
            &[f64::from_bits(decomposed.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }.to_bytes(),
            "é".as_bytes()
        );
        let invalid = CString::new("expr:s0,t626f677573,normalize:normalize_invalid").unwrap();
        assert!(!call(
            &invalid,
            &[f64::from_bits(decomposed.as_ptr() as usize as u64)]
        )
        .error
        .is_null());
        let split_value = CString::new("a,b").unwrap();
        let split = CString::new("expr:s0,t2c,c3ff0000000000000,split:split").unwrap();
        let result = call(
            &split,
            &[f64::from_bits(split_value.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()) }.to_bytes(),
            b"a"
        );
        unsafe { libc::free(output.cast()) };
        let emoji = CString::new("😀").unwrap();
        let characters = CString::new("expr:s0,strarray:characters").unwrap();
        let result = call(
            &characters,
            &[f64::from_bits(emoji.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()) }.to_bytes(),
            "😀".as_bytes()
        );
        unsafe { libc::free(output.cast()) };
        let values = [
            3_u64,
            10.0f64.to_bits(),
            20.0f64.to_bits(),
            30.0f64.to_bits(),
        ];
        let data = values.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        for (operation, expected) in [("rnmin", 10.0), ("rnmax", 30.0)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:array-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        let hypot = CString::new("expr:rn0,rnhypot:array-hypot").unwrap();
        assert_eq!(
            call(&hypot, &[f64::from_bits(handle as usize as u64)]).value,
            10.0f64.hypot(20.0).hypot(30.0)
        );
        for (operation, expected, first_expected) in [
            ("add", 68.0, 60.0),
            ("sub", -52.0, -40.0),
            ("mul", 48_000.0, 6_000.0),
            ("div", 8.0 / 10.0 / 20.0 / 30.0, 10.0 / 20.0 / 30.0),
            ("rem", 8.0, 10.0),
            ("pow", f64::INFINITY, f64::INFINITY),
            ("min", 8.0, 10.0),
            ("max", 30.0, 30.0),
        ] {
            let reduce = CString::new(format!(
                "expr:rn0,c4020000000000000,rnreduce{operation}:array-reduce"
            ))
            .unwrap();
            assert_eq!(
                call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
            let reduce = CString::new(format!(
                "expr:rn0,c0000000000000000,rnreduce{operation}0:array-reduce-first"
            ))
            .unwrap();
            assert_eq!(
                call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                first_expected
            );
            let (right_expected, last_expected) = match operation {
                "add" => (68.0, 60.0),
                "sub" => (-52.0, 0.0),
                "mul" => (48_000.0, 6_000.0),
                "div" => (8.0 / 30.0 / 20.0 / 10.0, 30.0 / 20.0 / 10.0),
                "rem" => (8.0, 0.0),
                "pow" => (f64::INFINITY, power(power(30.0, 20.0), 10.0)),
                "min" => (8.0, 10.0),
                "max" => (30.0, 30.0),
                _ => unreachable!(),
            };
            for (suffix, expected) in [("", right_expected), ("0", last_expected)] {
                let initial = if suffix.is_empty() {
                    "c4020000000000000"
                } else {
                    "c0000000000000000"
                };
                let reduce = CString::new(format!(
                    "expr:rn0,{initial},rnreduceright{operation}{suffix}:array-reduce-right"
                ))
                .unwrap();
                assert_eq!(
                    call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                    expected
                );
            }
        }
        for (operation, operand, some, every) in [
            ("lt", 25.0, true, false),
            ("lte", 30.0, true, true),
            ("gt", 25.0, true, false),
            ("gte", 10.0, true, true),
            ("eq", 20.0, true, false),
            ("ne", 20.0, true, false),
            ("eq", f64::NAN, false, false),
            ("ne", f64::NAN, true, true),
        ] {
            for (quantifier, expected) in [("some", some), ("every", every)] {
                let symbol = CString::new(format!(
                    "expr:rn0,c{:016x},rn{quantifier}{operation}:array-{quantifier}",
                    operand.to_bits()
                ))
                .unwrap();
                assert_eq!(
                    call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                    f64::from(expected)
                );
            }
        }
        for (method, expected) in [
            ("find", 20.0),
            ("findindex", 1.0),
            ("findlast", 30.0),
            ("findlastindex", 2.0),
        ] {
            let symbol = CString::new(format!(
                "expr:rn0,c402e000000000000,rn{method}gt:array-{method}"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        for method in ["find", "findlast"] {
            let symbol = CString::new(format!(
                "expr:rn0,c4058c00000000000,rn{method}eq:array-{method}-missing"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
                ABSENT_STATUS
            );
        }
        for method in ["findindex", "findlastindex"] {
            let symbol = CString::new(format!(
                "expr:rn0,c4058c00000000000,rn{method}eq:array-{method}-missing"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                -1.0
            );
        }
        let filter = CString::new("expr:rn0,c402e000000000000,rnfiltergt:array-filter").unwrap();
        let result = call(&filter, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let truthy_values = [
            4_u64,
            0.0f64.to_bits(),
            f64::NAN.to_bits(),
            (-2.0f64).to_bits(),
            3.0f64.to_bits(),
        ];
        let truthy_data = truthy_values.as_ptr().cast::<u8>();
        let truthy_handle = &truthy_data as *const *const u8;
        for (method, expected) in [
            ("some", 1.0),
            ("every", 0.0),
            ("find", -2.0),
            ("findindex", 2.0),
            ("findlast", 3.0),
            ("findlastindex", 3.0),
        ] {
            let symbol =
                CString::new(format!("expr:rn0,rn{method}truthy:array-{method}-truthy")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(truthy_handle as usize as u64)]).value,
                expected,
                "{method}"
            );
        }
        let filter_truthy = CString::new("expr:rn0,rnfiltertruthy:array-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(truthy_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, -2.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 3.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let bool_values = [3_u64, 0, 1, 0];
        let bool_data = bool_values.as_ptr().cast::<u8>();
        let bool_handle = &bool_data as *const *const u8;
        let filter_truthy = CString::new("expr:rb0,rbfiltertruthy:bool-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(bool_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(8).read() }, 1);
        unsafe { libc::free(output.cast_mut().cast()) };
        let empty_string = CString::new("").unwrap();
        let nonempty_string = CString::new("x").unwrap();
        let string_values = [
            3_u64,
            empty_string.as_ptr() as usize as u64,
            nonempty_string.as_ptr() as usize as u64,
            empty_string.as_ptr() as usize as u64,
        ];
        let string_data = string_values.as_ptr().cast::<u8>();
        let string_handle = &string_data as *const *const u8;
        let filter_truthy = CString::new("expr:rs0,rsfiltertruthy:string-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { output.add(8).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let find_truthy = CString::new("expr:rs0,rsfindtruthy:string-find-truthy").unwrap();
        assert_eq!(
            call(
                &find_truthy,
                &[f64::from_bits(string_handle as usize as u64)]
            )
            .value
            .to_bits(),
            nonempty_string.as_ptr() as usize as u64
        );
        let bool_some = CString::new("expr:rb0,b1,rbsomeeq:bool-some-equal").unwrap();
        assert_eq!(
            call(
                &bool_some,
                &[f64::from_bits(bool_handle as usize as u64), 1.0]
            )
            .value,
            1.0
        );
        let bool_not = CString::new("expr:rb0,rbmapnot:bool-map-not").unwrap();
        let result = call(&bool_not, &[f64::from_bits(bool_handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(16).cast::<u64>().read() }, 0);
        assert_eq!(unsafe { output.add(24).cast::<u64>().read() }, 1);
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_filter = CString::new("expr:rs0,s1,rsfiltergte:string-filter-gte").unwrap();
        let result = call(
            &string_filter,
            &[
                f64::from_bits(string_handle as usize as u64),
                f64::from_bits(nonempty_string.as_ptr() as usize as u64),
            ],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { output.add(8).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_identity = CString::new("expr:rs0,rsmapidentity:string-map-identity").unwrap();
        let result = call(
            &string_identity,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(
            unsafe { output.add(16).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_upper = CString::new("expr:rs0,rsmaptouppercase:string-map-uppercase").unwrap();
        let result = call(
            &string_upper,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(16).cast::<*const c_char>().read()).to_bytes() },
            b"X"
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_lengths = CString::new("expr:rs0,rsmaplength:string-map-length").unwrap();
        let result = call(
            &string_lengths,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_booleans = CString::new("expr:rs0,rsmaptoboolean:string-map-boolean").unwrap();
        let result = call(
            &string_booleans,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<u64>().read() }, 0);
        assert_eq!(unsafe { output.add(16).cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(24).cast::<u64>().read() }, 0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let bool_numbers = CString::new("expr:rb0,rbmaptonumber:boolean-map-number").unwrap();
        let result = call(
            &bool_numbers,
            &[f64::from_bits(bool_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let number_strings = CString::new("expr:rn0,rnmaptostring:number-map-string").unwrap();
        let result = call(&number_strings, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()).to_bytes() },
            b"42"
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let map = CString::new("expr:rn0,c4000000000000000,rnmapmul:array-map").unwrap();
        let result = call(&map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 40.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 60.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let index_map = CString::new("expr:rn0,rnmapindexadd:array-index-map").unwrap();
        let result = call(&index_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 21.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 32.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let select_map =
            CString::new("expr:rn0,c402e000000000000,rnmapselectgte0:array-select-map").unwrap();
        let result = call(&select_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 15.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let branch_map =
            CString::new("expr:rn0,c4034000000000000,rnmapbranch933:array-branch-map").unwrap();
        let result = call(&branch_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 10.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let remainder =
            CString::new("expr:rn0,c4018000000000000,rnmaprem:array-map-remainder").unwrap();
        let result = call(&remainder, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 4.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 2.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let reverse_map =
            CString::new("expr:rn0,c4059000000000000,rnmaprsub:array-map-reverse").unwrap();
        let result = call(&reverse_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 90.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 80.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 70.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let negate = CString::new("expr:rn0,rnmapneg:array-map-negate").unwrap();
        let result = call(&negate, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, -10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, -20.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, -30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let math_maps: [(&str, extern "C" fn(f64) -> f64); 27] = [
            ("acos", acos_number),
            ("acosh", acosh_number),
            ("asin", asin_number),
            ("asinh", asinh_number),
            ("atan", atan_number),
            ("atanh", atanh_number),
            ("cbrt", cbrt_number),
            ("ceil", ceil_number),
            ("clz32", clz32_number),
            ("cos", cos_number),
            ("cosh", cosh_number),
            ("exp", exp_number),
            ("expm1", expm1_number),
            ("floor", floor_number),
            ("fround", fround_number),
            ("log", log_number),
            ("log1p", log1p_number),
            ("log2", log2_number),
            ("log10", log10_number),
            ("round", round_number),
            ("sign", sign_number),
            ("sin", sin_number),
            ("sinh", sinh_number),
            ("sqrt", square_root_number),
            ("tan", tan_number),
            ("tanh", tanh_number),
            ("trunc", truncate_number),
        ];
        for (method, expected) in math_maps {
            let symbol = CString::new(format!("expr:rn0,rnmap{method}:array-map-math")).unwrap();
            let result = call(&symbol, &[f64::from_bits(handle as usize as u64)]);
            assert!(result.error.is_null(), "{method}");
            let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
            assert_eq!(
                unsafe { output.add(8).cast::<f64>().read() }.to_bits(),
                expected(10.0).to_bits(),
                "{method}"
            );
            unsafe { libc::free(output.cast_mut().cast()) };
        }
        let empty = [0_u64];
        let empty_data = empty.as_ptr().cast::<u8>();
        let empty_handle = &empty_data as *const *const u8;
        for operation in ["rnreduceadd0", "rnreducerightadd0"] {
            let empty_reduce = CString::new(format!(
                "expr:rn0,c0000000000000000,{operation}:empty-array-reduce"
            ))
            .unwrap();
            assert!(!call(
                &empty_reduce,
                &[f64::from_bits(empty_handle as usize as u64)]
            )
            .error
            .is_null());
        }
        for (operation, expected) in [("rnsomelt", 0.0), ("rneverylt", 1.0)] {
            let symbol = CString::new(format!(
                "expr:rn0,c0000000000000000,{operation}:empty-array-quantifier"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(empty_handle as usize as u64)]).value,
                expected
            );
        }
        for (operation, expected) in [("rnmin", f64::INFINITY), ("rnmax", f64::NEG_INFINITY)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:empty-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(empty_handle as usize as u64)]).value,
                expected
            );
        }
        assert_eq!(
            call(&hypot, &[f64::from_bits(empty_handle as usize as u64)]).value,
            0.0
        );
        let zeros = [2_u64, (-0.0f64).to_bits(), 0.0f64.to_bits()];
        let zeros_data = zeros.as_ptr().cast::<u8>();
        let zeros_handle = &zeros_data as *const *const u8;
        for (operation, expected) in [("rnmin", -0.0f64), ("rnmax", 0.0f64)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:zero-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(zeros_handle as usize as u64)])
                    .value
                    .to_bits(),
                expected.to_bits()
            );
        }
        let nan = [2_u64, 1.0f64.to_bits(), f64::NAN.to_bits()];
        let nan_data = nan.as_ptr().cast::<u8>();
        let nan_handle = &nan_data as *const *const u8;
        for operation in ["rnmin", "rnmax"] {
            let symbol = CString::new(format!("expr:rn0,{operation}:nan-extreme")).unwrap();
            assert!(call(&symbol, &[f64::from_bits(nan_handle as usize as u64)])
                .value
                .is_nan());
        }
        let large = [2_u64, 3e200f64.to_bits(), 4e200f64.to_bits()];
        let large_data = large.as_ptr().cast::<u8>();
        let large_handle = &large_data as *const *const u8;
        assert_eq!(
            call(&hypot, &[f64::from_bits(large_handle as usize as u64)]).value,
            0.0f64.hypot(3e200).hypot(4e200)
        );
        let infinite = [2_u64, f64::NAN.to_bits(), f64::INFINITY.to_bits()];
        let infinite_data = infinite.as_ptr().cast::<u8>();
        let infinite_handle = &infinite_data as *const *const u8;
        assert_eq!(
            call(&hypot, &[f64::from_bits(infinite_handle as usize as u64)]).value,
            f64::INFINITY
        );
        let replaced =
            CString::new("expr:rn0,c3ff0000000000000,c4058c00000000000,rnwith:with").unwrap();
        let result = call(&replaced, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 99.0);
        unsafe { libc::free(output.cast()) };
        let sorted = CString::new("expr:rn0,rnsorted:sorted").unwrap();
        let result = call(&sorted, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        unsafe { libc::free(output.cast()) };
        let reverse = CString::new("expr:rn0,arrayreversed:reverse").unwrap();
        let result = call(&reverse, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast()) };
        let slice =
            CString::new("expr:rn0,c3ff0000000000000,c4000000000000000,arrayslice:slice").unwrap();
        let result = call(&slice, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        unsafe { libc::free(output.cast()) };
        let literal = CString::new(
            "expr:arrayempty,c3ff0000000000000,rnappend,c4000000000000000,rnappend,arrayvalue:literal",
        )
        .unwrap();
        let result = call(&literal, &[]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 2.0);
        unsafe { libc::free(output.cast()) };

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

        let power = CString::new("expr:a0,a1,pow:power").unwrap();
        assert_eq!(call(&power, &[2.0, 5.0]).value, 32.0);
        assert_eq!(call(&power, &[f64::NAN, 0.0]).value, 1.0);
        assert!(call(&power, &[1.0, f64::INFINITY]).value.is_nan());
        assert!(call(&power, &[-2.0, 0.5]).value.is_nan());
        assert_eq!(
            call(&power, &[-0.0, 3.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(call(&power, &[-0.0, -3.0]).value, f64::NEG_INFINITY);

        for (operation, value, expected) in [
            ("isnan", f64::NAN, 1.0),
            ("isnan", 0.0, 0.0),
            ("isfinite", f64::INFINITY, 0.0),
            ("isfinite", 42.0, 1.0),
            ("isinteger", 42.5, 0.0),
            ("isinteger", -42.0, 1.0),
            ("issafeinteger", 9_007_199_254_740_991.0, 1.0),
            ("issafeinteger", 9_007_199_254_740_992.0, 0.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            assert_eq!(call(&symbol, &[value]).value, expected);
        }
    }

    #[test]
    fn persists_typed_globals_per_compiled_symbol() {
        let first = CString::new(
            "expr:c0000000000000000,c0000000000000000,globalget,c3ff0000000000000,+,globalset:first-global",
        )
        .unwrap();
        assert_eq!(call(&first, &[]).value, 1.0);
        assert_eq!(call(&first, &[]).value, 2.0);

        let second = CString::new(
            "expr:c0000000000000000,c0000000000000000,globalget,c3ff0000000000000,+,globalset:second-global",
        )
        .unwrap();
        assert_eq!(call(&second, &[]).value, 1.0);

        let writer = CString::new(
            "expr:c0000000000000000,c0000000000000000,c4014000000000000,globalinit,c3ff0000000000000,+,globalset:shared::write",
        )
        .unwrap();
        let reader =
            CString::new("expr:c0000000000000000,c4014000000000000,globalinit:shared::read")
                .unwrap();
        assert_eq!(call(&writer, &[]).value, 6.0);
        assert_eq!(call(&reader, &[]).value, 6.0);
    }

    #[test]
    fn persists_dynamic_callable_entries_per_module() {
        let writer = CString::new(
            "expr:t72756e,c0000000000000000,c4008000000000000,callableset:callable-table::write",
        )
        .unwrap();
        let reader =
            CString::new("expr:t72756e,c0000000000000000,callableget:callable-table::read")
                .unwrap();
        let missing = CString::new(
            "expr:t6d697373696e67,c0000000000000000,callableget:callable-table::missing",
        )
        .unwrap();
        assert_eq!(call(&writer, &[]).value, 3.0);
        assert_eq!(call(&reader, &[]).value, 3.0);
        assert_eq!(call(&missing, &[]).value, -1.0);
    }

    #[test]
    fn snapshots_and_reassigns_dynamic_callable_entries_in_locals() {
        let symbol = CString::new(
            "expr:t6669727374,c0000000000000000,c0000000000000000,callableset,drop,t7365636f6e64,c0000000000000000,c3ff0000000000000,callableset,drop,t6669727374,c0000000000000000,callableget,t7365636f6e64,c0000000000000000,callableget,setl0,t7365636f6e64,c0000000000000000,c0000000000000000,callableset,drop,ln0,nip:callable-snapshot",
        )
        .unwrap();
        let result = call(&symbol, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let full = CString::new(
            "expr:t6669727374,c0000000000000000,c0000000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,t7365636f6e64,c0000000000000000,c3ff0000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,t6669727374,dup,c0000000000000000,callableget,dup,cbff0000000000000,!=,if,dup,else,dup2,drop,dup,t72756e,strcmp,c0000000000000000,==,if,c0000000000000000,else,cbff0000000000000,end,nip,end,nip,nip,t7365636f6e64,dup,c0000000000000000,callableget,dup,cbff0000000000000,!=,if,dup,else,dup2,drop,dup,t72756e,strcmp,c0000000000000000,==,if,c0000000000000000,else,cbff0000000000000,end,nip,end,nip,nip,setl0,t7365636f6e64,c0000000000000000,c0000000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,ln0,c0000000000000000,==,if,c4010000000000000,c3ff0000000000000,+,else,ln0,c3ff0000000000000,==,if,c4010000000000000,c4000000000000000,*,else,missingcalln,end,end,nip:callable-full-snapshot",
        )
        .unwrap();
        let result = call(&full, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 8.0);

        let arguments = CString::new(
            full.to_str()
                .unwrap()
                .replace("t6669727374", "s0")
                .replace("t7365636f6e64", "s1")
                .replace("c4010000000000000", "a2")
                .replace("callable-full-snapshot", "callable-argument-snapshot"),
        )
        .unwrap();
        let first = CString::new("first").unwrap();
        let second = CString::new("second").unwrap();
        let result = call(
            &arguments,
            &[
                f64::from_bits(first.as_ptr() as usize as u64),
                f64::from_bits(second.as_ptr() as usize as u64),
                4.0,
            ],
        );
        assert!(result.error.is_null());
        assert_eq!(result.value, 8.0);
    }

    #[test]
    fn converts_primitives_to_arena_strings() {
        for (symbol, argument, expected) in [
            ("expr:a0,numstr:number-string", 42.0, "42"),
            ("expr:b0,boolstr:boolean-string", 1.0, "true"),
            ("expr:b0,boolstr:false-string", 0.0, "false"),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[argument]);
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        let symbol = CString::new("expr:s0,strnum:string-number").unwrap();
        let result = call(&symbol, &[1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        let symbol = CString::new("expr:s0,strbool:string-boolean").unwrap();
        for (value, expected) in [("", 0.0), ("value", 1.0)] {
            let value = CString::new(value).unwrap();
            let result = call(&symbol, &[f64::from_bits(value.as_ptr() as usize as u64)]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }
        let float = CString::new("  -12.5px").unwrap();
        let symbol = CString::new("expr:s0,parsefloat:parse-float").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(float.as_ptr() as usize as u64)]).value,
            -12.5
        );
        let integer = CString::new("11").unwrap();
        let symbol = CString::new("expr:s0,c4000000000000000,parseint:parse-int").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(integer.as_ptr() as usize as u64)]).value,
            3.0
        );
        for (operation, value, argument, expected) in [
            ("tofixed", 12.5, 1.0, "12.5"),
            ("toprecision", 12.5, 3.0, "12.5"),
            ("toradix", 255.0, 16.0, "ff"),
            ("toexponential", 12.5, 1.0, "1.3e+1"),
        ] {
            let symbol = CString::new(format!("expr:a0,a1,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[value, argument]);
            assert!(result.error.is_null());
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let shortest = CString::new("expr:a0,toexponential0:toexponential0").unwrap();
        let result = call(&shortest, &[12.5]);
        assert!(result.error.is_null());
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "1.25e+1"
        );
        unsafe { libc::free(result.cast()) };
        let invalid = CString::new("expr:a0,a1,tofixed:invalid-fixed").unwrap();
        assert!(!call(&invalid, &[1.0, 101.0]).error.is_null());
        let array = [
            3_u64,
            10.0f64.to_bits(),
            20.0f64.to_bits(),
            30.0f64.to_bits(),
        ];
        let data = array.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        let symbol = CString::new("expr:rn0,arraylen:array-length").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            3.0
        );
        let symbol = CString::new("expr:rn0,isarray:is-array").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            1.0
        );
        let symbol = CString::new("expr:a0,isnotarray:is-not-array").unwrap();
        assert_eq!(call(&symbol, &[42.0]).value, 0.0);
        let symbol = CString::new("expr:a0,a1,numsame:number-same-value").unwrap();
        assert_eq!(call(&symbol, &[f64::NAN, f64::NAN]).value, 1.0);
        assert_eq!(call(&symbol, &[0.0, -0.0]).value, 0.0);
        let symbol = CString::new("expr:s0,s1,strsame:string-same-value").unwrap();
        let text = CString::new("hello").unwrap();
        let other_text = CString::new("hello").unwrap();
        assert_eq!(
            call(
                &symbol,
                &[
                    f64::from_bits(text.as_ptr() as usize as u64),
                    f64::from_bits(other_text.as_ptr() as usize as u64),
                ],
            )
            .value,
            1.0
        );
        let symbol = CString::new("expr:rn0,rn1,refsame:reference-same-value").unwrap();
        let array = f64::from_bits(handle as usize as u64);
        assert_eq!(call(&symbol, &[array, array]).value, 1.0);
        for (symbol, expected) in [
            ("expr:a0,typeofnumber:type-of-number", "number"),
            ("expr:b0,typeofboolean:type-of-boolean", "boolean"),
            ("expr:s0,typeofstring:type-of-string", "string"),
            ("expr:rn0,typeofobject:type-of-array", "object"),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[array]);
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        for (operation, from, expected) in [
            ("rnindexof", 0.0f64, 1.0),
            ("rnincludes", 0.0, 1.0),
            ("rnlastindexof", f64::INFINITY, 1.0),
        ] {
            let symbol = CString::new(format!(
                "expr:rn0,c4034000000000000,c{:016x},{operation}:{operation}",
                from.to_bits()
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        let symbol = CString::new("expr:rn0,c4000000000000000,rnat:array-at").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            30.0
        );
        let symbol = CString::new("expr:rn0,c4010000000000000,rnat:array-at-missing").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
            ABSENT_STATUS
        );
        let symbol = CString::new("expr:rn0,c3ff0000000000000,rnget:array-get").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            20.0
        );
        let symbol = CString::new("expr:rn0,cbff0000000000000,rnget:array-get-negative").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
            ABSENT_STATUS
        );
        let symbol = CString::new("expr:rn0,t7c,rnjoin:array-join").unwrap();
        let result = call(&symbol, &[f64::from_bits(handle as usize as u64)]);
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "10|20|30"
        );
    }

    #[test]
    fn propagates_uncaught_typed_throws() {
        for token in [
            "missingcalln",
            "missingcallb",
            "missingcalls",
            "missingcalla",
            "missingcalld",
        ] {
            let result = call(
                &CString::new(format!("expr:{token}:not-callable")).unwrap(),
                &[],
            );
            assert_eq!(
                unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
                "value is not a function"
            );
        }

        for (symbol, expected) in [
            (
                "expr:c4045000000000000,throwoutn,c0000000000000000:number-throw",
                "42",
            ),
            (
                "expr:c3ff0000000000000,asbool,throwoutb,c0000000000000000:boolean-throw",
                "true",
            ),
            (
                "expr:t626f6f6d,throwouts,c0000000000000000:string-throw",
                "boom",
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert_eq!(
                unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
                expected
            );
        }

        let storage = [
            3_u64,
            10.0_f64.to_bits(),
            20.0_f64.to_bits(),
            30.0_f64.to_bits(),
        ];
        let data = storage.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        let result = call(
            &CString::new("expr:rn0,throwoutrn,c0000000000000000:array-throw").unwrap(),
            &[f64::from_bits(handle as usize as u64)],
        );
        assert_eq!(
            unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
            "10,20,30"
        );

        let result = call(
            &CString::new("expr:dn0,throwoutd,c0000000000000000:dictionary-throw").unwrap(),
            &[0.0],
        );
        assert_eq!(
            unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
            "[object Object]"
        );
    }
}
