/// The real number of minor-unit decimal digits for an ISO 4217
/// currency code (e.g. 0 for JPY/KRW, 2 for USD/EUR, 3 for BHD/KWD),
/// via the vendored `CurrencyFractionsV1` singleton data -- not a
/// hand-picked list of exceptions. Returns `None` (caller falls back to
/// the ECMA-402 default of 2) if the currency code doesn't parse or the
/// data fails to resolve.
fn intl_currency_fraction_info(currency: &str) -> Option<icu_experimental::dimension::provider::currency::fractions::FractionInfo> {
    use icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1;
    use icu_provider::{DataProvider, DataRequest};
    use std::str::FromStr;

    let currency_type =
        icu_experimental::dimension::currency::CurrencyType::from_str(&currency.to_ascii_lowercase()).ok()?;
    let response =
        DataProvider::<CurrencyFractionsV1>::load(&thaw_icu_data::ThawIcuDataProvider, DataRequest::default())
            .ok()?;
    Some(response.payload.get().resolve(currency_type))
}

fn intl_currency_fraction_digits(currency: &str) -> Option<u8> {
    intl_currency_fraction_info(currency).map(|info| info.digits)
}

// Mirror the cached CurrencyFormatter::apply_precision for its probe only.
// The caller's already-rounded exact numeric core remains authoritative.
fn intl_currency_probe_after_native_rounding(digits: &str, currency: &str) -> Option<String> {
    use fixed_decimal::{RoundingIncrement, SignedRoundingMode, UnsignedRoundingMode};
    use icu_experimental::dimension::provider::currency::fractions::Rounding;
    use std::str::FromStr;
    let mut value = icu_decimal::input::Decimal::from_str(digits).ok()?;
    let info = intl_currency_fraction_info(currency)?;
    let precision = i16::from(info.digits);
    let (magnitude, increment) = match info.rounding {
        Rounding::R50 => (-precision + 1, RoundingIncrement::MultiplesOf5),
        Rounding::R20 => (-precision + 1, RoundingIncrement::MultiplesOf2),
        Rounding::R5 => (-precision, RoundingIncrement::MultiplesOf5),
        Rounding::R1 => (-precision, RoundingIncrement::MultiplesOf1),
        _ => return None,
    };
    value.round_with_mode_and_increment(
        magnitude, SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand), increment,
    );
    Some(value.to_string())
}

/// Render the dimension formatter output. GROUP parts are omitted when a
/// formatter exposes them; the large-input path also verifies both numeric
/// grouping possibilities before replacing the native core.
fn intl_style_output(formatted: &impl writeable::Writeable, grouping: bool) -> Option<String> {
    struct StyleSink { output: String, grouping: bool }
    impl std::fmt::Write for StyleSink {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.output.push_str(text);
            Ok(())
        }
    }
    impl writeable::PartsWrite for StyleSink {
        type SubPartsWrite = Self;
        fn with_part(
            &mut self,
            part: writeable::Part,
            mut write: impl FnMut(&mut Self) -> std::fmt::Result,
        ) -> std::fmt::Result {
            if !self.grouping && part == icu_decimal::parts::GROUP { Ok(()) } else { write(self) }
        }
    }
    let mut sink = StyleSink { output: String::new(), grouping };
    formatted.write_to_parts(&mut sink).ok()?;
    Some(sink.output)
}

// The cached dimension formatters do not all emit Writeable parts. Compare
// two same-plural representatives to identify the numeric slot by position,
// then verify the complete locale-rendered core there before replacing it.
fn intl_alternate_representative(representative: &str) -> Option<String> {
    let (sign, unsigned) = representative.strip_prefix('-')
        .map_or_else(|| representative.strip_prefix('+')
            .map_or(("", representative), |unsigned| ("+", unsigned)),
            |unsigned| ("-", unsigned));
    Some(format!("{sign}2{}", unsigned.strip_prefix('1')?))
}

// The dimension formatters use the curated locale as their decimal-data
// base, with only the requested numbering-system override. This probe must
// use that same data path; the ordinary number formatter has an extra
// numbering-system representative-locale workaround for large exact output.
fn intl_dimension_numeric_probe(locale_tag: &str, digits: &str, grouping: bool) -> Option<String> {
    use std::str::FromStr;
    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let mut prefs = icu_decimal::DecimalFormatterPreferences::from(&curated);
    prefs.numbering_system = icu_decimal::DecimalFormatterPreferences::from(&locale).numbering_system;
    let options: icu_decimal::options::DecimalFormatterOptions = if grouping {
        icu_decimal::options::GroupingStrategy::Auto
    } else {
        icu_decimal::options::GroupingStrategy::Never
    }.into();
    let formatter = icu_decimal::DecimalFormatter::try_new_unstable(
        &thaw_icu_data::ThawIcuDataProvider, prefs, options,
    ).ok()?;
    let decimal = icu_decimal::input::Decimal::from_str(digits).ok()?;
    Some(formatter.format(&decimal).to_string())
}

fn intl_replace_style_numeric(
    locale: &str, first_numeric: &str, second_numeric: &str, exact_core: &str,
    first: String, second: &str, grouping: bool,
) -> Option<String> {
    // `chars()` locates a scalar difference; advance by UTF-8 bytes so the
    // verified replacement range is valid even for non-Latin affixes/digits.
    let mut offset = 0;
    for (left, right) in first.chars().zip(second.chars()) {
        if left != right { break; }
        offset += left.len_utf8();
    }
    if offset == first.len() { return None; }
    let unsigned = first_numeric.strip_prefix('-').or_else(|| first_numeric.strip_prefix('+')).unwrap_or(first_numeric);
    let alternate_unsigned = second_numeric.strip_prefix('-').or_else(|| second_numeric.strip_prefix('+')).unwrap_or(second_numeric);
    for grouped in [grouping, !grouping] {
        let native_core = intl_dimension_numeric_probe(locale, unsigned, grouped)?;
        let alternate_core = intl_dimension_numeric_probe(locale, alternate_unsigned, grouped)?;
        if first[offset..].starts_with(&native_core)
            && second[offset..].starts_with(&alternate_core)
            && first[offset + native_core.len()..] == second[offset + alternate_core.len()..]
        {
            let mut output = first;
            output.replace_range(offset..offset + native_core.len(), exact_core);
            return Some(output);
        }
    }
    None
}

fn intl_percent_format(locale_tag: &str, digits: &str, grouping: bool) -> Option<String> {
    intl_percent_format_impl(locale_tag, digits, grouping, None)
}

fn intl_percent_format_large(locale_tag: &str, representative: &str, core: &str, grouping: bool) -> Option<String> {
    intl_percent_format_impl(locale_tag, representative, grouping, Some(core))
}

fn intl_percent_format_impl(locale_tag: &str, digits: &str, grouping: bool, exact_core: Option<&str>) -> Option<String> {
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let mut prefs =
        icu_experimental::dimension::percent::formatter::PercentFormatterPreferences::from(&curated);
    prefs.numbering_system =
        icu_experimental::dimension::percent::formatter::PercentFormatterPreferences::from(&locale)
            .numbering_system;
    let formatter = icu_experimental::dimension::percent::formatter::PercentFormatter::try_new_unstable(
        &thaw_icu_data::ThawIcuDataProvider,
        prefs,
        Default::default(),
    )
    .ok()?;
    let decimal = icu_decimal::input::Decimal::from_str(digits).ok()?;
    let first = intl_style_output(&formatter.format(&decimal), grouping)?;
    let Some(core) = exact_core else { return Some(first) };
    let alternate = intl_alternate_representative(digits)?;
    let alternate_decimal = icu_decimal::input::Decimal::from_str(&alternate).ok()?;
    let second = intl_style_output(&formatter.format(&alternate_decimal), grouping)?;
    intl_replace_style_numeric(locale_tag, digits, &alternate, core, first, &second, grouping)
}

// The public currency-name data chooses plural names from the caller's
// exact displayed digits. CurrencyFormatter rounds to its minor units before
// selecting a name, which can erase an explicitly requested extra fraction.
fn intl_currency_name_large(
    locale_tag: &str, representative: &str, core: &str, currency: &str,
) -> Option<String> {
    use icu_experimental::dimension::currency::{
        formatter::CurrencyFormatterPreferences, CurrencyType,
    };
    use icu_experimental::dimension::provider::currency::{
        extended::CurrencyExtendedDataV1, patterns::CurrencyPatternsDataV1,
    };
    use icu_provider::{
        DataIdentifierBorrowed, DataMarkerAttributes, DataProvider, DataRequest, ResultDataError,
        marker::DataMarkerExt,
    };
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let mut prefs = CurrencyFormatterPreferences::from(&curated);
    prefs.numbering_system = CurrencyFormatterPreferences::from(&locale).numbering_system;
    let currency_type = CurrencyType::from_str(&currency.to_ascii_lowercase()).ok()?;
    let provider = &thaw_icu_data::ThawIcuDataProvider;
    let patterns = DataProvider::<CurrencyPatternsDataV1>::load(provider, DataRequest::default()).ok()?.payload;
    let data_locale = CurrencyPatternsDataV1::make_locale(prefs.locale_preferences);
    let iso_code = currency_type.iso_code();
    let attributes = DataMarkerAttributes::try_from_str(iso_code.as_str()).ok()?;
    let extended = DataProvider::<CurrencyExtendedDataV1>::load(provider, DataRequest {
        id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, &data_locale),
        ..Default::default()
    }).allow_identifier_not_found().ok()?;
    let plural_rules = icu_plurals::PluralRules::try_new_cardinal_unstable(
        provider, icu_plurals::PluralRulesPreferences::from(&prefs),
    ).ok()?;
    let decimal = icu_decimal::input::Decimal::from_str(representative).ok()?;
    let operands = icu_plurals::PluralOperands::from(&decimal);
    let decimal_formatter = icu_decimal::DecimalFormatter::try_new_unstable(
        provider, icu_decimal::DecimalFormatterPreferences::from(&prefs), Default::default(),
    ).ok()?;
    if let Some(extended) = extended {
        let name = extended.payload.get().get(operands, &plural_rules);
        let pattern = patterns.get().get(operands, &plural_rules);
        let value = pattern.interpolate((core, name));
        intl_style_output(&icu_decimal::AbstractFormatter::format_sign(
            &decimal_formatter, value, decimal.sign,
        ), true)
    } else {
        let pattern = patterns.get().elements.get_default().1;
        let value = pattern.interpolate((core, iso_code.as_str()));
        intl_style_output(&icu_decimal::AbstractFormatter::format_sign(
            &decimal_formatter, value, decimal.sign,
        ), true)
    }
}

fn intl_currency_format(
    locale_tag: &str, digits: &str, currency: &str, display: &str, grouping: bool,
) -> Option<String> {
    intl_currency_format_impl(locale_tag, digits, currency, display, grouping, None)
}

fn intl_currency_format_large(
    locale_tag: &str, representative: &str, core: &str,
    currency: &str, display: &str, grouping: bool,
) -> Option<String> {
    intl_currency_format_impl(locale_tag, representative, currency, display, grouping, Some(core))
}

fn intl_currency_format_impl(
    locale_tag: &str, digits: &str, currency: &str, display: &str,
    grouping: bool, exact_core: Option<&str>,
) -> Option<String> {
    use icu_experimental::dimension::currency::formatter::{
        CurrencyFormatter, CurrencyFormatterPreferences,
    };
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let mut prefs = CurrencyFormatterPreferences::from(&curated);
    prefs.numbering_system = CurrencyFormatterPreferences::from(&locale).numbering_system;
    let currency_type = currency
        .to_ascii_lowercase()
        .parse::<icu_experimental::dimension::currency::CurrencyType>()
        .ok()?;
    if display == "name" {
        if let Some(core) = exact_core {
            return intl_currency_name_large(locale_tag, digits, core, currency);
        }
    }
    let decimal = icu_decimal::input::Decimal::from_str(digits).ok()?;
    let provider = &thaw_icu_data::ThawIcuDataProvider;
    let formatter = match display {
        "code" => CurrencyFormatter::try_new_code_unstable(provider, prefs, currency_type, Default::default()).ok()?,
        "name" => CurrencyFormatter::try_new_name_unstable(provider, prefs, currency_type).ok()?,
        "narrowSymbol" => CurrencyFormatter::try_new_symbol_narrow_unstable(provider, prefs, currency_type, Default::default()).ok()?,
        _ => CurrencyFormatter::try_new_symbol_unstable(provider, prefs, currency_type, Default::default()).ok()?,
    };
    let first = intl_style_output(&formatter.format_fixed_decimal(&decimal), grouping)?;
    let Some(core) = exact_core else { return Some(first) };
    let alternate = intl_alternate_representative(digits)?;
    let alternate_decimal = icu_decimal::input::Decimal::from_str(&alternate).ok()?;
    let second = intl_style_output(&formatter.format_fixed_decimal(&alternate_decimal), grouping)?;
    let first_numeric = intl_currency_probe_after_native_rounding(digits, currency)?;
    let second_numeric = intl_currency_probe_after_native_rounding(&alternate, currency)?;
    intl_replace_style_numeric(
        locale_tag, &first_numeric, &second_numeric, core, first, &second, grouping,
    )
}

fn intl_unit_format(locale_tag: &str, digits: &str, unit: &str, width: &str, grouping: bool) -> Option<String> {
    intl_unit_format_impl(locale_tag, digits, unit, width, grouping, None)
}

fn intl_unit_format_large(
    locale_tag: &str, representative: &str, core: &str,
    unit: &str, width: &str, grouping: bool,
) -> Option<String> {
    intl_unit_format_impl(locale_tag, representative, unit, width, grouping, Some(core))
}

fn intl_unit_format_impl(
    locale_tag: &str, digits: &str, unit: &str, width: &str,
    grouping: bool, exact_core: Option<&str>,
) -> Option<String> {
    use icu_experimental::dimension::provider::units::{
        categorized_display_names::*, display_names::UnitsDisplayNames,
    };
    use icu_provider::{
        DataIdentifierBorrowed, DataMarker, DataMarkerAttributes, DataProvider, DataRequest,
        DynamicDataMarker,
    };
    use std::str::FromStr;

    fn format<M>(
        locale: &icu_provider::DataLocale,
        attributes: &DataMarkerAttributes,
        decimal: &icu_decimal::input::Decimal,
        decimal_formatter: &icu_decimal::DecimalFormatter,
        plurals: &icu_plurals::PluralRules,
        grouping: bool,
        exact_core: Option<&str>,
    ) -> Option<String>
    where
        M: DynamicDataMarker<DataStruct = UnitsDisplayNames<'static>> + DataMarker,
        thaw_icu_data::ThawIcuDataProvider: DataProvider<M>,
    {
        let names = DataProvider::<M>::load(
            &thaw_icu_data::ThawIcuDataProvider,
            DataRequest {
                id: DataIdentifierBorrowed::for_marker_attributes_and_locale(
                    attributes,
                    locale,
                ),
                ..Default::default()
            },
        )
        .ok()?
        .payload;
        let pattern = names.get().get(decimal.into(), plurals);
        if let Some(core) = exact_core {
            let signed = icu_decimal::AbstractFormatter::format_sign(
                decimal_formatter, core, decimal.sign,
            );
            intl_style_output(&pattern.interpolate((signed,)), grouping)
        } else {
            intl_style_output(&pattern.interpolate((decimal_formatter.format(decimal),)), grouping)
        }
    }

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let data_locale = icu_provider::DataLocale::from(curated.id.clone());
    let attribute_text = format!("{width}-{unit}");
    let attributes = DataMarkerAttributes::try_from_str(&attribute_text).ok()?;
    let decimal = icu_decimal::input::Decimal::from_str(digits).ok()?;
    let decimal_formatter = icu_decimal::DecimalFormatter::try_new_unstable(
        &thaw_icu_data::ThawIcuDataProvider,
        icu_decimal::DecimalFormatterPreferences::from(&locale),
        Default::default(),
    )
    .ok()?;
    let plurals = icu_plurals::PluralRules::try_new_cardinal_unstable(
        &thaw_icu_data::ThawIcuDataProvider,
        icu_plurals::PluralRulesPreferences::from(&curated),
    )
    .ok()?;
    macro_rules! try_markers {
        ($($marker:ty),+ $(,)?) => {{
            let mut output = None;
            $(if output.is_none() {
                output = format::<$marker>(&data_locale, attributes, &decimal, &decimal_formatter, &plurals, grouping, exact_core);
            })+
            output
        }};
    }
    try_markers!(
        UnitsNamesAreaCoreV1,
        UnitsNamesAreaExtendedV1,
        UnitsNamesAreaOutlierV1,
        UnitsNamesDurationCoreV1,
        UnitsNamesDurationExtendedV1,
        UnitsNamesDurationOutlierV1,
        UnitsNamesLengthCoreV1,
        UnitsNamesLengthExtendedV1,
        UnitsNamesLengthOutlierV1,
        UnitsNamesMassCoreV1,
        UnitsNamesMassExtendedV1,
        UnitsNamesMassOutlierV1,
        UnitsNamesVolumeCoreV1,
        UnitsNamesVolumeExtendedV1,
        UnitsNamesVolumeOutlierV1,
    )
}
