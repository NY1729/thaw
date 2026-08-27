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
    fn to_iso_string_formats_and_rejects_nan() {
        let text = unsafe { CStr::from_ptr(thaw_date_to_iso_string(1_704_067_200_500.0)) }
            .to_str()
            .unwrap();
        assert_eq!(text, "2024-01-01T00:00:00.500Z");
        assert!(thaw_date_to_iso_string(f64::NAN).is_null());
    }
}
