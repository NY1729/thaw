// Native approximations for the `Temporal` namespace, built on the same
// epoch-millisecond `f64` representation as `Date` (there is no
// nanosecond clock or timezone database). Every `Temporal` value is
// lowered as an ordinary object `{ timestamp, __temporal_<kind> }`, so the
// existing object codegen and `.timestamp` field access carry it; these
// functions only do the parsing, formatting, and arithmetic the methods
// need.

#[no_mangle]
/// `Temporal.Now.instant()` and friends: the current time in epoch
/// milliseconds, exactly `Date.now()`.
pub extern "C" fn thaw_temporal_now() -> f64 {
    thaw_date_now()
}

#[no_mangle]
/// `Temporal.Now.timeZoneId()`: thaw has no timezone database, so the only
/// zone is UTC.
pub extern "C" fn thaw_temporal_time_zone_id() -> *const c_char {
    arena_c_string("UTC").map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.Instant.from(text)`: reuses `Date.parse`'s ISO 8601 parser,
/// which already handles a `Z`/`+HH:mm` offset and returns `NaN` on
/// anything it doesn't recognize.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_instant_from_string(text: *const c_char) -> f64 {
    unsafe { thaw_date_parse(text) }
}

#[no_mangle]
/// `Temporal.Instant.prototype.toString()` / `.toJSON()`: ISO 8601 with a
/// `Z` offset. Temporal's nanosecond precision is approximated by the
/// `Date` representation's milliseconds.
pub extern "C" fn thaw_temporal_instant_to_string(timestamp: f64) -> *const c_char {
    thaw_date_to_iso_string(timestamp)
}

#[no_mangle]
/// `Temporal.PlainDate.prototype.toString()`: `YYYY-MM-DD` (no time or
/// offset), or a null pointer for an unrepresentable date.
pub extern "C" fn thaw_temporal_plain_date_to_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let text = format!("{:04}-{:02}-{:02}", fields.year, fields.month, fields.day);
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.PlainDateTime.prototype.toString()`: `YYYY-MM-DDTHH:MM:SS.sss`
/// (no offset).
pub extern "C" fn thaw_temporal_plain_date_time_to_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        fields.year,
        fields.month,
        fields.day,
        fields.hours,
        fields.minutes,
        fields.seconds,
        fields.milliseconds
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.PlainTime.prototype.toString()`: `HH:MM:SS.sss`.
pub extern "C" fn thaw_temporal_plain_time_to_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    let text = format!(
        "{:02}:{:02}:{:02}.{:03}",
        fields.hours, fields.minutes, fields.seconds, fields.milliseconds
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.PlainYearMonth.prototype.toString()`: `YYYY-MM`.
pub extern "C" fn thaw_temporal_plain_year_month_to_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let text = format!("{:04}-{:02}", fields.year, fields.month);
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.PlainMonthDay.prototype.toString()`: `MM-DD`.
pub extern "C" fn thaw_temporal_plain_month_day_to_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    let text = format!("{:02}-{:02}", fields.month, fields.day);
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.Instant.prototype.add`/`.subtract` and every `Plain*`
/// equivalent: shift a timestamp by a millisecond delta.
pub extern "C" fn thaw_temporal_shift(timestamp: f64, delta_milliseconds: f64) -> f64 {
    timestamp + delta_milliseconds
}

#[no_mangle]
/// `Temporal.*.compare(a, b)` and `.equals(other)`: `-1`/`0`/`1` (or `NaN`
/// when either side is an invalid timestamp), matching the spec's
/// comparator. `equals` is this call compared against `0` at the HIR level.
pub extern "C" fn thaw_temporal_compare(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left < right {
        -1.0
    } else if left > right {
        1.0
    } else {
        0.0
    }
}

/// Parses an ISO 8601 duration (`P1Y2M3DT4H5M6.5S`, any subset of the
/// components, optionally signed) into milliseconds. A month is
/// approximated as 30 days and a year as 365, since a duration has no
/// anchor date to resolve calendar lengths against.
fn duration_string_to_milliseconds(text: &str) -> Option<f64> {
    let text = text.trim();
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest.strip_prefix('P')?;
    let (date_part, time_part) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    let mut total = 0.0;
    let mut number = String::new();
    let mut any = false;
    for ch in date_part.chars() {
        if ch.is_ascii_digit() || ch == '.' || ch == '-' {
            number.push(ch);
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        any = true;
        total += match ch {
            'Y' => value * 365.0 * 86_400_000.0,
            'M' => value * 30.0 * 86_400_000.0,
            'W' => value * 7.0 * 86_400_000.0,
            'D' => value * 86_400_000.0,
            _ => return None,
        };
    }
    if !number.is_empty() {
        return None;
    }
    if let Some(time_part) = time_part {
        let mut number = String::new();
        for ch in time_part.chars() {
            if ch.is_ascii_digit() || ch == '.' || ch == '-' {
                number.push(ch);
                continue;
            }
            let value: f64 = number.parse().ok()?;
            number.clear();
            any = true;
            total += match ch {
                'H' => value * 3_600_000.0,
                'M' => value * 60_000.0,
                'S' => value * 1_000.0,
                _ => return None,
            };
        }
        if !number.is_empty() {
            return None;
        }
    }
    any.then_some(sign * total)
}

#[no_mangle]
/// `Temporal.Duration.from(text)`: an ISO 8601 duration as milliseconds.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_duration_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    duration_string_to_milliseconds(&text).unwrap_or(f64::NAN)
}

#[no_mangle]
/// `Temporal.Duration.prototype.total()`/`.toString()` support: a duration
/// held as milliseconds renders back as an ISO 8601 string with the
/// largest exact components (hours/minutes/seconds/milliseconds).
pub extern "C" fn thaw_temporal_duration_to_string(milliseconds: f64) -> *const c_char {
    if !milliseconds.is_finite() {
        return std::ptr::null();
    }
    let negative = milliseconds < 0.0;
    let mut remaining = milliseconds.abs().round() as i64;
    let hours = remaining / 3_600_000;
    remaining %= 3_600_000;
    let minutes = remaining / 60_000;
    remaining %= 60_000;
    let seconds = remaining / 1_000;
    let millis = remaining % 1_000;
    let mut parts = String::new();
    if hours != 0 {
        parts.push_str(&format!("{hours}H"));
    }
    if minutes != 0 {
        parts.push_str(&format!("{minutes}M"));
    }
    if seconds != 0 || millis != 0 {
        if millis != 0 {
            parts.push_str(&format!("{seconds}.{millis:03}S"));
        } else {
            parts.push_str(&format!("{seconds}S"));
        }
    }
    if parts.is_empty() {
        parts.push_str("0S");
    }
    let text = format!("{}PT{}", if negative { "-" } else { "" }, parts);
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}
