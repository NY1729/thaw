// Native approximations for the `Temporal` namespace. Every `Temporal`
// value is lowered as an ordinary object
// `{ timestamp, nanoseconds, __temporal_<kind> }`: `timestamp` is the same
// epoch-millisecond `f64` `Date` uses, and `nanoseconds` is the
// sub-millisecond remainder (0..999999), so `.timestamp` field access and
// the object codegen carry it while fractional seconds keep nanosecond
// precision. There is no nanosecond clock or timezone database, so `Now`
// and the zone are millisecond/UTC.

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

/// A fractional-seconds string (1-9 digits) as `(milliseconds,
/// sub-millisecond nanoseconds)`.
fn temporal_fraction_nanos(fraction: &str) -> Option<(f64, f64)> {
    if fraction.is_empty()
        || fraction.len() > 9
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let mut digits = fraction.to_string();
    while digits.len() < 9 {
        digits.push('0');
    }
    let nanos: i64 = digits.parse().ok()?;
    Some(((nanos / 1_000_000) as f64, (nanos % 1_000_000) as f64))
}

/// Parses a full ISO 8601 date(-time) into `(milliseconds,
/// sub-millisecond nanoseconds)`, or `None`. Handles a `Z`/`+HH:mm`
/// offset and up to 9 fractional-second digits (which `Date.parse`
/// truncates to milliseconds).
fn parse_temporal_date_time(text: &str) -> Option<(f64, f64)> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:\d{2})?)?$",
        )
        .unwrap()
    });
    let captures = pattern.captures(text.trim())?;
    let field = |index: usize| -> Option<i64> { captures.get(index)?.as_str().parse().ok() };
    let year = field(1)?;
    let month = field(2).unwrap_or(1) as u32;
    let day = field(3).unwrap_or(1) as u32;
    let hours = field(4).unwrap_or(0);
    let minutes = field(5).unwrap_or(0);
    let seconds = field(6).unwrap_or(0);
    let (fraction_ms, fraction_ns) = match captures.get(7) {
        Some(fraction) => temporal_fraction_nanos(fraction.as_str())?,
        None => (0.0, 0.0),
    };
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month) as i64).contains(&(day as i64))
        || !(0..=23).contains(&hours)
        || !(0..=59).contains(&minutes)
        || !(0..=59).contains(&seconds)
    {
        return None;
    }
    let mut timestamp = days_from_civil(year, month, day) as f64 * 86_400_000.0
        + hours as f64 * 3_600_000.0
        + minutes as f64 * 60_000.0
        + seconds as f64 * 1_000.0
        + fraction_ms;
    if let Some(offset) = captures.get(8) {
        let offset = offset.as_str();
        if offset != "Z" {
            let sign = if offset.starts_with('-') { -1.0 } else { 1.0 };
            let offset_hours: f64 = offset[1..3].parse().ok()?;
            let offset_minutes: f64 = offset[4..6].parse().ok()?;
            timestamp -= sign * (offset_hours * 3_600_000.0 + offset_minutes * 60_000.0);
        }
    }
    Some((timestamp, fraction_ns))
}

/// The `.NNNNNNNNN` suffix for a total of sub-second nanoseconds, with
/// trailing zeros removed, or empty for a whole second.
fn temporal_fraction_suffix(nanoseconds: i64) -> String {
    if nanoseconds == 0 {
        return String::new();
    }
    let mut text = format!(
        ".{:03}{:06}",
        nanoseconds / 1_000_000,
        nanoseconds % 1_000_000
    );
    while text.ends_with('0') {
        text.pop();
    }
    text
}

#[no_mangle]
/// `Temporal.Instant.from(text)` / `PlainDateTime.from`: the epoch
/// milliseconds.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_instant_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    parse_temporal_date_time(&text)
        .map(|(milliseconds, _)| milliseconds)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// The sub-millisecond nanoseconds of `Temporal.Instant.from(text)` /
/// `PlainDateTime.from` (0..999999).
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_instant_nanos_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    parse_temporal_date_time(&text)
        .map(|(_, nanoseconds)| nanoseconds)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// `Temporal.Instant.prototype.toString()` / `.toJSON()`: ISO 8601 with a
/// `Z` offset and up to 9 fractional-second digits.
pub extern "C" fn thaw_temporal_instant_to_string(
    timestamp: f64,
    nanoseconds: f64,
) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let suffix = temporal_fraction_suffix(
        fields.milliseconds as i64 * 1_000_000 + nanoseconds.round() as i64,
    );
    let text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{suffix}Z",
        fields.year, fields.month, fields.day, fields.hours, fields.minutes, fields.seconds
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.Instant.prototype.epochNanoseconds` as a decimal string (for
/// the HIR to wrap in a BigInt), computed in `i128` so any date is exact.
pub extern "C" fn thaw_temporal_epoch_nanoseconds(
    timestamp: f64,
    nanoseconds: f64,
) -> *const c_char {
    if !timestamp.is_finite() {
        return std::ptr::null();
    }
    let total =
        (timestamp.round() as i128) * 1_000_000 + (nanoseconds.round() as i128);
    arena_c_string(&total.to_string()).map_or(std::ptr::null(), |value| value.cast())
}

/// Parses `HH:MM`, `HH:MM:SS`, or `HH:MM:SS.sssssssss` (optionally
/// preceded by a date/`T`) into `(milliseconds since midnight,
/// sub-millisecond nanoseconds)`, or `None` on a bad shape or
/// out-of-range field.
fn parse_time_of_day(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    // A full date-time string: keep only the time part.
    let time = match text.rsplit_once('T') {
        Some((_, time)) => time.trim_end_matches('Z'),
        None => text,
    };
    let time = match time.split_once('+') {
        Some((time, _)) => time,
        None => time,
    };
    let mut parts = time.split(':');
    let hours: i64 = parts.next()?.trim().parse().ok()?;
    let minutes: i64 = parts.next()?.trim().parse().ok()?;
    let (seconds, fraction_ms, fraction_ns) = match parts.next() {
        Some(second) => {
            let (whole, fraction) = match second.split_once('.') {
                Some((whole, fraction)) => (whole, fraction),
                None => (second, ""),
            };
            let seconds: i64 = whole.trim().parse().ok()?;
            let (fraction_ms, fraction_ns) = if fraction.is_empty() {
                (0.0, 0.0)
            } else {
                temporal_fraction_nanos(fraction.trim())?
            };
            (seconds, fraction_ms, fraction_ns)
        }
        None => (0, 0.0, 0.0),
    };
    if parts.next().is_some()
        || !(0..=23).contains(&hours)
        || !(0..=59).contains(&minutes)
        || !(0..=59).contains(&seconds)
    {
        return None;
    }
    Some((
        (hours * 3_600_000 + minutes * 60_000 + seconds * 1_000) as f64 + fraction_ms,
        fraction_ns,
    ))
}

#[no_mangle]
/// `Temporal.PlainTime.from(text)`: the time-of-day as milliseconds since
/// midnight (stored as a 1970-01-01 timestamp, so the shared civil
/// formatter renders it correctly). A time-only string or the time part of
/// a full ISO date-time both work.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_plain_time_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    parse_time_of_day(&text)
        .map(|(milliseconds, _)| milliseconds)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// The sub-millisecond nanoseconds of `Temporal.PlainTime.from(text)`.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_plain_time_nanos_from_string(
    text: *const c_char,
) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    parse_time_of_day(&text)
        .map(|(_, nanoseconds)| nanoseconds)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// `Temporal.PlainDate.prototype.toString()`: `YYYY-MM-DD` (no time or
/// offset), or a null pointer for an unrepresentable date.
pub extern "C" fn thaw_temporal_plain_date_to_string(
    timestamp: f64,
    _nanoseconds: f64,
) -> *const c_char {
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
/// `Temporal.PlainDateTime.prototype.toString()`:
/// `YYYY-MM-DDTHH:MM:SS[.fffffffff]` (no offset).
pub extern "C" fn thaw_temporal_plain_date_time_to_string(
    timestamp: f64,
    nanoseconds: f64,
) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let suffix = temporal_fraction_suffix(
        fields.milliseconds as i64 * 1_000_000 + nanoseconds.round() as i64,
    );
    let text = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{suffix}",
        fields.year, fields.month, fields.day, fields.hours, fields.minutes, fields.seconds
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.PlainTime.prototype.toString()`: `HH:MM:SS[.fffffffff]`.
pub extern "C" fn thaw_temporal_plain_time_to_string(
    timestamp: f64,
    nanoseconds: f64,
) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    let suffix = temporal_fraction_suffix(
        fields.milliseconds as i64 * 1_000_000 + nanoseconds.round() as i64,
    );
    let text = format!(
        "{:02}:{:02}:{:02}{suffix}",
        fields.hours, fields.minutes, fields.seconds
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// One component of a `Duration` held as milliseconds: `unit` is
/// `0`=days, `1`=hours, `2`=minutes, `3`=seconds, `4`=milliseconds. Each
/// component is the remainder after the larger ones, so
/// `{ hours: 2, minutes: 30 }` reports `minutes` 30, not 150.
pub extern "C" fn thaw_temporal_duration_component(milliseconds: f64, unit: f64) -> f64 {
    if !milliseconds.is_finite() {
        return f64::NAN;
    }
    let sign = if milliseconds < 0.0 { -1.0 } else { 1.0 };
    let total = milliseconds.abs().trunc() as i64;
    let component = match unit as i64 {
        0 => total / 86_400_000,
        1 => (total % 86_400_000) / 3_600_000,
        2 => (total % 3_600_000) / 60_000,
        3 => (total % 60_000) / 1_000,
        _ => total % 1_000,
    };
    sign * component as f64
}

#[no_mangle]
/// `Temporal.PlainYearMonth.prototype.toString()`: `YYYY-MM`.
pub extern "C" fn thaw_temporal_plain_year_month_to_string(
    timestamp: f64,
    _nanoseconds: f64,
) -> *const c_char {
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
pub extern "C" fn thaw_temporal_plain_month_day_to_string(
    timestamp: f64,
    _nanoseconds: f64,
) -> *const c_char {
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
pub extern "C" fn thaw_temporal_compare(
    left_milliseconds: f64,
    left_nanoseconds: f64,
    right_milliseconds: f64,
    right_nanoseconds: f64,
) -> f64 {
    if left_milliseconds.is_nan() || right_milliseconds.is_nan() {
        return f64::NAN;
    }
    if left_milliseconds != right_milliseconds {
        return if left_milliseconds < right_milliseconds {
            -1.0
        } else {
            1.0
        };
    }
    if left_nanoseconds < right_nanoseconds {
        -1.0
    } else if left_nanoseconds > right_nanoseconds {
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

/// Splits a duration held as (possibly fractional) milliseconds into whole
/// milliseconds (floored) and a non-negative sub-millisecond remainder.
fn split_duration_nanoseconds(milliseconds: f64) -> (f64, f64) {
    let total_nanoseconds = (milliseconds * 1_000_000.0).round();
    let whole_milliseconds = (total_nanoseconds / 1_000_000.0).floor();
    (
        whole_milliseconds,
        total_nanoseconds - whole_milliseconds * 1_000_000.0,
    )
}

#[no_mangle]
/// `Temporal.Duration.from(text)`: an ISO 8601 duration's whole
/// milliseconds.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_duration_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    duration_string_to_milliseconds(&text)
        .map(|milliseconds| split_duration_nanoseconds(milliseconds).0)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// The sub-millisecond nanoseconds of `Temporal.Duration.from(text)`.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_duration_nanos_from_string(
    text: *const c_char,
) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    duration_string_to_milliseconds(&text)
        .map(|milliseconds| split_duration_nanoseconds(milliseconds).1)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// `Temporal.Duration.prototype.toString()`: an ISO 8601 string with the
/// largest exact components (hours/minutes/seconds, with up to 9
/// fractional-second digits).
pub extern "C" fn thaw_temporal_duration_to_string(
    milliseconds: f64,
    nanoseconds: f64,
) -> *const c_char {
    if !milliseconds.is_finite() {
        return std::ptr::null();
    }
    let total_nanoseconds =
        (milliseconds.round() as i128) * 1_000_000 + nanoseconds.round() as i128;
    let negative = total_nanoseconds < 0;
    let total = total_nanoseconds.abs();
    let hours = total / 3_600_000_000_000;
    let minutes = (total / 60_000_000_000) % 60;
    let seconds_total = total % 60_000_000_000;
    let seconds = seconds_total / 1_000_000_000;
    let fraction = seconds_total % 1_000_000_000;
    let mut parts = String::new();
    if hours != 0 {
        parts.push_str(&format!("{hours}H"));
    }
    if minutes != 0 {
        parts.push_str(&format!("{minutes}M"));
    }
    if seconds != 0 || fraction != 0 {
        if fraction != 0 {
            let mut seconds_text = format!("{seconds}.{fraction:09}");
            while seconds_text.ends_with('0') {
                seconds_text.pop();
            }
            parts.push_str(&seconds_text);
            parts.push('S');
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
