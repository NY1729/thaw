// Real per-locale `Intl.ListFormat` conjunction/disjunction rendering
// (M7 of docs/design/intl-polyfill.md's "Real CLDR data via icu4x"
// plan), via `icu_list`, replacing the previous fixed English-only
// joining ("a, b, and c") for any curated locale.

/// `__thaw_intl_list_format(locale, kind, style, items_json) -> String`.
/// `kind` is `"and"`/`"or"`/`"unit"` (ECMA-402's `type` option, already
/// validated JS-side); `style` is `"long"`/`"short"`/`"narrow"`
/// (`icu_list::options::ListLength::{Wide,Short,Narrow}`, matching
/// ECMA-402's naming exactly except `long` -> `Wide`). Falls back to a
/// plain comma-join of the input items if anything about the locale
/// fails to resolve, rather than crashing a program over a locale
/// quirk (matching this polyfill's existing graceful-degradation
/// philosophy).
fn intl_list_format(locale_tag: &str, kind: &str, style: &str, items_json: &str) -> String {
    use std::str::FromStr;

    let items: Vec<String> = serde_json::from_str(items_json).unwrap_or_default();
    let fallback = || items.join(", ");

    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return fallback();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let prefs = icu_list::ListFormatterPreferences::from(&curated_locale);

    let length = match style {
        "short" => icu_list::options::ListLength::Short,
        "narrow" => icu_list::options::ListLength::Narrow,
        _ => icu_list::options::ListLength::Wide,
    };
    let options = icu_list::options::ListFormatterOptions::default().with_length(length);

    let formatter = match kind {
        "or" => icu_list::ListFormatter::try_new_or_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, options),
        "unit" => icu_list::ListFormatter::try_new_unit_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, options),
        _ => icu_list::ListFormatter::try_new_and_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, options),
    };
    let Ok(formatter) = formatter else {
        return fallback();
    };
    formatter.format(items.iter()).to_string()
}
