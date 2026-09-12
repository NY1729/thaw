// Real BCP-47 locale resolution for `Intl` (`platform_globals/intl.js`),
// backed by `icu4x` + `thaw-icu-data`'s curated CLDR dataset -- see
// `docs/design/intl-polyfill.md`'s "Real CLDR data via icu4x" section.
//
// Every `Intl.*` constructor's `locale` argument is resolved through this
// one native primitive before anything locale-sensitive happens: it turns
// an arbitrary requested BCP-47 tag into the *curated* locale actually
// used for data lookups (this crate only ships baked data for a fixed,
// ~35-tag list -- `thaw-icu-data/src/lib.rs`'s doc comment has the exact
// list and the reasoning behind each entry), plus the resolved
// calendar/numbering-system/collation Unicode extension subtags. A tag
// outside the curated list degrades gracefully to the nearest curated
// ancestor (matching language, then matching language+region ignoring
// script, then language+script ignoring region, then bare language),
// finally `en-US` -- never a thrown error, matching this polyfill's
// existing graceful-degradation philosophy (an unrecognized `timeZone`
// is the one thing that legitimately throws, per `intl.rs`).
//
// Default calendar/numbering-system resolution here is deliberately
// simple (`"gregory"`/`"latn"` unless the request's own `-u-ca-`/`-u-nu-`
// subtag says otherwise) -- a locale-appropriate default calendar (e.g.
// real Node's `th-TH` defaults to `"buddhist"`) is M5's job, once
// non-Gregorian `DateTimeFormatter` support exists to make that
// meaningful; `thaw-icu-data` already vendors the `CalendarPreferredV1`
// marker M5 will need for it.

const CURATED_LOCALES: &[&str] = &[
    "en-US", "en-GB", "es", "es-419", "fr", "de", "it", "pt", "pt-BR", "nl", "sv", "pl", "ru",
    "uk", "tr", "ar", "ar-SA", "he", "hi", "bn", "ja", "ko", "zh-Hans", "zh-Hant", "th", "vi",
    "id", "ms", "fil", "el", "ro", "cs", "hu", "da", "fi", "nb",
];

const DEFAULT_CURATED_LOCALE: &str = "en-US";

// Not cached in a `static`: `LocaleExpander`'s `DataPayload` type isn't
// `Sync` (one of its internal cart variants, unused by our baked-data
// path, is `Rc`-based), and constructing it is cheap regardless -- it
// only wraps references into `thaw-icu-data`'s already-`'static` baked
// tables, no I/O or allocation of its own.
fn locale_expander() -> icu_locale::LocaleExpander {
    icu_locale::LocaleExpander::try_new_common_unstable(&thaw_icu_data::ThawIcuDataProvider)
        .expect("thaw-icu-data should carry locale likely-subtags data")
}

/// `(language, script, region)` of a tag's *maximized* form -- script/
/// region are empty strings when likely-subtags expansion doesn't fill
/// them in (shouldn't happen for a valid `language` subtag, but a plain
/// string comparison stays simple either way).
fn maximized_triple(id: &icu_locale::LanguageIdentifier) -> (String, String, String) {
    let mut id = id.clone();
    locale_expander().maximize(&mut id);
    (
        id.language.to_string(),
        id.script.map(|s| s.to_string()).unwrap_or_default(),
        id.region.map(|r| r.to_string()).unwrap_or_default(),
    )
}

/// Resolves an arbitrary requested tag to one of `CURATED_LOCALES`,
/// falling back progressively rather than ever failing -- see this
/// file's own doc comment for the exact fallback order.
fn resolve_curated_locale(requested: &icu_locale::LanguageIdentifier) -> &'static str {
    let (req_lang, req_script, req_region) = maximized_triple(requested);
    let curated: Vec<(&'static str, (String, String, String))> = CURATED_LOCALES
        .iter()
        .map(|tag| {
            let id: icu_locale::LanguageIdentifier =
                tag.parse().expect("CURATED_LOCALES entries must be valid tags");
            (*tag, maximized_triple(&id))
        })
        .collect();

    // (a) exact language+script+region match.
    if let Some((tag, _)) = curated
        .iter()
        .find(|(_, (lang, script, region))| *lang == req_lang && *script == req_script && *region == req_region)
    {
        return tag;
    }
    // (b) language+region match, ignoring script.
    if let Some((tag, _)) = curated
        .iter()
        .find(|(_, (lang, _, region))| *lang == req_lang && *region == req_region)
    {
        return tag;
    }
    // (c) language+script match, ignoring region.
    if let Some((tag, _)) = curated
        .iter()
        .find(|(_, (lang, script, _))| *lang == req_lang && *script == req_script)
    {
        return tag;
    }
    // (d) bare language match.
    if let Some((tag, _)) = curated.iter().find(|(_, (lang, _, _))| *lang == req_lang) {
        return tag;
    }
    DEFAULT_CURATED_LOCALE
}

fn json_string_or_null(value: Option<&str>) -> String {
    value
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()))
        .unwrap_or_else(|| "null".to_string())
}

/// The Unicode extension keywords ECMA-402 cares about, each falling
/// back to the ECMA-402 default when the tag doesn't specify one --
/// *not* a locale-appropriate default (e.g. real Node's `th-TH` defaults
/// to `"buddhist"`, not `"gregory"`); that's M5's job, once
/// non-Gregorian `DateTimeFormatter` support exists to make a real
/// per-locale default calendar meaningful (`thaw-icu-data` already
/// vendors the `CalendarPreferredV1` marker M5 will need for it).
struct UnicodeKeywords {
    calendar: String,
    numbering_system: String,
    collation: Option<String>,
}

fn unicode_keywords(locale: &icu_locale::Locale) -> UnicodeKeywords {
    let get = |key: &str| {
        locale
            .extensions
            .unicode
            .keywords
            .get(&key.parse().expect("a 2-letter ASCII key is a valid unicode extension key"))
            .map(|value| value.to_string())
    };
    UnicodeKeywords {
        calendar: get("ca").unwrap_or_else(|| "gregory".to_string()),
        numbering_system: get("nu").unwrap_or_else(|| "latn".to_string()),
        collation: get("co"),
    }
}

/// `__thaw_intl_locale_parse(tag)`: the requested tag's own canonical
/// identity -- `{"valid":true,"language":..,"script":..,"region":..,
/// "calendar":..,"numberingSystem":..,"collation":..}`, or
/// `{"valid":false}` for a tag that doesn't parse as BCP-47 syntax (a
/// `RangeError` in `Intl.Locale`'s constructor). Deliberately **not**
/// resolved against the curated locale list -- `Intl.Locale` is a pure
/// BCP-47/likely-subtags identity object in real ECMA-402, entirely
/// independent of which locales this polyfill actually ships formatting
/// data for (confirmed against real Node: `new Intl.Locale("zz-Zzzz-
/// ZZ")` doesn't throw, and `new Intl.Locale("ja").script` is `null`,
/// *not* auto-maximized to `"Jpan"` -- `maximize()`/`minimize()` are
/// separate, explicit methods). `calendar`/`numberingSystem`/`collation`
/// are `null` (JS `undefined`, per real `Intl.Locale`) when the tag
/// doesn't specify them -- unlike `intl_locale_resolve_json`, this does
/// **not** fill in an ECMA-402 default, since real `new Intl.Locale
/// ("de").calendar` is `undefined`, not `"gregory"` (confirmed against
/// real Node). Use `intl_locale_resolve_json` instead when a *formatter*
/// needs a concrete calendar/numbering system to pick data with.
fn intl_locale_parse_json(tag: &str) -> String {
    use std::str::FromStr;
    let Ok(locale) = icu_locale::Locale::from_str(tag) else {
        return r#"{"valid":false}"#.to_string();
    };
    let get = |key: &str| {
        locale
            .extensions
            .unicode
            .keywords
            .get(&key.parse().expect("a 2-letter ASCII key is a valid unicode extension key"))
            .map(|value| value.to_string())
    };
    format!(
        r#"{{"valid":true,"language":"{}","script":{},"region":{},"calendar":{},"numberingSystem":{},"collation":{}}}"#,
        locale.id.language,
        json_string_or_null(locale.id.script.as_ref().map(|s| s.as_str())),
        json_string_or_null(locale.id.region.as_ref().map(|r| r.as_str())),
        json_string_or_null(get("ca").as_deref()),
        json_string_or_null(get("nu").as_deref()),
        json_string_or_null(get("co").as_deref()),
    )
}

/// `__thaw_intl_locale_resolve(tag)`: resolves a requested BCP-47 tag to
/// `{"valid":true,"locale":"<curated tag>","language":..,"script":..,
/// "region":..,"calendar":..,"numberingSystem":..,"collation":..}`, or
/// `{"valid":false}` only for a tag that doesn't even parse as BCP-47
/// syntax. An *internal* primitive for formatters (`DateTimeFormat`/
/// `NumberFormat`/etc, M4+) to pick which curated dataset to render
/// with -- not what `Intl.Locale` itself reports (see
/// `intl_locale_parse_json`'s doc comment for that distinction). A tag
/// outside the curated list degrades gracefully to the nearest curated
/// ancestor (matching language, then matching language+region ignoring
/// script, then language+script ignoring region, then bare language),
/// finally `en-US` -- never fails, matching this polyfill's existing
/// graceful-degradation philosophy (an unrecognized `timeZone` is the
/// one thing that legitimately throws, per `intl.rs`).
fn intl_locale_resolve_json(tag: &str) -> String {
    use std::str::FromStr;
    let Ok(locale) = icu_locale::Locale::from_str(tag) else {
        return r#"{"valid":false}"#.to_string();
    };
    let curated_tag = resolve_curated_locale(&locale.id);
    let curated_id: icu_locale::LanguageIdentifier =
        curated_tag.parse().expect("resolve_curated_locale returns a valid tag");
    let keywords = unicode_keywords(&locale);

    format!(
        r#"{{"valid":true,"locale":"{}","language":"{}","script":{},"region":{},"calendar":"{}","numberingSystem":"{}","collation":{}}}"#,
        curated_tag,
        curated_id.language,
        json_string_or_null(curated_id.script.as_ref().map(|s| s.as_str())),
        json_string_or_null(curated_id.region.as_ref().map(|r| r.as_str())),
        keywords.calendar,
        keywords.numbering_system,
        json_string_or_null(keywords.collation.as_deref()),
    )
}

/// `__thaw_intl_locale_maximize(tag)`: BCP-47 likely-subtags expansion
/// (`Intl.Locale.prototype.maximize()`, M3) -- `{"valid":true,
/// "language":..,"script":..,"region":..}` or `{"valid":false}` for a
/// tag that doesn't parse.
fn intl_locale_maximize_json(tag: &str) -> String {
    intl_locale_transform_json(tag, |expander, id| {
        expander.maximize(id);
    })
}

/// `__thaw_intl_locale_minimize(tag)`: the converse of `maximize` --
/// drops the script/region subtags that are already implied by the
/// language alone (`Intl.Locale.prototype.minimize()`, M3).
fn intl_locale_minimize_json(tag: &str) -> String {
    intl_locale_transform_json(tag, |expander, id| {
        expander.minimize(id);
    })
}

fn intl_locale_transform_json(
    tag: &str,
    transform: impl FnOnce(&icu_locale::LocaleExpander, &mut icu_locale::LanguageIdentifier),
) -> String {
    use std::str::FromStr;
    let Ok(mut id) = icu_locale::LanguageIdentifier::from_str(tag) else {
        return r#"{"valid":false}"#.to_string();
    };
    transform(&locale_expander(), &mut id);
    format!(
        r#"{{"valid":true,"language":"{}","script":{},"region":{}}}"#,
        id.language,
        json_string_or_null(id.script.as_ref().map(|s| s.as_str())),
        json_string_or_null(id.region.as_ref().map(|r| r.as_str())),
    )
}
