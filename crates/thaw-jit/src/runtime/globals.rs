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
