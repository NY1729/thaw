/// Converts a day count relative to the Unix epoch (1970-01-01) into a
/// proleptic-Gregorian `(year, month, day)` triple, `month` and `day` both
/// 1-based. Howard Hinnant's `civil_from_days` algorithm (public domain),
/// valid over the full `i64` range with no leap-year special casing.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Inverse of `civil_from_days`: converts a proleptic-Gregorian `(year,
/// month 1-12, day 1-31)` into a day count relative to the Unix epoch.
/// Howard Hinnant's `days_from_civil` algorithm (public domain).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

/// The specification's `MakeDay(year, month, date)`: normalizes an
/// out-of-range zero-based `month` (folding the overflow into `year`) and
/// then an out-of-range `date`, computing the day count of the resulting
/// Gregorian calendar date. This is how JavaScript `Date` setters roll
/// over -- `setMonth(12)` on a January date advances into next January,
/// `setDate(0)` moves to the last day of the previous month. Returns `NaN`
/// if any input isn't finite.
fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !year.is_finite() || !month.is_finite() || !date.is_finite() {
        return f64::NAN;
    }
    let year = year.trunc();
    let month = month.trunc();
    let year_offset = (month / 12.0).floor();
    let normalized_year = (year + year_offset) as i64;
    let normalized_month = (month - year_offset * 12.0) as u32 + 1;
    let day_of_first = days_from_civil(normalized_year, normalized_month, 1);
    day_of_first as f64 + date.trunc() - 1.0
}

/// The specification's `MakeDate(day, time)`.
fn make_date(day: f64, time_within_day_ms: f64) -> f64 {
    day * 86_400_000.0 + time_within_day_ms
}

/// The milliseconds elapsed since local midnight for a decomposed
/// timestamp, i.e. the specification's `TimeWithinDay`.
fn time_within_day_ms(fields: &CivilDateTime) -> f64 {
    fields.hours as f64 * 3_600_000.0
        + fields.minutes as f64 * 60_000.0
        + fields.seconds as f64 * 1_000.0
        + fields.milliseconds as f64
}

/// `Date` setters treat a non-finite receiver timestamp as `+0`
/// (1970-01-01T00:00:00.000Z) rather than propagating `NaN`, matching the
/// specification (for example `Date.prototype.setMonth` starts from
/// `LocalTime(this value)`, substituting `+0` when that's `NaN`) --
/// setting one field of an otherwise-Invalid Date produces a valid one.
fn civil_for_setter(timestamp: f64) -> CivilDateTime {
    let effective = if timestamp.is_finite() { timestamp } else { 0.0 };
    civil_from_timestamp(effective).expect("finite timestamp always decomposes")
}

/// The calendar fields of a JavaScript timestamp, always UTC: there is no
/// host timezone database, so "local" `Date` methods simply alias their UTC
/// counterparts. `month` and `day` are 1-based; `weekday` is 0-6 starting
/// Sunday, matching `Date.prototype.getDay`.
#[derive(Debug, PartialEq)]
struct CivilDateTime {
    year: i64,
    month: u32,
    day: u32,
    weekday: u32,
    hours: u32,
    minutes: u32,
    seconds: u32,
    milliseconds: u32,
}

/// Splits a JavaScript timestamp (milliseconds since the Unix epoch) into
/// its calendar fields, or `None` for a non-finite timestamp.
fn civil_from_timestamp(timestamp: f64) -> Option<CivilDateTime> {
    if !timestamp.is_finite() {
        return None;
    }
    let millis_total = timestamp.floor() as i64;
    let days = millis_total.div_euclid(86_400_000);
    let ms_of_day = millis_total.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    Some(CivilDateTime {
        year,
        month,
        day,
        weekday: (days.rem_euclid(7) + 4).rem_euclid(7) as u32,
        hours: (ms_of_day / 3_600_000) as u32,
        minutes: ((ms_of_day / 60_000) % 60) as u32,
        seconds: ((ms_of_day / 1000) % 60) as u32,
        milliseconds: (ms_of_day % 1000) as u32,
    })
}

#[no_mangle]
/// The current time as a JavaScript timestamp (milliseconds since the Unix
/// epoch), matching `Date.now()`.
pub extern "C" fn thaw_date_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[no_mangle]
/// `Date.UTC(year, month?, date?, hours?, minutes?, seconds?, ms?)`: the
/// generated code always supplies every parameter, defaulting an omitted
/// trailing one to `Date.UTC`'s own spec default (month 0, date 1, and 0
/// for the rest) rather than reading it from an existing receiver, since
/// there is none. A two-digit `year` in `[0, 99]` is interpreted as
/// `1900 + year`, matching the specification's legacy behavior (shared
/// with the `Date(...)` constructor's numeric-argument form, which isn't
/// implemented separately since it would just call this).
pub extern "C" fn thaw_date_utc(
    year: f64,
    month: f64,
    date: f64,
    hours: f64,
    minutes: f64,
    seconds: f64,
    milliseconds: f64,
) -> f64 {
    let year = if year.is_finite() && (0.0..=99.0).contains(&year.trunc()) {
        year.trunc() + 1900.0
    } else {
        year
    };
    if !hours.is_finite() || !minutes.is_finite() || !seconds.is_finite() || !milliseconds.is_finite() {
        return f64::NAN;
    }
    make_date(
        make_day(year, month, date),
        hours.trunc() * 3_600_000.0
            + minutes.trunc() * 60_000.0
            + seconds.trunc() * 1_000.0
            + milliseconds.trunc(),
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[no_mangle]
/// Parses `text` against the ECMA-262 "Date Time String Format" (a
/// restricted ISO 8601 profile: `YYYY`, `YYYY-MM`, or `YYYY-MM-DD`,
/// optionally followed by `THH:mm`, `THH:mm:ss`, or `THH:mm:ss.sss`, and
/// then an optional `Z` or `+HH:mm`/`-HH:mm` offset), returning the
/// resulting timestamp, matching `Date.parse` and the `Date(text)`
/// constructor overload. Other date string formats are implementation-
/// defined by the specification and are not supported here -- they parse
/// as `NaN` (`Invalid Date`) rather than being rejected at compile time,
/// matching a real engine encountering a format it doesn't recognize.
/// Out-of-range fields (an invalid day for the given month, including
/// leap years, or an hour/minute/second outside `0-23`/`0-59`) also parse
/// as `NaN`, since the specification does not roll these over the way
/// `Date.UTC`/the setters do. A date-only form and a date-time form with
/// no offset are both interpreted as UTC, since there is no host timezone
/// database to make "local" time mean anything else.
///
/// # Safety
/// `text` must be null or point to a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_date_parse(text: *const c_char) -> f64 {
    if text.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{3}))?)?(Z|[+-]\d{2}:\d{2})?)?$",
        )
        .unwrap()
    });
    let Some(captures) = pattern.captures(&text) else {
        return f64::NAN;
    };
    let field = |index: usize| -> Option<i64> { captures.get(index)?.as_str().parse().ok() };
    let year = field(1).unwrap();
    let month = field(2).unwrap_or(1) as u32;
    let day = field(3).unwrap_or(1) as u32;
    let hours = field(4).unwrap_or(0);
    let minutes = field(5).unwrap_or(0);
    let seconds = field(6).unwrap_or(0);
    let milliseconds = field(7).unwrap_or(0);
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month) as i64).contains(&(day as i64))
        || !(0..=23).contains(&hours)
        || !(0..=59).contains(&minutes)
        || !(0..=59).contains(&seconds)
    {
        return f64::NAN;
    }
    let day_count = days_from_civil(year, month, day) as f64;
    let mut timestamp = day_count * 86_400_000.0
        + hours as f64 * 3_600_000.0
        + minutes as f64 * 60_000.0
        + seconds as f64 * 1_000.0
        + milliseconds as f64;
    if let Some(offset) = captures.get(8) {
        let offset = offset.as_str();
        if offset != "Z" {
            let sign = if offset.starts_with('-') { -1.0 } else { 1.0 };
            let offset_hours: f64 = offset[1..3].parse().unwrap();
            let offset_minutes: f64 = offset[4..6].parse().unwrap();
            timestamp -= sign * (offset_hours * 3_600_000.0 + offset_minutes * 60_000.0);
        }
    }
    timestamp
}

macro_rules! date_field_getter {
    ($name:ident, $field:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(timestamp: f64) -> f64 {
            civil_from_timestamp(timestamp)
                .map(|fields| fields.$field as f64)
                .unwrap_or(f64::NAN)
        }
    };
}

date_field_getter!(thaw_date_get_full_year, year);
date_field_getter!(thaw_date_get_day, weekday);
date_field_getter!(thaw_date_get_hours, hours);
date_field_getter!(thaw_date_get_minutes, minutes);
date_field_getter!(thaw_date_get_seconds, seconds);
date_field_getter!(thaw_date_get_milliseconds, milliseconds);

#[no_mangle]
/// `Date.prototype.getMonth`: 0-based, matching JavaScript (January is 0).
pub extern "C" fn thaw_date_get_month(timestamp: f64) -> f64 {
    civil_from_timestamp(timestamp)
        .map(|fields| (fields.month - 1) as f64)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// `Date.prototype.getDate`: the 1-based day of the month.
pub extern "C" fn thaw_date_get_date(timestamp: f64) -> f64 {
    civil_from_timestamp(timestamp)
        .map(|fields| fields.day as f64)
        .unwrap_or(f64::NAN)
}

#[no_mangle]
/// Renders `timestamp` as an ISO 8601 / RFC 3339 string with millisecond
/// precision and a `Z` (UTC) offset, matching `Date.prototype.toISOString`.
/// Returns a null pointer for a non-finite timestamp (to be reported as a
/// `RangeError: Invalid time value`, matching the specification), or when
/// the year falls outside the 4-digit range `toISOString` requires (years
/// outside `[0, 9999]` need the specification's `+/-YYYYYY` extended
/// format, which this does not produce).
///
/// # Safety
/// The returned pointer, if non-null, is arena-allocated and must not be
/// freed by the caller.
pub extern "C" fn thaw_date_to_iso_string(timestamp: f64) -> *const c_char {
    let Some(fields) = civil_from_timestamp(timestamp) else {
        return std::ptr::null();
    };
    if !(0..=9999).contains(&fields.year) {
        return std::ptr::null();
    }
    let CivilDateTime {
        year,
        month,
        day,
        hours,
        minutes,
        seconds,
        milliseconds,
        ..
    } = fields;
    let text = format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}Z"
    );
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

const WEEKDAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The date portion shared by `toDateString` and `toString`: `"Www Mmm
/// dd yyyy"`.
fn date_string_part(fields: &CivilDateTime) -> String {
    format!(
        "{} {} {:02} {:04}",
        WEEKDAY_NAMES[fields.weekday as usize],
        MONTH_NAMES[(fields.month - 1) as usize],
        fields.day,
        fields.year
    )
}

/// The time portion shared by `toTimeString` and `toString`. There is no
/// host timezone database, so the offset is always UTC's, rendered the way
/// Node.js does when its own timezone is UTC.
fn time_string_part(fields: &CivilDateTime) -> String {
    format!(
        "{:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        fields.hours, fields.minutes, fields.seconds
    )
}

fn arena_c_string_or_invalid_date(text: Option<String>) -> *const c_char {
    let text = text.unwrap_or_else(|| "Invalid Date".to_string());
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// `Date.prototype.toDateString`: `"Www Mmm dd yyyy"`, or the literal
/// string `"Invalid Date"` for a non-finite timestamp (unlike
/// `toISOString`, this does not signal failure with a null pointer --
/// there is nothing for the generated code to check and throw on).
pub extern "C" fn thaw_date_to_date_string(timestamp: f64) -> *const c_char {
    arena_c_string_or_invalid_date(civil_from_timestamp(timestamp).map(|fields| date_string_part(&fields)))
}

#[no_mangle]
/// `Date.prototype.toTimeString`: `"hh:mm:ss GMT+0000 (Coordinated
/// Universal Time)"` (there is no host timezone database, so this is
/// always UTC's offset and name), or `"Invalid Date"`.
pub extern "C" fn thaw_date_to_time_string(timestamp: f64) -> *const c_char {
    arena_c_string_or_invalid_date(civil_from_timestamp(timestamp).map(|fields| time_string_part(&fields)))
}

#[no_mangle]
/// `Date.prototype.toString`: the date and time portions joined by a
/// space, or `"Invalid Date"`.
pub extern "C" fn thaw_date_to_string(timestamp: f64) -> *const c_char {
    arena_c_string_or_invalid_date(
        civil_from_timestamp(timestamp)
            .map(|fields| format!("{} {}", date_string_part(&fields), time_string_part(&fields))),
    )
}

#[no_mangle]
/// `Date.prototype.toUTCString`: the RFC 7231 `IMF-fixdate`-shaped format
/// `Date.prototype.toUTCString` actually specifies, `"Www, dd Mmm yyyy
/// hh:mm:ss GMT"`, or `"Invalid Date"`.
pub extern "C" fn thaw_date_to_utc_string(timestamp: f64) -> *const c_char {
    arena_c_string_or_invalid_date(civil_from_timestamp(timestamp).map(|fields| {
        format!(
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
            WEEKDAY_NAMES[fields.weekday as usize],
            fields.day,
            MONTH_NAMES[(fields.month - 1) as usize],
            fields.year,
            fields.hours,
            fields.minutes,
            fields.seconds
        )
    }))
}

#[no_mangle]
/// `Date.prototype.setFullYear`: `year`, `month` (0-based) and `date` are
/// always explicit here -- the generated code fills in an omitted trailing
/// argument by reading the receiver's current value first, matching how
/// the specification consults `MonthFromTime`/`DateFromTime`. Returns the
/// new timestamp, which the caller assigns back into the receiver.
pub extern "C" fn thaw_date_set_full_year(timestamp: f64, year: f64, month: f64, date: f64) -> f64 {
    let fields = civil_for_setter(timestamp);
    make_date(make_day(year, month, date), time_within_day_ms(&fields))
}

#[no_mangle]
/// `Date.prototype.setMonth`: `month` is 0-based; `year` is always taken
/// from the receiver's current value (there is no `setYear`-style year
/// override here).
pub extern "C" fn thaw_date_set_month(timestamp: f64, month: f64, date: f64) -> f64 {
    let fields = civil_for_setter(timestamp);
    make_date(
        make_day(fields.year as f64, month, date),
        time_within_day_ms(&fields),
    )
}

#[no_mangle]
/// `Date.prototype.setDate`: the 1-based day of the month, replacing only
/// that field of the receiver's current value.
pub extern "C" fn thaw_date_set_date(timestamp: f64, date: f64) -> f64 {
    let fields = civil_for_setter(timestamp);
    make_date(
        make_day(fields.year as f64, (fields.month - 1) as f64, date),
        time_within_day_ms(&fields),
    )
}

#[no_mangle]
/// `Date.prototype.setHours`: `hours`, `minutes`, `seconds` and
/// `milliseconds` are always explicit here, following the same
/// fill-in-from-the-current-value convention as `setFullYear`.
pub extern "C" fn thaw_date_set_hours(
    timestamp: f64,
    hours: f64,
    minutes: f64,
    seconds: f64,
    milliseconds: f64,
) -> f64 {
    let fields = civil_for_setter(timestamp);
    let day = days_from_civil(fields.year, fields.month, fields.day) as f64;
    if !hours.is_finite() || !minutes.is_finite() || !seconds.is_finite() || !milliseconds.is_finite() {
        return f64::NAN;
    }
    make_date(
        day,
        hours.trunc() * 3_600_000.0
            + minutes.trunc() * 60_000.0
            + seconds.trunc() * 1_000.0
            + milliseconds.trunc(),
    )
}

#[no_mangle]
/// `Date.prototype.setMinutes`: the receiver's current hour is kept;
/// `minutes`, `seconds` and `milliseconds` are always explicit here.
pub extern "C" fn thaw_date_set_minutes(
    timestamp: f64,
    minutes: f64,
    seconds: f64,
    milliseconds: f64,
) -> f64 {
    let fields = civil_for_setter(timestamp);
    let day = days_from_civil(fields.year, fields.month, fields.day) as f64;
    if !minutes.is_finite() || !seconds.is_finite() || !milliseconds.is_finite() {
        return f64::NAN;
    }
    make_date(
        day,
        fields.hours as f64 * 3_600_000.0
            + minutes.trunc() * 60_000.0
            + seconds.trunc() * 1_000.0
            + milliseconds.trunc(),
    )
}

#[no_mangle]
/// `Date.prototype.setSeconds`: the receiver's current hour and minute are
/// kept; `seconds` and `milliseconds` are always explicit here.
pub extern "C" fn thaw_date_set_seconds(timestamp: f64, seconds: f64, milliseconds: f64) -> f64 {
    let fields = civil_for_setter(timestamp);
    let day = days_from_civil(fields.year, fields.month, fields.day) as f64;
    if !seconds.is_finite() || !milliseconds.is_finite() {
        return f64::NAN;
    }
    make_date(
        day,
        fields.hours as f64 * 3_600_000.0
            + fields.minutes as f64 * 60_000.0
            + seconds.trunc() * 1_000.0
            + milliseconds.trunc(),
    )
}

#[no_mangle]
/// `Date.prototype.setMilliseconds`: the receiver's current hour, minute
/// and second are kept.
pub extern "C" fn thaw_date_set_milliseconds(timestamp: f64, milliseconds: f64) -> f64 {
    let fields = civil_for_setter(timestamp);
    if !milliseconds.is_finite() {
        return f64::NAN;
    }
    make_date(
        days_from_civil(fields.year, fields.month, fields.day) as f64,
        fields.hours as f64 * 3_600_000.0
            + fields.minutes as f64 * 60_000.0
            + fields.seconds as f64 * 1_000.0
            + milliseconds.trunc(),
    )
}

#[cfg(test)]
mod date_native_tests {
    use super::*;

    #[test]
    fn epoch_is_1970_01_01_thursday() {
        assert_eq!(
            civil_from_timestamp(0.0),
            Some(CivilDateTime {
                year: 1970,
                month: 1,
                day: 1,
                weekday: 4,
                hours: 0,
                minutes: 0,
                seconds: 0,
                milliseconds: 0,
            })
        );
    }

    #[test]
    fn matches_known_timestamp() {
        assert_eq!(
            civil_from_timestamp(1_704_067_200_000.0),
            Some(CivilDateTime {
                year: 2024,
                month: 1,
                day: 1,
                weekday: 1,
                hours: 0,
                minutes: 0,
                seconds: 0,
                milliseconds: 0,
            })
        );
    }

    #[test]
    fn handles_pre_epoch_timestamps() {
        assert_eq!(
            civil_from_timestamp(-86_400_000.0 + 500.0),
            Some(CivilDateTime {
                year: 1969,
                month: 12,
                day: 31,
                weekday: 3,
                hours: 0,
                minutes: 0,
                seconds: 0,
                milliseconds: 500,
            })
        );
    }

    #[test]
    fn non_finite_timestamps_are_none() {
        assert_eq!(civil_from_timestamp(f64::NAN), None);
        assert_eq!(civil_from_timestamp(f64::INFINITY), None);
    }

    #[test]
    fn days_from_civil_round_trips_civil_from_days() {
        for days in [-800_000_i64, -1, 0, 1, 19_723, 730_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days, "days={days}");
        }
    }

    #[test]
    fn set_month_rolls_overflow_into_next_year() {
        let jan_1_2024 = 1_704_067_200_000.0;
        // December (month index 11) of the same year.
        assert_eq!(
            thaw_date_set_month(jan_1_2024, 11.0, 1.0),
            days_from_civil(2024, 12, 1) as f64 * 86_400_000.0
        );
        // Month index 12 (one past December) rolls into next January.
        assert_eq!(
            thaw_date_set_month(jan_1_2024, 12.0, 1.0),
            days_from_civil(2025, 1, 1) as f64 * 86_400_000.0
        );
    }

    #[test]
    fn set_date_zero_moves_to_last_day_of_previous_month() {
        let mar_15_2024 = days_from_civil(2024, 3, 15) as f64 * 86_400_000.0;
        // 2024 is a leap year, so day 0 of March is February 29th.
        assert_eq!(
            thaw_date_set_date(mar_15_2024, 0.0),
            days_from_civil(2024, 2, 29) as f64 * 86_400_000.0
        );
    }

    #[test]
    fn set_hours_overflow_rolls_into_next_day() {
        let midnight = days_from_civil(2024, 1, 1) as f64 * 86_400_000.0;
        assert_eq!(
            thaw_date_set_hours(midnight, 25.0, 0.0, 0.0, 0.0),
            days_from_civil(2024, 1, 2) as f64 * 86_400_000.0 + 3_600_000.0
        );
    }

    #[test]
    fn setters_treat_invalid_receiver_as_epoch() {
        assert_eq!(
            thaw_date_set_milliseconds(f64::NAN, 500.0),
            500.0
        );
    }

    #[test]
    fn setters_propagate_nan_argument() {
        assert!(thaw_date_set_full_year(0.0, f64::NAN, 0.0, 1.0).is_nan());
        assert!(thaw_date_set_milliseconds(0.0, f64::NAN).is_nan());
    }

    #[test]
    fn to_iso_string_formats_and_rejects_nan() {
        let text = unsafe { CStr::from_ptr(thaw_date_to_iso_string(1_704_067_200_500.0)) }
            .to_str()
            .unwrap();
        assert_eq!(text, "2024-01-01T00:00:00.500Z");
        assert!(thaw_date_to_iso_string(f64::NAN).is_null());
    }

    fn text_of(pointer: *const c_char) -> String {
        unsafe { CStr::from_ptr(pointer) }.to_str().unwrap().to_string()
    }

    #[test]
    fn formats_date_time_and_utc_strings() {
        // 2024-01-01T00:00:00.500Z is a Monday.
        let timestamp = 1_704_067_200_500.0;
        assert_eq!(text_of(thaw_date_to_date_string(timestamp)), "Mon Jan 01 2024");
        assert_eq!(
            text_of(thaw_date_to_time_string(timestamp)),
            "00:00:00 GMT+0000 (Coordinated Universal Time)"
        );
        assert_eq!(
            text_of(thaw_date_to_string(timestamp)),
            "Mon Jan 01 2024 00:00:00 GMT+0000 (Coordinated Universal Time)"
        );
        assert_eq!(
            text_of(thaw_date_to_utc_string(timestamp)),
            "Mon, 01 Jan 2024 00:00:00 GMT"
        );
    }

    #[test]
    fn formatting_functions_report_invalid_date_for_nan() {
        assert_eq!(text_of(thaw_date_to_date_string(f64::NAN)), "Invalid Date");
        assert_eq!(text_of(thaw_date_to_time_string(f64::NAN)), "Invalid Date");
        assert_eq!(text_of(thaw_date_to_string(f64::NAN)), "Invalid Date");
        assert_eq!(text_of(thaw_date_to_utc_string(f64::NAN)), "Invalid Date");
    }

    fn parse(text: &str) -> f64 {
        let text = CString::new(text).unwrap();
        unsafe { thaw_date_parse(text.as_ptr()) }
    }

    #[test]
    fn date_utc_matches_known_timestamp() {
        assert_eq!(
            thaw_date_utc(2024.0, 0.0, 1.0, 0.0, 0.0, 0.0, 500.0),
            1_704_067_200_500.0
        );
    }

    #[test]
    fn date_utc_applies_two_digit_year_quirk() {
        assert_eq!(
            thaw_date_utc(70.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0),
            thaw_date_utc(1970.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn parses_full_iso_string_round_trip() {
        assert_eq!(parse("2024-01-01T00:00:00.500Z"), 1_704_067_200_500.0);
    }

    #[test]
    fn parses_date_only_and_partial_forms() {
        assert_eq!(parse("2024-01-01"), 1_704_067_200_000.0);
        assert_eq!(parse("2024-01"), 1_704_067_200_000.0);
        assert_eq!(parse("2024"), 1_704_067_200_000.0);
        assert_eq!(parse("2024-01-01T00:00"), 1_704_067_200_000.0);
    }

    #[test]
    fn parses_timezone_offset() {
        // +05:00 is 5 hours ahead of UTC, so the UTC instant is 5 hours earlier.
        assert_eq!(
            parse("2024-01-01T05:00:00+05:00"),
            1_704_067_200_000.0
        );
    }

    #[test]
    fn rejects_invalid_calendar_fields_without_rolling_over() {
        assert!(parse("2024-02-30").is_nan()); // 2024 is a leap year; Feb has 29 days.
        assert!(parse("2023-02-29").is_nan()); // 2023 is not a leap year.
        assert!(parse("2024-13-01").is_nan());
        assert!(parse("2024-01-01T24:00:00").is_nan());
    }

    #[test]
    fn rejects_unrecognized_formats() {
        assert!(parse("not a date").is_nan());
        assert!(parse("01/01/2024").is_nan());
        assert!(parse("").is_nan());
    }
}
