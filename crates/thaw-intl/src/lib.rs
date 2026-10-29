//! Native (AOT, QuickJS-free) ECMA-402 subset: `String.prototype.localeCompare`,
//! `Number.prototype.toLocaleString` and `Date.prototype.toLocale*String`.
//!
//! The icu4x primitives are the same source files thaw-quickjs's `Intl` polyfill
//! uses (`include!`d below) so both hosts share one implementation; this file only
//! adds the option resolution that polyfill does in JavaScript. thaw-cli links this
//! archive only when the generated program references a `thaw_intl_*` symbol.
//!
//! Calling convention: every string is a NUL-terminated WTF-8 pointer, `locales` is
//! the first requested tag ("" = process default), `opts` is `key=value` pairs
//! joined by U+001F, as lowered by thaw-hir's `lower_intl_arguments`.

#![allow(dead_code, clippy::too_many_arguments, clippy::manual_map)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use thaw_arena::wtf8_decode_utf16;

include!("../../thaw-quickjs/src/quickjs/intl.rs");
include!("../../thaw-quickjs/src/quickjs/intl_time_zone_names.rs");
include!("../../thaw-quickjs/src/quickjs/intl_locale.rs");
include!("../../thaw-quickjs/src/quickjs/intl_datetime_skeleton.rs");
include!("../../thaw-quickjs/src/quickjs/intl_datetime.rs");
include!("../../thaw-quickjs/src/quickjs/intl_number.rs");
include!("../../thaw-quickjs/src/quickjs/intl_number_style.rs");
include!("../../thaw-quickjs/src/quickjs/intl_collator.rs");

/// Replaces each 3-byte WTF-8 lone surrogate with U+FFFD (same as thaw-quickjs's
/// `wtf8_to_icu_text`, which lives in its segmenter file).
fn wtf8_to_icu_text(bytes: &[u8]) -> String {
    let mut fixed = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0xED
            && index + 2 < bytes.len()
            && (0xA0..=0xBF).contains(&bytes[index + 1])
            && bytes[index + 2] & 0xC0 == 0x80
        {
            fixed.extend_from_slice("\u{FFFD}".as_bytes());
            index += 3;
        } else {
            fixed.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(fixed).unwrap_or_default()
}

unsafe fn bytes<'a>(value: *const c_char) -> &'a [u8] {
    if value.is_null() {
        &[]
    } else {
        unsafe { CStr::from_ptr(value) }.to_bytes()
    }
}

unsafe fn text(value: *const c_char) -> String {
    wtf8_to_icu_text(unsafe { bytes(value) })
}

fn to_native(value: &str) -> *const c_char {
    thaw_arena::arena_string(value.as_bytes()).cast_const()
}

const HALF_EXPAND: fixed_decimal::SignedRoundingMode =
    fixed_decimal::SignedRoundingMode::Unsigned(fixed_decimal::UnsignedRoundingMode::HalfExpand);

type Options = HashMap<String, String>;

fn parse_options(raw: &str) -> Options {
    raw.split('\u{1f}')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The first requested tag, else the process default (LC_ALL, LC_MESSAGES, LANG),
/// like Node's ICU default locale.
// ponytail: an invalid tag silently falls back to the default (Node throws RangeError).
fn resolve_locale(locales: &str) -> String {
    let requested = locales
        .split(',')
        .map(str::trim)
        .find(|tag| !tag.is_empty());
    if let Some(tag) = requested {
        if tag.parse::<icu_locale::Locale>().is_ok() {
            return tag.to_string();
        }
    }
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(value) = std::env::var(name) else {
            continue;
        };
        let tag = value
            .split(['.', '@'])
            .next()
            .unwrap_or("")
            .replace('_', "-");
        if tag.is_empty() || tag == "C" || tag == "POSIX" {
            break;
        }
        if tag.parse::<icu_locale::Locale>().is_ok() {
            return tag;
        }
    }
    "en-US".to_string()
}

fn flag(options: &Options, key: &str) -> bool {
    options.get(key).is_some_and(|value| value == "true")
}

// ---------------------------------------------------------------- collation

thread_local! {
    // A sort calls compare O(n log n) times with one option set; building a collator
    // loads its data tables, so keep the last one.
    static COLLATOR: RefCell<Option<(String, Option<icu_collator::Collator>)>> = const { RefCell::new(None) };
}

/// `a.localeCompare(b, locales, options)`: negative, zero or positive.
///
/// # Safety
/// All pointers must be null or NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_intl_compare(
    a: *const c_char,
    b: *const c_char,
    locales: *const c_char,
    opts: *const c_char,
) -> i32 {
    let (a, b) = unsafe { (bytes(a), bytes(b)) };
    if a == b {
        return 0;
    }
    let locale = resolve_locale(&unsafe { text(locales) });
    let raw = unsafe { text(opts) };
    let options = parse_options(&raw);
    let key = format!("{locale}\u{1f}{raw}");
    let (a_text, b_text) = (wtf8_to_icu_text(a), wtf8_to_icu_text(b));
    let order = COLLATOR.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let collator = intl_collator_build(
                &locale,
                options.get("sensitivity").map_or("variant", String::as_str),
                flag(&options, "ignorePunctuation"),
                flag(&options, "numeric"),
                options.get("caseFirst").map_or("false", String::as_str),
            );
            *cell = Some((key, collator));
        }
        cell.as_ref()
            .and_then(|(_, collator)| collator.as_ref())
            .map(|collator| collator.as_borrowed().compare(&a_text, &b_text))
    });
    match order.unwrap_or_else(|| wtf8_decode_utf16(a).cmp(&wtf8_decode_utf16(b))) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

// ------------------------------------------------------------------- number

fn integer_option(options: &Options, key: &str) -> Option<i16> {
    options
        .get(key)?
        .parse::<f64>()
        .ok()
        .map(|value| value as i16)
}

fn format_number(value: f64, locale: &str, options: &Options) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-∞" } else { "∞" }.to_string();
    }
    let style = options.get("style").map_or("decimal", String::as_str);
    let currency = options
        .get("currency")
        .map(|code| code.to_ascii_uppercase());
    let mut decimal: fixed_decimal::Decimal = format!("{value}").parse().unwrap_or_default();
    if style == "percent" {
        decimal.multiply_pow10(2);
    }
    let (default_min, default_max) = match style {
        "currency" => {
            let digits = currency
                .as_deref()
                .and_then(intl_currency_fraction_digits)
                .map_or(2, i16::from);
            (digits, digits)
        }
        "percent" => (0, 0),
        _ => (0, 3),
    };
    let min_sig = integer_option(options, "minimumSignificantDigits");
    let max_sig = integer_option(options, "maximumSignificantDigits");
    if min_sig.is_some() || max_sig.is_some() {
        let min_sig = min_sig.unwrap_or(1);
        let max_sig = max_sig.unwrap_or(21);
        if !decimal.absolute.is_zero() {
            decimal.round_with_mode(
                decimal.absolute.nonzero_magnitude_start() - max_sig + 1,
                HALF_EXPAND,
            );
        }
        let start = if decimal.absolute.is_zero() {
            0
        } else {
            decimal.absolute.nonzero_magnitude_start()
        };
        decimal.pad_end(start - min_sig + 1);
    } else {
        let min_frac = integer_option(options, "minimumFractionDigits");
        let max_frac = integer_option(options, "maximumFractionDigits");
        let min_frac_resolved =
            min_frac.unwrap_or_else(|| default_min.min(max_frac.unwrap_or(default_min)));
        let max_frac_resolved = max_frac.unwrap_or_else(|| default_max.max(min_frac_resolved));
        decimal.round_with_mode(-max_frac_resolved, HALF_EXPAND);
        decimal.pad_end(-min_frac_resolved);
    }
    decimal.pad_start(integer_option(options, "minimumIntegerDigits").unwrap_or(1));
    let mut digits = decimal.to_string();
    if options
        .get("signDisplay")
        .is_some_and(|display| display == "never")
    {
        digits = digits.trim_start_matches('-').to_string();
    }
    let grouping = !matches!(
        options.get("useGrouping").map(String::as_str),
        Some("false")
    );
    match style {
        "percent" => intl_percent_format(locale, &digits, grouping),
        "currency" => currency.as_deref().and_then(|code| {
            let display = options
                .get("currencyDisplay")
                .map_or("symbol", String::as_str);
            intl_currency_format(locale, &digits, code, display, grouping)
        }),
        _ => None,
    }
    .unwrap_or_else(|| intl_number_format(locale, &digits, grouping))
}

/// `value.toLocaleString(locales, options)` for a number.
///
/// # Safety
/// `locales` and `opts` must be null or NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_intl_number(
    value: f64,
    locales: *const c_char,
    opts: *const c_char,
) -> *const c_char {
    let locale = resolve_locale(&unsafe { text(locales) });
    let options = parse_options(&unsafe { text(opts) });
    to_native(&format_number(value, &locale, &options))
}

// --------------------------------------------------------------------- date

fn local_time_zone() -> String {
    if let Ok(name) = std::env::var("TZ") {
        let name = name.trim_start_matches(':');
        if !name.is_empty() {
            return name.to_string();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|target| {
            let target = target.to_string_lossy().into_owned();
            target
                .split_once("zoneinfo/")
                .map(|(_, name)| name.to_string())
        })
        .unwrap_or_else(|| "UTC".to_string())
}

fn parts_text(parts_json: &str) -> Option<String> {
    let parts: serde_json::Value = serde_json::from_str(parts_json).ok()?;
    let parts = parts.as_array().filter(|parts| !parts.is_empty())?;
    Some(
        parts
            .iter()
            .filter_map(|part| part["value"].as_str())
            .collect(),
    )
}

/// `ToDateTimeOptions` (ECMA-402 11.1.1): `required`/`defaults` are 0 = any/all,
/// 1 = date, 2 = time.
fn resolve_date_fields(options: &mut Options, kind: i32) {
    const DATE: [&str; 4] = ["weekday", "year", "month", "day"];
    const TIME: [&str; 5] = [
        "dayPeriod",
        "hour",
        "minute",
        "second",
        "fractionalSecondDigits",
    ];
    let styled = options.contains_key("dateStyle") || options.contains_key("timeStyle");
    if styled {
        let date: &[(&str, &str)] = match options.get("dateStyle").map(String::as_str) {
            Some("full") => &[
                ("weekday", "long"),
                ("year", "numeric"),
                ("month", "long"),
                ("day", "numeric"),
            ],
            Some("long") => &[("year", "numeric"), ("month", "long"), ("day", "numeric")],
            Some("medium") => &[("year", "numeric"), ("month", "short"), ("day", "numeric")],
            Some(_) => &[
                ("year", "2-digit"),
                ("month", "numeric"),
                ("day", "numeric"),
            ],
            None => &[],
        };
        let time: &[(&str, &str)] = match options.get("timeStyle").map(String::as_str) {
            Some("full") => &[
                ("hour", "numeric"),
                ("minute", "2-digit"),
                ("second", "2-digit"),
                ("timeZoneName", "long"),
            ],
            Some("long") => &[
                ("hour", "numeric"),
                ("minute", "2-digit"),
                ("second", "2-digit"),
                ("timeZoneName", "short"),
            ],
            Some("medium") => &[
                ("hour", "numeric"),
                ("minute", "2-digit"),
                ("second", "2-digit"),
            ],
            Some(_) => &[("hour", "numeric"), ("minute", "2-digit")],
            None => &[],
        };
        for (key, value) in date.iter().chain(time) {
            options.insert((*key).to_string(), (*value).to_string());
        }
        return;
    }
    let has = |keys: &[&str]| keys.iter().any(|key| options.contains_key(*key));
    let need_defaults = !((kind != 2 && has(&DATE)) || (kind != 1 && has(&TIME)));
    if need_defaults {
        let mut fill = |keys: &[&str]| {
            for key in keys {
                options.insert((*key).to_string(), "numeric".to_string());
            }
        };
        if kind != 2 {
            fill(&["year", "month", "day"]);
        }
        if kind != 1 {
            fill(&["hour", "minute", "second"]);
        }
    }
}

fn offset_string(offset_minutes: i64, two_digit_hour: bool) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let absolute = offset_minutes.abs();
    let (hours, minutes) = (absolute / 60, absolute % 60);
    let hour = if two_digit_hour {
        format!("{hours:02}")
    } else {
        hours.to_string()
    };
    let minute = if two_digit_hour || minutes != 0 {
        format!(":{minutes:02}")
    } else {
        String::new()
    };
    format!("GMT{sign}{hour}{minute}")
}

/// Port of `intlTimeZoneName` (intl.js) for the non-generic styles.
fn time_zone_name(style: &str, locale: &str, zoned: &serde_json::Value) -> String {
    let offset = zoned["offsetMinutes"].as_i64().unwrap_or(0);
    match style {
        "longOffset" => offset_string(offset, true),
        "long" | "longGeneric" => {
            let generic = style == "longGeneric";
            intl_time_zone_name(
                locale,
                zoned["timeZone"].as_str().unwrap_or("UTC"),
                zoned["timestampMs"].as_f64().unwrap_or(0.0),
                offset as i32,
                generic,
            )
            .or_else(|| {
                zoned[if generic {
                    "longGenericName"
                } else {
                    "longName"
                }]
                .as_str()
                .map(str::to_string)
            })
            .unwrap_or_else(|| offset_string(offset, true))
        }
        _ => {
            let abbreviation = zoned["abbreviation"].as_str().unwrap_or("");
            if style != "shortOffset"
                && !abbreviation.is_empty()
                && abbreviation.chars().all(|c| c.is_ascii_alphabetic())
            {
                abbreviation.to_string()
            } else {
                offset_string(offset, false)
            }
        }
    }
}

fn format_date(timestamp: f64, locale: &str, options: &Options, kind: i32) -> String {
    if !timestamp.is_finite() || timestamp.abs() > 8.64e15 {
        return "Invalid Date".to_string();
    }
    let mut options = options.clone();
    resolve_date_fields(&mut options, kind);
    let zone = options
        .get("timeZone")
        .cloned()
        .unwrap_or_else(local_time_zone);
    // ponytail: an unknown timeZone formats as UTC (Node throws RangeError).
    let zoned_json = [zone.as_str(), "UTC"]
        .into_iter()
        .map(|name| intl_zoned_parts_json(name, timestamp.floor()))
        .find(|json| !json.contains(r#""valid":false"#))
        .unwrap_or_default();
    let zoned: serde_json::Value = serde_json::from_str(&zoned_json).unwrap_or_default();

    let mut json = serde_json::Map::new();
    for key in [
        "weekday",
        "era",
        "year",
        "month",
        "day",
        "hour",
        "minute",
        "second",
        "hourCycle",
    ] {
        if let Some(value) = options.get(key) {
            json.insert(key.to_string(), value.clone().into());
        }
    }
    if let Some(value) = options.get("hour12") {
        json.insert("hour12".to_string(), (value == "true").into());
    }
    let options_json = serde_json::Value::Object(json).to_string();
    let styled = options.contains_key("dateStyle") || options.contains_key("timeStyle");

    let skeleton = (!styled)
        .then(|| {
            parts_text(&intl_datetime_skeleton_parts_native(
                locale,
                &options_json,
                &zoned_json,
            ))
        })
        .flatten();
    let mut rendered = skeleton
        .or_else(|| {
            let mut parts: serde_json::Value = serde_json::from_str(
                &intl_datetime_format_parts_json(locale, &options_json, &zoned_json),
            )
            .ok()?;
            for part in parts.as_array_mut()? {
                let style_of = |key: &str| options.get(key).map(String::as_str);
                let kind_name = part["type"].as_str().unwrap_or("").to_string();
                let value = part["value"].as_str().unwrap_or("").to_string();
                if styled || !value.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                // icu4x renders numeric fields unpadded: apply '2-digit', and strip the
                // calendar-period zero padding that 'numeric' must not have.
                let fixed = match (style_of(&kind_name), value.len()) {
                    (Some("2-digit"), 1) => format!("0{value}"),
                    (Some("numeric"), 2)
                        if matches!(kind_name.as_str(), "month" | "day")
                            && value.starts_with('0') =>
                    {
                        value[1..].to_string()
                    }
                    _ => value,
                };
                part["value"] = fixed.into();
            }
            parts_text(&parts.to_string())
        })
        .unwrap_or_default();
    if let Some(style) = options.get("timeZoneName") {
        if !rendered.is_empty() {
            rendered.push(' ');
        }
        rendered.push_str(&time_zone_name(style, locale, &zoned));
    }
    rendered
}

/// `date.toLocaleString/toLocaleDateString/toLocaleTimeString(locales, options)`;
/// `kind` is 0 (any), 1 (date) or 2 (time).
///
/// # Safety
/// `locales` and `opts` must be null or NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn thaw_intl_date(
    timestamp: f64,
    locales: *const c_char,
    opts: *const c_char,
    kind: f64,
) -> *const c_char {
    let locale = resolve_locale(&unsafe { text(locales) });
    let options = parse_options(&unsafe { text(opts) });
    to_native(&format_date(timestamp, &locale, &options, kind as i32))
}
