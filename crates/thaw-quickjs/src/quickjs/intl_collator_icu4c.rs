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
}

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
