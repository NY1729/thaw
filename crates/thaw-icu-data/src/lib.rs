//! Baked (compile-time-embedded) `icu4x` CLDR/ICU data, scoped to a curated
//! locale list, for Thaw's multi-locale `Intl` polyfill
//! (`crates/thaw-quickjs/src/quickjs/intl*.rs`).
//!
//! This crate contains **only generated data** (`src/data/*.rs.data`, plus
//! the generated `src/data/mod.rs` module they're `include!`d from) --
//! there is no hand-written logic here beyond this file, which just wires
//! the generated `impl_data_provider!` macro onto `ThawIcuDataProvider`.
//! Mirrors why `crates/thaw-quickjs/src/quickjs/intl_time_zone_names.rs`
//! is its own file: mechanically generated, separately diffable and
//! regeneratable from hand-written logic.
//!
//! # Why baked data, not `compiled_data` or a runtime blob
//!
//! Every `icu_*` crate's default `compiled_data` Cargo feature bakes in
//! data for the *entire* CLDR locale set (~700 locales) -- explicitly
//! documented upstream as adding "tens of megabytes" for exactly this
//! reason. Thaw targets a single statically-linked Lambda executable with
//! an existing binary-size/RSS discipline (see `benchmarks/runtime/`), so
//! this crate instead vendors a *curated* locale list's worth of data,
//! generated once via the separate `icu4x-datagen` CLI and committed as
//! plain Rust source -- the same "fetch/generate once, commit the result,
//! no runtime file, no build-time network access" shape already used by
//! `jiff`'s `tzdb-bundle-always` feature (see `crates/thaw-quickjs/Cargo.
//! toml`) and by `intl_time_zone_names.rs`'s own hand-extracted table.
//!
//! # How this was generated (2026-09-12, `icu4x-datagen` 2.3.0)
//!
//! 1. Installed the generator: `cargo install icu4x-datagen --version
//!    2.3.0 --locked --features unstable` (the `unstable` feature is
//!    required to export `icu_experimental::relativetime`'s markers for
//!    `Intl.RelativeTimeFormat`; the *runtime* dependency on
//!    `icu_experimental` in `crates/thaw-quickjs/Cargo.toml` stays
//!    pinned tightly regardless, since that crate's own API can move in
//!    a semver-*minor* release).
//! 2. Built a throwaway probe binary (not part of this workspace) that
//!    actually calls every icu4x API this polyfill effort plans to use:
//!    `DateTimeFormatter` (both a plain field set and a
//!    `fieldsets::zone::SpecificLong` zone-name field set, so the
//!    generic *and* the timezone-name-rendering marker sets are both
//!    covered), `DecimalFormatter`, `ListFormatter` (`and`, wide),
//!    `PluralRules` (cardinal and ordinal), `Collator`,
//!    `WordSegmenter`/`SentenceSegmenter`/`LineSegmenter`/
//!    `GraphemeClusterSegmenter`, `RelativeTimeFormatter`
//!    (`try_new_long_day`), and `LocaleExpander::new_common().maximize
//!    (...)` (BCP-47 likely-subtags expansion) -- confirmed each one
//!    actually runs and produces correct real-locale output (e.g. `ja-JP`
//!    renders `"2026年9月12日"`, maximizes to `"ja-Jpan-JP"`, and
//!    `RelativeTimeFormatter` renders `"1日前"` for -1 day).
//! 3. Ran `icu4x-datagen --format baked --markers-for-bin
//!    <probe binary>` against that probe to get its reachable marker
//!    set. This under-counts one thing: the probe used `LocaleExpander::
//!    new_common()` (the crate's own default-compiled-data constructor,
//!    which bakes in ALL locales' likely-subtags data internally rather
//!    than going through any `DataProvider` this tool's binary analysis
//!    can trace), so `LocaleLikelySubtagsLanguageV1`/
//!    `LocaleLikelySubtagsScriptRegionV1`/`LocaleLikelySubtagsExtendedV1`/
//!    `LocaleParentsV1` were added back explicitly by name (this crate's
//!    own `LocaleExpander::try_new_common_unstable` -- see
//!    `intl_locale.rs`'s eventual real use, and this file's own test
//!    below -- does go through `ThawIcuDataProvider` for real, so those
//!    four markers are genuinely needed). Final generation, one shot:
//!    ```text
//!    icu4x-datagen \
//!      --format baked \
//!      --locales en-US en-GB es es-419 fr de it pt pt-BR nl sv pl ru uk \
//!                tr ar ar-SA he hi bn ja ko zh-Hans zh-Hant th vi id ms \
//!                fil el ro cs hu da fi nb \
//!      --markers <the 61 names --markers-for-bin reported> \
//!                LocaleLikelySubtagsLanguageV1 \
//!                LocaleLikelySubtagsScriptRegionV1 \
//!                LocaleLikelySubtagsExtendedV1 LocaleParentsV1 \
//!      --use-separate-crates \
//!      --cldr-tag latest --icuexport-tag latest \
//!      --segmenter-lstm-tag latest --tzdb-tag latest \
//!      -o out_final
//!    ```
//!    `--cldr-tag latest`/`--icuexport-tag latest` resolved to CLDR
//!    `48.2.1` at generation time (recorded here since `latest` is not
//!    reproducible by itself -- re-pin to this exact tag, or a newer one
//!    deliberately, when regenerating). Note `--markers-for-bin` and
//!    `-m/--markers` do **not** combine when passed together in the same
//!    invocation (the later one silently wins) -- if regenerating,
//!    either list every needed marker name explicitly (as above) or run
//!    `--markers-for-bin` alone and manually re-add any marker reached
//!    only through a `compiled_data`-style constructor the probe used
//!    instead of `ThawIcuDataProvider` directly.
//! 4. Copied `out_final/*.rs.data` and `out_final/mod.rs` into
//!    `src/data/` verbatim (no hand edits).
//! 5. (M4, same day) Building the actual `DateTimeFormatter` field-set
//!    dispatch in `crates/thaw-quickjs/src/quickjs/intl_datetime.rs`
//!    surfaced 4 more required markers the probe hadn't reached
//!    (`DatetimePatternsTimeV1`/`DatetimePatternsGlueV1`/
//!    `DatetimeNamesWeekdayV1`/`DatetimeNamesDayperiodV1` -- the probe
//!    never exercised a *time*-inclusive or *weekday*/day-period-
//!    inclusive field set). Added by re-running step 3 with those 4
//!    names appended to the `--markers` list (this is exactly the
//!    "manually re-add any marker reached only through a compiled_data-
//!    style constructor" case from step 3's own note -- except here the
//!    probe simply hadn't called that code path at all yet, not that it
//!    used `compiled_data`). If a future milestone's real usage needs
//!    still more markers, the fastest way to find out which is the same
//!    as this: try building the real Rust code against the current
//!    dataset and read the compiler's "trait not implemented" errors,
//!    which name the exact missing marker type.
//! 6. (M7, same day) `ListOrV1`/`ListUnitV1` added the same way --
//!    the M1 probe only exercised `ListFormatter::try_new_and`.
//! 7. (M9, same day) `CollationSpecialPrimariesV1`/`CollationRootV1`/
//!    `CollationJamoV1`/`NormalizerNfdDataV1`/`NormalizerNfdTablesV1`
//!    added the same way for `Intl.Collator` -- `icu_collator` needs
//!    real NFD normalization data internally (added `icu_normalizer`
//!    as a `thaw-icu-data` dependency for this reason), which the M1
//!    probe never touched at all.
//!
//! ## Regenerating (e.g. to extend the curated locale list)
//!
//! Re-run step 3 with a longer `--locales` list (and, if new API surface
//! is added to the polyfill, a probe binary exercising it too, so its
//! markers are included), then replace `src/data/*.rs.data` and
//! `src/data/mod.rs` with the new output. No other crate needs to change
//! for a locale/API combination the regenerated data now covers.
//!
//! ## Curated locale list (35 tags)
//!
//! `en-US en-GB es es-419 fr de it pt pt-BR nl sv pl ru uk tr ar ar-SA he
//! hi bn ja ko zh-Hans zh-Hant th vi id ms fil el ro cs hu da fi nb` --
//! see `docs/design/intl-polyfill.md` for the rationale behind each
//! locale's inclusion.
#![allow(clippy::redundant_static_lifetimes, clippy::octal_escapes)]

extern crate alloc;

/// The `icu_provider::DataProvider` implementation backing every icu4x
/// formatter/segmenter/collator this polyfill constructs. Zero runtime
/// state -- all data is `include!`d as `const`s by `impl_data_provider!`
/// below.
pub struct ThawIcuDataProvider;

include!("data/mod.rs");

impl_data_provider!(ThawIcuDataProvider);
include!("m12_data.rs");

#[cfg(test)]
mod tests {
    use super::ThawIcuDataProvider;
    use icu_datetime::fieldsets::YMD;
    use icu_datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
    use icu_locale::LocaleExpander;
    use icu_locale_core::Locale;
    use icu_plurals::{PluralRules, PluralRulesPreferences};

    #[test]
    fn answers_a_real_curated_locale_via_the_baked_provider() {
        let locale: Locale = "ja-JP".parse().unwrap();

        let prefs = DateTimeFormatterPreferences::from(&locale);
        let formatter =
            DateTimeFormatter::try_new_unstable(&ThawIcuDataProvider, prefs, YMD::long())
                .expect("ja-JP should be in the curated baked dataset");
        let date = icu_calendar::Date::try_new_iso(2026, 9, 12).unwrap();
        assert_eq!(formatter.format(&date).to_string(), "2026年9月12日");

        let prefs = PluralRulesPreferences::from(&locale);
        let rules = PluralRules::try_new_cardinal_unstable(&ThawIcuDataProvider, prefs)
            .expect("ja-JP plural rules should be in the curated baked dataset");
        assert_eq!(
            rules.category_for(1_u32),
            icu_plurals::PluralCategory::Other
        );
    }

    #[test]
    fn expands_a_locale_to_its_likely_subtags_via_the_baked_provider() {
        let mut langid = "ja".parse::<Locale>().unwrap().id;
        let expander = LocaleExpander::try_new_common_unstable(&ThawIcuDataProvider)
            .expect("likely-subtags data should be in the curated baked dataset");
        expander.maximize(&mut langid);
        assert_eq!(langid.to_string(), "ja-Jpan-JP");
    }
}
