/// The real number of minor-unit decimal digits for an ISO 4217
/// currency code (e.g. 0 for JPY/KRW, 2 for USD/EUR, 3 for BHD/KWD),
/// via the vendored `CurrencyFractionsV1` singleton data -- not a
/// hand-picked list of exceptions. Returns `None` (caller falls back to
/// the ECMA-402 default of 2) if the currency code doesn't parse or the
/// data fails to resolve.
fn intl_currency_fraction_digits(currency: &str) -> Option<u8> {
    use icu_experimental::dimension::provider::currency::fractions::CurrencyFractionsV1;
    use icu_provider::{DataProvider, DataRequest};
    use std::str::FromStr;

    let currency_type =
        icu_experimental::dimension::currency::CurrencyType::from_str(&currency.to_ascii_lowercase()).ok()?;
    let response =
        DataProvider::<CurrencyFractionsV1>::load(&thaw_icu_data::ThawIcuDataProvider, DataRequest::default())
            .ok()?;
    Some(response.payload.get().resolve(currency_type).digits)
}

/// Render the dimension formatter's own parts so a currency's rounded core
/// and every affix remain intact when grouping is disabled.
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

fn intl_percent_format(locale_tag: &str, digits: &str, grouping: bool) -> Option<String> {
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
    intl_style_output(&formatter.format(&decimal), grouping)
}

fn intl_currency_format(
    locale_tag: &str,
    digits: &str,
    currency: &str,
    display: &str,
    grouping: bool,
) -> Option<String> {
    use icu_experimental::dimension::currency::formatter::{
        CurrencyFormatter, CurrencyFormatterPreferences,
    };
    use std::str::FromStr;

    let locale = icu_locale::Locale::from_str(locale_tag).ok()?;
    let curated: icu_locale::Locale = resolve_curated_locale(&locale.id).parse().ok()?;
    let mut prefs = CurrencyFormatterPreferences::from(&curated);
    prefs.numbering_system = CurrencyFormatterPreferences::from(&locale).numbering_system;
    let currency = currency
        .to_ascii_lowercase()
        .parse::<icu_experimental::dimension::currency::CurrencyType>()
        .ok()?;
    let decimal = icu_decimal::input::Decimal::from_str(digits).ok()?;
    let provider = &thaw_icu_data::ThawIcuDataProvider;
    match display {
        "code" => {
            let formatter = CurrencyFormatter::try_new_code_unstable(provider, prefs, currency, Default::default()).ok()?;
            intl_style_output(&formatter.format_fixed_decimal(&decimal), grouping)
        }
        "name" => {
            let formatter = CurrencyFormatter::try_new_name_unstable(provider, prefs, currency).ok()?;
            intl_style_output(&formatter.format_fixed_decimal(&decimal), grouping)
        }
        "narrowSymbol" => {
            let formatter = CurrencyFormatter::try_new_symbol_narrow_unstable(provider, prefs, currency, Default::default()).ok()?;
            intl_style_output(&formatter.format_fixed_decimal(&decimal), grouping)
        }
        _ => {
            let formatter = CurrencyFormatter::try_new_symbol_unstable(provider, prefs, currency, Default::default()).ok()?;
            intl_style_output(&formatter.format_fixed_decimal(&decimal), grouping)
        }
    }
}

fn intl_unit_format(locale_tag: &str, digits: &str, unit: &str, width: &str, grouping: bool) -> Option<String> {
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
        let output = names.get().get(decimal.into(), plurals)
            .interpolate((decimal_formatter.format(decimal),));
        intl_style_output(&output, grouping)
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
                output = format::<$marker>(&data_locale, attributes, &decimal, &decimal_formatter, &plurals, grouping);
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
