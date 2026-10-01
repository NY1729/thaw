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
// (hours 0-23) here since icu4x has no h24 equivalent; `intl.js`'s
// `_formatToPartsRealLocale` post-processes the midnight hour back to
// `24`, so the two cycles agree after all. `era` is only accepted
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
    day_period: Option<String>,
    hour12: Option<bool>,
    hour_cycle: Option<String>,
}

/// Whether an IANA zone's UTC offset ever changes across a year --
/// i.e. whether it observes DST at all (as opposed to merely not
/// currently being in DST at whatever instant is being formatted).
/// Reference instants match `intl_time_zone_names.rs`'s own
/// now-retired methodology: 2026-01-01 and 2026-07-01 UTC, picked to
/// be clear of any recent one-off zone-rule change (the pitfall real
/// Kazakhstan's 2024 restructuring already exposed once for this exact
/// kind of check).
fn zone_observes_dst(tz_name: &str) -> bool {
    let Ok(time_zone) = jiff::tz::TimeZone::get(tz_name) else {
        return false;
    };
    let Ok(january) = jiff::Timestamp::from_millisecond(1_767_225_600_000) else {
        return false;
    };
    let Ok(july) = jiff::Timestamp::from_millisecond(1_782_907_200_000) else {
        return false;
    };
    time_zone.to_offset_info(january).offset() != time_zone.to_offset_info(july).offset()
}

fn intl_time_zone_name(
    locale_tag: &str,
    time_zone: &str,
    timestamp_ms: f64,
    offset_minutes: i32,
    generic: bool,
) -> Option<String> {
    use icu_datetime::{DateTimeFormatterPreferences, NoCalendarFormatter, fieldsets::zone};
    use icu_time::zone::{IanaParser, UtcOffset, ZoneNameTimestamp};
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let prefs = DateTimeFormatterPreferences::from(&curated);
    let provider = &thaw_icu_data::ThawIcuDataProvider;
    let id = IanaParser::try_new_unstable(provider)
        .ok()?
        .as_borrowed()
        .parse(time_zone);
    if id == icu_time::TimeZone::UNKNOWN {
        return None;
    }
    let offset = UtcOffset::try_from_seconds(offset_minutes * 60).ok()?;
    let info = id
        .with_offset(Some(offset))
        .with_zone_name_timestamp(ZoneNameTimestamp::from_epoch_seconds(
            (timestamp_ms / 1000.0).floor() as i64,
        ));
    if generic {
        // Real `Intl`'s `longGeneric` for a zone that *never* observes
        // DST is identical to its `long` (standard) name (confirmed
        // against real Node across a sample of non-DST zones: Tokyo,
        // Seoul, Shanghai, Kolkata, Singapore, Dubai, Hong Kong all
        // give the same string for both styles -- "Japan Standard
        // Time", not a separate neutral "Japan Time" -- while a real
        // DST-observing zone's `longGeneric` genuinely differs from its
        // `long` name, e.g. `America/New_York` -> "Eastern Time" vs.
        // "Eastern Standard/Daylight Time", where icu4x's own
        // `GenericLong` is already correct). `zone_observes_dst` checks
        // two reference instants for a real offset difference, the same
        // "two fixed reference instants clear of any one-off zone-rule
        // change" methodology `intl_time_zone_names.rs`'s now-retired
        // hand-extracted table used to use.
        if !zone_observes_dst(time_zone) {
            return Some(
                NoCalendarFormatter::try_new_unstable(provider, prefs, zone::SpecificLong)
                    .ok()?
                    .format(&info)
                    .to_string(),
            );
        }
        Some(
            NoCalendarFormatter::try_new_unstable(provider, prefs, zone::GenericLong)
                .ok()?
                .format(&info)
                .to_string(),
        )
    } else {
        Some(
            NoCalendarFormatter::try_new_unstable(provider, prefs, zone::SpecificLong)
                .ok()?
                .format(&info)
                .to_string(),
        )
    }
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
            day_period: field("dayPeriod"),
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

fn recorder_to_json_parts(
    mut recorder: DateTimePartsRecorder,
    month_override: Option<&str>,
) -> String {
    recorder.spans.sort_by_key(|(start, _, _)| *start);
    let mut parts = Vec::new();
    let mut cursor = 0;
    for (start, end, value) in recorder.spans {
        if start > cursor {
            parts.push((&recorder.buffer[cursor..start], "literal"));
        }
        // `month_override` replaces the numeric month (e.g. icu4x's Hebrew
        // code number with the CLDR/ICU4C ordinal).
        let text = if value == "month" {
            month_override.unwrap_or(&recorder.buffer[start..end])
        } else {
            &recorder.buffer[start..end]
        };
        parts.push((text, value));
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
    Some(recorder_to_json_parts(recorder, None))
}

fn write_try_parts_json(
    formatted: &impl writeable::TryWriteable,
    month_override: Option<&str>,
) -> Option<String> {
    let mut recorder = DateTimePartsRecorder::default();
    match writeable::TryWriteable::try_write_to_parts(formatted, &mut recorder) {
        Ok(Ok(())) => Some(recorder_to_json_parts(recorder, month_override)),
        _ => None,
    }
}

/// Maps a resolved `AnyCalendarKind` to the CLDR calendar key used by
/// `thaw_icu_data::datetime_skeletons`.
#[allow(deprecated)] // `AnyCalendarKind::JapaneseExtended` is a deprecated alias for `Japanese`.
fn skeleton_calendar_name(kind: icu_calendar::AnyCalendarKind) -> Option<&'static str> {
    use icu_calendar::AnyCalendarKind as K;
    Some(match kind {
        K::Gregorian => "gregorian",
        K::Buddhist => "buddhist",
        K::Chinese => "chinese",
        K::Coptic => "coptic",
        K::Dangi => "dangi",
        K::Ethiopian | K::EthiopianAmeteAlem => "ethiopian",
        K::Hebrew => "hebrew",
        K::Indian => "indian",
        K::Japanese | K::JapaneseExtended => "japanese",
        K::Persian => "persian",
        K::Roc => "roc",
        K::HijriUmmAlQura
        | K::HijriTabularTypeIIFriday
        | K::HijriSimulatedMecca
        | K::HijriTabularTypeIIThursday => "hijri",
        _ => return None,
    })
}

/// Report the same calendar and digit system selected by the formatter's
/// curated locale and requested Unicode extensions.
fn intl_datetime_resolved_options_json(locale_tag: &str) -> String {
    use icu_provider::{DataIdentifierBorrowed, DataMarkerAttributes, DataProvider, DataRequest};
    use std::str::FromStr;

    let resolve = || -> Option<serde_json::Value> {
        let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
        let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
        let mut prefs = icu_datetime::DateTimeFormatterPreferences::from(&curated);
        let requested = icu_datetime::DateTimeFormatterPreferences::from(&locale);
        prefs.calendar_algorithm = requested.calendar_algorithm;
        prefs.numbering_system = requested.numbering_system;
        let provider = &thaw_icu_data::ThawIcuDataProvider;
        let formatter = icu_datetime::DateTimeFormatter::try_new_unstable(
            provider, prefs, icu_datetime::fieldsets::YMD::medium(),
        ).ok()?;
        let calendar = match formatter.calendar().kind() {
            icu_calendar::AnyCalendarKind::Gregorian => "gregory",
            icu_calendar::AnyCalendarKind::HijriUmmAlQura => "islamic-umalqura",
            icu_calendar::AnyCalendarKind::HijriTabularTypeIIFriday => "islamic-civil",
            icu_calendar::AnyCalendarKind::HijriTabularTypeIIThursday => "islamic-tbla",
            icu_calendar::AnyCalendarKind::HijriSimulatedMecca => "islamic",
            icu_calendar::AnyCalendarKind::EthiopianAmeteAlem => "ethioaa",
            kind => skeleton_calendar_name(kind)?,
        };

        let data_locale = icu_provider::DataLocale::from(curated.id);
        let symbols = DataProvider::<icu_decimal::provider::DecimalSymbolsV1>::load(
            provider,
            DataRequest { id: DataIdentifierBorrowed::for_locale(&data_locale), ..Default::default() },
        ).ok()?;
        let default_numbering = symbols.payload.get().numsys();
        let requested_numbering = locale.extensions.unicode.keywords
            .get(&"nu".parse().ok()?)
            .map(|value| value.to_string());
        let numbering = requested_numbering.as_deref().filter(|name| {
            DataMarkerAttributes::try_from_str(name).ok().is_some_and(|attributes| {
                DataProvider::<icu_decimal::provider::DecimalDigitsV1>::load(
                    provider,
                    DataRequest { id: DataIdentifierBorrowed::for_marker_attributes(&attributes), ..Default::default() },
                ).is_ok()
            })
        }).unwrap_or(default_numbering);
        Some(serde_json::json!({ "calendar": calendar, "numberingSystem": numbering }))
    };
    resolve().map_or_else(|| "{}".to_string(), |value| value.to_string())
}

/// Builds the UTS-35 classical skeleton for an ECMA-402 option set --
/// the piece icu4x's own `FieldSetBuilder` gets wrong (it derives widths
/// from the locale's `dateFormats` length patterns instead of the
/// requested option widths). Returns the fields plus the requested hour
/// symbol (kept separately so the matched pattern's hour can be forced to
/// it, e.g. `K` for `hourCycle: 'h11'`, which CLDR has no skeleton for).
fn skeleton_fields(
    options: &DateTimeOptions,
    locale_hour_cycle: Option<&str>,
) -> (Vec<Field>, Option<FieldSymbol>) {
    use icu_datetime::provider::fields::{
        Day, DayPeriod, Field, FieldLength, FieldSymbol, Hour, Month, Second, Weekday, Year,
    };

    fn name_length(style: &str) -> FieldLength {
        match style {
            "long" => FieldLength::Four,
            "narrow" => FieldLength::Five,
            _ => FieldLength::Three,
        }
    }
    fn numeric_length(style: &str) -> FieldLength {
        if style == "2-digit" {
            FieldLength::Two
        } else {
            FieldLength::One
        }
    }

    let mut fields = Vec::new();
    if let Some(style) = options.era.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Era, length: name_length(style) });
    }
    if let Some(style) = options.year.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Year(Year::Calendar), length: numeric_length(style) });
    }
    if let Some(style) = options.month.as_deref() {
        let length = match style {
            "2-digit" => FieldLength::Two,
            "short" => FieldLength::Three,
            "long" => FieldLength::Four,
            "narrow" => FieldLength::Five,
            _ => FieldLength::One,
        };
        fields.push(Field { symbol: FieldSymbol::Month(Month::Format), length });
    }
    if let Some(style) = options.day.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Day(Day::DayOfMonth), length: numeric_length(style) });
    }
    if let Some(style) = options.weekday.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Weekday(Weekday::Format), length: name_length(style) });
    }
    if let Some(style) = options.day_period.as_deref() {
        fields.push(Field { symbol: FieldSymbol::DayPeriod(DayPeriod::Flexible), length: name_length(style) });
    }
    let mut hour_symbol = None;
    if let Some(style) = options.hour.as_deref() {
        let cycle = options
            .hour_cycle
            .as_deref()
            .or(match options.hour12 {
                Some(true) => Some("h12"),
                Some(false) => Some("h23"),
                None => None,
            })
            .or(locale_hour_cycle)
            .unwrap_or("h23");
        let symbol = match cycle {
            "h11" => Hour::H11,
            "h12" => Hour::H12,
            // No `k`/h24 in icu4x; h24 is fixed up JS-side at the midnight instant.
            _ => Hour::H23,
        };
        hour_symbol = Some(FieldSymbol::Hour(symbol));
        fields.push(Field { symbol: FieldSymbol::Hour(symbol), length: numeric_length(style) });
    }
    if let Some(style) = options.minute.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Minute, length: numeric_length(style) });
    }
    if let Some(style) = options.second.as_deref() {
        fields.push(Field { symbol: FieldSymbol::Second(Second::Second), length: numeric_length(style) });
    }
    fields.sort_by(field_cmp);
    (fields, hour_symbol)
}

/// `Intl.DateTimeFormat.formatToParts` via real UTS-35 skeleton matching
/// against CLDR `availableFormats` -- matching Node/ICU4C's choice of
/// pattern. Returns `None` (caller falls back to the icu4x field-set
/// builder) for an unsupported calendar or if anything fails.
fn intl_datetime_skeleton_parts_json(
    locale_tag: &str,
    options_json: &str,
    zoned_parts_json: &str,
) -> Option<String> {
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale = curated_tag.parse().ok()?;
    let mut prefs = icu_datetime::DateTimeFormatterPreferences::from(&curated_locale);
    let requested_prefs = icu_datetime::DateTimeFormatterPreferences::from(&locale);
    prefs.calendar_algorithm = requested_prefs.calendar_algorithm;
    prefs.numbering_system = requested_prefs.numbering_system;

    let options = DateTimeOptions::from_json(options_json);
    prefs.hour_cycle = options.hour_cycle().or(requested_prefs.hour_cycle);

    let provider = &thaw_icu_data::ThawIcuDataProvider;
    // Resolve the calendar exactly as `DateTimeFormatter` would (explicit
    // `-u-ca-` or the locale's CLDR-default), via a throwaway formatter.
    let probe = icu_datetime::DateTimeFormatter::try_new_unstable(
        provider,
        prefs,
        icu_datetime::fieldsets::YMD::medium(),
    )
    .ok()?;
    let kind = probe.calendar().kind();
    let calendar_name = skeleton_calendar_name(kind)?;

    let (formats, glue) = thaw_icu_data::datetime_skeletons(calendar_name, curated_tag)?;
    let skeletons: Vec<(Vec<Field>, Pattern<'static>)> = formats
        .iter()
        .filter_map(|(skeleton, pattern)| {
            Some((parse_skeleton(skeleton)?, pattern.parse::<Pattern>().ok()?))
        })
        .collect();

    let locale_hour_cycle = thaw_icu_data::preferred_hour_cycle(curated_tag);
    let (fields, hour_symbol) = skeleton_fields(&options, locale_hour_cycle);
    if fields.is_empty() {
        return None;
    }

    let pattern = create_best_pattern_for_fields(&skeletons, glue, &fields, hour_symbol)?;

    let zoned: serde_json::Value = serde_json::from_str(zoned_parts_json).ok()?;
    let get = |name: &str| zoned.get(name).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let iso_date =
        icu_calendar::Date::try_new_iso(get("year"), get("month") as u8, get("day") as u8).ok()?;
    let time = icu_time::Time::try_new(
        get("hour") as u8,
        get("minute") as u8,
        get("second") as u8,
        0,
    )
    .ok()?;

    let has_date = fields.iter().any(|field| {
        matches!(
            field.symbol,
            FieldSymbol::Era
                | FieldSymbol::Year(_)
                | FieldSymbol::Month(_)
                | FieldSymbol::Week(_)
                | FieldSymbol::Day(_)
                | FieldSymbol::Weekday(_)
        )
    });
    let has_time = fields.iter().any(|field| {
        matches!(
            field.symbol,
            FieldSymbol::DayPeriod(_)
                | FieldSymbol::Hour(_)
                | FieldSymbol::Minute
                | FieldSymbol::Second(_)
                | FieldSymbol::DecimalSecond(_)
        )
    });

    format_skeleton_pattern(
        kind,
        provider,
        prefs,
        &pattern,
        iso_date,
        time,
        has_date,
        has_time,
    )
}

/// `__thaw_intl_datetime_skeleton_parts(locale, options_json,
/// zoned_parts_json) -> parts JSON`, or `"[]"` when skeleton matching
/// isn't available (caller then falls back to the icu4x field-set
/// builder, which needs its own width fixups JS-side). When this returns
/// non-empty, the pattern's field widths are already Node-final, so the
/// caller must *not* zero-pad/unpad.
fn intl_datetime_skeleton_parts_native(
    locale_tag: &str,
    options_json: &str,
    zoned_parts_json: &str,
) -> String {
    let Some(parts) = intl_datetime_skeleton_parts_json(locale_tag, options_json, zoned_parts_json)
    else {
        return "[]".to_string();
    };
    // CLDR patterns use U+202F (narrow no-break space) e.g. before ru/bg's
    // `г.` era, but real Node's DateTimeFormat always renders a plain space
    // (verified across every curated locale) -- unlike NumberFormat, where
    // fr's U+202F grouping is legitimate.
    parts.replace('\u{202f}', " ")
}

#[allow(deprecated)] // `AnyCalendarKind::JapaneseExtended` is a deprecated alias for `Japanese`.
#[allow(clippy::too_many_arguments)]
fn format_skeleton_pattern(
    kind: icu_calendar::AnyCalendarKind,
    provider: &thaw_icu_data::ThawIcuDataProvider,
    prefs: icu_datetime::DateTimeFormatterPreferences,
    pattern: &Pattern<'static>,
    iso_date: icu_calendar::Date<icu_calendar::Iso>,
    time: icu_time::Time,
    has_date: bool,
    has_time: bool,
) -> Option<String> {
    use icu_datetime::fieldsets::enums::{DateAndTimeFieldSet, DateFieldSet, TimeFieldSet};

    let pattern_str = pattern.to_string();
    let pattern_fields: Vec<Field> = pattern
        .items
        .iter()
        .filter_map(|item| match item {
            PatternItem::Field(field) => Some(field),
            PatternItem::Literal(_) => None,
        })
        .collect();

    // icu4x renders a Hebrew numeric month with its code number (and a
    // `6a`/`6b` leap suffix), but CLDR/ICU4C use the *ordinal* month
    // (`MonthInfo::ordinal`, which counts Adar I). Compute it once and swap
    // the rendered month part below.
    let month_override = if kind == icu_calendar::AnyCalendarKind::Hebrew
        && pattern_fields.iter().any(|field| {
            matches!(field.symbol, FieldSymbol::Month(_))
                && matches!(field.length, FieldLength::One | FieldLength::Two)
        })
    {
        let hebrew = iso_date.to_calendar(icu_calendar::cal::Hebrew::new());
        let ordinal = hebrew.month().ordinal;
        let decimal = icu_decimal::DecimalFormatter::try_new_unstable(
            provider,
            icu_decimal::DecimalFormatterPreferences::from(&prefs),
            icu_decimal::options::GroupingStrategy::Never.into(),
        )
        .ok()?;
        Some(decimal.format(&icu_decimal::input::Decimal::from(ordinal as i64)).to_string())
    } else {
        None
    };

    macro_rules! format_cal {
        ($C:ty, $FSet:ty, $input:expr) => {{
            use icu_datetime::pattern::{
                DayPeriodNameLength, MonthNameLength, WeekdayNameLength, YearNameLength,
            };
            let mut names =
                icu_datetime::pattern::FixedCalendarDateTimeNames::<$C, $FSet>::try_new_unstable(
                    provider, prefs,
                )
                .ok()?;
            // Load only the name data this pattern actually references.
            // (`load_for_pattern` would also demand time-zone data this
            // polyfill deliberately doesn't vendor -- `timeZoneName` is
            // rendered JS-side -- so load names explicitly instead.) Each
            // `load_*_names` holds a single length, so pick one length per
            // field type: an `Era` field's names come from the same marker
            // as a (cyclic) year's, and loading both lengths would clobber
            // the era's wide name with the numeric year's abbreviated one.
            let year_name_length = |length: FieldLength| match length {
                FieldLength::Four => YearNameLength::Wide,
                FieldLength::Five => YearNameLength::Narrow,
                _ => YearNameLength::Abbreviated,
            };
            let mut month_len = None;
            let mut weekday_len = None;
            let mut year_len = None;
            let mut dayperiod_len = None;
            for field in &pattern_fields {
                match field.symbol {
                    FieldSymbol::Month(month) => {
                        use icu_datetime::provider::fields::Month;
                        let standalone = month == Month::StandAlone;
                        month_len = Some(match (field.length, standalone) {
                            (FieldLength::One | FieldLength::Two, false) => MonthNameLength::Numeric,
                            (FieldLength::One | FieldLength::Two, true) => {
                                MonthNameLength::StandaloneNumeric
                            }
                            (FieldLength::Four, false) => MonthNameLength::Wide,
                            (FieldLength::Four, true) => MonthNameLength::StandaloneWide,
                            (FieldLength::Five, false) => MonthNameLength::Narrow,
                            (FieldLength::Five, true) => MonthNameLength::StandaloneNarrow,
                            (_, false) => MonthNameLength::Abbreviated,
                            (_, true) => MonthNameLength::StandaloneAbbreviated,
                        });
                    }
                    FieldSymbol::Weekday(weekday) => {
                        use icu_datetime::provider::fields::Weekday;
                        let standalone = weekday == Weekday::StandAlone;
                        weekday_len = Some(match (field.length, standalone) {
                            (FieldLength::Four, false) => WeekdayNameLength::Wide,
                            (FieldLength::Five, false) => WeekdayNameLength::Narrow,
                            (FieldLength::Four, true) => WeekdayNameLength::StandaloneWide,
                            (FieldLength::Five, true) => WeekdayNameLength::StandaloneNarrow,
                            (_, false) => WeekdayNameLength::Abbreviated,
                            (_, true) => WeekdayNameLength::StandaloneAbbreviated,
                        });
                    }
                    FieldSymbol::Era => {
                        year_len = Some(year_name_length(field.length));
                    }
                    FieldSymbol::Year(icu_datetime::provider::fields::Year::Cyclic) => {
                        if year_len.is_none() {
                            year_len = Some(year_name_length(field.length));
                        }
                    }
                    FieldSymbol::DayPeriod(_) => {
                        dayperiod_len = Some(match field.length {
                            FieldLength::Four => DayPeriodNameLength::Wide,
                            FieldLength::Five => DayPeriodNameLength::Narrow,
                            _ => DayPeriodNameLength::Abbreviated,
                        });
                    }
                    _ => {}
                }
            }
            if let Some(length) = month_len {
                let _ = names.load_month_names(provider, length);
            }
            if let Some(length) = weekday_len {
                let _ = names.load_weekday_names(provider, length);
            }
            if let Some(length) = year_len {
                let _ = names.load_year_names(provider, length);
            }
            if let Some(length) = dayperiod_len {
                let _ = names.load_day_period_names(provider, length);
            }
            let pattern = icu_datetime::pattern::DateTimePattern::try_from_pattern_str(&pattern_str)
                .ok()?;
            let formatter = names.with_pattern_unchecked(&pattern);
            let formatted = formatter.format(&$input);
            write_try_parts_json(&formatted, month_override.as_deref())
        }};
    }
    macro_rules! go {
        ($C:ty) => {{
            if has_date && has_time {
                let date = iso_date.to_calendar(<$C>::default());
                let datetime = icu_datetime::input::DateTime { date, time };
                format_cal!($C, DateAndTimeFieldSet, datetime)
            } else if has_date {
                let date = iso_date.to_calendar(<$C>::default());
                format_cal!($C, DateFieldSet, date)
            } else {
                format_cal!($C, TimeFieldSet, time)
            }
        }};
    }

    use icu_calendar::AnyCalendarKind as K;
    match kind {
        K::Gregorian => go!(icu_calendar::cal::Gregorian),
        K::Buddhist => go!(icu_calendar::cal::Buddhist),
        K::Japanese | K::JapaneseExtended => go!(icu_calendar::cal::Japanese),
        K::Coptic => go!(icu_calendar::cal::Coptic),
        K::Indian => go!(icu_calendar::cal::Indian),
        K::Ethiopian | K::EthiopianAmeteAlem => go!(icu_calendar::cal::Ethiopian),
        K::Chinese => go!(icu_calendar::cal::ChineseTraditional),
        K::Dangi => go!(icu_calendar::cal::KoreanTraditional),
        K::Hebrew => go!(icu_calendar::cal::Hebrew),
        K::HijriUmmAlQura
        | K::HijriTabularTypeIIFriday
        | K::HijriSimulatedMecca
        | K::HijriTabularTypeIIThursday => go!(icu_calendar::cal::Hijri<icu_calendar::cal::hijri::UmmAlQura>),
        K::Persian => go!(icu_calendar::cal::Persian),
        K::Roc => go!(icu_calendar::cal::Roc),
        _ => None,
    }
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
            // ICU4X classifies `M`/`YM`/`Y` as *calendar-period* field sets
            // (a standalone month/year), which `build_date()` rejects --
            // so a bare `{month}`/`{year}`/`{year,month}` request must go
            // through `build_calendar_period()`. Using `build_date()` for
            // them silently produced an empty string.
            use icu_datetime::fieldsets::builder::DateFields;
            if matches!(
                builder.date_fields,
                Some(DateFields::M | DateFields::YM | DateFields::Y)
            ) {
                let Ok(field_set) = builder.build_calendar_period() else {
                    return "[]".to_string();
                };
                let Ok(formatter) = icu_datetime::DateTimeFormatter::try_new_unstable(
                    &thaw_icu_data::ThawIcuDataProvider,
                    prefs,
                    field_set,
                ) else {
                    return "[]".to_string();
                };
                let date = iso_date.to_calendar(formatter.calendar());
                write_parts_json(&formatter.format(&date))
            } else {
                let Ok(field_set) = builder.build_date() else {
                    return "[]".to_string();
                };
                let Ok(formatter) = icu_datetime::DateTimeFormatter::try_new_unstable(
                    &thaw_icu_data::ThawIcuDataProvider,
                    prefs,
                    field_set,
                ) else {
                    return "[]".to_string();
                };
                let date = iso_date.to_calendar(formatter.calendar());
                write_parts_json(&formatter.format(&date))
            }
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
    let formatted = formatted.unwrap_or_else(|| "[]".to_string());
    // CLDR patterns use U+202F (narrow no-break space) e.g. between a
    // numeric hour and the day-period marker for English (`4\u{202f}PM`)
    // or before ru/bg's `г.` era, but real Node's DateTimeFormat always
    // renders a plain space (verified across every curated locale and
    // field combination). Unlike NumberFormat, where fr's U+202F grouping
    // is legitimate, so this must not leak into `intl_number.rs`.
    formatted.replace('\u{202f}', " ")
}
