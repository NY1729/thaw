// Real `Intl.PluralRules` (M8 of docs/design/intl-polyfill.md's "Real
// CLDR data via icu4x" plan), via `icu_plurals` -- entirely new
// capability, no prior English-only version existed.
//
// `intl.js` formats the operand with the requested `minimum`/
// `maximumFractionDigits`/`SignificantDigits` before handing it here, so
// the ICU rules see the real visible-fraction-digit count (the `v`
// operand, which is what distinguishes e.g. `en` `one` for `1` from
// `other` for `1.0`); this file only resolves the category from the
// already-formatted ASCII decimal string.

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

fn plural_category_name(category: icu_plurals::PluralCategory) -> &'static str {
    match category {
        icu_plurals::PluralCategory::Zero => "zero",
        icu_plurals::PluralCategory::One => "one",
        icu_plurals::PluralCategory::Two => "two",
        icu_plurals::PluralCategory::Few => "few",
        icu_plurals::PluralCategory::Many => "many",
        icu_plurals::PluralCategory::Other => "other",
    }
}

/// `__thaw_intl_plural_categories(locale, kind) -> String` (a JSON array).
/// Backs `Intl.PluralRules.prototype.resolvedOptions().pluralCategories`,
/// which real ECMA-402 populates with the locale's own categories in
/// canonical order (`["one","other"]` for `en`, `["few","many","one",
/// "other"]` for `ru` (cardinal), ...) -- previously hardcoded to
/// `["other"]` regardless of locale. Falls back to `["other"]` (the one
/// category every locale always has) if the locale/kind fails to resolve.
fn intl_plural_categories(locale_tag: &str, kind: &str) -> String {
    use std::str::FromStr;

    let fallback = Some(vec![icu_plurals::PluralCategory::Other]);
    let categories = (|| {
        let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
        let curated_tag = resolve_curated_locale(&locale.id);
        let curated_locale: icu_locale::Locale =
            curated_tag.parse().ok()?;
        let prefs = icu_plurals::PluralRulesPreferences::from(&curated_locale);
        let rules = if kind == "ordinal" {
            icu_plurals::PluralRules::try_new_ordinal_unstable(
                &thaw_icu_data::ThawIcuDataProvider,
                prefs,
            )
        } else {
            icu_plurals::PluralRules::try_new_cardinal_unstable(
                &thaw_icu_data::ThawIcuDataProvider,
                prefs,
            )
        }
        .ok()?;
        Some(rules.categories().collect::<Vec<_>>())
    })()
    .or(fallback)
    .unwrap_or_else(|| vec![icu_plurals::PluralCategory::Other]);
    // Match V8/Node's own `pluralCategories` ordering (confirmed
    // empirically): `["few","many","one","two","zero","other"]` with
    // absent categories skipped, e.g. `["one","other"]` for `en`.
    let order = |name: &str| match name {
        "few" => 0,
        "many" => 1,
        "one" => 2,
        "two" => 3,
        "zero" => 4,
        _ => 5,
    };
    let mut names: Vec<&str> = categories.into_iter().map(plural_category_name).collect();
    names.sort_by_key(|name| order(name));
    serde_json::to_string(&names).unwrap_or_else(|_| "[\"other\"]".to_string())
}
