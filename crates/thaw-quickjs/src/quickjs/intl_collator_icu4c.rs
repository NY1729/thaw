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
    // Narrow `month`/`weekday` date formatting (`udat`), which ICU4X's
    // field-set builder cannot express (its `Length` has no Narrow).
    udatpg_open: Option<unsafe extern "C" fn(*const c_char, *mut c_int) -> *mut UDateTimePatternGenerator>,
    udatpg_get_best_pattern: Option<
        unsafe extern "C" fn(*mut UDateTimePatternGenerator, *const u16, c_int, *mut u16, c_int, *mut c_int) -> c_int,
    >,
    udatpg_close: Option<unsafe extern "C" fn(*mut UDateTimePatternGenerator)>,
    udat_open: Option<
        unsafe extern "C" fn(c_int, c_int, *const c_char, *const u16, c_int, *const u16, c_int, *mut c_int) -> *mut UDateFormat,
    >,
    udat_format_for_fields: Option<
        unsafe extern "C" fn(*const UDateFormat, f64, *mut u16, c_int, *mut UFieldPositionIterator, *mut c_int) -> c_int,
    >,
    udat_close: Option<unsafe extern "C" fn(*mut UDateFormat)>,
    ufieldpositer_open: Option<unsafe extern "C" fn(*mut c_int) -> *mut UFieldPositionIterator>,
    ufieldpositer_next: Option<unsafe extern "C" fn(*mut UFieldPositionIterator, *mut c_int, *mut c_int) -> c_int>,
    ufieldpositer_close: Option<unsafe extern "C" fn(*mut UFieldPositionIterator)>,
}

type UDateFormat = c_void;
type UDateTimePatternGenerator = c_void;
type UFieldPositionIterator = c_void;

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
                udatpg_open: resolve(handle, "udatpg_open"),
                udatpg_get_best_pattern: resolve(handle, "udatpg_getBestPattern"),
                udatpg_close: resolve(handle, "udatpg_close"),
                udat_open: resolve(handle, "udat_open"),
                udat_format_for_fields: resolve(handle, "udat_formatForFields"),
                udat_close: resolve(handle, "udat_close"),
                ufieldpositer_open: resolve(handle, "ufieldpositer_open"),
                ufieldpositer_next: resolve(handle, "ufieldpositer_next"),
                ufieldpositer_close: resolve(handle, "ufieldpositer_close"),
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

const UDAT_PATTERN: c_int = -2;

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// ICU skeleton field letters for the requested options. Unlike the
/// `FieldSetBuilder` (no Narrow), ICU4C's `udatpg_getBestPattern` maps
/// these to the locale's own pattern with the right field widths -- so
/// `month:'narrow'`/`weekday:'narrow'` get both the correct narrow name
/// *and* the pattern narrow uses (`ja` `{month:'narrow'}` -> `7月`).
fn datetime_skeleton(options: &serde_json::Value) -> String {
    let get = |key: &str| options.get(key).and_then(|value| value.as_str());
    let mut skeleton = String::new();
    if let Some(era) = get("era") {
        skeleton.push_str(match era {
            "long" => "GGGG",
            "narrow" => "GGGGG",
            _ => "G",
        });
    }
    if let Some(year) = get("year") {
        skeleton.push_str(if year == "2-digit" { "yy" } else { "y" });
    }
    if let Some(month) = get("month") {
        skeleton.push_str(match month {
            "2-digit" => "MM",
            "long" => "MMMM",
            "short" => "MMM",
            "narrow" => "MMMMM",
            _ => "M",
        });
    }
    if let Some(day) = get("day") {
        skeleton.push_str(if day == "2-digit" { "dd" } else { "d" });
    }
    if let Some(weekday) = get("weekday") {
        skeleton.push_str(match weekday {
            "long" => "EEEE",
            "narrow" => "EEEEE",
            _ => "EEE",
        });
    }
    if let Some(hour) = get("hour") {
        let hour12 = options.get("hour12").and_then(|value| value.as_bool());
        let cycle = get("hourCycle").unwrap_or("");
        let letter = match (hour12, cycle) {
            (Some(true), _) => 'h',
            (Some(false), _) => 'H',
            (_, "h11") => 'K',
            (_, "h12") => 'h',
            (_, "h23") => 'H',
            (_, "h24") => 'k',
            _ => 'h',
        };
        skeleton.push(letter);
        if hour == "2-digit" {
            skeleton.push(letter);
        }
    }
    if let Some(minute) = get("minute") {
        skeleton.push_str(if minute == "2-digit" { "mm" } else { "m" });
    }
    if let Some(second) = get("second") {
        skeleton.push_str(if second == "2-digit" { "ss" } else { "s" });
    }
    skeleton
}

fn datetime_field_type(field: c_int) -> Option<&'static str> {
    Some(match field {
        0 => "era",
        1 | 30 => "year",
        2 => "month",
        3 => "day",
        4 | 5 => "hour",
        6 => "minute",
        7 => "second",
        8 => "fractionalSecond",
        9 => "weekday",
        14 => "dayPeriod",
        _ => return None,
    })
}

fn skeleton_field_type(letter: u16) -> Option<&'static str> {
    match letter as u8 as char {
        'G' => Some("era"),
        'y' | 'u' => Some("year"),
        'M' | 'L' => Some("month"),
        'd' => Some("day"),
        'E' | 'c' => Some("weekday"),
        'h' | 'H' | 'K' | 'k' => Some("hour"),
        'm' => Some("minute"),
        's' => Some("second"),
        _ => None,
    }
}

/// `__thaw_intl_datetime_narrow_icu4c(locale, options_json, zoned_json)
/// -> String` (a JSON parts array, same shape as the ICU4X path): real
/// ICU4C date/time rendering, used only for the `month:'narrow'`/
/// `weekday:'narrow'` requests ICU4X's field-set builder can't express.
/// Returns `"[]"` on any failure, so the caller falls back to ICU4X.
fn intl_datetime_narrow_icu4c(locale: &str, options_json: &str, zoned_json: &str) -> String {
    intl_datetime_narrow_icu4c_inner(locale, options_json, zoned_json)
        .unwrap_or_else(|| "[]".to_string())
}

fn intl_datetime_narrow_icu4c_inner(
    locale: &str,
    options_json: &str,
    zoned_json: &str,
) -> Option<String> {
    let icu = icu()?;
    let (
        Some(pg_open),
        Some(best_pattern),
        Some(pg_close),
        Some(date_open),
        Some(format_fields),
        Some(date_close),
        Some(fpi_open),
        Some(fpi_next),
        Some(fpi_close),
    ) = (
        icu.udatpg_open,
        icu.udatpg_get_best_pattern,
        icu.udatpg_close,
        icu.udat_open,
        icu.udat_format_for_fields,
        icu.udat_close,
        icu.ufieldpositer_open,
        icu.ufieldpositer_next,
        icu.ufieldpositer_close,
    )
    else {
        return None;
    };

    let options: serde_json::Value = serde_json::from_str(options_json).ok()?;
    let zoned: serde_json::Value = serde_json::from_str(zoned_json).ok()?;
    let skeleton = datetime_skeleton(&options);
    if skeleton.is_empty() {
        return None;
    }
    let locale = CString::new(locale).ok()?;
    let skeleton = utf16(&skeleton);

    let mut status: c_int = 0;
    let generator = unsafe { pg_open(locale.as_ptr(), &mut status) };
    if generator.is_null() || status > 0 {
        return None;
    }
    let mut pattern = vec![0_u16; 128];
    let pattern_length = unsafe {
        best_pattern(
            generator,
            skeleton.as_ptr(),
            skeleton.len() as c_int,
            pattern.as_mut_ptr(),
            pattern.len() as c_int,
            &mut status,
        )
    };
    unsafe { pg_close(generator) };
    if status > 0 || pattern_length <= 0 {
        return None;
    }
    pattern.truncate(pattern_length as usize);

    let time_zone = zoned
        .get("timeZone")
        .and_then(|value| value.as_str())
        .unwrap_or("UTC");
    let time_zone = utf16(time_zone);
    status = 0;
    let formatter = unsafe {
        date_open(
            UDAT_PATTERN,
            UDAT_PATTERN,
            locale.as_ptr(),
            time_zone.as_ptr(),
            time_zone.len() as c_int,
            pattern.as_ptr(),
            pattern.len() as c_int,
            &mut status,
        )
    };
    if formatter.is_null() || status > 0 {
        return None;
    }

    let date = zoned
        .get("timestampMs")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    status = 0;
    let iterator = unsafe { fpi_open(&mut status) };
    if iterator.is_null() || status > 0 {
        unsafe { date_close(formatter) };
        return None;
    }
    let mut buffer = vec![0_u16; 512];
    status = 0;
    let length = unsafe {
        format_fields(
            formatter,
            date,
            buffer.as_mut_ptr(),
            buffer.len() as c_int,
            iterator,
            &mut status,
        )
    };
    if status > 0 || length <= 0 || length as usize > buffer.len() {
        unsafe {
            fpi_close(iterator);
            date_close(formatter);
        }
        return None;
    }
    let text = &buffer[..length as usize];

    let mut parts: Vec<serde_json::Value> = Vec::new();
    let mut cursor = 0_usize;
    loop {
        let mut begin: c_int = 0;
        let mut end: c_int = 0;
        let field = unsafe { fpi_next(iterator, &mut begin, &mut end) };
        if field < 0 {
            break;
        }
        let (begin, end) = (begin.max(0) as usize, end.max(0) as usize);
        if begin > cursor && end <= text.len() {
            parts.push(serde_json::json!({
                "type": "literal",
                "value": String::from_utf16_lossy(&text[cursor..begin]),
            }));
        }
        if let Some(kind) = datetime_field_type(field) {
            if end <= text.len() {
                parts.push(serde_json::json!({
                    "type": kind,
                    "value": String::from_utf16_lossy(&text[begin..end]),
                }));
            }
        }
        cursor = cursor.max(end);
    }
    if cursor < text.len() {
        parts.push(serde_json::json!({
            "type": "literal",
            "value": String::from_utf16_lossy(&text[cursor..]),
        }));
    }
    // ICU's field-position iterator reports nothing for a pattern that is
    // a single bare field (`{month:'narrow'}` -> `"LLLLL"`), so those
    // would otherwise end up empty; the whole text is that one field.
    if parts.is_empty() && !text.is_empty() {
        if let Some(kind) = skeleton.first().and_then(|letter| skeleton_field_type(*letter)) {
            parts.push(serde_json::json!({
                "type": kind,
                "value": String::from_utf16_lossy(text),
            }));
        }
    }
    unsafe {
        fpi_close(iterator);
        date_close(formatter);
    }
    serde_json::to_string(&parts).ok()
}
