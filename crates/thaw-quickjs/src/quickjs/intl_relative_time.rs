// Real `Intl.RelativeTimeFormat` (M11 of docs/design/intl-polyfill.md's
// "Real CLDR data via icu4x" plan), via `icu_experimental::relativetime`
// -- entirely new capability. `icu_experimental` is the one
// deliberately-unstable dependency in this whole effort (icu4x's own
// docs warn it "may change at any time, in breaking or non-breaking
// ways, including in SemVer minor releases") -- pinned tightly
// (`=0.6.0` in `crates/thaw-quickjs/Cargo.toml`) for exactly that
// reason. Every other milestone in this effort sits on stable icu4x
// components.
//
// `icu_experimental`'s `RelativeTimeFormatter` has no dynamic field-set
// builder the way `icu_datetime` does -- one constructor per (style,
// unit) pair (`try_new_long_day_unstable`, `try_new_short_week_unstable`,
// ...), 24 combinations total (3 styles x 8 units). The match below is
// mechanical, not a design choice -- there's no shorter correct way to
// express "call the compile-time-selected function matching these two
// runtime strings" against this particular upstream API shape.

/// `__thaw_intl_relative_time_format(locale, unit, style, numeric,
/// value) -> String`. `unit` is one of `"year"`/`"quarter"`/`"month"`/
/// `"week"`/`"day"`/`"hour"`/`"minute"`/`"second"` (ECMA-402's own unit
/// names); `style` is `"long"`/`"short"`/`"narrow"`; `numeric` is
/// `"always"`/`"auto"`. Falls back to a plain `"{value} {unit}(s)
/// ago"`/`"in {value} {unit}(s)"` (English, matching this polyfill's
/// existing degrade-to-English philosophy elsewhere) if the locale
/// fails to resolve.
fn intl_relative_time_format(locale_tag: &str, unit: &str, style: &str, numeric: &str, value: f64) -> String {
    use icu_experimental::relativetime::{RelativeTimeFormatter, RelativeTimeFormatterOptions};
    use std::str::FromStr;

    let fallback = || {
        let magnitude = value.abs();
        let plural_s = if magnitude == 1.0 { "" } else { "s" };
        if value < 0.0 {
            format!("{magnitude} {unit}{plural_s} ago")
        } else {
            format!("in {value} {unit}{plural_s}")
        }
    };

    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return fallback();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let prefs = icu_experimental::relativetime::RelativeTimeFormatterPreferences::from(&curated_locale);

    let mut options = RelativeTimeFormatterOptions::default();
    options.numeric = if numeric == "always" {
        icu_experimental::relativetime::options::Numeric::Always
    } else {
        icu_experimental::relativetime::options::Numeric::Auto
    };

    macro_rules! build {
        ($constructor:ident) => {
            RelativeTimeFormatter::$constructor(&thaw_icu_data::ThawIcuDataProvider, prefs, options)
        };
    }
    let formatter = match (style, unit) {
        ("long", "second") => build!(try_new_long_second_unstable),
        ("long", "minute") => build!(try_new_long_minute_unstable),
        ("long", "hour") => build!(try_new_long_hour_unstable),
        ("long", "day") => build!(try_new_long_day_unstable),
        ("long", "week") => build!(try_new_long_week_unstable),
        ("long", "month") => build!(try_new_long_month_unstable),
        ("long", "quarter") => build!(try_new_long_quarter_unstable),
        ("long", "year") => build!(try_new_long_year_unstable),
        ("short", "second") => build!(try_new_short_second_unstable),
        ("short", "minute") => build!(try_new_short_minute_unstable),
        ("short", "hour") => build!(try_new_short_hour_unstable),
        ("short", "day") => build!(try_new_short_day_unstable),
        ("short", "week") => build!(try_new_short_week_unstable),
        ("short", "month") => build!(try_new_short_month_unstable),
        ("short", "quarter") => build!(try_new_short_quarter_unstable),
        ("short", "year") => build!(try_new_short_year_unstable),
        ("narrow", "second") => build!(try_new_narrow_second_unstable),
        ("narrow", "minute") => build!(try_new_narrow_minute_unstable),
        ("narrow", "hour") => build!(try_new_narrow_hour_unstable),
        ("narrow", "day") => build!(try_new_narrow_day_unstable),
        ("narrow", "week") => build!(try_new_narrow_week_unstable),
        ("narrow", "month") => build!(try_new_narrow_month_unstable),
        ("narrow", "quarter") => build!(try_new_narrow_quarter_unstable),
        ("narrow", "year") => build!(try_new_narrow_year_unstable),
        _ => return fallback(),
    };
    let Ok(formatter) = formatter else {
        return fallback();
    };

    let Ok(decimal) = icu_decimal::input::Decimal::from_str(&value.to_string()) else {
        return fallback();
    };
    writeable::Writeable::write_to_string(&formatter.format(decimal)).into_owned()
}
