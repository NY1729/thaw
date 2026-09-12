// Real `Intl.Collator` (M9 of docs/design/intl-polyfill.md's "Real
// CLDR data via icu4x" plan), via `icu_collator` -- entirely new
// capability. `String.prototype.localeCompare` (`thaw-runtime`'s
// `thaw_string_compare`, plain UTF-16 codepoint ordering) is
// deliberately left untouched: `thaw-runtime` is linked into *every*
// compiled program unconditionally (unlike `thaw-quickjs`, only linked
// when a program actually uses the QuickJS fallback path), and
// `thaw-icu-data` bundles every vendored marker as one crate --
// pulling collation data into `thaw-runtime` would drag in the whole
// dataset (including the multi-megabyte segmenter dictionaries) into
// every single Lambda binary regardless of whether it ever calls
// `localeCompare`. This needs its own real size measurement before
// deciding whether/how to fold it in (splitting `thaw-icu-data` by
// concern, a scoped collation-only sub-crate, etc.) rather than being
// bundled into this milestone as a side effect.
//
// `usage: 'search'` isn't supported (falls back to `'sort'` behavior,
// a documented gap) -- real search-collation tailoring needs the
// `--include-collations search` marker data this crate doesn't vendor,
// a real, separate capability from plain sort-order comparison.

/// `__thaw_intl_collator_compare(locale, sensitivity, ignore_punctuation,
/// numeric, case_first, a, b) -> i32` (negative/zero/positive, matching
/// `Intl.Collator.prototype.compare`'s own contract -- real ECMA-402
/// doesn't guarantee exactly `-1`/`0`/`1`). Falls back to plain UTF-16
/// codepoint ordering (the same behavior `localeCompare` already has)
/// if the locale fails to resolve.
#[allow(clippy::too_many_arguments)]
fn intl_collator_compare(
    locale_tag: &str,
    sensitivity: &str,
    ignore_punctuation: bool,
    numeric: bool,
    case_first: &str,
    a: &str,
    b: &str,
) -> i32 {
    use std::str::FromStr;

    let codepoint_fallback = || match a.encode_utf16().cmp(b.encode_utf16()) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };

    let Ok(locale) = icu_locale::Locale::from_str(locale_tag) else {
        return codepoint_fallback();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_locale: icu_locale::Locale =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let mut prefs = icu_collator::CollatorPreferences::from(&curated_locale);
    prefs.numeric_ordering = Some(if numeric {
        icu_collator::preferences::CollationNumericOrdering::True
    } else {
        icu_collator::preferences::CollationNumericOrdering::False
    });
    prefs.case_first = match case_first {
        "upper" => Some(icu_collator::preferences::CollationCaseFirst::Upper),
        "lower" => Some(icu_collator::preferences::CollationCaseFirst::Lower),
        _ => None,
    };

    let mut options = icu_collator::options::CollatorOptions::default();
    options.strength = Some(match sensitivity {
        "base" => icu_collator::options::Strength::Primary,
        "accent" => icu_collator::options::Strength::Secondary,
        "case" => icu_collator::options::Strength::Primary,
        _ => icu_collator::options::Strength::Tertiary,
    });
    if sensitivity == "case" {
        options.case_level = Some(icu_collator::options::CaseLevel::On);
    }
    if ignore_punctuation {
        options.alternate_handling = Some(icu_collator::options::AlternateHandling::Shifted);
        options.max_variable = Some(icu_collator::options::MaxVariable::Punctuation);
    }

    let Ok(collator) =
        icu_collator::Collator::try_new_unstable(&thaw_icu_data::ThawIcuDataProvider, prefs, options)
    else {
        return codepoint_fallback();
    };
    match collator.as_borrowed().compare(a, b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}
