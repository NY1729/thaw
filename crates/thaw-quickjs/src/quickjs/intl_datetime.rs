// Real per-locale `Intl.DateTimeFormat` field rendering (M4+M5 of
// docs/design/intl-polyfill.md's "Real CLDR data via icu4x" plan) --
// month/weekday/era/day-period names, field ordering, literal
// punctuation, and (M5) non-Gregorian calendar systems, via
// `icu_datetime`, for any curated locale (`thaw-icu-data`). `jiff`
// (`intl.rs`) stays the raw offset/DST/timezone-name engine unchanged;
// this file only ever receives already-zone-adjusted calendar fields
// (year/month/day/hour/minute/second) and formats *those*, so it needs
// no timezone awareness of its own.
//
// `timeZoneName` rendering is deliberately NOT part of this file --
// `intl.js` keeps using the existing jiff-backed abbreviation/long-name
// path for that one field until M13 retires `intl_time_zone_names.rs`'s
// English-only table in favor of real per-locale `icu_datetime` zone
// data.
//
// ## Why a `FieldSetBuilder`, and its real limits
//
// `icu_datetime`'s 2.x API is built around a small set of *named*
// field-set/length presets (`DateFields::{Y,M,D,YM,MD,YMD,E,DE,MDE,
// YMDE}` x `Length::{Long,Medium,Short}`) rather than ECMA-402's fully
// independent per-field styling (`month` and `weekday` share one
// `Length`, not a style each) -- confirmed empirically: `YMD::long()`
// renders `"July 4, 2024"`, `YMD::medium()` renders `"Jul 4, 2024"`,
// `YMD::short()` renders `"7/4/24"` for `en-US`. `YearStyle::Full`
// (independent of `Length`) does let year stay 4-digit even in a
// `Short`-length preset (`YMD::short().with_year_style(YearStyle::
// Full)` -> `"7/4/2024"`), which covers the common
// `{year:'numeric',month:'2-digit',day:'2-digit'}` shape. There is no
// equivalent independent control for `month`, and no `Length::Narrow`
// at all -- `month: 'narrow'` degrades to the same rendering as
// `'short'` (a known, documented gap, not a silent wrong answer).
// `hourCycle: 'h24'` (hours 1-24, real ECMA-402) maps to icu4x's `H23`
// (hours 0-23) since icu4x has no h24 equivalent -- differs only at the
// midnight instant, also a known documented gap. `era` is only accepted
// by the builder when both `month` and `day` are also present
// (confirmed empirically: `YearStyle::WithEra` with `DateFields::Y`/`YM`
// fails with `InvalidDateFields`) -- a bare `{year,era}` request (no
// month/day, unusual but valid real ECMA-402) is upgraded to
// `YMD`/`YMDE` here, rendering month/day the caller didn't ask for
// rather than dropping the era. Also, one purely cosmetic, narrow
// CLDR-data-version artifact found while cross-checking against real
// Node (not a logic bug): the vendored CLDR (48.2.1) renders U+202F
// (narrow no-break space) between the hour and `dayPeriod` for `en-US`,
// where the specific Node build used for cross-checks in this session
// (v22.22.2, bundled ICU 78) still renders a plain space -- see the
// test `intl_datetime_format_matches_real_node_for_curated_non_english_
// locales` in `tests.rs`.

struct DateTimeOptions {
    weekday: Option<String>,
    era: Option<String>,
    year: Option<String>,
    month: Option<String>,
    day: Option<String>,
    hour: Option<String>,
    minute: Option<String>,
    second: Option<String>,
    hour12: Option<bool>,
    hour_cycle: Option<String>,
}

impl DateTimeOptions {
    fn from_json(options_json: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(options_json).unwrap_or_default();
        let field = |name: &str| {
            value
                .get(name)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        DateTimeOptions {
            weekday: field("weekday"),
            era: field("era"),
            year: field("year"),
            month: field("month"),
            day: field("day"),
            hour: field("hour"),
            minute: field("minute"),
            second: field("second"),
            hour12: value.get("hour12").and_then(|v| v.as_bool()),
            hour_cycle: field("hourCycle"),
        }
    }

    fn date_fields(&self) -> Option<icu_datetime::fieldsets::builder::DateFields> {
        use icu_datetime::fieldsets::builder::DateFields;
        let (y, m, d, e) = (
            self.year.is_some(),
            self.month.is_some(),
            self.day.is_some(),
            self.weekday.is_some(),
        );
        // `era` (-> `YearStyle::WithEra`) is only accepted by icu4x's
        // builder when both month and day are also present (confirmed
        // empirically: `DateFields::Y`/`YM` + `YearStyle::WithEra`
        // both fail with `InvalidDateFields`, but `YMD`/`YMDE` succeed)
        // -- real ECMA-402 does allow a bare `{year,era}` request with
        // no month/day, so upgrade to `YMD`/`YMDE` in that case rather
        // than silently dropping the era. A known, documented
        // approximation: renders month/day the caller didn't ask for,
        // rather than nothing at all.
        if self.era.is_some() && !(m && d) {
            return Some(if e { DateFields::YMDE } else { DateFields::YMD });
        }
        match (y, m, d, e) {
            (false, false, false, false) => None,
            (true, true, true, true) => Some(DateFields::YMDE),
            (true, true, true, false) => Some(DateFields::YMD),
            (false, true, true, true) => Some(DateFields::MDE),
            (false, true, true, false) => Some(DateFields::MD),
            (true, true, false, false) => Some(DateFields::YM),
            (false, false, true, true) => Some(DateFields::DE),
            (false, false, false, true) => Some(DateFields::E),
            (false, true, false, false) => Some(DateFields::M),
            (true, false, false, false) => Some(DateFields::Y),
            (false, false, true, false) => Some(DateFields::D),
            // Uncommon combinations with no direct icu4x preset (e.g.
            // year+day without month, or year+weekday without
            // month/day) -- fall back to the closest superset that
            // includes every requested field, accepting that this may
            // render one or two fields the caller didn't ask for
            // rather than silently dropping a requested one.
            _ => Some(DateFields::YMDE),
        }
    }

    fn length(&self) -> icu_datetime::options::Length {
        use icu_datetime::options::Length;
        let style = self
            .month
            .as_deref()
            .or(self.weekday.as_deref())
            .or(self.era.as_deref());
        match style {
            Some("long") => Length::Long,
            Some("short") | Some("narrow") => Length::Medium,
            _ => Length::Short,
        }
    }

    fn year_style(&self) -> Option<icu_datetime::options::YearStyle> {
        use icu_datetime::options::YearStyle;
        if self.era.is_some() {
            return Some(YearStyle::WithEra);
        }
        match self.year.as_deref() {
            Some("numeric") => Some(YearStyle::Full),
            Some(_) => Some(YearStyle::Auto),
            None => None,
        }
    }

    fn time_precision(&self) -> Option<icu_datetime::options::TimePrecision> {
        use icu_datetime::options::TimePrecision;
        if self.second.is_some() {
            Some(TimePrecision::Second)
        } else if self.minute.is_some() {
            Some(TimePrecision::Minute)
        } else if self.hour.is_some() {
            Some(TimePrecision::Hour)
        } else {
            None
        }
    }

    fn hour_cycle(&self) -> Option<icu_datetime::preferences::HourCycle> {
        use icu_datetime::preferences::HourCycle;
        match self.hour12 {
            Some(true) => return Some(HourCycle::H12),
            Some(false) => return Some(HourCycle::H23),
            None => {}
        }
        match self.hour_cycle.as_deref() {
            Some("h11") => Some(HourCycle::H11),
            Some("h12") => Some(HourCycle::H12),
            Some("h23") => Some(HourCycle::H23),
            // No h24 (hours 1-24) equivalent in icu4x -- H23 (hours
            // 0-23) is the closest available, differing only at the
            // midnight instant. See this file's own doc comment.
            Some("h24") => Some(HourCycle::H23),
            _ => None,
        }
    }
}

/// Records only `"datetime"`-category [`Part`]s (nested
/// `"decimal"`-category parts, e.g. the digit grouping inside a
/// rendered year, are ignored -- their byte range is always a subset of
/// the enclosing `"datetime"` part, and real `Intl.DateTimeFormat.
/// formatToParts()` reports one part per date/time field, not a nested
/// digit-vs-field breakdown).
#[derive(Default)]
struct DateTimePartsRecorder {
    buffer: String,
    spans: Vec<(usize, usize, &'static str)>,
}

impl std::fmt::Write for DateTimePartsRecorder {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.buffer.push_str(s);
        Ok(())
    }
}

impl writeable::PartsWrite for DateTimePartsRecorder {
    type SubPartsWrite = Self;

    fn with_part(
        &mut self,
        part: writeable::Part,
        mut f: impl FnMut(&mut Self) -> std::fmt::Result,
    ) -> std::fmt::Result {
        if part.category == "datetime" {
            let start = self.buffer.len();
            f(self)?;
            let end = self.buffer.len();
            self.spans.push((start, end, part.value));
            Ok(())
        } else {
            f(self)
        }
    }
}

fn recorder_to_json_parts(mut recorder: DateTimePartsRecorder) -> String {
    recorder.spans.sort_by_key(|(start, _, _)| *start);
    let mut parts = Vec::new();
    let mut cursor = 0;
    for (start, end, value) in recorder.spans {
        if start > cursor {
            parts.push((&recorder.buffer[cursor..start], "literal"));
        }
        parts.push((&recorder.buffer[start..end], value));
        cursor = end;
    }
    if cursor < recorder.buffer.len() {
        parts.push((&recorder.buffer[cursor..], "literal"));
    }
    let mut json = String::from("[");
    for (index, (value, part_type)) in parts.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            r#"{{"type":"{}","value":{}}}"#,
            part_type,
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        ));
    }
    json.push(']');
    json
}

fn write_parts_json(formatted: &impl writeable::Writeable) -> Option<String> {
    let mut recorder = DateTimePartsRecorder::default();
    writeable::Writeable::write_to_parts(formatted, &mut recorder).ok()?;
    Some(recorder_to_json_parts(recorder))
}

/// `__thaw_intl_datetime_format_parts(locale, options_json,
/// zoned_parts_json) -> JSON array of `{"type":..,"value":..}``,
/// matching `Intl.DateTimeFormat.prototype.formatToParts()`'s shape for
/// every field *except* `timeZoneName` (still rendered by `intl.js`'s
/// existing jiff-backed logic). `zoned_parts_json` is
/// `__thaw_intl_zoned_parts`'s own output -- only its already
/// zone-adjusted `year`/`month`/`day`/`hour`/`minute`/`second` fields
/// are used here.
///
/// Any calendar system the resolved locale calls for (M5): a date field
/// set uses the fully calendar-generic `DateTimeFormatter` (not
/// `FixedCalendarDateTimeFormatter<Gregorian, _>`), whose `AnyCalendar`
/// dispatch already reads both the request's `-u-ca-` subtag and the
/// locale's own CLDR-default calendar (`thaw-icu-data`'s vendored
/// `CalendarPreferredV1` marker) with zero extra code here -- confirmed
/// empirically to match real Node exactly for `th-TH` (defaults to
/// Buddhist, no explicit `calendar` needed), `ja-JP-u-ca-japanese`
/// (Reiwa-era years), and `ar-SA-u-ca-islamic-umalqura` (real Hijri
/// dates), with no per-calendar Rust dispatch code at all: the ISO date
/// from `zoned_parts_json` is simply converted into whichever calendar
/// the formatter already selected, via `Date::to_calendar(formatter.
/// calendar())`. A time-only field set has no calendar to speak of, so
/// it stays on `FixedCalendarDateTimeFormatter<Gregorian, _>` (any fixed
/// calendar would render hour/minute/second identically).
///
/// Dispatches to the narrowest of `build_date()`/`build_time()`/
/// `build_date_and_time()` that fits the request (never
/// `build_composite()`, which is additionally *zone*-generic and would
/// require zone data/input this milestone deliberately doesn't have --
/// no zone rendering here, see this file's own header comment).
fn intl_datetime_format_parts_json(locale_tag: &str, options_json: &str, zoned_parts_json: &str) -> String {
    use std::str::FromStr;

    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return "[]".to_string();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    // `curated_locale` only carries language/script/region (`thaw-icu-
    // data` only vendors data keyed by those) -- its own `-u-ca-`/
    // `-u-nu-` extensions, if any, come back from `resolve_curated_
    // locale` empty. The *requested* locale's real extensions (already
    // folded in JS-side for an explicit `calendar`/`numberingSystem`
    // constructor option, via `Intl.Locale`'s own tag-rewriting) must
    // still reach the formatter, or `ja-JP-u-ca-japanese`/`{calendar:
    // 'japanese'}` would silently render Gregorian -- found exactly
    // this way, cross-checking against real Node.
    let mut prefs = icu_datetime::DateTimeFormatterPreferences::from(&curated_locale);
    let requested_prefs = icu_datetime::DateTimeFormatterPreferences::from(&locale);
    prefs.calendar_algorithm = requested_prefs.calendar_algorithm;
    prefs.numbering_system = requested_prefs.numbering_system;

    let options = DateTimeOptions::from_json(options_json);
    prefs.hour_cycle = options.hour_cycle().or(requested_prefs.hour_cycle);

    let mut builder = icu_datetime::fieldsets::builder::FieldSetBuilder::new();
    builder.date_fields = options.date_fields();
    builder.length = Some(options.length());
    builder.year_style = options.year_style();
    builder.time_precision = options.time_precision();

    let zoned: serde_json::Value = serde_json::from_str(zoned_parts_json).unwrap_or_default();
    let get_i32 = |name: &str| zoned.get(name).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    // ISO, not Gregorian: the source date `Date::to_calendar(...)`
    // converts *from*, regardless of which calendar the formatter (and
    // therefore the locale) actually selects.
    let iso_date_result =
        icu_calendar::Date::try_new_iso(get_i32("year"), get_i32("month") as u8, get_i32("day") as u8);
    let time_result = icu_time::Time::try_new(
        get_i32("hour") as u8,
        get_i32("minute") as u8,
        get_i32("second") as u8,
        0,
    );

    let has_date = builder.date_fields.is_some();
    let has_time = builder.time_precision.is_some();

    let formatted = match (has_date, has_time) {
        (true, true) => {
            let (Ok(iso_date), Ok(time)) = (iso_date_result, time_result) else {
                return "[]".to_string();
            };
            let Ok(field_set) = builder.build_date_and_time() else {
                return "[]".to_string();
            };
            let Ok(formatter) =
                icu_datetime::DateTimeFormatter::try_new_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, field_set)
            else {
                return "[]".to_string();
            };
            let date = iso_date.to_calendar(formatter.calendar());
            let datetime = icu_datetime::input::DateTime { date, time };
            write_parts_json(&formatter.format(&datetime))
        }
        (true, false) => {
            let Ok(iso_date) = iso_date_result else {
                return "[]".to_string();
            };
            let Ok(field_set) = builder.build_date() else {
                return "[]".to_string();
            };
            let Ok(formatter) =
                icu_datetime::DateTimeFormatter::try_new_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, field_set)
            else {
                return "[]".to_string();
            };
            let date = iso_date.to_calendar(formatter.calendar());
            write_parts_json(&formatter.format(&date))
        }
        (false, true) => {
            let Ok(time) = time_result else {
                return "[]".to_string();
            };
            let Ok(field_set) = builder.build_time() else {
                return "[]".to_string();
            };
            let Ok(formatter) = icu_datetime::FixedCalendarDateTimeFormatter::<icu_calendar::Gregorian, _>::try_new_unstable(
                &thaw_icu_data::ThawIcuDataProvider,
                prefs,
                field_set,
            ) else {
                return "[]".to_string();
            };
            write_parts_json(&formatter.format(&time))
        }
        (false, false) => None,
    };
    formatted.unwrap_or_else(|| "[]".to_string())
}
