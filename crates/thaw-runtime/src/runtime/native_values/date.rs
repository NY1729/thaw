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
}
