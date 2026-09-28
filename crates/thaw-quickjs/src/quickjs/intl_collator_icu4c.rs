// Optional ICU4C backend (feature `icu4c`, default off) for
// `Intl.Collator`'s `usage: 'search'`, which ICU4X's `icu_collator`
// cannot express (it has no search-collation concept at all). Selected
// only by `thaw build --icu4c`.
//
// ICU4C is resolved at *runtime* via `dlopen`, not link time -- so the
// feature adds no build-time or linker dependency, and a generated
// program still builds and runs without ICU4C installed (it just falls
// back to ICU4X sort behavior). This also sidesteps the `--as-needed`
// link-ordering a `#[link]` would introduce. All other
// `Intl.Collator` behavior (`usage: 'sort'`) stays on ICU4X; this module
// is reached only from `intl.js`'s `compare` when `usage === 'search'`.

use std::os::raw::c_int;
use std::sync::OnceLock;

const RTLD_LAZY: c_int = 1;
const RTLD_GLOBAL: c_int = 0x100;

// `UColAttribute` (ucol.h).
const UCOL_ALTERNATE_HANDLING: c_int = 1;
const UCOL_CASE_FIRST: c_int = 2;
const UCOL_CASE_LEVEL: c_int = 3;
const UCOL_STRENGTH: c_int = 5;
const UCOL_NUMERIC_COLLATION: c_int = 7;
// `UColAttributeValue`.
const UCOL_PRIMARY: c_int = 0;
const UCOL_SECONDARY: c_int = 1;
const UCOL_TERTIARY: c_int = 2;
const UCOL_OFF: c_int = 16;
const UCOL_ON: c_int = 17;
const UCOL_SHIFTED: c_int = 20;
const UCOL_LOWER_FIRST: c_int = 24;
const UCOL_UPPER_FIRST: c_int = 25;
// `UColReorderCode`.
const UCOL_REORDER_CODE_PUNCTUATION: c_int = 0x1001;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

type UCollator = c_void;

struct Icu {
    open: unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut UCollator,
    close: unsafe extern "C" fn(*mut UCollator),
    set_attribute: unsafe extern "C" fn(*mut UCollator, c_int, c_int, *mut c_int),
    set_max_variable: unsafe extern "C" fn(*mut UCollator, c_int, *mut c_int) -> *mut UCollator,
    strcoll_utf8: unsafe extern "C" fn(
        *const UCollator,
        *const c_char,
        c_int,
        *const c_char,
        c_int,
        *mut c_int,
    ) -> c_int,
    // Plural range selection (`Intl.PluralRules.selectRange`) needs a
    // `UFormattedNumberRange`, so this is a separate, optional API group
    // (an older ICU without it still keeps search collation working).
    uplrules_open_for_type:
        Option<unsafe extern "C" fn(*const c_char, c_int, *mut c_int) -> *mut UPluralRules>,
    uplrules_select_for_range: Option<
        unsafe extern "C" fn(
            *const UPluralRules,
            *const UFormattedNumberRange,
            *mut u16,
            c_int,
            *mut c_int,
        ) -> c_int,
    >,
    unumrf_open: Option<
        unsafe extern "C" fn(
            *const u16,
            c_int,
            c_int,
            c_int,
            *const c_char,
            *mut c_void,
            *mut c_int,
        ) -> *mut UNumberRangeFormatter,
    >,
    unumrf_open_result: Option<unsafe extern "C" fn(*mut c_int) -> *mut UFormattedNumberRange>,
    unumrf_format_decimal_range: Option<
        unsafe extern "C" fn(
            *const UNumberRangeFormatter,
            *const c_char,
            c_int,
            *const c_char,
            c_int,
            *mut UFormattedNumberRange,
            *mut c_int,
        ),
    >,
    unumrf_close_result: Option<unsafe extern "C" fn(*mut UFormattedNumberRange)>,
}

type UPluralRules = c_void;
type UFormattedNumberRange = c_void;
type UNumberRangeFormatter = c_void;

unsafe fn resolve<T>(handle: *mut c_void, base: &str) -> Option<T> {
    // Fedora (and some other distros) build ICU with a per-major symbol
    // suffix for parallel installability (`ucol_open_77`), while upstream
    // exports the unsuffixed name (versioned); try the suffixed names
    // first, then the plain one.
    for major in [77_i32, 76, 75, 74, 73, 72, 71, 70] {
        let name = CString::new(format!("{base}_{major}")).ok()?;
        let symbol = unsafe { dlsym(handle, name.as_ptr()) };
        if !symbol.is_null() {
            return Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&symbol) });
        }
    }
    let name = CString::new(base).ok()?;
    let symbol = unsafe { dlsym(handle, name.as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&symbol) })
}

fn icu() -> Option<&'static Icu> {
    static ICU: OnceLock<Option<Icu>> = OnceLock::new();
    ICU.get_or_init(|| unsafe {
        let handle = ["libicui18n.so", "libicui18n.so.77", "libicui18n.so.76"]
            .into_iter()
            .find_map(|name| {
                let name = CString::new(name).ok()?;
                let handle = dlopen(name.as_ptr(), RTLD_LAZY | RTLD_GLOBAL);
                (!handle.is_null()).then_some(handle)
            })?;
            Some(Icu {
                open: resolve(handle, "ucol_open")?,
                close: resolve(handle, "ucol_close")?,
                set_attribute: resolve(handle, "ucol_setAttribute")?,
                set_max_variable: resolve(handle, "ucol_setMaxVariable")?,
                strcoll_utf8: resolve(handle, "ucol_strcollUTF8")?,
                uplrules_open_for_type: resolve(handle, "uplrules_openForType"),
                uplrules_select_for_range: resolve(handle, "uplrules_selectForRange"),
                unumrf_open: resolve(
                    handle,
                    "unumrf_openForSkeletonWithCollapseAndIdentityFallback",
                ),
                unumrf_open_result: resolve(handle, "unumrf_openResult"),
                unumrf_format_decimal_range: resolve(handle, "unumrf_formatDecimalRange"),
                unumrf_close_result: resolve(handle, "unumrf_closeResult"),
            })
    })
    .as_ref()
}

thread_local! {
    // Opened collators are cached for the process lifetime (never closed)
    // -- immutable once configured, and the program is short-lived; a `0`
    // entry records a failed open so it isn't retried.
    static COLLATORS: std::cell::RefCell<std::collections::HashMap<String, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn open_collator(
    icu: &Icu,
    locale: &str,
    sensitivity: &str,
    ignore_punctuation: bool,
    numeric: bool,
    case_first: &str,
) -> Option<*mut UCollator> {
    let key = format!(
        "{locale}\u{1}{sensitivity}\u{1}{ignore_punctuation}\u{1}{numeric}\u{1}{case_first}"
    );
    COLLATORS.with(|cache| {
        if let Some(&pointer) = cache.borrow().get(&key) {
            return if pointer == 0 {
                None
            } else {
                Some(pointer as *mut UCollator)
            };
        }
        let opened = open_collator_uncached(
            icu,
            locale,
            sensitivity,
            ignore_punctuation,
            numeric,
            case_first,
        );
        cache
            .borrow_mut()
            .insert(key, opened.map_or(0, |pointer| pointer as usize));
        opened
    })
}

fn open_collator_uncached(
    icu: &Icu,
    locale: &str,
    sensitivity: &str,
    ignore_punctuation: bool,
    numeric: bool,
    case_first: &str,
) -> Option<*mut UCollator> {
    // ICU4C's own search collation type (`usage: 'search'`).
    let identifier = CString::new(format!("{locale}@collation=search")).ok()?;
    let mut status: c_int = 0; // U_ZERO_ERROR
    let collator = unsafe { (icu.open)(identifier.as_ptr(), &mut status) };
    if status > 0 || collator.is_null() {
        return None;
    }
    fn set(icu: &Icu, collator: *mut UCollator, attribute: c_int, value: c_int) -> bool {
        let mut status: c_int = 0;
        unsafe { (icu.set_attribute)(collator, attribute, value, &mut status) };
        status <= 0
    }
    let strength = match sensitivity {
        "base" | "case" => UCOL_PRIMARY,
        "accent" => UCOL_SECONDARY,
        _ => UCOL_TERTIARY,
    };
    let mut ok = set(icu, collator, UCOL_STRENGTH, strength);
    if sensitivity == "case" {
        ok &= set(icu, collator, UCOL_CASE_LEVEL, UCOL_ON);
    }
    ok &= set(
        icu,
        collator,
        UCOL_NUMERIC_COLLATION,
        if numeric { UCOL_ON } else { UCOL_OFF },
    );
    if ignore_punctuation {
        ok &= set(icu, collator, UCOL_ALTERNATE_HANDLING, UCOL_SHIFTED);
        let mut max_status: c_int = 0;
        unsafe { (icu.set_max_variable)(collator, UCOL_REORDER_CODE_PUNCTUATION, &mut max_status) };
        ok &= max_status <= 0;
    }
    match case_first {
        "upper" => ok &= set(icu, collator, UCOL_CASE_FIRST, UCOL_UPPER_FIRST),
        "lower" => ok &= set(icu, collator, UCOL_CASE_FIRST, UCOL_LOWER_FIRST),
        _ => {}
    }
    if !ok {
        unsafe { (icu.close)(collator) };
        return None;
    }
    Some(collator)
}

/// `__thaw_intl_collator_compare_search(locale, sensitivity,
/// ignore_punctuation, numeric, case_first, a, b) -> i32`: ICU4C's
/// search-collation comparison (`Intl.Collator` with `usage: 'search'`),
/// falling back to plain UTF-16 codepoint ordering if ICU4C can't be
/// loaded or the search collator fails to open.
#[allow(clippy::too_many_arguments)]
fn intl_collator_compare_search(
    locale: &str,
    sensitivity: &str,
    ignore_punctuation: bool,
    numeric: bool,
    case_first: &str,
    a: &str,
    b: &str,
) -> i32 {
    let codepoint_fallback = || match a.encode_utf16().cmp(b.encode_utf16()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    let Some(icu) = icu() else {
        return codepoint_fallback();
    };
    let Some(collator) = open_collator(
        icu,
        locale,
        sensitivity,
        ignore_punctuation,
        numeric,
        case_first,
    ) else {
        return codepoint_fallback();
    };
    let mut status: c_int = 0;
    let result = unsafe {
        (icu.strcoll_utf8)(
            collator,
            a.as_ptr().cast::<c_char>(),
            a.len() as c_int,
            b.as_ptr().cast::<c_char>(),
            b.len() as c_int,
            &mut status,
        )
    };
    if status > 0 {
        return codepoint_fallback();
    }
    result
}

// `UPluralType`.
const UPLURAL_TYPE_ORDINAL: c_int = 1;
// `UNumberRangeCollapse::UNUM_RANGE_COLLAPSE_AUTO`.
const UNUM_RANGE_COLLAPSE_AUTO: c_int = 0;
// `UNumberRangeIdentityFallback::UNUM_IDENTITY_FALLBACK_APPROXIMATELY_OR_SINGLE_VALUE`.
const UNUM_IDENTITY_FALLBACK_APPROXIMATELY_OR_SINGLE_VALUE: c_int = 1;

thread_local! {
    // Plural rules and range formatters are cached for the process
    // lifetime (never closed), like `COLLATORS`; `0` records a failure.
    static PLURAL_RULES: std::cell::RefCell<std::collections::HashMap<String, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    static RANGE_FORMATTERS: std::cell::RefCell<std::collections::HashMap<String, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

fn open_plural_rules(
    icu: &Icu,
    locale: &str,
    kind: &str,
) -> Option<*mut UPluralRules> {
    let open = icu.uplrules_open_for_type?;
    let type_ = if kind == "ordinal" {
        UPLURAL_TYPE_ORDINAL
    } else {
        0 // UPLURAL_TYPE_CARDINAL
    };
    let key = format!("{type_}\u{1}{locale}");
    PLURAL_RULES.with(|cache| {
        if let Some(&pointer) = cache.borrow().get(&key) {
            return if pointer == 0 {
                None
            } else {
                Some(pointer as *mut UPluralRules)
            };
        }
        let identifier = CString::new(locale).ok();
        let mut status: c_int = 0;
        let opened = identifier.and_then(|identifier| {
            let rules = unsafe { open(identifier.as_ptr(), type_, &mut status) };
            (!rules.is_null() && status <= 0).then_some(rules)
        });
        cache
            .borrow_mut()
            .insert(key, opened.map_or(0, |pointer| pointer as usize));
        opened
    })
}

fn open_range_formatter(icu: &Icu, locale: &str) -> Option<*mut UNumberRangeFormatter> {
    let open = icu.unumrf_open?;
    RANGE_FORMATTERS.with(|cache| {
        if let Some(&pointer) = cache.borrow().get(locale) {
            return if pointer == 0 {
                None
            } else {
                Some(pointer as *mut UNumberRangeFormatter)
            };
        }
        let identifier = CString::new(locale).ok();
        let skeleton: [u16; 1] = [b'0' as u16];
        let mut status: c_int = 0;
        let opened = identifier.and_then(|identifier| {
            let formatter = unsafe {
                open(
                    skeleton.as_ptr(),
                    skeleton.len() as c_int,
                    UNUM_RANGE_COLLAPSE_AUTO,
                    UNUM_IDENTITY_FALLBACK_APPROXIMATELY_OR_SINGLE_VALUE,
                    identifier.as_ptr(),
                    std::ptr::null_mut(),
                    &mut status,
                )
            };
            (!formatter.is_null() && status <= 0).then_some(formatter)
        });
        cache
            .borrow_mut()
            .insert(locale.to_string(), opened.map_or(0, |pointer| pointer as usize));
        opened
    })
}

/// `__thaw_intl_plural_range_icu4c(locale, kind, start, end) -> String`:
/// `Intl.PluralRules.prototype.selectRange` with real ICU4C semantics
/// (ICU4X's vendored plural-range data diverges from ICU4C/Node). Formats
/// the already-digit-optioned decimal strings into a
/// `UFormattedNumberRange` and selects via `uplrules_selectForRange`.
/// Returns `"other"` if anything fails or the range API is unavailable.
fn intl_plural_range_icu4c(locale: &str, kind: &str, start: &str, end: &str) -> String {
    let fallback = "other".to_string();
    let Some(icu) = icu() else {
        return fallback;
    };
    let (Some(format_decimal_range), Some(open_result), Some(close_result), Some(select)) = (
        icu.unumrf_format_decimal_range,
        icu.unumrf_open_result,
        icu.unumrf_close_result,
        icu.uplrules_select_for_range,
    ) else {
        return fallback;
    };
    let (Some(rules), Some(formatter)) = (
        open_plural_rules(icu, locale, kind),
        open_range_formatter(icu, locale),
    ) else {
        return fallback;
    };
    let (Ok(start), Ok(end)) = (CString::new(start), CString::new(end)) else {
        return fallback;
    };
    let mut status: c_int = 0;
    let result = unsafe { open_result(&mut status) };
    if result.is_null() || status > 0 {
        return fallback;
    }
    unsafe {
        format_decimal_range(
            formatter,
            start.as_ptr(),
            start.as_bytes().len() as c_int,
            end.as_ptr(),
            end.as_bytes().len() as c_int,
            result,
            &mut status,
        )
    };
    if status > 0 {
        unsafe { close_result(result) };
        return fallback;
    }
    let mut buffer = [0_u16; 64];
    let length = unsafe {
        select(
            rules,
            result,
            buffer.as_mut_ptr(),
            buffer.len() as c_int,
            &mut status,
        )
    };
    unsafe { close_result(result) };
    if status > 0 || length <= 0 {
        return fallback;
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}
