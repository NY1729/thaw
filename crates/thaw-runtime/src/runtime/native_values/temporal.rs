// Native support for the `Temporal` namespace. Every `Temporal` value is
// lowered as an ordinary object `{ timestamp, nanoseconds, calendar,
// [time_zone,] __temporal_<kind> }`: `timestamp` is the same
// epoch-millisecond `f64` `Date` uses, `nanoseconds` is the sub-millisecond
// remainder (0..999999), `calendar` is an ICU4X calendar identifier
// (`"iso8601"` by default), and `time_zone` is only on `ZonedDateTime`.
// Time zones use jiff's bundled tzdb, calendars use ICU4X, and `Now` reads a
// real nanosecond `CLOCK_REALTIME`. Calendar *arithmetic* (`add`/`subtract`/
// `since`/`until`) is still computed on the ISO/epoch timeline.

/// Reads the complete registered JavaScript string, including embedded NULs.
///
/// # Safety
/// `text` must be null or reference a live native string or C string.
unsafe fn temporal_input<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() { return None; }
    let text = unsafe { thaw_arena::NativeStr::from_ptr(text) };
    if text.to_bytes().contains(&0) { return None; }
    text.to_str().ok()
}

/// Parses an offset string (`"Z"`, `"+09:00"`, `"-0500"`, `"+09"`) into
/// seconds east of UTC, or `None`.
fn temporal_offset_seconds(text: &str) -> Option<i32> {
    if text == "Z" || text == "z" {
        return Some(0);
    }
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+')?),
    };
    if !rest.bytes().all(|byte| byte.is_ascii_digit() || byte == b':') { return None; }
    let (hours, minutes) = match rest.split_once(':') {
        Some((hours, minutes)) => (hours.parse::<i32>().ok()?, minutes.parse::<i32>().ok()?),
        None if rest.len() == 4 => (rest[..2].parse().ok()?, rest[2..].parse().ok()?),
        None if rest.len() == 2 => (rest.parse().ok()?, 0),
        None => return None,
    };
    if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

fn temporal_offset_string(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let magnitude = seconds.abs();
    let base = format!("{sign}{:02}:{:02}", magnitude / 3600, (magnitude % 3600) / 60);
    if magnitude % 60 == 0 { base } else { format!("{base}:{:02}", magnitude % 60) }
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

#[no_mangle]
/// Converts a plain ISO date/time stored on the UTC-like timeline into the
/// matching wall-clock time in `zone`. `part` is `0` for epoch milliseconds
/// and `1` for sub-millisecond nanoseconds.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_plain_to_zoned(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
    part: f64,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let Some(fields) = civil_from_timestamp(milliseconds) else {
        return f64::NAN;
    };
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
    let Some(zone) = jiff_time_zone(&zone) else {
        return f64::NAN;
    };
    let subsecond = fields.milliseconds as i32 * 1_000_000 + nanoseconds.round() as i32;
    let Ok(datetime) = jiff::civil::DateTime::new(
        fields.year as i16,
        fields.month as i8,
        fields.day as i8,
        fields.hours as i8,
        fields.minutes as i8,
        fields.seconds as i8,
        subsecond,
    ) else {
        return f64::NAN;
    };
    let Ok(zoned) = datetime.to_zoned(zone) else {
        return f64::NAN;
    };
    let total = zoned.timestamp().as_nanosecond();
    if part == 0.0 {
        total.div_euclid(1_000_000) as f64
    } else {
        total.rem_euclid(1_000_000) as f64
    }
}

#[no_mangle]
/// Replaces selected fields of a plain date or date-time. `present_mask`
/// distinguishes omitted fields from supplied non-finite values. Invalid
/// fields return NaN for the LLVM caller to raise a RangeError. `part` is
/// `0` for milliseconds and `1` for the sub-millisecond nanosecond remainder.
pub extern "C" fn thaw_temporal_with_fields(
    milliseconds: f64,
    nanoseconds: f64,
    year: f64,
    month: f64,
    day: f64,
    hour: f64,
    minute: f64,
    second: f64,
    millisecond: f64,
    microsecond: f64,
    nanosecond: f64,
    part: f64,
    reject_overflow: f64,
    present_mask: f64,
) -> f64 {
    let Some(fields) = civil_from_timestamp(milliseconds) else {
        return f64::NAN;
    };
    let replacements = [year, month, day, hour, minute, second, millisecond, microsecond, nanosecond];
    let mask = present_mask as u16;
    if replacements.iter().enumerate().any(|(index, value)| mask & (1 << index) != 0 && !value.is_finite()) {
        return f64::NAN;
    }
    let pick = |index: usize, replacement: f64, original: i64| {
        if mask & (1 << index) == 0 { original } else { replacement.trunc() as i64 }
    };
    let requested_year = if mask & 1 == 0 { fields.year as f64 } else { year.trunc() };
    if !requested_year.is_finite() || !(-1_000_000.0..=1_000_000.0).contains(&requested_year) {
        return f64::NAN;
    }
    let year = requested_year as i64;
    let requested_month = pick(1, month, fields.month as i64);
    let month = requested_month.clamp(1, 12);
    let requested_day = pick(2, day, fields.day as i64);
    if reject_overflow != 0.0 && (requested_month != month
        || requested_day < 1 || requested_day > days_in_month(year, month as u32) as i64) {
        return f64::NAN;
    }
    let day = requested_day.clamp(1, days_in_month(year, month as u32) as i64);
    let hour = pick(3, hour, fields.hours as i64);
    let minute = pick(4, minute, fields.minutes as i64);
    let second = pick(5, second, fields.seconds as i64);
    let old_subsecond = fields.milliseconds as i64 * 1_000_000 + nanoseconds.round() as i64;
    let millisecond = pick(6, millisecond, old_subsecond / 1_000_000);
    let microsecond = pick(7, microsecond, old_subsecond / 1_000 % 1_000);
    let nanosecond = pick(8, nanosecond, old_subsecond % 1_000);
    if !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
        || !(0..=999).contains(&millisecond)
        || !(0..=999).contains(&microsecond)
        || !(0..=999).contains(&nanosecond)
    {
        return f64::NAN;
    }
    let epoch_milliseconds = days_from_civil(year, month as u32, day as u32) as i128 * 86_400_000
        + hour as i128 * 3_600_000
        + minute as i128 * 60_000
        + second as i128 * 1_000
        + millisecond as i128;
    let clipped = time_clip(epoch_milliseconds as f64);
    if part == 0.0 || clipped.is_nan() {
        clipped
    } else {
        (microsecond * 1_000 + nanosecond) as f64
    }
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

thread_local! {
    static TEMPORAL_NOW_SNAPSHOT: std::cell::Cell<Option<(i64, i64)>> = const { std::cell::Cell::new(None) };
}

#[no_mangle]
/// `Temporal.Now.instant()` and friends: the current time as epoch
/// milliseconds.
pub extern "C" fn thaw_temporal_now() -> f64 {
    let now = realtime_now();
    TEMPORAL_NOW_SNAPSHOT.with(|snapshot| snapshot.set(Some(now)));
    now.0 as f64 * 1000.0 + (now.1 / 1_000_000) as f64
}

#[no_mangle]
/// The sub-millisecond nanoseconds of the current time, complementing
/// `thaw_temporal_now` for a nanosecond-precision `Temporal.Now`.
pub extern "C" fn thaw_temporal_now_nanos() -> f64 {
    TEMPORAL_NOW_SNAPSHOT.with(|snapshot| snapshot.take().unwrap_or_else(realtime_now).1.rem_euclid(1_000_000) as f64)
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
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return false; };
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
    let Some(calendar) = (unsafe { temporal_input(calendar) }) else { return false; };
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
    let Some(calendar) = (unsafe { temporal_input(calendar) }) else { return f64::NAN; };
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
    let Some(calendar) = (unsafe { temporal_input(calendar) }) else { return std::ptr::null(); };
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
    let Some(calendar) = (unsafe { temporal_input(calendar) }) else { return std::ptr::null(); };
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
        let Some(text) = (unsafe { temporal_input(text) }) else { return std::ptr::null(); };
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
/// `Temporal.PlainMonthDay.from(text)`: parses `"MM-DD"` (or a full
/// `YYYY-MM-DD`, taking its month/day) into the spec's 1972 reference
/// date, so `.monthCode`/`.day` and `toString` work.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_plain_month_day_from_string(
    text: *const c_char,
) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    let text = text.trim();
    let text = text.split_once('[').map_or(text, |(head, _)| head);
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(r"^(?:([0-9]{4})-)?([0-9]{2})-([0-9]{2})$").unwrap()
    });
    let Some(captures) = pattern.captures(text) else {
        return f64::NAN;
    };
    let month: u32 = captures[2].parse().unwrap_or(0);
    let day: u32 = captures[3].parse().unwrap_or(0);
    // The reference year is 1972 (a leap year), so `02-29` is representable.
    let validation_year = captures.get(1).and_then(|year| year.as_str().parse().ok()).unwrap_or(1972);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(validation_year, month) {
        return f64::NAN;
    }
    days_from_civil(1972, month, day) as f64 * 86_400_000.0
}

#[no_mangle]
/// `Temporal.ZonedDateTime.prototype.startOfDay` / `PlainDateTime`
/// equivalent in a zone: the instant of the zone's local midnight.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_start_of_day(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return f64::NAN;
    };
    let Ok(start) = zoned.start_of_day() else {
        return f64::NAN;
    };
    (start.timestamp().as_nanosecond().div_euclid(1_000_000)) as f64
}

#[no_mangle]
/// The length of the receiver's local day in hours. This observes daylight
/// saving transitions, so a day can contain 23 or 25 hours.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_hours_in_day(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return f64::NAN;
    };
    let Ok(start) = zoned.start_of_day() else {
        return f64::NAN;
    };
    let Ok(end) = start.tomorrow().and_then(|value| value.start_of_day()) else {
        return f64::NAN;
    };
    (end.timestamp().as_nanosecond() - start.timestamp().as_nanosecond()) as f64
        / 3_600_000_000_000.0
}

#[no_mangle]
/// The next (`direction > 0`) or previous time-zone transition. `part` is
/// `0` for epoch milliseconds and `1` for sub-millisecond nanoseconds. A
/// fixed-offset zone has no transitions and returns NaN.
///
/// # Safety
/// `zone` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_zoned_transition(
    milliseconds: f64,
    nanoseconds: f64,
    zone: *const c_char,
    direction: f64,
    part: f64,
) -> f64 {
    if zone.is_null() {
        return f64::NAN;
    }
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
    let Some(zoned) = zoned_for(milliseconds, nanoseconds, &zone) else {
        return f64::NAN;
    };
    let transition = if direction > 0.0 {
        zoned.time_zone().following(zoned.timestamp()).next()
    } else {
        zoned.time_zone().preceding(zoned.timestamp()).next()
    };
    let Some(transition) = transition else {
        return f64::NAN;
    };
    let total = transition.timestamp().as_nanosecond();
    if part == 0.0 {
        total.div_euclid(1_000_000) as f64
    } else {
        total.rem_euclid(1_000_000) as f64
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
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return std::ptr::null(); };
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
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return std::ptr::null(); };
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return std::ptr::null(); };
    let zone = if let Ok(zoned) = text.parse::<jiff::Zoned>() {
        zoned
            .time_zone()
            .iana_name()
            .map(str::to_string)
            .unwrap_or_else(|| temporal_offset_string(zoned.offset().seconds()))
    } else if let Ok(timestamp) = text.parse::<jiff::Timestamp>() {
        // No `[Zone]`: derive it from the trailing offset (an instant has
        // no zone of its own).
        let offset = text.rfind(['+', '-'])
            .and_then(|index| temporal_offset_seconds(&text[index..]))
            .unwrap_or_else(|| { let _ = timestamp; 0 });
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
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
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
    let Some(zone) = (unsafe { temporal_input(zone) }) else { return f64::NAN; };
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
fn parse_temporal_date_time(text: &str, instant: bool) -> Option<(f64, f64)> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"^([0-9]{4})(?:-([0-9]{2})(?:-([0-9]{2}))?)?(?:T([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:\.([0-9]{1,9}))?)?(Z|[+-][0-9]{2}:[0-9]{2})?)?$",
        )
        .unwrap()
    });
    // Drop any `[u-ca=...]`/`[Zone]` annotation before the date/time itself.
    let text = text.trim();
    let text = text.split_once('[').map_or(text, |(head, _)| head);
    let captures = pattern.captures(text)?;
    if instant && (captures.get(4).is_none() || captures.get(8).is_none()) { return None; }
    if !instant && captures.get(8).is_some_and(|offset| offset.as_str() == "Z") { return None; }
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
    if instant {
        if let Some(offset) = captures.get(8) {
            let offset = offset.as_str();
            if offset != "Z" {
                let sign = if offset.starts_with('-') { -1.0 } else { 1.0 };
                let offset_hours: f64 = offset[1..3].parse().ok()?;
                let offset_minutes: f64 = offset[4..6].parse().ok()?;
                if offset_hours > 23.0 || offset_minutes > 59.0 { return None; }
                timestamp -= sign * (offset_hours * 3_600_000.0 + offset_minutes * 60_000.0);
            }
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    parse_temporal_date_time(&text, true)
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    parse_temporal_date_time(&text, true)
        .map(|(_, nanoseconds)| nanoseconds)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// Plain date/time string as wall-clock milliseconds; numeric offsets are ignored.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_plain_date_time_from_string(text: *const c_char) -> f64 {
    if text.is_null() { return f64::NAN; }
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    parse_temporal_date_time(&text, false).map(|(ms, _)| ms).unwrap_or(f64::NAN)
}

#[no_mangle]
/// Sub-millisecond remainder for plain date/time string parsing.
///
/// # Safety
/// `text` must be null or a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_temporal_plain_date_time_nanos_from_string(text: *const c_char) -> f64 {
    if text.is_null() { return f64::NAN; }
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    parse_temporal_date_time(&text, false).map(|(_, ns)| ns).unwrap_or(f64::NAN)
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
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

#[no_mangle]
/// Rounds a Temporal nanosecond total by a positive increment. `mode` is
/// 0=halfExpand, 1=ceil, 2=floor, 3=trunc, 4=expand, 5=halfCeil,
/// 6=halfFloor, 7=halfTrunc, 8=halfEven.
pub extern "C" fn thaw_temporal_round(total: f64, increment: f64, mode: f64) -> f64 {
    if !total.is_finite() || !increment.is_finite() || increment <= 0.0 {
        return f64::NAN;
    }
    let quotient = total / increment;
    let rounded = match mode as i32 {
        1 => quotient.ceil(),
        2 => quotient.floor(),
        3 => quotient.trunc(),
        4 => {
            if quotient.is_sign_negative() { quotient.floor() } else { quotient.ceil() }
        }
        5..=8 => {
            let lower = quotient.floor();
            let fraction = quotient - lower;
            if fraction < 0.5 {
                lower
            } else if fraction > 0.5 {
                lower + 1.0
            } else {
                match mode as i32 {
                    5 => lower + 1.0,
                    6 => lower,
                    7 => quotient.trunc(),
                    _ if (lower as i128) % 2 == 0 => lower,
                    _ => lower + 1.0,
                }
            }
        }
        _ => {
            if quotient.is_sign_negative() {
                (quotient - 0.5).ceil()
            } else {
                (quotient + 0.5).floor()
            }
        }
    };
    rounded * increment
}

#[cfg(test)]
mod temporal_parse_tests {
    use super::*;

    #[test]
    fn instant_requires_offset_and_plain_date_time_keeps_wall_clock() {
        assert!(parse_temporal_date_time("2020-01-01T00:00", true).is_none());
        assert!(parse_temporal_date_time("2020-01-01", true).is_none());
        assert!(parse_temporal_date_time("2020-01-01T00:00Z", false).is_none());
        assert_eq!(parse_temporal_date_time("2020-01-01T00:00+01:00", false),
            parse_temporal_date_time("2020-01-01T00:00", false));
        assert!(parse_temporal_date_time("2020-01-01T00:00+00:99", true).is_none());
        assert!(parse_temporal_date_time("2020-01-01T00:00+24:00", true).is_none());
        assert_eq!(temporal_offset_seconds("+00:09"), Some(540));
        assert_eq!(temporal_offset_seconds("+00:99"), None);
    }

    #[test]
    fn registered_nul_suffix_is_rejected_at_string_boundaries() {
        let bytes = b"2020-01-01T00:00Z\0invalid\0";
        let pointer = bytes.as_ptr().cast();
        unsafe { thaw_arena::thaw_string_register(pointer, bytes.len() - 1) };
        assert!(unsafe { thaw_temporal_instant_from_string(pointer) }.is_nan());
        assert!(unsafe { thaw_temporal_plain_date_time_from_string(pointer) }.is_nan());
        assert!(unsafe { thaw_temporal_zoned_from_string(pointer) }.is_nan());
        assert!(unsafe { thaw_temporal_zoned_zone_from_string(pointer) }.is_null());
    }

    #[test]
    fn property_bag_year_is_not_date_utc_adjusted_or_clamped() {
        let make = |year| thaw_temporal_with_fields(0.0, 0.0, year, 1.0, 1.0,
            f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0, 0.0, 7.0);
        for year in [0, 99, 100, 50_000] {
            assert_eq!(civil_from_timestamp(make(year as f64)).unwrap().year, year as i64);
        }
        assert!(make(1_000_000.0).is_nan());
    }

    #[test]
    fn supplied_nonfinite_fields_do_not_use_omission_defaults() {
        let replace = |year, month, day, hour, mask| thaw_temporal_with_fields(
            0.0, 0.0, year, month, day, hour,
            f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0, 0.0, mask,
        );
        assert_eq!(replace(f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0), 0.0);
        assert!(replace(f64::NAN, f64::NAN, f64::NAN, f64::NAN, 1.0).is_nan());
        assert!(replace(f64::NAN, f64::INFINITY, f64::NAN, f64::NAN, 2.0).is_nan());
        assert!(replace(f64::NAN, f64::NAN, f64::NAN, f64::NAN, 4.0).is_nan());
        assert!(replace(f64::NAN, f64::NAN, f64::NAN, f64::NEG_INFINITY, 8.0).is_nan());
    }

    #[test]
    fn month_day_validates_its_input_year() {
        let invalid = std::ffi::CString::new("2023-02-29").unwrap();
        let valid = std::ffi::CString::new("02-29").unwrap();
        assert!(unsafe { thaw_temporal_plain_month_day_from_string(invalid.as_ptr()) }.is_nan());
        assert!(unsafe { thaw_temporal_plain_month_day_from_string(valid.as_ptr()) }.is_finite());
    }
}

#[cfg(test)]
mod temporal_round_tests {
    use super::thaw_temporal_round;

    #[test]
    fn rounds_ties_according_to_the_requested_mode() {
        assert_eq!(thaw_temporal_round(150.0, 100.0, 0.0), 200.0);
        assert_eq!(thaw_temporal_round(-150.0, 100.0, 0.0), -200.0);
        assert_eq!(thaw_temporal_round(250.0, 100.0, 8.0), 200.0);
        assert_eq!(thaw_temporal_round(150.0, 100.0, 8.0), 200.0);
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

/// `(name, nanoseconds)` for each `Duration` unit, largest first.
const TEMPORAL_DURATION_UNITS: [(&str, i128); 10] = [
    ("year", 365 * 86_400_000_000_000),
    ("month", 30 * 86_400_000_000_000),
    ("week", 7 * 86_400_000_000_000),
    ("day", 86_400_000_000_000),
    ("hour", 3_600_000_000_000),
    ("minute", 60_000_000_000),
    ("second", 1_000_000_000),
    ("millisecond", 1_000_000),
    ("microsecond", 1_000),
    ("nanosecond", 1),
];

#[no_mangle]
/// `Temporal.Duration.prototype.balance({ largestUnit })`: redistributes a
/// duration's total into components from `largestUnit` down. Unknown or
/// absent units default to `"nanosecond"`. Returns a components JSON
/// object.
///
/// # Safety
/// `largest_unit` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_duration_balance(
    milliseconds: f64,
    nanoseconds: f64,
    largest_unit: *const c_char,
) -> *const c_char {
    let unit = if largest_unit.is_null() {
        "nanosecond".to_string()
    } else {
        let Some(unit) = (unsafe { temporal_input(largest_unit) }) else { return std::ptr::null(); };
        unit.to_ascii_lowercase()
    };
    let start = TEMPORAL_DURATION_UNITS
        .iter()
        .position(|(name, _)| *name == unit || format!("{name}s") == unit)
        .unwrap_or(TEMPORAL_DURATION_UNITS.len() - 1);
    let total = (milliseconds.round() as i128) * 1_000_000 + nanoseconds.round() as i128;
    let mut remaining = total;
    let mut components: Vec<(&str, i128)> = Vec::with_capacity(TEMPORAL_DURATION_UNITS.len());
    for (index, (name, factor)) in TEMPORAL_DURATION_UNITS.iter().enumerate() {
        if index < start {
            components.push((name, 0));
        } else {
            components.push((name, remaining / factor));
            remaining %= factor;
        }
    }
    // The component names are singular here (for unit matching); the
    // `Duration` components object uses the plural spellings.
    let object = components
        .iter()
        .map(|(name, value)| (format!("{name}s"), serde_json::json!(value)))
        .collect::<serde_json::Map<_, _>>();
    let value = serde_json::Value::Object(object);
    arena_c_string(&value.to_string()).map_or(std::ptr::null(), |value| value.cast())
}

#[cfg(test)]
mod temporal_duration_balance_tests {
    use super::*;

    fn balance(milliseconds: f64, unit: &str) -> String {
        let unit = std::ffi::CString::new(unit).unwrap();
        let result = unsafe { thaw_temporal_duration_balance(milliseconds, 0.0, unit.as_ptr()) };
        assert!(!result.is_null());
        unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn balances_minutes_into_hours() {
        let json = balance(5_400_000.0, "hour");
        assert!(json.contains("\"hours\":1"), "{json}");
        assert!(json.contains("\"minutes\":30"), "{json}");
    }

    #[test]
    fn balances_hours_into_days() {
        let json = balance(108_000_000.0, "day");
        assert!(json.contains("\"days\":1"), "{json}");
        assert!(json.contains("\"hours\":6"), "{json}");
    }
}

#[no_mangle]
/// `PlainDate.prototype.since`/`until`: the calendar difference from
/// `from` to `to` as a components JSON object, in the requested
/// `largest_unit` (`"year"`/`"month"`/`"week"`/`"day"`; day default).
///
/// # Safety
/// `largest_unit` must be null or a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_temporal_date_difference(
    from: f64,
    to: f64,
    largest_unit: *const c_char,
) -> *const c_char {
    let unit = if largest_unit.is_null() {
        "day".to_string()
    } else {
        let Some(unit) = (unsafe { temporal_input(largest_unit) }) else { return std::ptr::null(); };
        unit.to_ascii_lowercase()
    };
    let date = |milliseconds: f64| -> Option<jiff::civil::Date> {
        let fields = civil_from_timestamp(milliseconds)?;
        jiff::civil::Date::new(
            fields.year as i16,
            fields.month as i8,
            fields.day as i8,
        )
        .ok()
    };
    let (Some(from_date), Some(to_date)) = (date(from), date(to)) else {
        return std::ptr::null();
    };
    let largest = match unit.as_str() {
        "year" | "years" => jiff::Unit::Year,
        "month" | "months" => jiff::Unit::Month,
        "week" | "weeks" => jiff::Unit::Week,
        _ => jiff::Unit::Day,
    };
    let Ok(span) = from_date.until(jiff::civil::DateDifference::new(to_date).largest(largest))
    else {
        return std::ptr::null();
    };
    let value = serde_json::json!({
        "years": span.get_years(),
        "months": span.get_months(),
        "weeks": span.get_weeks(),
        "days": span.get_days(),
        "hours": 0,
        "minutes": 0,
        "seconds": 0,
        "milliseconds": 0,
        "microseconds": 0,
        "nanoseconds": 0,
    });
    arena_c_string(&value.to_string()).map_or(std::ptr::null(), |value| value.cast())
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
        (unsafe { temporal_input(text) }).and_then(|text| text.parse::<jiff::Span>().ok())
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
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
    let Some(text) = (unsafe { temporal_input(text) }) else { return f64::NAN; };
    duration_string_to_milliseconds(&text)
        .map(|milliseconds| split_duration_nanoseconds(milliseconds).1)
        .unwrap_or(f64::NAN)
}

/// Reads one numeric field of a `Duration` components JSON object.
///
/// # Safety
/// `components` must point to a live thaw-std `Json` object.
unsafe fn duration_component(components: *const u8, name: &str) -> f64 {
    if components.is_null() {
        return 0.0;
    }
    // `components` is a thaw-std `Json` value (its own `Value` layout, not
    // `serde_json::Value`) -- read through thaw-std's exported accessors
    // rather than reinterpreting the pointer here (see `maps.rs`'s
    // `AnyKey` doc comment for why a Rust-level dependency isn't possible).
    unsafe extern "C" {
        fn thaw_json_get(value: *mut u8, key: *const c_char) -> *mut u8;
        fn thaw_json_as_number(value: *mut u8) -> f64;
    }
    let Ok(key) = std::ffi::CString::new(name) else {
        return 0.0;
    };
    let field = unsafe { thaw_json_get(components.cast_mut(), key.as_ptr()) };
    if field.is_null() {
        return 0.0;
    }
    unsafe { thaw_json_as_number(field) }
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
            let subsecond = milliseconds.abs() as i128 * 1_000_000
                + microseconds.abs() as i128 * 1_000
                + nanoseconds.abs() as i128;
            let seconds_text_value = seconds.abs() as i128 + subsecond / 1_000_000_000;
            seconds_text = seconds_text_value.to_string();
            let fraction = format!("{:09}", subsecond % 1_000_000_000);
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
