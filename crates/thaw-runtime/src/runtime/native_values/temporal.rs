// Native support for the `Temporal` namespace. Every `Temporal` value is
// lowered as an ordinary object `{ timestamp, nanoseconds, calendar,
// [time_zone,] __temporal_<kind> }`: `timestamp` is the same
// epoch-millisecond `f64` `Date` uses, `nanoseconds` is the sub-millisecond
// remainder (0..999999), `calendar` is an ICU4X calendar identifier
// (`"iso8601"` by default), and `time_zone` is only on `ZonedDateTime`.
// Time zones use jiff's bundled tzdb, calendars use ICU4X, and `Now` reads a
// real nanosecond `CLOCK_REALTIME`. Calendar *arithmetic* (`add`/`subtract`/
// `since`/`until`) is still computed on the ISO/epoch timeline.

/// Parses an offset string (`"Z"`, `"+09:00"`, `"-0500"`, `"+09"`) into
/// seconds east of UTC, or `None`.
fn temporal_offset_seconds(text: &str) -> Option<i32> {
    let text = text.trim();
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let (hours, minutes) = match rest.split_once(':') {
        Some((hours, minutes)) => (hours.parse::<i32>().ok()?, minutes.parse::<i32>().ok()?),
        None if rest.len() == 4 => (rest[..2].parse().ok()?, rest[2..].parse().ok()?),
        None if rest.len() == 2 => (rest.parse().ok()?, 0),
        None => return None,
    };
    Some(sign * (hours * 3600 + minutes * 60))
}

fn temporal_offset_string(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let magnitude = seconds.abs();
    format!("{sign}{:02}:{:02}", magnitude / 3600, (magnitude % 3600) / 60)
}

/// A jiff time zone for an IANA name, `"UTC"`, or a fixed offset string.
fn jiff_time_zone(name: &str) -> Option<jiff::tz::TimeZone> {
    let name = name.trim();
    if name.is_empty() || name == "UTC" {
        return Some(jiff::tz::TimeZone::UTC);
    }
    if let Ok(zone) = jiff::tz::TimeZone::get(name) {
        return Some(zone);
    }
    let seconds = temporal_offset_seconds(name)?;
    jiff::tz::Offset::from_seconds(seconds)
        .ok()
        .map(jiff::tz::TimeZone::fixed)
}

fn jiff_timestamp(milliseconds: f64, nanoseconds: f64) -> Option<jiff::Timestamp> {
    if !milliseconds.is_finite() {
        return None;
    }
    let total = (milliseconds.round() as i128) * 1_000_000 + nanoseconds.round() as i128;
    jiff::Timestamp::from_nanosecond(total).ok()
}

fn zoned_for(
    milliseconds: f64,
    nanoseconds: f64,
    zone: &str,
) -> Option<jiff::Zoned> {
    let time_zone = jiff_time_zone(zone)?;
    let timestamp = jiff_timestamp(milliseconds, nanoseconds)?;
    Some(jiff::Zoned::new(timestamp, time_zone))
}

/// The wall clock as `(seconds, nanoseconds since the epoch)` from
/// `CLOCK_REALTIME`, or the epoch on failure.
fn realtime_now() -> (i64, i64) {
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut timespec) } != 0 {
        return (0, 0);
    }
    (timespec.tv_sec, timespec.tv_nsec)
}

#[no_mangle]
/// `Temporal.Now.instant()` and friends: the current time as epoch
/// milliseconds.
pub extern "C" fn thaw_temporal_now() -> f64 {
    let (seconds, nanoseconds) = realtime_now();
    seconds as f64 * 1000.0 + (nanoseconds / 1_000_000) as f64
}

#[no_mangle]
/// The sub-millisecond nanoseconds of the current time, complementing
/// `thaw_temporal_now` for a nanosecond-precision `Temporal.Now`.
pub extern "C" fn thaw_temporal_now_nanos() -> f64 {
    (realtime_now().1 % 1_000_000) as f64
}

#[no_mangle]
/// Whether `zone` names a valid IANA time zone, `"UTC"`, or a fixed offset
/// string.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zone_valid(zone: *const c_char) -> bool {
    if zone.is_null() {
        return false;
    }
    let zone = unsafe { CStr::from_ptr(zone) }.to_string_lossy();
    jiff_time_zone(&zone).is_some()
}

/// Maps a Temporal calendar identifier to an ICU4X calendar.
fn temporal_calendar_kind(id: &str) -> Option<icu_calendar::AnyCalendarKind> {
    use icu_calendar::AnyCalendarKind as Kind;
    Some(match id.trim().to_ascii_lowercase().as_str() {
        "iso8601" => Kind::Iso,
        "gregory" => Kind::Gregorian,
        "buddhist" => Kind::Buddhist,
        "chinese" => Kind::Chinese,
        "coptic" => Kind::Coptic,
        "dangi" => Kind::Dangi,
        "ethiopic" => Kind::Ethiopian,
        "ethioaa" => Kind::EthiopianAmeteAlem,
        "hebrew" => Kind::Hebrew,
        "indian" => Kind::Indian,
        "islamic" | "islamic-rgsa" => Kind::HijriSimulatedMecca,
        "islamic-umalqura" => Kind::HijriUmmAlQura,
        "islamic-tbla" => Kind::HijriTabularTypeIIThursday,
        "islamic-civil" => Kind::HijriTabularTypeIIFriday,
        "japanese" => Kind::Japanese,
        "persian" => Kind::Persian,
        "roc" => Kind::Roc,
        _ => return None,
    })
}

fn temporal_calendar_date(
    milliseconds: f64,
    calendar: &str,
) -> Option<icu_calendar::Date<icu_calendar::AnyCalendar>> {
    let fields = civil_from_timestamp(milliseconds)?;
    let iso = icu_calendar::Date::try_new_iso(
        fields.year as i32,
        fields.month as u8,
        fields.day as u8,
    )
    .ok()?;
    let kind = temporal_calendar_kind(calendar)?;
    Some(iso.to_calendar(icu_calendar::AnyCalendar::new(kind)))
}

#[no_mangle]
/// Whether `calendar` is a calendar identifier thaw supports.
///
/// # Safety
/// `calendar` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_calendar_valid(calendar: *const c_char) -> bool {
    if calendar.is_null() {
        return false;
    }
    let calendar = unsafe { CStr::from_ptr(calendar) }.to_string_lossy();
    temporal_calendar_kind(&calendar).is_some()
}

#[no_mangle]
/// One field of a date in the given calendar. `field` is `0`=year,
/// `1`=month, `2`=day, `3`=dayOfWeek (1=Mon..7=Sun), `4`=dayOfYear,
/// `5`=daysInMonth, `6`=daysInYear, `7`=monthsInYear, `8`=inLeapYear
/// (1/0), `9`=eraYear.
///
/// # Safety
/// `calendar` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_calendar_field(
    milliseconds: f64,
    calendar: *const c_char,
    field: f64,
) -> f64 {
    if calendar.is_null() {
        return f64::NAN;
    }
    let calendar = unsafe { CStr::from_ptr(calendar) }.to_string_lossy();
    let Some(date) = temporal_calendar_date(milliseconds, &calendar) else {
        return f64::NAN;
    };
    match field as i64 {
        0 => date.year().extended_year() as f64,
        1 => date.month().number() as f64,
        2 => date.day_of_month().0 as f64,
        3 => date.weekday() as i32 as f64,
        4 => date.day_of_year().0 as f64,
        5 => date.days_in_month() as f64,
        6 => date.days_in_year() as f64,
        7 => date.months_in_year() as f64,
        8 => {
            if date.is_in_leap_year() {
                1.0
            } else {
                0.0
            }
        }
        _ => date.year().era_year_or_related_iso() as f64,
    }
}

#[no_mangle]
/// The `monthCode` of a date in the given calendar (e.g. `"M05"`, or
/// `"M05L"` for a leap month).
///
/// # Safety
/// `calendar` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_calendar_month_code(
    milliseconds: f64,
    calendar: *const c_char,
) -> *const c_char {
    if calendar.is_null() {
        return std::ptr::null();
    }
    let calendar = unsafe { CStr::from_ptr(calendar) }.to_string_lossy();
    let Some(date) = temporal_calendar_date(milliseconds, &calendar) else {
        return std::ptr::null();
    };
    let leap = if date.month().to_input().is_leap() {
        "L"
    } else {
        ""
    };
    let text = format!("M{:02}{leap}", date.month().number());
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// The `era` of a date in the given calendar (`"reiwa"`, `"am"`, ...), or
/// an empty string when the calendar has no era.
///
/// # Safety
/// `calendar` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_calendar_era(
    milliseconds: f64,
    calendar: *const c_char,
) -> *const c_char {
    if calendar.is_null() {
        return std::ptr::null();
    }
    let calendar = unsafe { CStr::from_ptr(calendar) }.to_string_lossy();
    let Some(date) = temporal_calendar_date(milliseconds, &calendar) else {
        return std::ptr::null();
    };
    let text = date
        .year()
        .era()
        .map(|era| era.era.as_str().to_string())
        .unwrap_or_default();
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// The `[u-ca=<id>]` calendar annotation of an ISO 8601 string, or
/// `"iso8601"` when absent.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_calendar_from_string(
    text: *const c_char,
) -> *const c_char {
    let annotation = if text.is_null() {
        None
    } else {
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        text.split_once("[u-ca=")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(id, _)| id.to_string())
    };
    let calendar = annotation.unwrap_or_else(|| "iso8601".to_string());
    arena_c_string(&calendar).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// A `PlainDate`-family calendar helper. `field` is `0`=daysInMonth,
/// `1`=daysInYear, `2`=monthsInYear, `3`=inLeapYear (1/0).
pub extern "C" fn thaw_temporal_plain_date_field(milliseconds: f64, field: f64) -> f64 {
    let Some(fields) = civil_from_timestamp(milliseconds) else {
        return f64::NAN;
    };
    let Ok(date) = jiff::civil::Date::new(
        fields.year as i16,
        fields.month as i8,
        fields.day as i8,
    ) else {
        return f64::NAN;
    };
    match field as i64 {
        0 => date.days_in_month() as f64,
        1 => date.days_in_year() as f64,
        2 => 12.0,
        _ => {
            if date.in_leap_year() {
                1.0
            } else {
                0.0
            }
        }
    }
}

#[no_mangle]
/// `Temporal.PlainDate.prototype.monthCode`: `"M01"`..`"M12"` (ISO).
pub extern "C" fn thaw_temporal_month_code(milliseconds: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(milliseconds) else {
        return std::ptr::null();
    };
    let text = format!("M{:02}", fields.month);
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.ZonedDateTime.prototype.toString()`: RFC 9557, e.g.
/// `2020-01-02T03:04:05.678+09:00[Asia/Tokyo]`.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_to_string(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
) -> *const c_char {
    if zone.is_null() {
        return std::ptr::null();
    }
    let zone = unsafe { CStr::from_ptr(zone) }.to_string_lossy();
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return std::ptr::null();
    };
    arena_c_string(&zoned.to_string()).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.ZonedDateTime.prototype.offset`: `±HH:MM`.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_offset(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
) -> *const c_char {
    if zone.is_null() {
        return std::ptr::null();
    }
    let zone = unsafe { CStr::from_ptr(zone) }.to_string_lossy();
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return std::ptr::null();
    };
    let text = temporal_offset_string(zoned.offset().seconds());
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Temporal.ZonedDateTime.from(text)`: the instant's epoch milliseconds.
/// Accepts an RFC 9557 string with a `[Zone]` annotation, or a plain
/// ISO 8601 string with an offset.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_from_string(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    if let Ok(zoned) = text.parse::<jiff::Zoned>() {
        let total = zoned.timestamp().as_nanosecond();
        return (total.div_euclid(1_000_000)) as f64;
    }
    match text.parse::<jiff::Timestamp>() {
        Ok(timestamp) => (timestamp.as_nanosecond().div_euclid(1_000_000)) as f64,
        Err(_) => f64::NAN,
    }
}

#[no_mangle]
/// The sub-millisecond nanoseconds of
/// `Temporal.ZonedDateTime.from(text)`.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_nanos_from_string(
    text: *const c_char,
) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let total = if let Ok(zoned) = text.parse::<jiff::Zoned>() {
        zoned.timestamp().as_nanosecond()
    } else if let Ok(timestamp) = text.parse::<jiff::Timestamp>() {
        timestamp.as_nanosecond()
    } else {
        return f64::NAN;
    };
    (total.rem_euclid(1_000_000)) as f64
}

#[no_mangle]
/// The time zone of `Temporal.ZonedDateTime.from(text)`: the `[Zone]`
/// annotation when present, otherwise `"UTC"` for a zero offset or the
/// offset string itself.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_zone_from_string(
    text: *const c_char,
) -> *const c_char {
    if text.is_null() {
        return std::ptr::null();
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    let zone = if let Ok(zoned) = text.parse::<jiff::Zoned>() {
        zoned
            .time_zone()
            .iana_name()
            .map(str::to_string)
            .unwrap_or_else(|| temporal_offset_string(zoned.offset().seconds()))
    } else if let Ok(timestamp) = text.parse::<jiff::Timestamp>() {
        // No `[Zone]`: derive it from the trailing offset (an instant has
        // no zone of its own).
        let offset = text
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .rsplit(['T', 't'])
            .next()
            .and_then(temporal_offset_seconds)
            .unwrap_or_else(|| {
                // A trailing `Z` (or no offset) is UTC.
                let _ = timestamp;
                0
            });
        if offset == 0 {
            "UTC".to_string()
        } else {
            temporal_offset_string(offset)
        }
    } else {
        return std::ptr::null();
    };
    arena_c_string(&zone).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// A `Temporal.ZonedDateTime` field in its own zone. `field` is `0`=year,
/// `1`=month, `2`=day, `3`=hour, `4`=minute, `5`=second, `6`=millisecond,
/// `7`=day of week (1=Monday..7=Sunday).
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_field(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
    field: f64,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let zone = unsafe { CStr::from_ptr(zone) }.to_string_lossy();
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return f64::NAN;
    };
    match field as i64 {
        0 => zoned.year() as f64,
        1 => zoned.month() as f64,
        2 => zoned.day() as f64,
        3 => zoned.hour() as f64,
        4 => zoned.minute() as f64,
        5 => zoned.second() as f64,
        6 => zoned.millisecond() as f64,
        _ => zoned.weekday().to_monday_one_offset() as f64,
    }
}

#[no_mangle]
/// The UTC timestamp (epoch milliseconds) of a `ZonedDateTime`'s local
/// wall clock, for its `toPlain*` casts. `mode` is `0`=date (local
/// midnight), `1`=time (local time-of-day on 1970-01-01), `2`=date-time.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_plain_timestamp(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
    mode: f64,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let zone = unsafe { CStr::from_ptr(zone) }.to_string_lossy();
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return f64::NAN;
    };
    let time_of_day = zoned.hour() as f64 * 3_600_000.0
        + zoned.minute() as f64 * 60_000.0
        + zoned.second() as f64 * 1_000.0
        + zoned.millisecond() as f64;
    let days = days_from_civil(zoned.year() as i64, zoned.month() as u32, zoned.day() as u32);
    match mode as i64 {
        0 => days as f64 * 86_400_000.0,
        1 => time_of_day,
        _ => days as f64 * 86_400_000.0 + time_of_day,
    }
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
    // Drop any `[u-ca=...]`/`[Zone]` annotation before the date/time itself.
    let text = text.trim();
    let text = text.split_once('[').map_or(text, |(head, _)| head);
    let captures = pattern.captures(text)?;
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
    // Drop any `[u-ca=...]`/`[Zone]` annotation.
    let text = text.split_once('[').map_or(text, |(head, _)| head);
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
/// The components of an ISO 8601 duration as a JSON object, so
/// `Duration.from(string)` preserves unnormalized components the same way
/// its object-literal form does. All-zero on an unparsable input.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_duration_components_json(
    text: *const c_char,
) -> *const c_char {
    let span = if text.is_null() {
        None
    } else {
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        text.parse::<jiff::Span>().ok()
    };
    let (years, months, weeks, days, hours, minutes, seconds, milliseconds, microseconds, nanoseconds) =
        match &span {
            Some(span) => (
                span.get_years() as f64,
                span.get_months() as f64,
                span.get_weeks() as f64,
                span.get_days() as f64,
                span.get_hours() as f64,
                span.get_minutes() as f64,
                span.get_seconds() as f64,
                span.get_milliseconds() as f64,
                span.get_microseconds() as f64,
                span.get_nanoseconds() as f64,
            ),
            None => (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        };
    let value = serde_json::json!({
        "years": years,
        "months": months,
        "weeks": weeks,
        "days": days,
        "hours": hours,
        "minutes": minutes,
        "seconds": seconds,
        "milliseconds": milliseconds,
        "microseconds": microseconds,
        "nanoseconds": nanoseconds,
    });
    arena_c_string(&value.to_string()).map_or(std::ptr::null(), |value| value.cast())
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

/// Reads one numeric field of a `Duration` components JSON object.
///
/// # Safety
/// `components` must point to a live `serde_json::Value` object.
unsafe fn duration_component(components: *const u8, name: &str) -> f64 {
    if components.is_null() {
        return 0.0;
    }
    let value = unsafe { &*(components as *const serde_json::Value) };
    value.get(name).and_then(serde_json::Value::as_f64).unwrap_or(0.0)
}

#[no_mangle]
/// `Temporal.Duration.prototype.toString()` from its component breakdown,
/// preserving an unnormalized duration (`{ minutes: 90 }` -> `"PT90M"`).
///
/// # Safety
/// `components` must point to a live `serde_json::Value` object.
pub unsafe extern "C" fn thaw_temporal_duration_to_string_components(
    components: *const u8,
) -> *const c_char {
    let field = |name: &str| unsafe { duration_component(components, name) };
    let (years, months, weeks, days) = (
        field("years"),
        field("months"),
        field("weeks"),
        field("days"),
    );
    let (hours, minutes, seconds) = (
        field("hours"),
        field("minutes"),
        field("seconds"),
    );
    let (milliseconds, microseconds, nanoseconds) = (
        field("milliseconds"),
        field("microseconds"),
        field("nanoseconds"),
    );
    // A `Duration` has a consistent sign, so the first non-zero component
    // (in canonical order) gives it.
    let order = [
        years, months, weeks, days, hours, minutes, seconds, milliseconds, microseconds,
        nanoseconds,
    ];
    let negative = order.iter().find(|value| **value != 0.0).is_some_and(|value| *value < 0.0);
    let mut text = String::from(if negative { "-P" } else { "P" });
    let mut any_date = false;
    for (value, suffix) in [
        (years, "Y"),
        (months, "M"),
        (weeks, "W"),
        (days, "D"),
    ] {
        if value != 0.0 {
            text.push_str(&format!("{}{suffix}", value.abs()));
            any_date = true;
        }
    }
    let _ = any_date;
    let has_time = [hours, minutes, seconds, milliseconds, microseconds, nanoseconds]
        .iter()
        .any(|value| *value != 0.0);
    if has_time {
        text.push('T');
        if hours != 0.0 {
            text.push_str(&format!("{}H", hours.abs()));
        }
        if minutes != 0.0 {
            text.push_str(&format!("{}M", minutes.abs()));
        }
        if seconds != 0.0 || milliseconds != 0.0 || microseconds != 0.0 || nanoseconds != 0.0 {
            let mut seconds_text = format!("{}", seconds.abs());
            let fraction = format!(
                "{:03}{:03}{:03}",
                milliseconds.abs() as i64,
                microseconds.abs() as i64,
                nanoseconds.abs() as i64,
            );
            let fraction = fraction.trim_end_matches('0');
            if !fraction.is_empty() {
                seconds_text.push('.');
                seconds_text.push_str(fraction);
            }
            text.push_str(&seconds_text);
            text.push('S');
        }
    }
    if text == "P" {
        text.push_str("T0S");
    }
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
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
