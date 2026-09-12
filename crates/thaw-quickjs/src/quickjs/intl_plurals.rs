// Real `Intl.PluralRules` (M8 of docs/design/intl-polyfill.md's "Real
// CLDR data via icu4x" plan), via `icu_plurals` -- entirely new
// capability, no prior English-only version existed.
//
// `minimumFractionDigits`/`maximumFractionDigits`/significant-digit
// options aren't honored (a known, documented gap): real `Intl.
// PluralRules.select()` with no such option just uses the plain JS
// number's own representation (confirmed against real Node: `1.0` and
// `1` select identically, since a JS number carries no separate
// "how many fraction digits were written" concept the way a formatted
// *string* would), which `String(number)` already gives for free --
// `intl.js` passes that straight through, no Rust-side rounding needed
// for the common case this covers.

/// `__thaw_intl_plural_category(locale, kind, digits) -> String`.
/// `kind` is `"cardinal"`/`"ordinal"`; `digits` is `String(number)`
/// (JS-side). Returns the CLDR plural category (`"zero"`/`"one"`/
/// `"two"`/`"few"`/`"many"`/`"other"`), or `"other"` (a real, valid
/// category, always present in every locale's rule set) if the locale/
/// value fails to resolve -- graceful degradation, matching this
/// polyfill's existing philosophy.
fn intl_plural_category(locale_tag: &str, kind: &str, digits: &str) -> String {
    use std::str::FromStr;

    let fallback = "other".to_string();
    let Ok(decimal) = icu_decimal::input::Decimal::from_str(digits) else {
        return fallback;
    };
    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return fallback;
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let prefs = icu_plurals::PluralRulesPreferences::from(&curated_locale);

    let rules = if kind == "ordinal" {
        icu_plurals::PluralRules::try_new_ordinal_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs)
    } else {
        icu_plurals::PluralRules::try_new_cardinal_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs)
    };
    let Ok(rules) = rules else {
        return fallback;
    };

    match rules.category_for(&decimal) {
        icu_plurals::PluralCategory::Zero => "zero",
        icu_plurals::PluralCategory::One => "one",
        icu_plurals::PluralCategory::Two => "two",
        icu_plurals::PluralCategory::Few => "few",
        icu_plurals::PluralCategory::Many => "many",
        icu_plurals::PluralCategory::Other => "other",
    }
    .to_string()
}
