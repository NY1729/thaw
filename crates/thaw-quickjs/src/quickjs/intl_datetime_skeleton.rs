// UTS-35 classical skeleton matching for `Intl.DateTimeFormat`.
//
// Based on icu4x 2.3.0's `icu_datetime::provider::skeleton::helpers`
// (Apache-2.0/MIT), which upstream gates behind its `datagen` feature --
// so it isn't reachable at runtime. The matching/adjustment structure is
// its, but the distance is ICU4C's own `dtTypes`/`DateTimeMatcher::
// getDistance` model (numeric widths equivalent, text widths 1-3 "short",
// `L`/`c`/`e` context offsets, 12h/24h distinguished) and the width
// adjustment follows ICU's `adjustFieldTypes` using the matched skeleton
// key's widths -- both tuned against Node/ICU 78. It operates on plain
// `Vec<Field>` skeletons and the `availableFormats` table baked by
// `crates/thaw-icu-data/tools/gen_datetime_skeletons.py`, instead of
// icu4x's datagen-only `reference::Skeleton`/`components::Bag` types.
//
// The point: icu4x's own `fieldsets::YMD`/`FieldSetBuilder` derive the
// requested field *widths* from the locale's `dateFormats` length
// patterns, which is why `de` `{month:'short'}` came out numeric
// (`dd.MM.y`). Node/ICU4C instead build the skeleton from the *requested
// option widths* (`yMMMd` -> `d. MMM y`). This module does the latter.

use core::cmp::Ordering as CmpOrdering;

use icu_datetime::provider::fields::{Field, FieldLength, FieldSymbol};
use icu_datetime::provider::pattern::runtime::{GenericPattern, Pattern};
use icu_datetime::provider::pattern::PatternItem;

// Distance constants from ICU's `DateTimePatternGenerator` (dtptngen):
// each field has a `type` value encoding numeric/text and width, and the
// distance between two fields is `abs(type_a - type_b)`. Note that, unlike
// icu4x's own datagen matcher, *numeric widths are not distinguished* (M
// and MM are both `DT_NUMERIC`) and text widths 1..=3 are all `DT_SHORT`.
const DT_NUMERIC: i32 = 0x100;
const DT_DELTA: i32 = 0x10;
const DT_SHORT: i32 = -0x103;
const DT_LONG: i32 = -0x104;
const DT_NARROW: i32 = -0x101;
const DT_SHORTER: i32 = -0x102;
// ICU's `EXTRA_FIELD` (candidate has a field the request doesn't) and
// `MISSING_FIELD` (request has a field the candidate doesn't).
const EXTRA_FIELD: u32 = 0x10000;
const MISSING_FIELD: u32 = 0x1000;
// A single-field request whose best match is text-vs-numeric is a bad fit.
const TEXT_VS_NUMERIC_DISTANCE: u32 = 0x200;

/// Local mirror of icu4x's `TextOrNumeric` (not exported at runtime).
#[derive(PartialEq, Eq)]
enum TextOrNumeric {
    Text,
    Numeric,
}

/// ICU's `dtTypes` type value for a field (numeric/text/width bracket), the
/// primary term of `DateTimeMatcher::getDistance`.
fn field_type_value(field: &Field) -> i32 {
    use icu_datetime::provider::fields::{Day, Hour, Month, TimeZone, Weekday, Year};
    use FieldSymbol as S;
    let text = |length: FieldLength| match length {
        FieldLength::Four => DT_LONG,
        FieldLength::Five => DT_NARROW,
        FieldLength::Six => DT_SHORTER,
        _ => DT_SHORT,
    };
    match field.symbol {
        S::Era => text(field.length),
        S::Year(Year::Cyclic) => text(field.length),
        S::Year(_) => DT_NUMERIC,
        S::Month(month) => {
            let base = match field.length {
                FieldLength::One | FieldLength::Two | FieldLength::NumericOverride(_) => DT_NUMERIC,
                _ => text(field.length),
            };
            match month {
                Month::Format => base,
                // `L` is `M` +/- `DT_DELTA` (ICU `dtTypes`).
                Month::StandAlone if base == DT_NUMERIC => base + DT_DELTA,
                Month::StandAlone => base - DT_DELTA,
            }
        }
        S::Week(_) => DT_NUMERIC,
        // ICU puts `D`/`F` in different field categories from `d`; without
        // the offset they tie (both numeric) and the earlier bucket wins.
        S::Day(Day::DayOfYear) => DT_NUMERIC + 0x1000,
        S::Day(Day::DayOfWeekInMonth) => DT_NUMERIC + 0x2000,
        S::Day(_) => DT_NUMERIC,
        S::Weekday(weekday) => {
            let base = text(field.length);
            // `e` is `E` - `DT_DELTA`, `c` is `E` - 2*`DT_DELTA` (ICU `dtTypes`).
            match weekday {
                Weekday::Format => base,
                Weekday::Local => base - DT_DELTA,
                Weekday::StandAlone => base - 2 * DT_DELTA,
            }
        }
        S::DayPeriod(_) => text(field.length),
        S::Hour(hour) => match hour {
            Hour::H11 => DT_NUMERIC + DT_DELTA,
            Hour::H23 => DT_NUMERIC + 10 * DT_DELTA,
            _ => DT_NUMERIC, // H12
        },
        S::Minute | S::Second(_) | S::DecimalSecond(_) => DT_NUMERIC,
        S::TimeZone(TimeZone::Iso | TimeZone::IsoWithZ) => DT_NUMERIC,
        S::TimeZone(_) => text(field.length),
    }
}

fn skeleton_idx(symbol: FieldSymbol) -> u8 {
    match symbol {
        FieldSymbol::Era => 0,
        FieldSymbol::Year(_) => 1,
        FieldSymbol::Month(_) => 2,
        FieldSymbol::Week(_) => 3,
        FieldSymbol::Day(_) => 4,
        FieldSymbol::Weekday(_) => 5,
        FieldSymbol::DayPeriod(_) => 6,
        FieldSymbol::Hour(_) => 7,
        FieldSymbol::Minute => 8,
        FieldSymbol::Second(_) | FieldSymbol::DecimalSecond(_) => 9,
        FieldSymbol::TimeZone(_) => 10,
    }
}

fn skeleton_cmp(a: FieldSymbol, b: FieldSymbol) -> CmpOrdering {
    skeleton_idx(a).cmp(&skeleton_idx(b))
}

fn field_length_ord(length: FieldLength) -> u8 {
    match length {
        FieldLength::One => 1,
        FieldLength::Two => 2,
        FieldLength::Three => 3,
        FieldLength::Four => 4,
        FieldLength::Five => 5,
        FieldLength::Six => 6,
        FieldLength::NumericOverride(_) => 0,
    }
}

fn field_cmp(a: &Field, b: &Field) -> CmpOrdering {
    skeleton_cmp(a.symbol, b.symbol).then(field_length_ord(a.length).cmp(&field_length_ord(b.length)))
}

fn length_type(field: &Field) -> TextOrNumeric {
    use icu_datetime::provider::fields::{TimeZone, Weekday, Year};
    use FieldSymbol as S;
    match field.symbol {
        S::Era => TextOrNumeric::Text,
        S::Year(Year::Cyclic) => TextOrNumeric::Text,
        S::Year(_) => TextOrNumeric::Numeric,
        S::Month(_) => match field.length {
            FieldLength::One | FieldLength::Two | FieldLength::NumericOverride(_) => {
                TextOrNumeric::Numeric
            }
            _ => TextOrNumeric::Text,
        },
        S::Week(_) | S::Day(_) => TextOrNumeric::Numeric,
        S::Weekday(Weekday::Format) => TextOrNumeric::Text,
        S::Weekday(_) => match field.length {
            FieldLength::One | FieldLength::Two => TextOrNumeric::Numeric,
            _ => TextOrNumeric::Text,
        },
        S::DayPeriod(_) => TextOrNumeric::Text,
        S::Hour(_) | S::Minute | S::Second(_) | S::DecimalSecond(_) => TextOrNumeric::Numeric,
        S::TimeZone(TimeZone::Iso | TimeZone::IsoWithZ) => TextOrNumeric::Numeric,
        S::TimeZone(_) => TextOrNumeric::Text,
    }
}

fn is_at_least_abbreviated(symbol: FieldSymbol) -> bool {
    matches!(
        symbol,
        FieldSymbol::Era
            | FieldSymbol::Year(icu_datetime::provider::fields::Year::Cyclic)
            | FieldSymbol::Weekday(icu_datetime::provider::fields::Weekday::Format)
            | FieldSymbol::DayPeriod(_)
            | FieldSymbol::TimeZone(icu_datetime::provider::fields::TimeZone::SpecificNonLocation)
    )
}

fn numeric_to_abbr(length: FieldLength) -> FieldLength {
    match length {
        FieldLength::One | FieldLength::Two => FieldLength::Three,
        other => other,
    }
}

fn length_from_count(count: u8) -> Option<FieldLength> {
    Some(match count {
        1 => FieldLength::One,
        2 => FieldLength::Two,
        3 => FieldLength::Three,
        4 => FieldLength::Four,
        5 => FieldLength::Five,
        6 => FieldLength::Six,
        _ => return None,
    })
}

/// Parses a UTS-35 skeleton string into canonically-ordered fields,
/// mirroring icu4x's `reference::Skeleton::try_from`. Returns `None` for
/// a skeleton containing a field symbol this crate can't represent
/// (`Q`/`w`/`W`/...); such a skeleton must be *skipped* entirely rather
/// than have the field dropped, or `MMMMW` would masquerade as `MMMM`.
pub fn parse_skeleton(skeleton: &str) -> Option<Vec<Field>> {
    use icu_datetime::provider::fields::{DayPeriod, Hour};

    let mut fields: Vec<Field> = Vec::new();
    let mut iter = skeleton.chars().peekable();
    while let Some(ch) = iter.next() {
        let mut count: u8 = 1;
        while iter.peek() == Some(&ch) {
            count += 1;
            iter.next();
        }
        let (symbol, count) = if ch == 'Z' {
            match count {
                1..=3 => ('x', 4),
                4 => ('O', 1),
                5 => ('X', 4),
                _ => (ch, count),
            }
        } else {
            (ch, count)
        };
        let mut symbol = FieldSymbol::try_from(symbol).ok()?;
        // Keep the Month/Weekday Format-vs-StandAlone context (ICU's
        // `dtTypes` gives `L`/`c`/`e` a different type value from `M`/`E`,
        // which matters for matching); only day periods and the 12-hour
        // variants are normalized.
        symbol = match symbol {
            FieldSymbol::DayPeriod(DayPeriod::AmPm | DayPeriod::NoonMidnight) => continue,
            FieldSymbol::Hour(Hour::H11 | Hour::H12) => FieldSymbol::Hour(Hour::H12),
            other => other,
        };
        let length = length_from_count(count)?;
        let field = Field { symbol, length };
        if !fields.contains(&field) {
            fields.push(field);
        }
    }
    fields.sort_by(field_cmp);
    Some(fields)
}

struct SkeletonMatch<'a> {
    skeleton: &'a [Field],
    value: &'a Pattern<'static>,
    distance: u32,
    missing_fields: usize,
}

fn find_best_skeleton<'a>(
    skeletons: &'a [(Vec<Field>, Pattern<'static>)],
    fields: &[Field],
) -> Option<SkeletonMatch<'a>> {
    let mut closest: Option<SkeletonMatch<'a>> = None;
    let mut closest_distance = u32::MAX;
    let mut closest_width = u32::MAX;
    let mut closest_exact = false;

    for (skeleton, value) in skeletons {
        let mut missing_fields = 0usize;
        let mut distance = 0u32;
        let mut width_distance = 0u32;

        let mut requested = fields.iter().peekable();
        let mut skeleton_fields = skeleton.iter().peekable();
        loop {
            match (requested.peek(), skeleton_fields.peek()) {
                (Some(requested_field), Some(skeleton_field)) => {
                    match skeleton_cmp(skeleton_field.symbol, requested_field.symbol) {
                        CmpOrdering::Less => {
                            skeleton_fields.next();
                            distance += EXTRA_FIELD;
                            continue;
                        }
                        CmpOrdering::Greater => {
                            distance += MISSING_FIELD;
                            missing_fields += 1;
                            requested.next();
                            continue;
                        }
                        CmpOrdering::Equal => {}
                    }
                    distance += (field_type_value(requested_field)
                        - field_type_value(skeleton_field))
                    .unsigned_abs();
                    if field_length_ord(requested_field.length)
                        != field_length_ord(skeleton_field.length)
                    {
                        width_distance += 1;
                    }
                    requested.next();
                    skeleton_fields.next();
                }
                (None, Some(_)) => {
                    distance += EXTRA_FIELD;
                    skeleton_fields.next();
                }
                (Some(_), None) => {
                    distance += MISSING_FIELD;
                    missing_fields += 1;
                    requested.next();
                }
                (None, None) => break,
            }
        }

        // Break ties (the primary type distance treats numeric widths as
        // equal, which Chinese/Dangi's `rU` patterns need) by the number of
        // field-width mismatches, then by an exact skeleton match. This is
        // ICU's `dtTypes` numeric `+length` term, applied only when the
        // primary distance is equal -- so `it`/`uk` `{month:'2-digit'}`
        // prefer `yMd` over `yyMMdd`, while Chinese keeps its `yyyyMMMMd`
        // (which already wins on the primary distance).
        let exact = skeleton.len() == fields.len()
            && skeleton.iter().zip(fields.iter()).all(|(a, b)| a == b);
        let better = distance < closest_distance
            || (distance == closest_distance && width_distance < closest_width)
            || (distance == closest_distance
                && width_distance == closest_width
                && exact
                && !closest_exact);
        if better {
            closest_distance = distance;
            closest_width = width_distance;
            closest_exact = exact;
            closest = Some(SkeletonMatch {
                skeleton,
                value,
                distance,
                missing_fields,
            });
        }
    }
    closest
}

enum BestSkeleton {
    AllFieldsMatch(Pattern<'static>),
    MissingOrExtraFields(Pattern<'static>),
    NoMatch,
}

fn is_bad_match_for_single_field(fields: &[Field], distance: u32) -> bool {
    fields.len() == 1 && distance >= TEXT_VS_NUMERIC_DISTANCE
}

fn pattern_from_items(items: Vec<PatternItem>) -> Pattern<'static> {
    Pattern::from(items)
}

fn adjust_pattern_field_lengths(
    fields: &[Field],
    matched_skeleton: &[Field],
    pattern: &mut Pattern<'static>,
) {
    let mut items: Vec<PatternItem> = pattern.items.iter().collect();
    for item in items.iter_mut() {
        let PatternItem::Field(pattern_field) = item else {
            continue;
        };
        let Some(requested_field) = fields
            .iter()
            .find(|field| skeleton_cmp(field.symbol, pattern_field.symbol).is_eq())
        else {
            continue;
        };
        // ICU's `adjustFieldTypes`: `skelFieldLen` is the *matched
        // skeleton's* length (the availableFormat key), `reqFieldLen` the
        // requested length. The pattern's field is set to the requested
        // length unless they already agree or the numeric/text type
        // differs (in which case the pattern's own length is kept).
        let skeleton_field = matched_skeleton
            .iter()
            .find(|field| skeleton_cmp(field.symbol, pattern_field.symbol).is_eq());
        let requested_length = if is_at_least_abbreviated(requested_field.symbol) {
            numeric_to_abbr(requested_field.length)
        } else {
            requested_field.length
        };
        let skeleton_length = skeleton_field
            .map(|field| field.length)
            .unwrap_or(pattern_field.length);
        if skeleton_length == requested_length
            || length_type(requested_field) != length_type(pattern_field)
        {
            continue;
        }
        *pattern_field = Field { length: requested_length, ..*pattern_field };
    }
    *pattern = pattern_from_items(items);
}

fn get_best_available_format_pattern(
    skeletons: &[(Vec<Field>, Pattern<'static>)],
    fields: &[Field],
) -> BestSkeleton {
    let matched = find_best_skeleton(skeletons, fields);
    let closest_distance = matched.as_ref().map(|m| m.distance).unwrap_or(u32::MAX);
    let closest_missing_fields = matched.as_ref().map(|m| m.missing_fields).unwrap_or(0);

    if is_bad_match_for_single_field(fields, closest_distance) {
        if let [field] = fields {
            return BestSkeleton::AllFieldsMatch(pattern_from_items(vec![
                PatternItem::Field(*field),
            ]));
        }
    }

    let Some(matched) = matched else {
        return BestSkeleton::NoMatch;
    };
    if closest_missing_fields == fields.len() {
        return BestSkeleton::NoMatch;
    }

    let mut pattern = matched.value.clone();
    adjust_pattern_field_lengths(fields, matched.skeleton, &mut pattern);

    if closest_distance >= EXTRA_FIELD {
        BestSkeleton::MissingOrExtraFields(pattern)
    } else {
        BestSkeleton::AllFieldsMatch(pattern)
    }
}

fn group_by_type(fields: &[Field]) -> (Vec<Field>, Vec<Field>) {
    let mut date = Vec::new();
    let mut time = Vec::new();
    for field in fields {
        match field.symbol {
            FieldSymbol::Era
            | FieldSymbol::Year(_)
            | FieldSymbol::Month(_)
            | FieldSymbol::Week(_)
            | FieldSymbol::Day(_)
            | FieldSymbol::Weekday(_) => date.push(*field),
            FieldSymbol::DayPeriod(_)
            | FieldSymbol::Hour(_)
            | FieldSymbol::Minute
            | FieldSymbol::Second(_)
            | FieldSymbol::TimeZone(_)
            | FieldSymbol::DecimalSecond(_) => time.push(*field),
        }
    }
    (date, time)
}

// CLDR has Bhm/Bhms, but no Bm/Bms. Match the locale's B+hour pattern,
// then remove only the hour (and, for second-only, its minute) plus the
// separators following those synthetic fields. This keeps B on its locale-
// specific side of the requested numeric fields.
fn flexible_period_without_hour_pattern(
    skeletons: &[(Vec<Field>, Pattern<'static>)],
    fields: &[Field],
) -> Option<Pattern<'static>> {
    use icu_datetime::provider::fields::{DayPeriod, Hour};

    let synthetic_minute = !fields.iter().any(|field| matches!(field.symbol, FieldSymbol::Minute));
    let mut match_fields = fields.to_vec();
    match_fields.push(Field { symbol: FieldSymbol::Hour(Hour::H12), length: FieldLength::One });
    if synthetic_minute {
        match_fields.push(Field { symbol: FieldSymbol::Minute, length: FieldLength::One });
    }
    match_fields.sort_by(field_cmp);
    let pattern = match get_best_available_format_pattern(skeletons, &match_fields) {
        BestSkeleton::AllFieldsMatch(pattern) | BestSkeleton::MissingOrExtraFields(pattern) => pattern,
        BestSkeleton::NoMatch => return None,
    };
    if !fields.iter().all(|requested| pattern.items.iter().any(|item| {
        matches!(item, PatternItem::Field(found) if found.symbol == requested.symbol)
    })) || !pattern.items.iter().any(|item| matches!(item, PatternItem::Field(field)
        if matches!(field.symbol, FieldSymbol::Hour(_)))) {
        return None;
    }
    let mut filtered = Vec::new();
    let mut after_synthetic = false;
    for item in pattern.items.iter() {
        match item {
            PatternItem::Field(field) if matches!(field.symbol, FieldSymbol::Hour(_))
                || (synthetic_minute && matches!(field.symbol, FieldSymbol::Minute)) => {
                after_synthetic = true;
            }
            PatternItem::Literal(_) if after_synthetic => {}
            other => {
                after_synthetic = false;
                filtered.push(other);
            }
        }
    }
    if !fields.iter().all(|requested| filtered.iter().any(|item| matches!(item,
        PatternItem::Field(found) if found.symbol == requested.symbol)))
        || filtered.iter().any(|item| matches!(item, PatternItem::Field(field)
            if !fields.iter().any(|requested| requested.symbol == field.symbol)))
        || !filtered.iter().any(|item| matches!(item, PatternItem::Field(field)
            if matches!(field.symbol, FieldSymbol::DayPeriod(DayPeriod::Flexible))))
    {
        return None;
    }
    Some(pattern_from_items(filtered))
}

/// Matches a full requested field set (date and/or time) against the
/// locale's `availableFormats` table and returns the final pattern,
/// combining date and time with `glue` (`[full, long, medium, short]`)
/// exactly as UTS-35/ICU4C do.
pub fn create_best_pattern_for_fields(
    skeletons: &[(Vec<Field>, Pattern<'static>)],
    glue: &[&str],
    fields: &[Field],
    hour_symbol: Option<FieldSymbol>,
) -> Option<Pattern<'static>> {
    let (date_fields, time_fields) = group_by_type(fields);

    let date_pattern = if date_fields.is_empty() {
        None
    } else {
        match get_best_available_format_pattern(skeletons, &date_fields) {
            BestSkeleton::AllFieldsMatch(p) | BestSkeleton::MissingOrExtraFields(p) => Some(p),
            BestSkeleton::NoMatch => None,
        }
    };
    let time_pattern = if time_fields.is_empty() {
        None
    } else if time_fields.iter().any(|field| matches!(field.symbol,
        FieldSymbol::DayPeriod(icu_datetime::provider::fields::DayPeriod::Flexible)))
        && !time_fields.iter().any(|field| matches!(field.symbol, FieldSymbol::Hour(_)))
        && time_fields.iter().any(|field| matches!(field.symbol,
            FieldSymbol::Minute | FieldSymbol::Second(_)))
    {
        // A failed B+minute/second match must not silently return date only.
        Some(flexible_period_without_hour_pattern(skeletons, &time_fields)?)
    } else {
        match get_best_available_format_pattern(skeletons, &time_fields) {
            BestSkeleton::AllFieldsMatch(p) | BestSkeleton::MissingOrExtraFields(p) => Some(p),
            BestSkeleton::NoMatch => None,
        }
    };

    let mut pattern = match (date_pattern, time_pattern) {
        (Some(date), Some(time)) => {
            let month_field = fields
                .iter()
                .find(|f| matches!(f.symbol, FieldSymbol::Month(_)));
            let glue_index = match month_field.map(|f| f.length) {
                Some(FieldLength::Four) => {
                    if fields.iter().any(|f| matches!(f.symbol, FieldSymbol::Weekday(_))) {
                        0 // full
                    } else {
                        1 // long
                    }
                }
                Some(FieldLength::Three) => 2, // medium
                _ => 3,                        // short
            };
            let glue = glue
                .get(glue_index)
                .and_then(|g| g.parse::<GenericPattern>().ok())
                .unwrap_or_default();
            glue.combined(date, time).ok()?
        }
        (Some(date), None) => date,
        (None, Some(time)) => time,
        (None, None) => return None,
    };

    if let Some(symbol) = hour_symbol {
        apply_hour_symbol(&mut pattern, symbol);
    }
    Some(pattern)
}

/// Forces the pattern's hour field to the requested symbol (e.g. `K` for
/// `hourCycle: 'h11'`), which skeleton matching alone can't express --
/// CLDR only carries `h`/`H` skeletons.
fn apply_hour_symbol(pattern: &mut Pattern<'static>, symbol: FieldSymbol) {
    let mut items: Vec<PatternItem> = pattern.items.iter().collect();
    for item in items.iter_mut() {
        if let PatternItem::Field(field) = item {
            if matches!(field.symbol, FieldSymbol::Hour(_)) {
                field.symbol = symbol;
            }
        }
    }
    *pattern = pattern_from_items(items);
}
