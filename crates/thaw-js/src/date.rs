//! Date: local time (system zone through jiff), setters, string forms and string parsing.
//! The UTC core (`__time`, UTC getters, `toISOString`) lives in `builtins.rs`.

use crate::builtins::{civil_from_days, civil_to_ms};
use crate::*;

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MAX_TIME: f64 = 8.64e15;

thread_local! {
    static ZONE: jiff::tz::TimeZone = system_zone();
}

/// The process time zone: `$TZ`, else the `/etc/localtime` link target, else UTC (like Node on a bare host).
fn system_zone() -> jiff::tz::TimeZone {
    let name = match std::env::var("TZ") {
        Ok(tz) => tz.trim_start_matches(':').to_string(),
        Err(_) => std::fs::read_link("/etc/localtime")
            .ok()
            .and_then(|p| {
                let s = p.to_string_lossy().to_string();
                s.split_once("zoneinfo/").map(|(_, n)| n.to_string())
            })
            .unwrap_or_default(),
    };
    if name.is_empty() {
        return jiff::tz::TimeZone::UTC;
    }
    jiff::tz::TimeZone::get(&name).unwrap_or(jiff::tz::TimeZone::UTC)
}

/// Offset of the system zone from UTC at the instant `utc_ms`, in milliseconds (east positive).
fn offset_at(utc_ms: f64) -> f64 {
    if !utc_ms.is_finite() {
        return 0.0;
    }
    // jiff's range ends a little before 9999-12-31; the zone offset is constant out there
    let clamped = (utc_ms as i64).clamp(-377_705_023_201_000, 253_402_207_200_000);
    match jiff::Timestamp::from_millisecond(clamped) {
        Ok(ts) => ZONE.with(|z| z.to_offset(ts).seconds() as f64 * 1000.0),
        Err(_) => 0.0,
    }
}

/// Abbreviation of the zone at `utc_ms` ("JST", "PDT", "UTC").
fn zone_abbrev(utc_ms: f64) -> String {
    match jiff::Timestamp::from_millisecond(
        (utc_ms as i64).clamp(-377_705_023_201_000, 253_402_207_200_000),
    ) {
        Ok(ts) => ZONE.with(|z| z.to_offset_info(ts).abbreviation().to_string()),
        Err(_) => "UTC".into(),
    }
}

/// V8 prints the long zone name; only the common ones are tabulated, others fall back to the abbreviation.
fn zone_long_name(abbrev: &str) -> String {
    match abbrev {
        "UTC" | "Z" => "Coordinated Universal Time",
        "GMT" => "Greenwich Mean Time",
        "JST" => "Japan Standard Time",
        "KST" => "Korean Standard Time",
        "IST" => "India Standard Time",
        "CET" => "Central European Standard Time",
        "CEST" => "Central European Summer Time",
        "EET" => "Eastern European Standard Time",
        "EEST" => "Eastern European Summer Time",
        "BST" => "British Summer Time",
        "EST" => "Eastern Standard Time",
        "EDT" => "Eastern Daylight Time",
        "CST" => "Central Standard Time",
        "CDT" => "Central Daylight Time",
        "MST" => "Mountain Standard Time",
        "MDT" => "Mountain Daylight Time",
        "PST" => "Pacific Standard Time",
        "PDT" => "Pacific Daylight Time",
        other => other,
    }
    .into()
}

/// A UTC-shaped civil time (`local_ms` read as if it were UTC) in the system zone, as a UTC instant.
fn local_to_utc(local_ms: f64) -> f64 {
    if !local_ms.is_finite() {
        return f64::NAN;
    }
    let c = civil(local_ms);
    let dt = jiff::civil::DateTime::new(
        c[0] as i16,
        c[1] as i8 + 1,
        c[2] as i8,
        c[3] as i8,
        c[4] as i8,
        c[5] as i8,
        (c[6] * 1_000_000) as i32,
    );
    let utc = dt
        .ok()
        .and_then(|dt| ZONE.with(|z| z.to_ambiguous_timestamp(dt).compatible().ok()))
        .map(|ts| ts.as_millisecond() as f64);
    utc.unwrap_or(local_ms)
}

/// `[year, month0, day, hours, minutes, seconds, ms, weekday]` of a time value read as UTC.
fn civil(t: f64) -> [i64; 8] {
    let days = (t / 86_400_000.0).floor();
    let r = (t - days * 86_400_000.0) as i64;
    let (y, m, d) = civil_from_days(days as i64);
    [
        y,
        m - 1,
        d,
        r / 3_600_000,
        r / 60_000 % 60,
        r / 1000 % 60,
        r % 1000,
        (days as i64 + 4).rem_euclid(7),
    ]
}

pub(crate) fn clip(t: f64) -> f64 {
    if t.is_finite() && t.abs() <= MAX_TIME {
        t.trunc() + 0.0
    } else {
        f64::NAN
    }
}

/// Time value for date components interpreted in the system zone (the `new Date(y, m, ...)` form).
pub(crate) fn local_from_parts(n: &[f64]) -> f64 {
    let mut parts = n.to_vec();
    if let Some(y) = parts.first_mut() {
        if y.is_finite() && (0.0..=99.0).contains(&y.trunc()) {
            *y = 1900.0 + y.trunc();
        }
    }
    clip(local_to_utc(civil_to_ms(&parts)))
}

pub(crate) fn utc_from_parts(n: &[f64]) -> f64 {
    let mut parts = n.to_vec();
    if let Some(y) = parts.first_mut() {
        if y.is_finite() && (0.0..=99.0).contains(&y.trunc()) {
            *y = 1900.0 + y.trunc();
        }
    }
    clip(civil_to_ms(&parts))
}

// ----- parsing -----

/// Parse the ES date-time string format, then the legacy forms V8 accepts.
pub(crate) fn parse_date(s: &str) -> f64 {
    let s = s.trim();
    let t = parse_iso(s).or_else(|| parse_legacy(s));
    t.map_or(f64::NAN, clip)
}

fn digits(b: &[u8], i: &mut usize, min: usize, max: usize) -> Option<f64> {
    let start = *i;
    while *i < b.len() && *i - start < max && b[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i - start < min {
        return None;
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

fn parse_iso(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let sign = match b.first()? {
        b'+' => {
            i += 1;
            Some(1.0)
        }
        b'-' => {
            i += 1;
            Some(-1.0)
        }
        _ => None,
    };
    let y = match sign {
        Some(sg) => sg * digits(b, &mut i, 6, 6)?,
        None => digits(b, &mut i, 4, 4)?,
    };
    let (mut mo, mut d) = (1.0, 1.0);
    if b.get(i) == Some(&b'-') {
        i += 1;
        mo = digits(b, &mut i, 2, 2)?;
        if b.get(i) == Some(&b'-') {
            i += 1;
            d = digits(b, &mut i, 2, 2)?;
        }
    }
    if !(1.0..=12.0).contains(&mo) || !(1.0..=31.0).contains(&d) {
        return None;
    }
    let date_only = i == b.len();
    let (mut h, mut mi, mut sec, mut ms) = (0.0, 0.0, 0.0, 0.0);
    let mut offset: Option<f64> = None;
    if !date_only {
        if !matches!(b.get(i), Some(b'T' | b't' | b' ')) {
            return None;
        }
        i += 1;
        h = digits(b, &mut i, 2, 2)?;
        if b.get(i) != Some(&b':') {
            return None;
        }
        i += 1;
        mi = digits(b, &mut i, 2, 2)?;
        if b.get(i) == Some(&b':') {
            i += 1;
            sec = digits(b, &mut i, 2, 2)?;
            if matches!(b.get(i), Some(b'.' | b',')) {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let frac = format!("0.{}", &s[start..i]).parse::<f64>().ok()?;
                ms = (frac * 1000.0).floor();
            }
        }
        match b.get(i) {
            Some(b'Z' | b'z') => {
                i += 1;
                offset = Some(0.0);
            }
            Some(sg @ (b'+' | b'-')) => {
                i += 1;
                let hh = digits(b, &mut i, 2, 2)?;
                if b.get(i) == Some(&b':') {
                    i += 1;
                }
                let mm = digits(b, &mut i, 2, 2).unwrap_or(0.0);
                let total = (hh * 60.0 + mm) * 60_000.0;
                offset = Some(if *sg == b'-' { -total } else { total });
            }
            _ => {}
        }
        if i != b.len() {
            return None;
        }
        if h > 24.0 || mi > 59.0 || sec > 59.0 || (h == 24.0 && (mi > 0.0 || sec > 0.0 || ms > 0.0))
        {
            return None;
        }
    }
    let as_utc = civil_to_ms(&[y, mo - 1.0, d, h, mi, sec, ms]);
    Some(match (date_only, offset) {
        (true, _) => as_utc,
        (false, Some(off)) => as_utc - off,
        (false, None) => local_to_utc(as_utc),
    })
}

fn month_index(word: &str) -> Option<f64> {
    let w = word.to_ascii_lowercase();
    if w.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| w.starts_with(&m.to_ascii_lowercase()))
        .map(|i| i as f64)
}

/// `Thu, 01 Jan 1970 00:00:00 GMT`, `Jan 2, 2020 10:00`, `1/2/2020`, `2020/01/02 03:04`, ...
fn parse_legacy(s: &str) -> Option<f64> {
    let (mut year, mut month, mut day) = (None, None, None);
    let (mut h, mut mi, mut sec, mut ms) = (0.0, 0.0, 0.0, 0.0);
    let mut offset: Option<f64> = None;
    let mut pm: Option<bool> = None;
    let mut nums: Vec<f64> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            let start = i;
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            let w = &s[start..i];
            let lw = w.to_ascii_lowercase();
            match lw.as_str() {
                "am" => pm = Some(false),
                "pm" => pm = Some(true),
                "gmt" | "utc" | "ut" | "z" => offset = Some(offset.unwrap_or(0.0)),
                "t" => {}
                _ => {
                    if let Some(m) = month_index(w) {
                        month = Some(m);
                    } else if !DAYS.iter().any(|d| lw.starts_with(&d.to_ascii_lowercase())) {
                        return None;
                    }
                }
            }
        } else if c.is_ascii_digit() {
            let start = i;
            let n = digits(b, &mut i, 1, 9)?;
            if b.get(i) == Some(&b':') {
                // time of day
                h = n;
                i += 1;
                mi = digits(b, &mut i, 1, 2)?;
                if b.get(i) == Some(&b':') {
                    i += 1;
                    sec = digits(b, &mut i, 1, 2)?;
                    if b.get(i) == Some(&b'.') {
                        i += 1;
                        let fs = i;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                        ms = (format!("0.{}", &s[fs..i]).parse::<f64>().ok()? * 1000.0).floor();
                    }
                }
            } else if matches!(b.get(i), Some(b'/' | b'-'))
                && nums.is_empty()
                && month.is_none()
                && i - start <= 4
            {
                // numeric date: m/d/y or y/m/d
                let sep = b[i];
                let mut parts = vec![n];
                while b.get(i) == Some(&sep) {
                    i += 1;
                    parts.push(digits(b, &mut i, 1, 4)?);
                }
                if parts.len() != 3 {
                    return None;
                }
                if i - start >= 8 && parts[0] > 31.0 {
                    year = Some(parts[0]);
                    month = Some(parts[1] - 1.0);
                    day = Some(parts[2]);
                } else {
                    month = Some(parts[0] - 1.0);
                    day = Some(parts[1]);
                    year = Some(if parts[2] < 50.0 {
                        2000.0 + parts[2]
                    } else if parts[2] < 100.0 {
                        1900.0 + parts[2]
                    } else {
                        parts[2]
                    });
                }
            } else {
                nums.push(n);
            }
        } else if (c == b'+' || c == b'-') && offset.is_some()
            || (c == b'+' || c == b'-') && i > 0 && h + mi > 0.0
        {
            i += 1;
            let hh = digits(b, &mut i, 2, 2)?;
            if b.get(i) == Some(&b':') {
                i += 1;
            }
            let mm = digits(b, &mut i, 2, 2).unwrap_or(0.0);
            let total = (hh * 60.0 + mm) * 60_000.0;
            offset = Some(if c == b'-' { -total } else { total });
        } else if c == b'(' {
            while i < b.len() && b[i] != b')' {
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    // remaining bare numbers are day and year ("Jan 2, 2020" / "2 Jan 2020")
    for n in nums {
        if day.is_none() && n <= 31.0 && month.is_some() {
            day = Some(n);
        } else if year.is_none() {
            year = Some(if n < 50.0 {
                2000.0 + n
            } else if n < 100.0 {
                1900.0 + n
            } else {
                n
            });
        } else if day.is_none() {
            day = Some(n);
        } else {
            return None;
        }
    }
    let (y, mo) = (year?, month?);
    let d = day.unwrap_or(1.0);
    if !(0.0..12.0).contains(&mo) || !(1.0..=31.0).contains(&d) {
        return None;
    }
    if let Some(p) = pm {
        if !(1.0..=12.0).contains(&h) {
            return None;
        }
        h = h % 12.0 + if p { 12.0 } else { 0.0 };
    }
    let as_utc = civil_to_ms(&[y, mo, d, h, mi, sec, ms]);
    Some(match offset {
        Some(off) => as_utc - off,
        None => local_to_utc(as_utc),
    })
}

// ----- formatting -----

fn year_str(y: i64) -> String {
    if y < 0 {
        format!("-{:06}", -y)
    } else {
        format!("{y:04}")
    }
}

fn gmt_offset(off_ms: f64) -> String {
    let mins = (off_ms / 60_000.0) as i64;
    let sign = if mins < 0 { '-' } else { '+' };
    format!("GMT{sign}{:02}{:02}", mins.abs() / 60, mins.abs() % 60)
}

fn date_part(c: &[i64; 8]) -> String {
    format!(
        "{} {} {:02} {}",
        DAYS[c[7] as usize],
        MONTHS[c[1] as usize],
        c[2],
        year_str(c[0])
    )
}

fn time_part(c: &[i64; 8]) -> String {
    format!("{:02}:{:02}:{:02}", c[3], c[4], c[5])
}

impl Interp {
    fn date_set(&mut self, this: &Value, t: f64) {
        if let Value::Object(o) = this {
            put_hidden(o, "__time", Value::Number(t));
        }
    }

    pub(crate) fn install_date_extras(&mut self, dp: &Obj) {
        // local getters: (name, field index into `civil`)
        for (name, idx) in [
            ("getFullYear", 0usize),
            ("getMonth", 1),
            ("getDate", 2),
            ("getHours", 3),
            ("getMinutes", 4),
            ("getSeconds", 5),
            ("getMilliseconds", 6),
            ("getDay", 7),
            ("getYear", 100),
        ] {
            self.set_native(dp, name, move |it, this, _| {
                let t = it.date_time(this)?;
                if t.is_nan() {
                    return Ok(Value::Number(f64::NAN));
                }
                let c = civil(t + offset_at(t));
                Ok(Value::Number(if idx == 100 {
                    (c[0] - 1900) as f64
                } else {
                    c[idx] as f64
                }))
            });
        }
        self.set_native(dp, "getTimezoneOffset", |it, this, _| {
            let t = it.date_time(this)?;
            Ok(Value::Number(if t.is_nan() {
                f64::NAN
            } else {
                -offset_at(t) / 60_000.0 + 0.0
            }))
        });
        // setters: (name, first field, max args, local)
        for (name, first, max, local) in [
            ("setMilliseconds", 6usize, 1usize, true),
            ("setSeconds", 5, 2, true),
            ("setMinutes", 4, 3, true),
            ("setHours", 3, 4, true),
            ("setDate", 2, 1, true),
            ("setMonth", 1, 2, true),
            ("setFullYear", 0, 3, true),
            ("setUTCMilliseconds", 6, 1, false),
            ("setUTCSeconds", 5, 2, false),
            ("setUTCMinutes", 4, 3, false),
            ("setUTCHours", 3, 4, false),
            ("setUTCDate", 2, 1, false),
            ("setUTCMonth", 1, 2, false),
            ("setUTCFullYear", 0, 3, false),
        ] {
            self.set_native(dp, name, move |it, this, args| {
                let t = it.date_time(this)?;
                let nums: Vec<f64> = args.iter().take(max).map(|v| it.num(v)).collect();
                let base = if t.is_nan() {
                    if first != 0 {
                        return Ok(Value::Number(f64::NAN));
                    }
                    0.0
                } else if local {
                    t + offset_at(t)
                } else {
                    t
                };
                let c = civil(base);
                let mut f: Vec<f64> = c[..7].iter().map(|x| *x as f64).collect();
                if nums.is_empty() {
                    f[first] = f64::NAN;
                }
                for (k, n) in nums.iter().enumerate() {
                    f[first + k] = *n;
                }
                let shaped = civil_to_ms(&f);
                let new = clip(if local { local_to_utc(shaped) } else { shaped });
                it.date_set(this, new);
                Ok(Value::Number(new))
            });
        }
        self.set_native(dp, "setTime", |it, this, args| {
            it.date_time(this)?;
            let n = it.num(&arg(args, 0));
            let t = clip(n);
            it.date_set(this, t);
            Ok(Value::Number(t))
        });
        // string forms
        self.set_native(dp, "toString", |it, this, _| {
            let t = it.date_time(this)?;
            if t.is_nan() {
                return Ok(Value::str("Invalid Date"));
            }
            let off = offset_at(t);
            let c = civil(t + off);
            Ok(Value::str(&format!(
                "{} {} {} ({})",
                date_part(&c),
                time_part(&c),
                gmt_offset(off),
                zone_long_name(&zone_abbrev(t))
            )))
        });
        self.set_native(dp, "toDateString", |it, this, _| {
            let t = it.date_time(this)?;
            Ok(Value::str(&if t.is_nan() {
                "Invalid Date".to_string()
            } else {
                date_part(&civil(t + offset_at(t)))
            }))
        });
        self.set_native(dp, "toTimeString", |it, this, _| {
            let t = it.date_time(this)?;
            if t.is_nan() {
                return Ok(Value::str("Invalid Date"));
            }
            let off = offset_at(t);
            Ok(Value::str(&format!(
                "{} {} ({})",
                time_part(&civil(t + off)),
                gmt_offset(off),
                zone_long_name(&zone_abbrev(t))
            )))
        });
        for name in ["toUTCString", "toGMTString"] {
            self.set_native(dp, name, |it, this, _| {
                let t = it.date_time(this)?;
                if t.is_nan() {
                    return Ok(Value::str("Invalid Date"));
                }
                let c = civil(t);
                Ok(Value::str(&format!(
                    "{}, {:02} {} {} {} GMT",
                    DAYS[c[7] as usize],
                    c[2],
                    MONTHS[c[1] as usize],
                    year_str(c[0]),
                    time_part(&c)
                )))
            });
        }
        // en-US only; honours `{ timeZone: 'UTC' }`, ignores the rest of the options
        for (name, date, time) in [
            ("toLocaleDateString", true, false),
            ("toLocaleTimeString", false, true),
            ("toLocaleString", true, true),
        ] {
            self.set_native(dp, name, move |it, this, args| {
                let t = it.date_time(this)?;
                if t.is_nan() {
                    return Ok(Value::str("Invalid Date"));
                }
                let opts = arg(args, 1);
                let utc = matches!(&opts, Value::Object(_))
                    && matches!(it.get(&opts, "timeZone")?, Value::Str(z) if z.eq_ignore_ascii_case("utc"));
                let c = civil(if utc { t } else { t + offset_at(t) });
                let mut parts = Vec::new();
                if date {
                    parts.push(format!("{}/{}/{}", c[1] + 1, c[2], c[0]));
                }
                if time {
                    let h12 = if c[3] % 12 == 0 { 12 } else { c[3] % 12 };
                    parts.push(format!(
                        "{h12}:{:02}:{:02} {}",
                        c[4],
                        c[5],
                        if c[3] < 12 { "AM" } else { "PM" }
                    ));
                }
                Ok(Value::str(&parts.join(", ")))
            });
        }
    }
}
