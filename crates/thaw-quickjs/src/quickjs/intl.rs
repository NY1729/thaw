// The single native primitive backing thaw's `Intl` polyfill
// (`platform_globals/intl.js`): given an IANA timezone name and an
// instant, returns that instant's local (zone-adjusted) date/time
// broken down into fields, plus the zone's UTC offset and short
// abbreviation at that instant. Real, DST-aware IANA timezone math
// (via `jiff`, whose bundled `tzdb-bundle-always` data is compiled
// directly into the binary -- no dependency on the host's own
// `/usr/share/zoneinfo`, keeping the binary self-contained and
// portable) -- everything else (English month/weekday names, literal
// punctuation, number formatting) is plain JS in `intl.js`.
//
// This is also the *only* thing `Intl.DateTimeFormat`'s constructor
// needs to validate a `timeZone` string: an unrecognized name simply
// comes back with `"valid": false`, which `intl.js` turns into a
// `RangeError` -- matching real `Intl`'s own eager validation (real
// example: luxon's `IANAZone.isValidZone`, which constructs an
// `Intl.DateTimeFormat` in a try/catch specifically to detect this).

/// Encodes `{"valid":false}` for an unrecognized zone name or a
/// non-finite timestamp -- deliberately not an error return: the caller
/// (`intl.js`) decides whether that's a thrown `RangeError` (a bad
/// `timeZone`) or something else, and JSON keeps this native/JS
/// boundary as simple as every other native builtin in this crate
/// (`digest_bytes`/`hmac_bytes` and friends all cross the boundary as
/// plain strings too).
fn intl_zoned_parts_json(tz_name: &str, timestamp_ms: f64) -> String {
    if !timestamp_ms.is_finite() {
        return r#"{"valid":false}"#.to_string();
    }
    let Ok(time_zone) = jiff::tz::TimeZone::get(tz_name) else {
        return r#"{"valid":false}"#.to_string();
    };
    // `jiff::Timestamp` only accepts a nanosecond-precision instant
    // within its own supported range; clamp rather than panic on an
    // extreme (but finite) `Date` value -- matches every other native
    // builtin in this crate degrading gracefully instead of aborting.
    let millis = timestamp_ms.round().clamp(-8_640_000_000_000_000.0, 8_640_000_000_000_000.0) as i64;
    let Ok(timestamp) = jiff::Timestamp::from_millisecond(millis) else {
        return r#"{"valid":false}"#.to_string();
    };
    let zoned = timestamp.to_zoned(time_zone);
    let info = zoned.time_zone().to_offset_info(timestamp);
    let offset_minutes = info.offset().seconds() / 60;
    // `Weekday::to_monday_one_offset()` gives ISO weekday numbering
    // (Monday = 1 ... Sunday = 7), matching real `Intl.DateTimeFormat`'s
    // own `formatToParts` "weekday" field semantics once mapped through
    // `intl.js`'s own English weekday-name table (also Monday-first).
    let weekday = zoned.weekday().to_monday_one_offset();
    let abbreviation = info.abbreviation().replace('"', "");
    let dst = info.dst().is_dst();
    format!(
        r#"{{"valid":true,"year":{},"month":{},"day":{},"hour":{},"minute":{},"second":{},"millisecond":{},"weekday":{},"offsetMinutes":{},"abbreviation":"{}","dst":{},"timeZone":{},"timestampMs":{}}}"#,
        zoned.year(),
        zoned.month(),
        zoned.day(),
        zoned.hour(),
        zoned.minute(),
        zoned.second(),
        zoned.subsec_nanosecond() / 1_000_000,
        weekday,
        offset_minutes,
        abbreviation,
        dst,
        serde_json::to_string(tz_name).unwrap_or_else(|_| "\"UTC\"".to_string()),
        millis,
    )
}
