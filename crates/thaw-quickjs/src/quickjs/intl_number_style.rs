fn intl_percent_format(locale_tag: &str, digits: &str) -> Option<String> {
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
    Some(formatter.format(&decimal).to_string())
}

fn intl_currency_format(
    locale_tag: &str,
    digits: &str,
    currency: &str,
    display: &str,
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
    let output = match display {
        "code" => CurrencyFormatter::try_new_code_unstable(provider, prefs, currency, Default::default())
            .ok()?
            .format_fixed_decimal(&decimal)
            .to_string(),
        "name" => CurrencyFormatter::try_new_name_unstable(provider, prefs, currency)
            .ok()?
            .format_fixed_decimal(&decimal)
            .to_string(),
        "narrowSymbol" => CurrencyFormatter::try_new_symbol_narrow_unstable(
            provider,
            prefs,
            currency,
            Default::default(),
        )
        .ok()?
        .format_fixed_decimal(&decimal)
        .to_string(),
        _ => CurrencyFormatter::try_new_symbol_unstable(provider, prefs, currency, Default::default())
            .ok()?
            .format_fixed_decimal(&decimal)
            .to_string(),
    };
    Some(output)
}

fn intl_unit_format(locale_tag: &str, digits: &str, unit: &str, width: &str) -> Option<String> {
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
        let output =
            names
                .get()
                .get(decimal.into(), plurals)
                .interpolate((decimal_formatter.format(decimal),))
                .to_string();
        Some(output)
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
                output = format::<$marker>(&data_locale, attributes, &decimal, &decimal_formatter, &plurals);
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
