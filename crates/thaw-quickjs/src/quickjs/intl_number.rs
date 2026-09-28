// Real per-locale `Intl.NumberFormat` digit/grouping rendering (M6 of
// docs/design/intl-polyfill.md's "Real CLDR data via icu4x" plan) --
// non-Latin numbering systems (confirmed: Bangla/Thai/Arabic-Indic
// digits) and locale-correct grouping/decimal separators, via
// `icu_decimal`.
//
// All of `NumberFormat`'s existing sign handling, rounding
// (`minimumFractionDigits`/`maximumFractionDigits`), and integer
// zero-padding (`minimumIntegerDigits`) stays exactly where it already
// was, in `intl.js` -- that arithmetic is locale-independent, so
// reimplementing it here would just be the same logic twice. This file
// only takes the already-rounded/padded digit string `intl.js` computed
// (e.g. `"12345.680"`) and renders *those exact digits* with the
// requested locale's own digit glyphs, grouping separator, and decimal
// separator, via `fixed_decimal::Decimal::from_str` (an exact string
// parse, not a float -- this is what keeps a caller-computed trailing
// zero like `"3.50"` intact instead of losing it to float rounding).

/// `__thaw_intl_number_format(locale, digits, use_grouping) -> String`:
/// `digits` is a plain `-`-prefixed decimal string with the exact
/// fraction-digit count already applied (`intl.js`'s own job); returns
/// it rendered with the requested locale's digits/grouping/decimal
/// separators, or `digits` itself unchanged if anything about the
/// locale/value fails to resolve (graceful degradation, matching this
/// polyfill's existing philosophy -- a formatting primitive should
/// never crash a program over a locale quirk).
fn intl_number_format(locale_tag: &str, digits: &str, use_grouping: bool) -> String {
    use std::str::FromStr;

    let Ok(decimal) = icu_decimal::input::Decimal::from_str(digits) else {
        return digits.to_string();
    };
    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return digits.to_string();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let requested_prefs = icu_decimal::DecimalFormatterPreferences::from(&locale);
    // CLDR defines a non-Latin numbering system's *symbols* only in the
    // locales that use it (e.g. `symbols-numberSystem-arab` under
    // `ar-SA`), never at the root locale. So an explicit `numberingSystem`
    // on an unrelated locale (`en` + `arab`) would otherwise fall back to
    // the base locale's Latin symbols (digits localize, but the grouping
    // separator stays a comma). For the systems whose separators actually
    // differ from Latin, resolve through a representative locale that
    // natively carries that system; a system that differs only in its
    // digit glyphs keeps the base locale's symbols (the grouping *sizes*
    // are locale-defined, not numbering-system-defined -- `en` groups
    // `1234567` as `1,234,567`, not Bengali's `12,34,567`).
    let base_locale = requested_prefs
        .numbering_system
        .as_ref()
        .and_then(|nu| intl_numbering_system_representative(nu.as_str()))
        .and_then(|tag| tag.parse::<icu_locale::Locale>().ok())
        .unwrap_or(curated_locale);
    let mut prefs = icu_decimal::DecimalFormatterPreferences::from(&base_locale);
    prefs.numbering_system = requested_prefs.numbering_system;

    let options: icu_decimal::options::DecimalFormatterOptions = if use_grouping {
        icu_decimal::options::GroupingStrategy::Auto
    } else {
        icu_decimal::options::GroupingStrategy::Never
    }
    .into();
    let Ok(formatter) =
        icu_decimal::DecimalFormatter::try_new_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, options)
    else {
        return digits.to_string();
    };
    formatter.format(&decimal).to_string()
}

/// A curated locale that natively uses the given numbering system, used
/// to source that system's symbols when it's been explicitly requested on
/// a locale that doesn't define them (see `intl_number_format`'s own
/// comment).
fn intl_numbering_system_representative(numbering_system: &str) -> Option<&'static str> {
    // Only the systems whose CLDR *separators* differ from Latin
    // (grouping `٬`, decimal `٫`): a system that differs from Latin only
    // in its digit glyphs (`beng`/`deva`/`thai`/`hanidec`) keeps the base
    // locale's own symbols (and, crucially, its grouping *sizes*, which
    // are locale-defined, not numbering-system-defined -- `en` groups
    // `1234567` as `1,234,567`, not Bengali's `12,34,567`).
    match numbering_system {
        "arab" => Some("ar-SA"),
        "arabext" => Some("fa"),
        _ => None,
    }
}
