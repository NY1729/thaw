  // A practical `Intl` (ECMA-402) polyfill -- see
  // `docs/design/intl-polyfill.md` for the full, deliberate scope this
  // covers (real, jiff-backed IANA timezone offsets/DST; English names
  // and Latin digits only) and what it doesn't (no other locales, no
  // ICU per-zone long names, no `Intl.RelativeTimeFormat`/`Locale`/
  // `PluralRules`/`Collator`/`Segmenter` at all -- confirmed real
  // luxon's own feature-detection degrades gracefully without them).
  const INTL_MONTHS_LONG = [
    'January', 'February', 'March', 'April', 'May', 'June',
    'July', 'August', 'September', 'October', 'November', 'December',
  ];
  const INTL_MONTHS_SHORT = [
    'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
    'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
  ];
  const INTL_MONTHS_NARROW = ['J', 'F', 'M', 'A', 'M', 'J', 'J', 'A', 'S', 'O', 'N', 'D'];
  // Index 0 = Monday, matching `__thaw_intl_zoned_parts`'s own ISO
  // weekday numbering (`jiff`'s `to_monday_one_offset()`).
  const INTL_WEEKDAYS_LONG = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
  const INTL_WEEKDAYS_SHORT = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
  const INTL_WEEKDAYS_NARROW = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];
  const INTL_ERA = {
    long: ['Before Christ', 'Anno Domini'],
    short: ['BC', 'AD'],
    narrow: ['B', 'A'],
  };
  // `Intl.NumberFormat({style: 'unit', unit, unitDisplay})` -- only the
  // units real luxon's own `Duration.toHuman()` ever requests (the same
  // set its `orderedUnits` iterates). Each entry is `[style]:
  // [singularForm, pluralForm]`; `narrow` never distinguishes plural in
  // English, and `short` only does for year/month/week -- confirmed
  // against real Node's own `Intl.NumberFormat` output for every unit
  // and style combination, not guessed.
  const INTL_UNITS = {
    year: { long: ['year', 'years'], short: ['yr', 'yrs'], narrow: ['y', 'y'] },
    month: { long: ['month', 'months'], short: ['mth', 'mths'], narrow: ['m', 'm'] },
    week: { long: ['week', 'weeks'], short: ['wk', 'wks'], narrow: ['w', 'w'] },
    day: { long: ['day', 'days'], short: ['day', 'days'], narrow: ['d', 'd'] },
    hour: { long: ['hour', 'hours'], short: ['hr', 'hr'], narrow: ['h', 'h'] },
    minute: { long: ['minute', 'minutes'], short: ['min', 'min'], narrow: ['m', 'm'] },
    second: { long: ['second', 'seconds'], short: ['sec', 'sec'], narrow: ['s', 's'] },
    millisecond: { long: ['millisecond', 'milliseconds'], short: ['ms', 'ms'], narrow: ['ms', 'ms'] },
  };
  const INTL_SANCTIONED_UNITS = new Set([
    'acre', 'bit', 'byte', 'celsius', 'centimeter', 'day', 'degree', 'fahrenheit',
    'fluid-ounce', 'foot', 'gallon', 'gigabit', 'gigabyte', 'gram', 'hectare', 'hour',
    'inch', 'kilobit', 'kilobyte', 'kilogram', 'kilometer', 'liter', 'megabit',
    'megabyte', 'meter', 'microsecond', 'mile', 'mile-scandinavian', 'milliliter',
    'millimeter', 'millisecond', 'minute', 'month', 'ounce', 'percent', 'petabyte',
    'pound', 'second', 'stone', 'terabit', 'terabyte', 'week', 'yard', 'year',
  ]);

  function intlValidUnit(unit) {
    if (INTL_SANCTIONED_UNITS.has(unit)) return true;
    const parts = String(unit).split('-per-');
    return parts.length === 2 && parts.every(part => INTL_SANCTIONED_UNITS.has(part));
  }

  const INTL_LOCALE_TAGS = new WeakMap();

  function intlRequestedLocale(locales, fallback = 'en-US') {
    if (locales === undefined) return fallback;
    if (locales === null) throw new TypeError('Invalid locales list');
    const localeObject = typeof locales === 'string' || INTL_LOCALE_TAGS.has(locales)
      ? [locales] : Object(locales);
    const numericLength = +localeObject.length;
    const length = Number.isNaN(numericLength) || numericLength <= 0 ? 0
      : Math.min(Math.trunc(numericLength), Number.MAX_SAFE_INTEGER);
    const canonical = [];
    const seen = new Set();
    for (let index = 0; index < length; index++) {
      if (!(index in localeObject)) continue;
      const candidate = localeObject[index];
      if (typeof candidate !== 'string' &&
          (candidate === null || (typeof candidate !== 'object' && typeof candidate !== 'function'))) {
        throw new TypeError('Locale list elements must be strings or objects');
      }
      const tag = INTL_LOCALE_TAGS.has(candidate) ? INTL_LOCALE_TAGS.get(candidate) : `${candidate}`;
      if (typeof __thaw_intl_locale_parse === 'function') {
        const parsed = JSON.parse(__thaw_intl_locale_parse(tag));
        if (!parsed.valid) throw new RangeError(`Invalid language tag: ${tag}`);
        if (!seen.has(parsed.tag)) {
          seen.add(parsed.tag);
          canonical.push(parsed);
        }
      } else {
        if (!seen.has(tag)) {
          seen.add(tag);
          canonical.push({ tag });
        }
      }
    }
    for (const parsed of canonical) {
      if (typeof __thaw_intl_locale_resolve === 'function') {
        const resolved = JSON.parse(__thaw_intl_locale_resolve(parsed.tag));
        if (resolved.valid && resolved.language === parsed.language) return parsed.tag;
      } else {
        return parsed.tag;
      }
    }
    return fallback;
  }

  function intlZonedParts(timeZone, epochMs) {
    const zone = String(timeZone);
    const value = Number(epochMs);
    const native = JSON.parse(__thaw_intl_zoned_parts(zone, value));
    if (native.valid || !Number.isFinite(value) || Math.abs(value) > 8640000000000000) return native;
    const match = /^([+-])([0-9]{2}):([0-9]{2})$/.exec(zone);
    if (zone !== 'UTC' && (!match || Number(match[2]) > 23 || Number(match[3]) > 59)) return native;
    const offsetMinutes = match ? (match[1] === '-' ? -1 : 1) * (Number(match[2]) * 60 + Number(match[3])) : 0;
    const millis = Math.trunc(value);
    const local = millis + offsetMinutes * 60000;
    const days = Math.floor(local / 86400000);
    const time = local - days * 86400000;
    const z = days + 719468;
    const era = Math.floor(z / 146097);
    const doe = z - era * 146097;
    const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
    const y = yoe + era * 400;
    const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
    const mp = Math.floor((5 * doy + 2) / 153);
    const day = doy - Math.floor((153 * mp + 2) / 5) + 1;
    const month = mp < 10 ? mp + 3 : mp - 9;
    return {
      valid: true, year: y + (month <= 2 ? 1 : 0), month, day,
      hour: Math.floor(time / 3600000), minute: Math.floor(time / 60000) % 60,
      second: Math.floor(time / 1000) % 60, millisecond: time % 1000,
      weekday: ((days + 3) % 7 + 7) % 7 + 1, offsetMinutes,
      abbreviation: zone === 'UTC' ? 'UTC' : `GMT${zone}`, dst: false,
      timeZone: zone, timestampMs: millis, longName: null, longGenericName: null,
    };
  }

  function intlPad(value, width) {
    const text = String(Math.trunc(value));
    return text.length >= width ? text : '0'.repeat(width - text.length) + text;
  }

  // Splits two endpoint part lists into their shared prefix/suffix and the
  // differing middles -- the shape ECMA-402 range `formatToParts` needs
  // (`shared`/`startRange`/`endRange`). Used only when the opt-in ICU4C
  // backend isn't available, where the exact locale range pattern isn't
  // accessible.
  function intlPartitionRange(startParts, endParts) {
    const equal = (a, b) => a.type === b.type && a.value === b.value;
    let prefix = 0;
    while (prefix < startParts.length && prefix < endParts.length && equal(startParts[prefix], endParts[prefix])) {
      prefix += 1;
    }
    let suffix = 0;
    while (
      suffix < startParts.length - prefix &&
      suffix < endParts.length - prefix &&
      equal(startParts[startParts.length - 1 - suffix], endParts[endParts.length - 1 - suffix])
    ) {
      suffix += 1;
    }
    return {
      shared: startParts.slice(0, prefix),
      startMiddle: startParts.slice(prefix, startParts.length - suffix),
      endMiddle: endParts.slice(prefix, endParts.length - suffix),
      trailing: startParts.slice(startParts.length - suffix),
    };
  }

  // Real Intl's own `timeZoneName: 'shortOffset'`/`'longOffset'` (and
  // this polyfill's honest fallback for `'short'`/`'long'`/`*Generic`
  // when `jiff`'s abbreviation isn't a plain alphabetic code, or for
  // the styles this polyfill doesn't have real per-zone English names
  // for at all): `alwaysTwoDigitHour` forces a zero-padded hour and
  // always shows minutes (`longOffset`'s own real behavior, e.g.
  // `"GMT-04:00"`); otherwise minutes are shown only when nonzero and
  // the hour isn't padded (`shortOffset`'s own real behavior, e.g.
  // `"GMT-4"`, `"GMT+5:30"`, `"GMT+0"`).
  function intlOffsetString(offsetMinutes, alwaysTwoDigitHour) {
    const sign = offsetMinutes < 0 ? '-' : '+';
    const absolute = Math.abs(offsetMinutes);
    const hours = Math.floor(absolute / 60);
    const minutes = absolute % 60;
    const hourText = alwaysTwoDigitHour ? intlPad(hours, 2) : String(hours);
    const minuteText = alwaysTwoDigitHour || minutes ? `:${intlPad(minutes, 2)}` : '';
    return `GMT${sign}${hourText}${minuteText}`;
  }

  function intlTimeZoneName(style, zoned, locale) {
    if (style === 'longOffset') {
      return intlOffsetString(zoned.offsetMinutes, true);
    }
    if (style === 'long') {
      if (typeof __thaw_intl_time_zone_name === 'function') {
        const name = __thaw_intl_time_zone_name(locale, zoned.timeZone, zoned.timestampMs, zoned.offsetMinutes, false);
        if (name) return name;
      }
      // Real per-zone English long name (e.g. "Eastern Daylight
      // Time"/"Eastern Standard Time", DST-aware) -- `intl_zoned_
      // parts_json`'s `longName`, mechanically extracted from a real
      // `node`/ICU run (`intl_time_zone_names.rs`). Falls back to the
      // synthesized numeric offset for a zone the table has no entry
      // for (shouldn't happen for a real IANA zone, but keeps this
      // honestly degrading rather than throwing either way).
      return zoned.longName != null ? zoned.longName : intlOffsetString(zoned.offsetMinutes, true);
    }
    if (style === 'longGeneric') {
      if (typeof __thaw_intl_time_zone_name === 'function') {
        const name = __thaw_intl_time_zone_name(locale, zoned.timeZone, zoned.timestampMs, zoned.offsetMinutes, true);
        if (name) return name;
      }
      // Same table, DST-independent form (e.g. "Eastern Time" rather
      // than "Eastern Standard/Daylight Time").
      return zoned.longGenericName != null ? zoned.longGenericName : intlOffsetString(zoned.offsetMinutes, true);
    }
    // 'short' / 'shortOffset' / 'shortGeneric' -- 'shortOffset' always
    // wants a numeric offset; the others prefer `jiff`'s own
    // abbreviation (e.g. "EDT", "JST", "UTC") when it's a plain
    // alphabetic code, falling back to the same numeric form otherwise
    // (some zones only have a numeric designation even in real Intl).
    if (style !== 'shortOffset' && /^[A-Za-z]+$/.test(zoned.abbreviation)) {
      return zoned.abbreviation;
    }
    return intlOffsetString(zoned.offsetMinutes, false);
  }

  function intlHourParts(hourCycle, hour24) {
    // Real Intl's own quirk (confirmed against real Node): a
    // `hourCycle` of `'h23'`/`'h24'` always zero-pads the hour even
    // under the `'numeric'` style, unlike `'h11'`/`'h12'`, whose
    // `'numeric'` style never pads. `.formatToParts`'s own `'2-digit'`
    // style always pads regardless of cycle -- handled by the caller.
    let displayHour = hour24;
    let dayPeriod = null;
    if (hourCycle === 'h11') {
      displayHour = hour24 % 12;
      dayPeriod = hour24 < 12 ? 'AM' : 'PM';
    } else if (hourCycle === 'h12') {
      displayHour = hour24 % 12 === 0 ? 12 : hour24 % 12;
      dayPeriod = hour24 < 12 ? 'AM' : 'PM';
    } else if (hourCycle === 'h24') {
      displayHour = hour24 === 0 ? 24 : hour24;
    }
    return { displayHour, dayPeriod, padByDefault: hourCycle === 'h23' || hourCycle === 'h24' };
  }

  function intlIntegerOption(value, fallback, min, max) {
    if (value === undefined) return fallback;
    const n = Number(value);
    if (!Number.isFinite(n) || n < min || n > max) {
      throw new RangeError(`Invalid value: ${String(value)}`);
    }
    return Math.floor(n);
  }

  // Real `Intl.NumberFormat`'s `useGrouping` accepts a boolean (legacy)
  // or `"auto"`/`"always"`/`"min2"`, and `resolvedOptions` reports the
  // normalized string form (`true` -> `"always"`, `false` stays `false`).
  function intlNormalizeUseGrouping(value) {
    if (value === undefined) return 'auto';
    if (value === true) return 'always';
    if (value === false) return false;
    const text = String(value);
    if (text === 'auto' || text === 'always' || text === 'min2') return text;
    throw new RangeError(`Invalid useGrouping: ${text}`);
  }

  // Real ECMA-402 constructors throw a `RangeError` for an option value
  // outside its enum (rather than silently substituting the default).
  function intlEnumOption(value, allowed, fallback, name) {
    if (value === undefined) return fallback;
    const text = String(value);
    if (!allowed.includes(text)) throw new RangeError(`Invalid ${name}: ${text}`);
    return text;
  }

  const INTL_ROUNDING_MODES = [
    'ceil', 'floor', 'expand', 'trunc',
    'halfCeil', 'halfFloor', 'halfTrunc', 'halfEven', 'halfExpand',
  ];

  function intlIncrementDecimal(digits) {
    let index = digits.length - 1;
    while (index >= 0 && digits[index] === '9') index--;
    return index < 0 ? `1${'0'.repeat(digits.length)}`
      : `${digits.slice(0, index)}${String.fromCharCode(digits.charCodeAt(index) + 1)}${'0'.repeat(digits.length - index - 1)}`;
  }

  function intlNativeRangeInputFits(value) {
    if (typeof value === 'bigint') return value.toString().length <= 32000;
    if (typeof value !== 'string') return true;
    if (value.length > 32000) return false;
    const exponent = /[eE]([+-]?\d+)$/.exec(value);
    return !exponent || (BigInt(exponent[1]) >= -32000n && BigInt(exponent[1]) <= 32000n);
  }

  // Round an ASCII coefficient without constructing 10 ** discardedPlaces.
  // Only the guard digit, sticky bit, sign, and retained parity affect the
  // nine supported rounding modes, even for a very long input.
  function intlRoundDecimalDigits(digits, discardedPlaces, negative, mode) {
    if (discardedPlaces <= 0n) return digits;
    const length = BigInt(digits.length);
    const kept = discardedPlaces >= length ? '0' : digits.slice(0, digits.length - Number(discardedPlaces));
    const guardIndex = discardedPlaces > length ? -1 : digits.length - Number(discardedPlaces);
    const guard = guardIndex < 0 ? 0 : Number(digits[guardIndex]);
    const sticky = guardIndex < 0 ? digits !== '0' : /[1-9]/.test(digits.slice(guardIndex + 1));
    const nonzero = guard !== 0 || sticky;
    const aboveHalf = guard > 5 || (guard === 5 && sticky);
    const tie = guard === 5 && !sticky;
    const increment = nonzero && (
      mode === 'expand' ||
      (mode === 'ceil' && !negative) ||
      (mode === 'floor' && negative) ||
      (mode === 'halfExpand' && (aboveHalf || tie)) ||
      (mode === 'halfTrunc' && aboveHalf) ||
      (mode === 'halfEven' && (aboveHalf || (tie && Number(kept[kept.length - 1]) % 2 !== 0))) ||
      (mode === 'halfCeil' && (aboveHalf || (tie && !negative))) ||
      (mode === 'halfFloor' && (aboveHalf || (tie && negative)))
    );
    return increment ? intlIncrementDecimal(kept) : kept;
  }

  // Rounds a *signed* value to the nearest multiple of `increment` under
  // the requested ECMA-402 `roundingMode` (default `halfExpand`).
  function intlRound(value, increment, mode) {
    const quotient = value / increment;
    const floor = Math.floor(quotient);
    const difference = quotient - floor;
    let rounded;
    switch (mode) {
      case 'ceil': rounded = Math.ceil(quotient); break;
      case 'floor': rounded = floor; break;
      case 'expand': rounded = quotient < 0 ? floor : (difference === 0 ? floor : floor + 1); break;
      case 'trunc': rounded = Math.trunc(quotient); break;
      case 'halfCeil': rounded = difference > 0.5 ? floor + 1 : difference < 0.5 ? floor : floor + 1; break;
      case 'halfFloor': rounded = difference > 0.5 ? floor + 1 : difference < 0.5 ? floor : floor; break;
      case 'halfTrunc': rounded = difference > 0.5 ? floor + 1 : difference < 0.5 ? floor : (quotient >= 0 ? floor : floor + 1); break;
      case 'halfEven': rounded = difference > 0.5 ? floor + 1 : difference < 0.5 ? floor : (floor % 2 === 0 ? floor : floor + 1); break;
      // halfExpand (default): ties away from zero.
      default: rounded = difference > 0.5 ? floor + 1 : difference < 0.5 ? floor : (quotient < 0 ? floor : floor + 1);
    }
    return rounded * increment;
  }

  // Expands `Number.prototype.toPrecision`'s exponential form (`"1.23e+3"`)
  // back into plain decimal (`"1230"`) -- the plural operands and
  // `Intl.NumberFormat`'s own digit string both need a plain decimal, and
  // ICU's `Decimal` parser doesn't accept exponent notation.
  function intlExpandExponential(text) {
    if (!/e/i.test(text)) return text;
    const [mantissa, exponentPart] = text.toLowerCase().split('e');
    const exponent = parseInt(exponentPart, 10);
    const negative = mantissa.startsWith('-');
    const mantissaDigits = negative ? mantissa.slice(1) : mantissa;
    const [whole, fraction = ''] = mantissaDigits.split('.');
    const digits = whole + fraction;
    const pointPosition = whole.length + exponent;
    let expanded;
    if (pointPosition <= 0) {
      expanded = `0.${'0'.repeat(-pointPosition)}${digits}`;
    } else if (pointPosition >= digits.length) {
      expanded = `${digits}${'0'.repeat(pointPosition - digits.length)}`;
    } else {
      expanded = `${digits.slice(0, pointPosition)}.${digits.slice(pointPosition)}`;
    }
    return negative ? `-${expanded}` : expanded;
  }

  // `Intl.PluralRules.select`'s operand: the number formatted with the
  // requested visible digits, as a plain ASCII decimal string (ICU's
  // `Decimal` parser and the CLDR plural `v`/`i` operands both only care
  // about the value and its visible fraction-digit count, not the
  // locale's digit script). Mirrors `Intl.NumberFormat`'s own fraction
  // handling; `ToRawPrecision` for the significant-digit mode.
  function intlPluralOperand(number, significant, minFrac, maxFrac, minSig, maxSig, mode) {
    const magnitude = Math.abs(number);
    let digits;
    if (significant) {
      const shortest = magnitude === 0 ? 1 : magnitude.toExponential().split('e')[0].replace('.', '').replace(/^0+/, '').length;
      const visible = Math.max(minSig, Math.min(maxSig, shortest || 1));
      const exponent = Math.floor(Math.log10(magnitude || 1));
      const step = Math.pow(10, exponent - (visible - 1));
      const rounded = Math.abs(intlRound(number, step, mode));
      digits = intlExpandExponential(rounded.toPrecision(visible));
    } else {
      const step = Math.pow(10, -maxFrac);
      const fixed = Math.abs(intlRound(number, step, mode)).toFixed(maxFrac);
      let [whole, frac = ''] = fixed.split('.');
      while (frac.length > minFrac && frac.endsWith('0')) frac = frac.slice(0, -1);
      digits = frac ? `${whole}.${frac}` : whole;
    }
    return digits;
  }

  // Real `Intl.DateTimeFormat` throws a `RangeError` for an
  // out-of-enum field option at construction time; previously such a
  // value was silently degraded instead.
  function intlValidateDateTimeFields(opts) {
    const allowed = {
      weekday: ['long', 'short', 'narrow'],
      era: ['long', 'short', 'narrow'],
      year: ['numeric', '2-digit'],
      month: ['numeric', '2-digit', 'long', 'short', 'narrow'],
      day: ['numeric', '2-digit'],
      hour: ['numeric', '2-digit'],
      minute: ['numeric', '2-digit'],
      second: ['numeric', '2-digit'],
      dayPeriod: ['narrow', 'short', 'long'],
      timeZoneName: ['long', 'short', 'shortOffset', 'longOffset', 'shortGeneric', 'longGeneric'],
    };
    for (const [key, values] of Object.entries(allowed)) {
      if (opts[key] !== undefined && !values.includes(String(opts[key]))) {
        throw new RangeError(`Invalid ${key}: ${opts[key]}`);
      }
    }
    if (opts.fractionalSecondDigits !== undefined && ![1, 2, 3].includes(Number(opts.fractionalSecondDigits))) {
      throw new RangeError(`Invalid fractionalSecondDigits: ${opts.fractionalSecondDigits}`);
    }
  }

  class DateTimeFormat {
    constructor(locale, options) {
      const opts = options || {};
      // Real per-locale rendering (M4) only when the `intl` Cargo
      // feature is compiled in; otherwise the same fixed English/
      // Latin-numeral fast path this polyfill always had. Requesting a
      // locale doesn't itself turn the feature on (`crates/thaw-cli/
      // src/build.rs`'s `source_uses_intl` only watches for the
      // genuinely new capabilities), so this constructor must degrade
      // gracefully rather than assume the native call exists.
      this._useRealLocaleData = typeof __thaw_intl_datetime_format_parts === 'function';
      // An explicit `calendar`/`numberingSystem` constructor option
      // overrides any `-u-ca-`/`-u-nu-` the locale tag itself already
      // carries (confirmed against real Node), the same precedence
      // `Intl.Locale`'s constructor already implements above -- reuse
      // its tag-rewriting approach rather than duplicating it.
      const localeTag = intlRequestedLocale(locale);
      this.locale = this._useRealLocaleData
        ? (opts.calendar !== undefined || opts.numberingSystem !== undefined
            ? new Locale(localeTag, { calendar: opts.calendar, numberingSystem: opts.numberingSystem }).toString()
            : localeTag)
        : 'en-US';
      this._timeZone = opts.timeZone ? String(opts.timeZone) : 'UTC';
      this.format = this.format.bind(this);
      // Eager validation -- real Intl throws a `RangeError` for an
      // unrecognized `timeZone` at construction time, which is exactly
      // what real luxon's own `IANAZone.isValidZone` relies on (a
      // try/catch around `new Intl.DateTimeFormat(...)`).
      if (!intlZonedParts(this._timeZone, Date.now()).valid) {
        throw new RangeError(`Invalid time zone specified: ${this._timeZone}`);
      }
      this._weekday = opts.weekday;
      this._era = opts.era;
      this._year = opts.year;
      this._month = opts.month;
      this._day = opts.day;
      this._hour = opts.hour;
      this._minute = opts.minute;
      this._second = opts.second;
      this._dayPeriod = opts.dayPeriod;
      this._timeZoneName = opts.timeZoneName;
      this._fractionalSecondDigits = opts.fractionalSecondDigits === undefined
        ? undefined
        : Number(opts.fractionalSecondDigits);
      // `dateStyle`/`timeStyle` (ECMA-402) expand into the individual
      // field options with the locale's own CLDR patterns for that
      // style -- the same field-ordering machinery already used for an
      // explicit `{year, month, ...}` request, so no separate Rust path
      // is needed. Individual field options and a style are mutually
      // exclusive (real ECMA-402 throws `TypeError`).
      this._dateStyle = opts.dateStyle;
      this._timeStyle = opts.timeStyle;
      if (this._dateStyle !== undefined || this._timeStyle !== undefined) {
        if (['weekday', 'era', 'year', 'month', 'day', 'hour', 'minute', 'second', 'dayPeriod', 'fractionalSecondDigits'].some(key => opts[key] !== undefined)) {
          throw new TypeError('dateStyle/timeStyle can not be used with individual field options');
        }
        for (const [key, value] of [['dateStyle', this._dateStyle], ['timeStyle', this._timeStyle]]) {
          if (value !== undefined && !['full', 'long', 'medium', 'short'].includes(value)) {
            throw new RangeError(`Invalid ${key}: ${value}`);
          }
        }
        const dateStyles = {
          full: { weekday: 'long', year: 'numeric', month: 'long', day: 'numeric' },
          long: { year: 'numeric', month: 'long', day: 'numeric' },
          medium: { year: 'numeric', month: 'short', day: 'numeric' },
          short: { year: '2-digit', month: 'numeric', day: 'numeric' },
        };
        const timeStyles = {
          full: { hour: 'numeric', minute: '2-digit', second: '2-digit', timeZoneName: 'long' },
          long: { hour: 'numeric', minute: '2-digit', second: '2-digit', timeZoneName: 'short' },
          medium: { hour: 'numeric', minute: '2-digit', second: '2-digit' },
          short: { hour: 'numeric', minute: '2-digit' },
        };
        const expanded = {
          ...(this._dateStyle ? dateStyles[this._dateStyle] : {}),
          ...(this._timeStyle ? timeStyles[this._timeStyle] : {}),
        };
        this._weekday = expanded.weekday;
        this._era = expanded.era;
        this._year = expanded.year;
        this._month = expanded.month;
        this._day = expanded.day;
        this._hour = expanded.hour;
        this._minute = expanded.minute;
        this._second = expanded.second;
        // `format` includes the zone name for `full`/`long`, but
        // `resolvedOptions` (matching Node) omits it for a style.
        this._timeZoneName = expanded.timeZoneName;
      }
      // ECMA-402 `ToDateTimeOptions(options, "any", "date")`: with no date
      // *or* time field at all, default to `{year,month,day:'numeric'}`.
      // `era` and `timeZoneName` are not "date/time fields" for this check,
      // so `{era:'short'}` alone still gets the numeric y/m/d (confirmed
      // against Node: `"7/4/2024 AD"`).
      const hasAnyField =
        this._weekday !== undefined || this._year !== undefined ||
        this._month !== undefined || this._day !== undefined ||
        this._hour !== undefined || this._minute !== undefined ||
        this._second !== undefined || this._dayPeriod !== undefined ||
        this._fractionalSecondDigits !== undefined;
      if (!hasAnyField) {
        this._year = 'numeric';
        this._month = 'numeric';
        this._day = 'numeric';
      }
      // Kept separate from `this._hourCycle` below (which always
      // forces a concrete value for the legacy English-only path's own
      // internal am/pm logic): the *real* per-locale path must leave
      // hour12/hourCycle unset when the caller didn't request either,
      // so the native call can apply the requested locale's own actual
      // default hour cycle (e.g. most of Europe defaults to h23, not
      // en-US's h12) instead of a hardcoded English default.
      this._explicitHour12 = opts.hour12 === undefined ? undefined : Boolean(opts.hour12);
      this._explicitHourCycle = opts.hourCycle;
      // Real ECMA-402: an explicit `hour12` wins over `hourCycle`
      // (confirmed against Node: `{hourCycle:'h23', hour12:true}` resolves
      // to `h12`).
      if (this._explicitHour12 !== undefined) {
        this._hourCycle = this._explicitHour12 ? 'h12' : 'h23';
        this._explicitHourCycle = this._hourCycle;
      } else if (opts.hourCycle !== undefined) {
        if (!['h11', 'h12', 'h23', 'h24'].includes(opts.hourCycle)) {
          throw new RangeError(`Invalid hourCycle: ${opts.hourCycle}`);
        }
        this._hourCycle = opts.hourCycle;
      } else {
        const resolved = this._useRealLocaleData && typeof __thaw_intl_datetime_resolved_options === 'function'
          ? JSON.parse(__thaw_intl_datetime_resolved_options(this.locale)) : null;
        this._hourCycle = resolved && resolved.hourCycle ? resolved.hourCycle : 'h12';
      }
      intlValidateDateTimeFields(opts);
    }

    resolvedOptions() {
      const resolvedLocale = this._useRealLocaleData && typeof __thaw_intl_datetime_resolved_options === 'function'
        ? JSON.parse(__thaw_intl_datetime_resolved_options(this.locale)) : null;
      const result = {
        locale: this.locale,
        calendar: resolvedLocale && resolvedLocale.calendar ? resolvedLocale.calendar : 'gregory',
        numberingSystem: resolvedLocale && resolvedLocale.numberingSystem ? resolvedLocale.numberingSystem : 'latn',
        timeZone: this._timeZone,
      };
      if (this._dateStyle !== undefined || this._timeStyle !== undefined) {
        if (this._hour) {
          result.hourCycle = this._hourCycle;
          result.hour12 = this._hourCycle === 'h11' || this._hourCycle === 'h12';
        }
        if (this._dateStyle) result.dateStyle = this._dateStyle;
        if (this._timeStyle) result.timeStyle = this._timeStyle;
        return result;
      }
      if (this._weekday) result.weekday = this._weekday;
      if (this._era) result.era = this._era;
      if (this._year) result.year = this._year;
      if (this._month) result.month = this._month;
      if (this._day) result.day = this._day;
      if (this._hour) {
        result.hour = this._hour;
        result.hourCycle = this._hourCycle;
        result.hour12 = this._hourCycle === 'h11' || this._hourCycle === 'h12';
      }
      if (this._minute) result.minute = this._minute;
      if (this._second) result.second = this._second;
      if (this._dayPeriod) result.dayPeriod = this._dayPeriod;
      if (this._fractionalSecondDigits !== undefined) result.fractionalSecondDigits = this._fractionalSecondDigits;
      if (this._timeZoneName) result.timeZoneName = this._timeZoneName;
      return result;
    }

    formatToParts(date) {
      if (typeof date === 'bigint') throw new TypeError('Cannot convert BigInt to number');
      const epochMs = date === undefined ? Date.now() : Number(date);
      const zoned = intlZonedParts(this._timeZone, epochMs);
      if (!zoned.valid) {
        throw new RangeError('Invalid time value');
      }
      return this._useRealLocaleData
        ? this._formatToPartsRealLocale(zoned)
        : this._formatToPartsEnglishFastPath(zoned);
    }

    // Real per-locale month/weekday/era/day-period names, field
    // ordering, and literal punctuation (`__thaw_intl_datetime_
    // format_parts`, `intl_datetime.rs`) -- everything except
    // `timeZoneName`, which stays on the existing jiff-backed English
    // path below until M13 gives it real per-locale zone-name data too.
    _formatToPartsRealLocale(zoned) {
      // `icu_datetime`'s time rendering is driven by a single
      // `TimePrecision` (hour < minute < second), so a `minute`/`second`
      // request without `hour` would still render the hour (real
      // ECMA-402 renders only the requested fields: `{second:'numeric'}`
      // -> `"45"`). Time-only requests without an hour are built here
      // instead; a date+time combination still goes through ICU for its
      // locale-specific glue.
      const hasDateFields = Boolean(this._weekday || this._era || this._year || this._month || this._day);
      // A `minute`/`second` request without `hour` (and without any date
      // field) must not go through the builder -- it would render the
      // hour. The skeleton native handles it correctly (`ms` -> `mm.ss`),
      // so try it first and only fall back to `_formatTimeOnlyParts`.
      const timeOnlyNoHour =
        !hasDateFields && !this._hour &&
        (this._minute !== undefined || this._second !== undefined || this._fractionalSecondDigits !== undefined);
      // ...but only `minute`+`second` together (the locale's `ms` pattern,
      // whose separator differs per locale) can use the skeleton native;
      // a bare minute is not zero-padded even for `'2-digit'` (Node:
      // `{minute:'2-digit'}` -> `"5"`), which `_formatTimeOnlyParts` handles.
      const skeletonTimeOnly = !hasDateFields && !this._hour && (
        (this._minute !== undefined && this._second !== undefined) ||
        (this._dayPeriod !== undefined && (this._minute !== undefined || this._second !== undefined))
      );
      const options = {
        weekday: this._weekday,
        era: this._era,
        year: this._year,
        month: this._month,
        day: this._day,
        hour: this._hour,
        minute: this._minute,
        second: this._second,
        dayPeriod: this._dayPeriod,
        hour12: this._explicitHour12,
        hourCycle: this._explicitHourCycle,
      };
      // `month:'narrow'`/`weekday:'narrow'` need real ICU4C (`udat`),
      // since ICU4X's field-set builder has no Narrow `Length` and even
      // picks the wrong *pattern* (ja narrow is `7月`, MID is `7/04`).
      // Only when the opt-in backend is compiled in; otherwise the ICU4X
      // path below degrades narrow to short.
      let parts = null;
      let fractionHandled = false;
      const periodFractionOnly = !hasDateFields && !this._hour && !this._minute && !this._second &&
        this._dayPeriod !== undefined && this._fractionalSecondDigits !== undefined;
      if (periodFractionOnly && typeof __thaw_intl_datetime_narrow_icu4c !== 'function' &&
          typeof __thaw_intl_datetime_skeleton_parts === 'function') {
        // Bh is the curated locale's flexible-period/hour pattern. Use its
        // field order and interfield whitespace, but never show its hour or
        // hour-unit suffix (Uhr, 時, 시, etc.) as fractional seconds.
        const probe = { ...options, hour: 'numeric', hour12: true, hourCycle: 'h12' };
        const template = JSON.parse(__thaw_intl_datetime_skeleton_parts(
          this.locale, JSON.stringify(probe), JSON.stringify(zoned)));
        const hour = template.findIndex(part => part.type === 'hour');
        const period = template.findIndex(part => part.type === 'dayPeriod');
        const fields = template.filter(part => part.type !== 'literal');
        const first = Math.min(hour, period);
        const last = Math.max(hour, period);
        const middle = template.slice(first + 1, last);
        const suffix = template.slice(last + 1);
        const between = middle.map(part => part.value).join('');
        const after = suffix.map(part => part.value).join('');
        // Exactly the nine Bh shapes in the generated curated CLDR table.
        // Unknown hour-unit literals must not be copied or silently erased.
        // ponytail: if curated Bh expands, use a native B+fraction pattern
        // rather than growing this literal list indefinitely.
        const knownShape = hour < period
          ? after === '' && [' ', ' Uhr ', ' giờ ', ' ч. '].includes(between)
          : (between === ' ' && (after === '' || after === '시')) ||
            (between === '' && (after === '時' || after === '时'));
        if (hour >= 0 && period >= 0 && fields.length === 2 && first === 0 &&
            middle.every(part => part.type === 'literal') &&
            suffix.every(part => part.type === 'literal') && knownShape) {
          const boundary = hour < period ? between.match(/\s+$/u) : between.match(/^\s+/u);
          const fraction = intlPad(zoned.millisecond, 3).slice(0, this._fractionalSecondDigits);
          const rendered = [...fraction].map(digit => String(__thaw_intl_number_format(this.locale, digit, false))).join('');
          const fractional = { type: 'fractionalSecond', value: rendered };
          const literal = boundary ? [{ type: 'literal', value: boundary[0] }] : [];
          parts = hour < period
            ? [fractional, ...literal, template[period]]
            : [template[period], ...literal, fractional];
          fractionHandled = true;
        }
      }
      if (
        parts === null &&
        (this._month === 'narrow' || this._weekday === 'narrow' || this._dayPeriod !== undefined) &&
        typeof __thaw_intl_datetime_narrow_icu4c === 'function'
      ) {
        const icuParts = JSON.parse(
          __thaw_intl_datetime_narrow_icu4c(this.locale, JSON.stringify(options), JSON.stringify(zoned)),
        );
        if (icuParts.length) parts = icuParts;
      }
      // `dateStyle`/`timeStyle` must use the locale's `dateFormats`
      // length patterns (ICU4X's field-set builder), not UTS-35
      // skeleton matching -- real Node renders e.g. de
      // `dateStyle:'medium'` as `04.07.2024` (the medium dateFormat),
      // not the `yMMMd` skeleton's `4. Juli 2024`.
      if (
        parts === null &&
        (!timeOnlyNoHour || skeletonTimeOnly) &&
        this._dateStyle === undefined &&
        this._timeStyle === undefined &&
        typeof __thaw_intl_datetime_skeleton_parts === 'function'
      ) {
        const skeletonParts = JSON.parse(
          __thaw_intl_datetime_skeleton_parts(this.locale, JSON.stringify(options), JSON.stringify(zoned)),
        );
        // Non-empty means real UTS-35 skeleton matching ran; its field
        // widths are already Node-final, so skip the pad/unpad fixups.
        if (skeletonParts.length) parts = skeletonParts;
      }
      if (parts === null && timeOnlyNoHour) {
        return this._formatTimeOnlyParts(zoned);
      }
      if (parts === null) {
        parts = JSON.parse(
          __thaw_intl_datetime_format_parts(this.locale, JSON.stringify(options), JSON.stringify(zoned)),
        );
        // `dateStyle`/`timeStyle` render the locale's `dateFormats` length
        // pattern verbatim, so its widths are already Node-final.
        if (this._dateStyle === undefined && this._timeStyle === undefined) {
        // `icu_datetime`'s numeric fields render un-padded by default
        // (confirmed: `YMD::short()` gives `"7/4/24"`, not `"07/04/24"`)
        // -- there's no independent "always 2 digits" mode in its simple
        // Length-based builder (unlike real ECMA-402's `'2-digit'`
        // option), so zero-pad here, matching real Node's own zero-padded
        // `'2-digit'` output (confirmed for month/day/hour/minute/second,
        // including 12-hour-clock hours, e.g. `"01 AM"` for 1am).
        const twoDigitStyleFor = {
          year: this._year,
          month: this._month,
          day: this._day,
          hour: this._hour,
          minute: this._minute,
          second: this._second,
        };
        for (const part of parts) {
          if (twoDigitStyleFor[part.type] === '2-digit' && /^\d+$/.test(part.value) && part.value.length < 2) {
            part.value = `0${part.value}`;
          }
        }
        // Conversely, real ECMA-402 `'numeric'` never zero-pads, but some
        // ICU4X calendar-period patterns do (`ja` `{year,month:'numeric'}`
        // -> `"2024/07"`, Node `"2024/7"`) -- strip a single leading zero.
        // Not year (4-digit) nor hour (h23/h24 genuinely pad, per above).
        const numericUnpadFor = {
          month: this._month,
          day: this._day,
          minute: this._minute,
          second: this._second,
        };
        for (const part of parts) {
          if (numericUnpadFor[part.type] === 'numeric' && /^0\d$/.test(part.value)) {
            part.value = part.value.slice(1);
          }
        }
        }
      }
      // `icu_datetime` has no `h24` equivalent (only `H23`, hours
      // 0-23), so its midnight hour comes back as `0`/`00`; real
      // ECMA-402 `hourCycle: 'h24'` renders that same instant as `24`
      // (the one instant where the two cycles differ).
      if (this._hourCycle === 'h24' && zoned.hour === 0) {
        for (const part of parts) {
          if (part.type === 'hour') {
            part.value = String(__thaw_intl_number_format(this.locale, '24', false));
          }
        }
      }
      if (this._fractionalSecondDigits !== undefined && !fractionHandled) {
        const fraction = intlPad(zoned.millisecond, 3).slice(0, this._fractionalSecondDigits);
        const render = text => this._useRealLocaleData
          ? String(__thaw_intl_number_format(this.locale, text, false))
          : text;
        const secondIndex = parts.findIndex(part => part.type === 'second');
        if (secondIndex >= 0) {
          const decimal = render('0.1').slice(render('0').length, -render('1').length);
          parts.splice(secondIndex + 1, 0,
            { type: 'literal', value: decimal },
            { type: 'fractionalSecond', value: render(fraction) });
        } else {
          parts.push({ type: 'fractionalSecond', value: render(fraction) });
        }
      }
      if (this._timeZoneName) {
        if (parts.length) parts.push({ type: 'literal', value: ' ' });
        parts.push({ type: 'timeZoneName', value: intlTimeZoneName(this._timeZoneName, zoned, this.locale) });
      }
      // `icu_datetime` renders a non-Gregorian `relatedYear` (Chinese/Dangi)
      // in Latin digits by design ("always in latin digits according to
      // spec"), but ICU4C/Node localizes it with the locale's numbering
      // system (`ar-SA` -> `٢٠٢٤`). Re-localize it here.
      if (this._useRealLocaleData && typeof __thaw_intl_number_format === 'function') {
        for (const part of parts) {
          if (part.type === 'relatedYear' && /^[0-9]+$/.test(part.value)) {
            part.value = String(__thaw_intl_number_format(this.locale, part.value, false));
          }
        }
      }
      return parts;
    }

    _formatTimeOnlyParts(zoned) {
      const render = text => this._useRealLocaleData
        ? String(__thaw_intl_number_format(this.locale, text, false))
        : text;
      const fraction = this._fractionalSecondDigits === undefined
        ? undefined
        : intlPad(zoned.millisecond, 3).slice(0, this._fractionalSecondDigits);
      const fields = [];
      if (this._minute) {
        // A minute shown alongside seconds is zero-padded (`"05:45"`);
        // a bare minute isn't, even with `'2-digit'` (`"5"`) -- odd but
        // confirmed against real Node.
        const text = this._second ? intlPad(zoned.minute, 2) : String(zoned.minute);
        fields.push(['minute', render(text)]);
      }
      if (this._second) {
        const whole = this._second === '2-digit' ? intlPad(zoned.second, 2) : String(zoned.second);
        fields.push(['second', render(whole)]);
        if (fraction !== undefined) {
          const decimal = render('0.1').slice(render('0').length, -render('1').length);
          fields.push(['fractionalSecond', render(fraction), decimal]);
        }
      } else if (fraction !== undefined) {
        fields.push(['fractionalSecond', render(fraction)]);
      }
      const parts = [];
      fields.forEach(([type, value, separator], index) => {
        if (index > 0) parts.push({ type: 'literal', value: separator || ':' });
        parts.push({ type, value });
      });
      if (this._timeZoneName) {
        if (parts.length) parts.push({ type: 'literal', value: ' ' });
        parts.push({ type: 'timeZoneName', value: intlTimeZoneName(this._timeZoneName, zoned, this.locale) });
      }
      return parts;
    }

    _formatToPartsEnglishFastPath(zoned) {
      const leading = [];
      if (this._weekday) {
        const name = this._weekday === 'long' ? INTL_WEEKDAYS_LONG
          : this._weekday === 'narrow' ? INTL_WEEKDAYS_NARROW
          : INTL_WEEKDAYS_SHORT;
        leading.push({ type: 'weekday', value: name[zoned.weekday - 1] });
      }

      const hasDate = Boolean(this._year || this._month || this._day);
      if (hasDate) {
        const monthIsText = this._month === 'long' || this._month === 'short' || this._month === 'narrow';
        if (leading.length) leading.push({ type: 'literal', value: ', ' });
        if (monthIsText) {
          const table = this._month === 'long' ? INTL_MONTHS_LONG
            : this._month === 'narrow' ? INTL_MONTHS_NARROW
            : INTL_MONTHS_SHORT;
          leading.push({ type: 'month', value: table[zoned.month - 1] });
          if (this._day) {
            leading.push({ type: 'literal', value: ' ' });
            leading.push({ type: 'day', value: this._day === '2-digit' ? intlPad(zoned.day, 2) : String(zoned.day) });
          }
          if (this._year) {
            leading.push({ type: 'literal', value: ', ' });
            leading.push({
              type: 'year',
              value: this._year === '2-digit' ? intlPad(zoned.year % 100, 2) : String(zoned.year),
            });
          }
        } else {
          const fields = [];
          if (this._month) fields.push(['month', this._month === '2-digit' ? intlPad(zoned.month, 2) : String(zoned.month)]);
          if (this._day) fields.push(['day', this._day === '2-digit' ? intlPad(zoned.day, 2) : String(zoned.day)]);
          if (this._year) fields.push(['year', this._year === '2-digit' ? intlPad(zoned.year % 100, 2) : String(zoned.year)]);
          fields.forEach(([type, value], index) => {
            if (index > 0) leading.push({ type: 'literal', value: '/' });
            leading.push({ type, value });
          });
        }
      }
      if (this._era) {
        const table = INTL_ERA[this._era] || INTL_ERA.short;
        if (leading.length) leading.push({ type: 'literal', value: ' ' });
        leading.push({ type: 'era', value: zoned.year > 0 ? table[1] : table[0] });
      }

      const trailing = [];
      const hasTime = Boolean(this._hour || this._minute || this._second || this._fractionalSecondDigits !== undefined);
      if (hasTime) {
        const fields = [];
        let dayPeriod = null;
        if (this._hour) {
          const { displayHour, dayPeriod: period, padByDefault } = intlHourParts(this._hourCycle, zoned.hour);
          dayPeriod = period;
          const padded = this._hour === '2-digit' || padByDefault;
          fields.push(['hour', padded ? intlPad(displayHour, 2) : String(displayHour)]);
        }
        if (this._minute) fields.push(['minute', this._minute === '2-digit' ? intlPad(zoned.minute, 2) : String(zoned.minute)]);
        if (this._second) {
          const whole = this._second === '2-digit' ? intlPad(zoned.second, 2) : String(zoned.second);
          fields.push(['second', whole]);
          if (this._fractionalSecondDigits !== undefined) {
            fields.push(['fractionalSecond', intlPad(zoned.millisecond, 3).slice(0, this._fractionalSecondDigits), '.']);
          }
        } else if (this._fractionalSecondDigits !== undefined) {
          fields.push(['fractionalSecond', intlPad(zoned.millisecond, 3).slice(0, this._fractionalSecondDigits)]);
        }
        fields.forEach(([type, value, separator], index) => {
          if (index > 0) trailing.push({ type: 'literal', value: separator || ':' });
          trailing.push({ type, value });
        });
        if (dayPeriod) {
          trailing.push({ type: 'literal', value: ' ' });
          trailing.push({ type: 'dayPeriod', value: dayPeriod });
        }
      }
      if (this._timeZoneName) {
        if (trailing.length) trailing.push({ type: 'literal', value: ' ' });
        trailing.push({ type: 'timeZoneName', value: intlTimeZoneName(this._timeZoneName, zoned, this.locale) });
      }

      if (leading.length && trailing.length) {
        // Real en-US `Intl.DateTimeFormat` joins a combined date+time
        // with `" at "` when the month is spelled out in full (`'long'`
        // -- real ICU's "full"/"long" date patterns), and with `", "`
        // otherwise (`'numeric'`/`'2-digit'`/`'short'`/`'narrow'` month,
        // or no month at all) -- confirmed against real Node's own
        // output for both cases, not guessed.
        const connector = this._month === 'long' ? ' at ' : ', ';
        return [...leading, { type: 'literal', value: connector }, ...trailing];
      }
      return leading.length ? leading : trailing;
    }

    format(date) {
      return this.formatToParts(date).map(part => part.value).join('');
    }

    // The ECMA-402 default field set (year/month/day numeric) when no
    // field options were given.
    _effectiveOptions() {
      const hasAny = this._weekday || this._era || this._year || this._month || this._day ||
        this._hour || this._minute || this._second || this._dayPeriod ||
        this._fractionalSecondDigits !== undefined;
      return {
        weekday: this._weekday,
        era: this._era,
        year: this._year || (hasAny ? undefined : 'numeric'),
        month: this._month || (hasAny ? undefined : 'numeric'),
        day: this._day || (hasAny ? undefined : 'numeric'),
        hour: this._hour,
        minute: this._minute,
        second: this._second,
        dayPeriod: this._dayPeriod,
        fractionalSecondDigits: this._fractionalSecondDigits,
        timeZoneName: this._timeZoneName,
        hour12: this._explicitHour12,
        hourCycle: this._explicitHourCycle,
      };
    }

    // `Intl.DateTimeFormat.prototype.formatRange` (ECMA-402) via ICU4C's
    // interval formatter under the opt-in `--icu4c`; without it, a
    // documented approximation joining the two formatted endpoints.
    formatRange(startDate, endDate) {
      if (typeof startDate === 'bigint' || typeof endDate === 'bigint') throw new TypeError('Cannot convert BigInt to number');
      const startMs = Number(startDate);
      const endMs = Number(endDate);
      if (!Number.isFinite(startMs) || !Number.isFinite(endMs)) {
        throw new RangeError('Invalid time value');
      }
      const start = intlZonedParts(this._timeZone, startMs);
      const end = intlZonedParts(this._timeZone, endMs);
      if (!start.valid || !end.valid) {
        throw new RangeError('Invalid time value');
      }
      if (this._useRealLocaleData && typeof __thaw_intl_datetime_range_icu4c === 'function') {
        const options = this._effectiveOptions();
        const startParts = { ...start, timeZone: this._timeZone };
        const endParts = { ...end, timeZone: this._timeZone };
        const formatted = __thaw_intl_datetime_range_icu4c(
          this.locale,
          JSON.stringify(options),
          JSON.stringify(startParts),
          JSON.stringify(endParts),
        );
        if (formatted) return formatted;
      }
      const startParts = this.formatToParts(startMs);
      if (startMs === endMs) return startParts.map(part => part.value).join('');
      const { shared, startMiddle, endMiddle, trailing } = intlPartitionRange(
        startParts,
        this.formatToParts(endMs),
      );
      const separator = '\u2009\u2013\u2009';
      return [...shared, ...startMiddle, { value: separator }, ...endMiddle, ...trailing]
        .map(part => part.value)
        .join('');
    }

    // `Intl.DateTimeFormat.prototype.formatRangeToParts` -- ICU4C marks
    // each argument's own fields with a span, giving the real
    // shared/startRange/endRange sources.
    formatRangeToParts(startDate, endDate) {
      if (typeof startDate === 'bigint' || typeof endDate === 'bigint') throw new TypeError('Cannot convert BigInt to number');
      const startMs = Number(startDate);
      const endMs = Number(endDate);
      if (!Number.isFinite(startMs) || !Number.isFinite(endMs)) {
        throw new RangeError('Invalid time value');
      }
      const start = intlZonedParts(this._timeZone, startMs);
      const end = intlZonedParts(this._timeZone, endMs);
      if (!start.valid || !end.valid) {
        throw new RangeError('Invalid time value');
      }
      if (this._useRealLocaleData && typeof __thaw_intl_datetime_range_parts_icu4c === 'function') {
        const options = this._effectiveOptions();
        const startParts = { ...start, timeZone: this._timeZone };
        const endParts = { ...end, timeZone: this._timeZone };
        const parts = JSON.parse(
          __thaw_intl_datetime_range_parts_icu4c(
            this.locale,
            JSON.stringify(options),
            JSON.stringify(startParts),
            JSON.stringify(endParts),
          ),
        );
        if (parts.length) return parts;
      }
      const startParts = this.formatToParts(startMs);
      if (startMs === endMs) {
        return startParts.map(part => ({ ...part, source: 'shared' }));
      }
      const { shared, startMiddle, endMiddle, trailing } = intlPartitionRange(
        startParts,
        this.formatToParts(endMs),
      );
      return [
        ...shared.map(part => ({ ...part, source: 'shared' })),
        ...startMiddle.map(part => ({ ...part, source: 'startRange' })),
        { type: 'literal', value: '\u2009\u2013\u2009', source: 'shared' },
        ...endMiddle.map(part => ({ ...part, source: 'endRange' })),
        ...trailing.map(part => ({ ...part, source: 'shared' })),
      ];
    }
  }

  class NumberFormat {
    constructor(locale, options) {
      const opts = options || {};
      // Same real-per-locale-or-English-fast-path split as
      // `DateTimeFormat` above (M6): only the digit/grouping rendering
      // changes here -- rounding, sign, and `style: 'unit'`'s English
      // forms are locale-independent arithmetic already correct as-is.
      this._useRealLocaleData = typeof __thaw_intl_number_format === 'function';
      const localeTag = intlRequestedLocale(locale);
      // An explicit `numberingSystem` overrides the locale's own default,
      // exactly like `DateTimeFormat`'s `calendar`/`numberingSystem`
      // above (reusing `Intl.Locale`'s own tag rewriting).
      this.locale = this._useRealLocaleData
        ? (opts.numberingSystem !== undefined
            ? new Locale(localeTag, { numberingSystem: opts.numberingSystem }).toString()
            : localeTag)
        : 'en-US';
      this._style = opts.style || 'decimal';
      if (!['decimal', 'percent', 'currency', 'unit'].includes(this._style)) throw new RangeError(`Invalid style: ${this._style}`);
      this._unit = opts.unit;
      this._unitDisplay = opts.unitDisplay || 'short';
      if (this._style === 'unit' && !intlValidUnit(this._unit)) {
        throw new RangeError(`Invalid unit argument for Intl.NumberFormat() '${this._unit}'`);
      }
      if (!['long', 'short', 'narrow'].includes(this._unitDisplay)) {
        throw new RangeError(`Invalid unitDisplay: ${this._unitDisplay}`);
      }
      this._currency = opts.currency === undefined ? undefined : String(opts.currency).toUpperCase();
      this._currencyDisplay = opts.currencyDisplay || 'symbol';
      if (this._style === 'currency' && !/^[A-Z]{3}$/.test(this._currency || '')) {
        throw new TypeError('Currency code is required with currency style');
      }
      if (!['symbol', 'narrowSymbol', 'code', 'name'].includes(this._currencyDisplay)) {
        throw new RangeError(`Invalid currencyDisplay: ${this._currencyDisplay}`);
      }
      this._useGrouping = intlNormalizeUseGrouping(opts.useGrouping);
      this._minimumIntegerDigits = intlIntegerOption(opts.minimumIntegerDigits, 1, 1, 21);
      let defaultMinFraction = 0;
      let defaultMaxFraction = this._style === 'percent' ? 0 : 3;
      if (this._style === 'currency') {
        let digits = 2;
        if (typeof __thaw_intl_currency_fraction_digits === 'function') {
          const resolved = __thaw_intl_currency_fraction_digits(this._currency);
          if (resolved !== null && resolved !== undefined) digits = resolved;
        }
        defaultMinFraction = digits;
        defaultMaxFraction = digits;
      }
      const minimumFraction = intlIntegerOption(opts.minimumFractionDigits, undefined, 0, 100);
      const maximumFraction = intlIntegerOption(opts.maximumFractionDigits, undefined, 0, 100);
      this._minimumFractionDigits = minimumFraction === undefined
        ? Math.min(defaultMinFraction, maximumFraction === undefined ? defaultMinFraction : maximumFraction)
        : minimumFraction;
      this._maximumFractionDigits = maximumFraction === undefined
        ? Math.max(defaultMaxFraction, this._minimumFractionDigits)
        : maximumFraction;
      if (this._maximumFractionDigits < this._minimumFractionDigits) {
        throw new RangeError('maximumFractionDigits is less than minimumFractionDigits');
      }
      this._significant = opts.minimumSignificantDigits !== undefined || opts.maximumSignificantDigits !== undefined;
      this._minimumSignificantDigits = intlIntegerOption(opts.minimumSignificantDigits, 1, 1, 21);
      this._maximumSignificantDigits = intlIntegerOption(opts.maximumSignificantDigits, Math.max(this._minimumSignificantDigits, 21), this._minimumSignificantDigits, 21);
      this._signDisplay = opts.signDisplay === undefined ? 'auto' : String(opts.signDisplay);
      if (!['auto', 'never', 'always', 'exceptZero', 'negative'].includes(this._signDisplay)) {
        throw new RangeError(`Invalid signDisplay: ${this._signDisplay}`);
      }
      this._currencySign = opts.currencySign === undefined ? 'standard' : String(opts.currencySign);
      if (!['standard', 'accounting'].includes(this._currencySign)) {
        throw new RangeError(`Invalid currencySign: ${this._currencySign}`);
      }
      this._roundingMode = intlEnumOption(opts.roundingMode, INTL_ROUNDING_MODES, 'halfExpand', 'roundingMode');
      this._numberingSystem = opts.numberingSystem;
      const requestedNotation = opts.notation === undefined ? 'standard' : String(opts.notation);
      if (!['standard', 'scientific', 'engineering', 'compact'].includes(requestedNotation)) {
        throw new RangeError(`Invalid notation: ${requestedNotation}`);
      }
      this._compactDisplay = intlEnumOption(opts.compactDisplay, ['short', 'long'], 'short', 'compactDisplay');
      // `scientific`/`engineering`/`compact` are implemented for the plain
      // decimal style only (the overwhelmingly common case); anything else
      // keeps standard formatting and reports `standard`, so
      // `resolvedOptions` stays truthful about what was actually done.
      this._notation = this._style === 'decimal' && requestedNotation !== 'standard'
        ? requestedNotation
        : 'standard';
      this.format = this.format.bind(this);
    }

    // Real `SetNumberFormatDigitOptions`' `signDisplay` handling: `-0`
    // counts as negative for `auto`/`always` (so `-0` renders `"-0"`)
    // but not for `negative`/`exceptZero` (which treat it as zero).
    _signFor(number, exact = null, roundedNonzero = null) {
      const negative = exact === null ? number < 0 || Object.is(number, -0) : exact.negative;
      const nonzero = roundedNonzero === null
        ? (exact === null ? number !== 0 : exact.nonzero) : roundedNonzero;
      switch (this._signDisplay) {
        case 'never': return '';
        case 'always': return negative ? '-' : '+';
        case 'exceptZero': return nonzero ? (negative ? '-' : '+') : '';
        case 'negative': return negative && nonzero ? '-' : '';
        default: return negative ? '-' : '';
      }
    }

    // The scientific/engineering rendering (`notation`): the digit
    // options apply to the *mantissa*, and the (locale-rendered)
    // exponent is appended as `E<exp>`. `engineering` uses an exponent
    // that's a multiple of 3. `E` (not a locale exponent symbol like
    // `ar-SA`'s `أس` or `fa`'s `×۱۰^`) is a documented approximation --
    // it matches every curated locale whose numbering system is Latin
    // (`en`/`de`/`fr`/`ja`/`ar`/`bn`), where localized exponent symbols
    // aren't available without new data.
    _formatScientific(number, original) {
      const exact = this._exactScientificDigits(original);
      if (exact !== null) {
        const render = digits => this._renderDigits(digits, false);
        return { text: `${render(exact.mantissa)}E${exact.exponent < 0n ? '-' : ''}${render(String(exact.exponent < 0n ? -exact.exponent : exact.exponent))}`,
          nonzero: exact.nonzero };
      }
      const magnitude = Math.abs(number);
      const minFrac = this._minimumFractionDigits === undefined ? 0 : this._minimumFractionDigits;
      const maxFrac = this._maximumFractionDigits === undefined ? 3 : this._maximumFractionDigits;
      let exponent = 0;
      if (magnitude !== 0) {
        exponent = this._notation === 'engineering'
          ? Math.floor(Math.log10(magnitude) / 3) * 3
          : Math.floor(Math.log10(magnitude));
        const mantissa = magnitude / Math.pow(10, exponent);
        const rounded = this._significant
          ? intlPluralOperand(mantissa, true, 0, 0, this._minimumSignificantDigits, this._maximumSignificantDigits, this._roundingMode)
          : intlPluralOperand(mantissa, false, minFrac, Math.max(minFrac, maxFrac), 0, 0, this._roundingMode);
        // Rounding the mantissa can carry it up a magnitude (`9.99` ->
        // `10`); pair it with the exponent that produced.
        const roundedMagnitude = Number(rounded);
        if (roundedMagnitude >= (this._notation === 'engineering' ? 1000 : 10)) {
          exponent += this._notation === 'engineering' ? 3 : 1;
        }
      }
      const mantissa = magnitude === 0 ? 0 : magnitude / Math.pow(10, exponent);
      const mantissaText = this._significant
        ? intlPluralOperand(mantissa, true, 0, 0, this._minimumSignificantDigits, this._maximumSignificantDigits, this._roundingMode)
        : intlPluralOperand(mantissa, false, minFrac, Math.max(minFrac, maxFrac), 0, 0, this._roundingMode);
      const render = digits => this._useRealLocaleData
        ? String(__thaw_intl_number_format(this.locale, digits, false))
        : digits;
      return { text: `${render(mantissaText)}E${exponent < 0 ? '-' : ''}${render(String(Math.abs(exponent)))}`,
        nonzero: /[1-9]/.test(mantissaText) };
    }

    _exactScientificDigits(value) {
      const text = typeof value === 'bigint' ? value.toString()
        : typeof value === 'string' ? value.trim()
        : typeof value === 'number' && Number.isFinite(value) && this._style === 'percent' &&
            !Number.isFinite(value * 100) ? String(value) : null;
      if (text === null) return null;
      const match = /^([+-]?)(?:(\d+)(?:\.(\d*))?|\.(\d+))(?:[eE]([+-]?\d+))?$/.exec(text);
      if (!match) return null;
      const power = BigInt(match[5] || 0);
      const coefficient = `${match[2] || '0'}${match[3] === undefined ? match[4] || '' : match[3]}`;
      const leading = /^0*/.exec(coefficient)[0].length;
      const significant = coefficient.slice(leading);
      const exponent10 = significant ? BigInt((match[2] || '0').length - leading - 1) + power +
        (this._style === 'percent' ? 2n : 0n) : 0n;
      let exponent = significant && this._notation === 'engineering'
        ? exponent10 - ((exponent10 % 3n) + 3n) % 3n : exponent10;
      const width = significant ? Number(exponent10 - exponent + 1n) : 1;
      const padded = (significant || '0').padEnd(width, '0');
      const signed = match[1] === '-' ? '-' : '';
      const raw = `${signed}${padded.slice(0, width)}${padded.length > width ? `.${padded.slice(width)}` : ''}`;
      // Percent's two decimal places are already reflected in exponent10;
      // rounding this mantissa must not scale it again, including on carry.
      let rounded = this._exactStandardDigits(raw, false);
      if (rounded === null) return null;
      const carryWidth = this._notation === 'engineering' ? 3 : 1;
      if (rounded.intPart.length > carryWidth) {
        exponent += BigInt(carryWidth);
        const shifted = `${signed}${rounded.intPart.slice(0, -carryWidth)}.${rounded.intPart.slice(-carryWidth)}${rounded.fracPart}`;
        rounded = this._exactStandardDigits(shifted, false);
        if (rounded === null) return null;
      }
      return { mantissa: rounded.fracPart ? `${rounded.intPart}.${rounded.fracPart}` : rounded.intPart,
        exponent, negative: signed === '-', nonzero: rounded.nonzero };
    }

    _exactNotationDigits(value) {
      // Compact notation has its own ICU rounding/formatting route. Retain
      // its existing input ceiling until that separate route is repaired.
      if (this._notation === 'compact' &&
          ((typeof value === 'string' && value.trim().length > 10000) ||
           (typeof value === 'bigint' && value.toString().length > 10000))) return null;
      return this._notation === 'scientific' || this._notation === 'engineering'
        ? this._exactScientificDigits(value) : this._exactStandardDigits(value);
    }

    resolvedOptions() {
      const resolvedLocale = this._useRealLocaleData && typeof __thaw_intl_datetime_resolved_options === 'function'
        ? JSON.parse(__thaw_intl_datetime_resolved_options(this.locale)) : null;
      const result = {
        locale: this.locale,
        numberingSystem: resolvedLocale && resolvedLocale.numberingSystem ? resolvedLocale.numberingSystem : 'latn',
        style: this._style,
        useGrouping: this._useGrouping,
        minimumIntegerDigits: this._minimumIntegerDigits,
      };
      if (this._significant) {
        result.minimumSignificantDigits = this._minimumSignificantDigits;
        result.maximumSignificantDigits = this._maximumSignificantDigits;
      } else {
        const minFrac = this._minimumFractionDigits === undefined ? 0 : this._minimumFractionDigits;
        const maxFrac = this._maximumFractionDigits === undefined
          ? (this._style === 'percent' ? 0 : 3)
          : this._maximumFractionDigits;
        result.minimumFractionDigits = minFrac;
        result.maximumFractionDigits = Math.max(minFrac, maxFrac);
      }
      if (this._style === 'unit') {
        result.unit = this._unit;
        result.unitDisplay = this._unitDisplay;
      } else if (this._style === 'currency') {
        result.currency = this._currency;
        result.currencyDisplay = this._currencyDisplay;
        result.currencySign = this._currencySign;
      }
      result.notation = this._notation;
      if (this._notation === 'compact') result.compactDisplay = this._compactDisplay;
      result.signDisplay = this._signDisplay;
      result.roundingIncrement = 1;
      result.roundingMode = this._roundingMode;
      result.roundingPriority = 'auto';
      result.trailingZeroDisplay = 'auto';
      return result;
    }

    // The standard-notation digit computation (significant- or
    // fraction-digit rounding, integer zero-padding, effective grouping),
    // shared by `format` and `formatToParts`.
    _exactStandardDigits(value, applyPercentScale = true) {
      const text = typeof value === 'bigint' ? value.toString()
        : typeof value === 'string' ? value.trim()
        : typeof value === 'number' && Number.isFinite(value) && this._style === 'percent' &&
            !Number.isFinite(value * 100) ? String(value) : null;
      if (text === null) return null;
      const match = /^([+-]?)(?:(\d+)(?:\.(\d*))?|\.(\d+))(?:[eE]([+-]?\d+))?$/.exec(text);
      if (!match) return null;
      const exponent = BigInt(match[5] || 0);
      const fraction = match[3] === undefined ? (match[4] || '') : match[3];
      let scale = BigInt(fraction.length) - exponent - (applyPercentScale && this._style === 'percent' ? 2n : 0n);
      let digits = `${match[2] || '0'}${fraction}`.replace(/^0+/, '') || '0';
      const cut = this._significant
        ? BigInt(Math.max(0, digits.length - this._maximumSignificantDigits))
        : scale > BigInt(this._maximumFractionDigits) ? scale - BigInt(this._maximumFractionDigits) : 0n;
      if (cut > 0n) {
        digits = intlRoundDecimalDigits(digits, cut, match[1] === '-', this._roundingMode);
        scale -= cut;
      }
      if (this._significant) {
        if (digits === '0') {
          scale = BigInt(this._minimumSignificantDigits - 1);
        } else if (digits.length < this._minimumSignificantDigits) {
          const padding = this._minimumSignificantDigits - digits.length;
          digits += '0'.repeat(padding);
          scale += BigInt(padding);
        }
      } else if (digits === '0') {
        scale = 0n;
      }
      let significantDigits = digits.length;
      // A standard-notation result of this size cannot be materialized as a
      // JS string. Never turn its finite input into binary64 Infinity.
      if (scale > BigInt(Number.MAX_SAFE_INTEGER) || scale < -BigInt(Number.MAX_SAFE_INTEGER)) {
        throw new RangeError('Intl numeric output is too large');
      }
      const places = Number(scale);
      const intPart = places < 0 ? digits + '0'.repeat(-places)
        : places >= digits.length ? '0' : digits.slice(0, -places) || digits;
      let fracPart = places <= 0 ? '' : places >= digits.length
        ? '0'.repeat(places - digits.length) + digits : digits.slice(-places);
      while (fracPart.endsWith('0') && (this._significant
        ? significantDigits > this._minimumSignificantDigits
        : fracPart.length > this._minimumFractionDigits)) {
        fracPart = fracPart.slice(0, -1);
        significantDigits -= 1;
      }
      if (!this._significant) fracPart = fracPart.padEnd(this._minimumFractionDigits, '0');
      return { intPart, fracPart, negative: match[1] === '-', nonzero: /[1-9]/.test(digits) };
    }

    _standardDigits(number, original) {
      let intPart;
      let fracPart;
      const exact = this._exactStandardDigits(original);
      if (exact !== null) {
        intPart = exact.intPart;
        fracPart = exact.fracPart;
      } else if (this._significant) {
        // `ToRawPrecision` -- the requested significant-digit count is
        // what governs the visible digits (and thus the trailing zeros),
        // not `toFixed`'s fraction count.
        const [whole, frac = ''] = intlPluralOperand(
          number,
          true,
          0,
          0,
          this._minimumSignificantDigits,
          this._maximumSignificantDigits,
          this._roundingMode,
        ).split('.');
        intPart = whole;
        fracPart = frac;
      } else {
        let minFrac = this._minimumFractionDigits;
        let maxFrac = this._maximumFractionDigits;
        if (minFrac === undefined && maxFrac === undefined) {
          // The real per-currency minor-unit digit count (e.g. 0 for
          // JPY/KRW, 2 for USD/EUR, 3 for BHD/KWD), via
          // `__thaw_intl_currency_fraction_digits`'s vendored CLDR data
          // -- not a single hand-picked "JPY is the only exception"
          // guess, which would have been wrong for every other real
          // zero-decimal currency (confirmed: real Node also gives KRW 0
          // digits and BHD 3, not just JPY 0/everything-else 2).
          let currencyDigits = 2;
          if (this._style === 'currency' && typeof __thaw_intl_currency_fraction_digits === 'function') {
            const resolved = __thaw_intl_currency_fraction_digits(this._currency);
            if (resolved !== null && resolved !== undefined) currencyDigits = resolved;
          }
          minFrac = this._style === 'currency' ? currencyDigits : 0;
          maxFrac = this._style === 'currency' ? minFrac : (this._style === 'percent' ? 0 : 3);
        } else if (minFrac === undefined) {
          minFrac = 0;
        } else if (maxFrac === undefined) {
          maxFrac = Math.max(minFrac, 3);
        }
        if (maxFrac < minFrac) maxFrac = minFrac;
        const fixed = Math.abs(intlRound(number, Math.pow(10, -maxFrac), this._roundingMode)).toFixed(maxFrac);
        const [wholePart, fracPartRaw = ''] = fixed.split('.');
        fracPart = fracPartRaw;
        while (fracPart.length > minFrac && fracPart.endsWith('0')) {
          fracPart = fracPart.slice(0, -1);
        }
        intPart = wholePart;
      }
      while (intPart.length < this._minimumIntegerDigits) intPart = `0${intPart}`;
      // `useGrouping: 'min2'` groups only once the integer part exceeds
      // four digits (real ECMA-402); `'auto'`/`'always'` group normally.
      const groupDigits = this._useGrouping === false
        ? false
        : this._useGrouping === 'min2' ? intPart.length > 4 : true;
      return { intPart, fracPart, groupDigits, nonzero: /[1-9]/.test(intPart + fracPart) };
    }

    format(value) {
      const input = Number(value);
      const number = this._style === 'percent' ? input * 100 : input;
      const exact = this._exactNotationDigits(value);
      if (this._notation === 'compact' && typeof __thaw_intl_compact_number === 'function') {
        // The compact pattern itself chooses the mantissa's precision, so
        // the raw number is passed through rather than `intl.js`'s own
        // rounded digit string. A known divergence: icu4x rounds compact
        // mantissas half-to-even (real ECMA-402 default is `halfExpand`),
        // so an exact tie differs (`1650` -> `1.6K` vs Node's `1.7K`).
        return String(__thaw_intl_compact_number(this.locale,
          typeof value === 'bigint' || typeof value === 'string' ? String(value) : String(number),
          this._compactDisplay === 'long'));
      }
      if (this._notation !== 'standard') {
        const scientific = this._formatScientific(number, value);
        return `${this._signFor(number, exact,
          Number.isFinite(number) || exact !== null ? scientific.nonzero : null)}${scientific.text}`;
      }
      const { intPart, fracPart, groupDigits, nonzero } = this._standardDigits(number, value);
      const sign = this._signFor(number, exact,
        Number.isFinite(number) || exact !== null ? nonzero : null);
      const digits = fracPart ? `${intPart}.${fracPart}` : intPart;
      const signedDigits = `${sign}${digits}`;
      if (this._style === 'percent' && typeof __thaw_intl_percent_format === 'function') {
        const result = digits.length > 32000 && typeof __thaw_intl_percent_format_large === 'function'
          ? __thaw_intl_percent_format_large(this.locale, this._styleProbeRepresentative(signedDigits),
              this._renderDigits(digits, groupDigits), groupDigits)
          : __thaw_intl_percent_format(this.locale, signedDigits, groupDigits);
        if (result) return result;
      }
      if (this._style === 'currency') {
        // `currencySign: 'accounting'` renders a negative amount in
        // parentheses (no minus sign), matching real ECMA-402.
        const accounting = this._currencySign === 'accounting' && sign === '-';
        const value = accounting ? digits : signedDigits;
        if (typeof __thaw_intl_currency_format === 'function') {
          const result = digits.length > 32000 && typeof __thaw_intl_currency_format_large === 'function'
            ? __thaw_intl_currency_format_large(this.locale,
                this._currencyDisplay === 'name' ? this._styleRepresentative(value) : this._styleProbeRepresentative(value),
                this._renderDigits(digits, groupDigits), this._currency, this._currencyDisplay, groupDigits)
            : __thaw_intl_currency_format(this.locale, value, this._currency, this._currencyDisplay, groupDigits);
          if (result) {
            return accounting ? `(${result})` : result;
          }
        }
        const text = `${this._currency} ${value}`;
        return accounting ? `(${text})` : text;
      }
      if (this._style === 'unit' && typeof __thaw_intl_unit_format === 'function') {
        const result = digits.length > 32000 && typeof __thaw_intl_unit_format_large === 'function'
          ? __thaw_intl_unit_format_large(this.locale, this._styleRepresentative(signedDigits),
              this._renderDigits(digits, groupDigits), this._unit, this._unitDisplay, groupDigits)
          : __thaw_intl_unit_format(this.locale, signedDigits, this._unit, this._unitDisplay, groupDigits);
        if (result) return result;
      }
      // Sign is handled here, not passed to the native call: it's
      // applied identically across every curated locale (confirmed),
      // so there's no need to push it through a locale-data lookup.
      const rendered = this._renderDigits(digits, groupDigits);
      const signed = `${sign}${rendered}`;
      if (this._style === 'percent') return `${signed}%`;
      if (this._style === 'currency') return `${this._currency} ${signed}`;
      if (this._style !== 'unit') return signed;
      const fallback = INTL_UNITS[this._unit];
      if (!fallback) return `${signed} ${this._unit.replaceAll('-', ' ')}`;
      const forms = fallback[this._unitDisplay] || fallback.short;
      const unitText = forms[Math.abs(number) === 1 ? 0 : 1];
      // 'narrow' has no space between the number and unit (`"3d"`);
      // 'long'/'short' both do (`"3 days"`/`"3 days"`) -- confirmed
      // against real Node's own output for every unit above.
      return this._unitDisplay === 'narrow' ? `${signed}${unitText}` : `${signed} ${unitText}`;
    }

    _renderDigits(text, grouping) {
      if (!this._useRealLocaleData) {
        return grouping ? text.replace(/\B(?=(\d{3})+(?!\d))/g, ',') : text;
      }
      // fixed_decimal has an i16 magnitude. The common path stays native;
      // long exact input uses symbols sampled from that same locale.
      if (text.length <= 32000) return String(__thaw_intl_number_format(this.locale, text, grouping));
      const { glyphs, group, primary, secondary, decimal } = this._largeLocaleSymbols();
      const sign = text.startsWith('-') || text.startsWith('+') ? text[0] : '';
      const unsigned = sign ? text.slice(1) : text;
      const [integer, fraction] = unsigned.split('.');
      const localize = digits => [...digits].map(digit => glyphs[digit.charCodeAt(0) - 48]).join('');
      let whole = localize(integer);
      if (grouping && group && integer.length > primary) {
        const pieces = [];
        let end = integer.length;
        let width = primary;
        while (end > width) {
          pieces.unshift(localize(integer.slice(end - width, end)));
          end -= width;
          width = secondary;
        }
        pieces.unshift(localize(integer.slice(0, end)));
        whole = pieces.join(group);
      }
      return `${sign}${whole}${fraction === undefined ? '' : decimal + localize(fraction)}`;
    }

    // icu_plurals only observes the low 18 integer digits, plus a high-
    // magnitude flag, and the first 18 visible fraction digits. Keep those
    // operands while asking ICU for the original style's affixes.
    _styleRepresentative(text) {
      const sign = text.startsWith('-') || text.startsWith('+') ? text[0] : '';
      const [integer, fraction = ''] = (sign ? text.slice(1) : text).split('.');
      const significant = integer.replace(/^0+/, '') || '0';
      const reduced = significant.length > 18 ? `1${significant.slice(-18)}` : significant;
      return `${sign}${reduced}${fraction ? `.${fraction.slice(0, 18)}` : ''}`;
    }

    // Percent and currency symbol/code patterns are independent of numeric
    // plural category. A fixed high-magnitude pair keeps their first-digit
    // probe valid even when significant-digit formatting yields 40k leading
    // fractional zeros and the true value's integer part is zero.
    _styleProbeRepresentative(text) {
      const sign = text.startsWith('-') || text.startsWith('+') ? text[0] : '';
      return `${sign}1${'0'.repeat(18)}`;
    }

    _largeLocaleSymbols() {
      if (this._cachedLargeLocaleSymbols) return this._cachedLargeLocaleSymbols;
      const glyphs = Array.from({ length: 10 }, (_, digit) =>
        String(__thaw_intl_number_format(this.locale, String(digit), false)));
      const decimal = String(__thaw_intl_number_format(this.locale, '1.5', false))
        .slice(glyphs[1].length, -glyphs[5].length);
      // Some locales leave 1000 ungrouped but group larger numbers. Probe
      // enough digits to see both primary and secondary group widths, then
      // count digit glyphs rather than Unicode scalars in each segment.
      const probe = String(__thaw_intl_number_format(this.locale, '1234567890123', true));
      let offset = 0;
      let segment = 0;
      let group = '';
      const widths = [];
      for (const digit of '1234567890123') {
        const glyph = glyphs[digit.charCodeAt(0) - 48];
        if (!probe.startsWith(glyph, offset)) {
          const next = probe.indexOf(glyph, offset);
          if (next < 0) throw new RangeError('Cannot identify Intl grouping symbols');
          const separator = probe.slice(offset, next);
          if (group && separator !== group) throw new RangeError('Inconsistent Intl grouping symbols');
          group = separator;
          widths.push(segment);
          segment = 0;
          offset = next;
        }
        offset += glyph.length;
        segment++;
      }
      if (offset !== probe.length) throw new RangeError('Cannot identify Intl grouping symbols');
      widths.push(segment);
      const primary = widths.length > 1 ? widths[widths.length - 1] : 3;
      const secondary = widths.length > 2 ? widths[widths.length - 2] : primary;
      this._cachedLargeLocaleSymbols = { glyphs, decimal, group, primary, secondary };
      return this._cachedLargeLocaleSymbols;
    }

    _groupSeparator() {
      if (!this._useRealLocaleData) return ',';
      if (this._cachedLargeLocaleSymbols) return this._cachedLargeLocaleSymbols.group;
      const probe = String(__thaw_intl_number_format(this.locale, '1000', true));
      return probe.length > 4 ? probe.slice(1, probe.length - 3) : ',';
    }

    _decimalSeparator() {
      if (!this._useRealLocaleData) return '.';
      const probe = String(__thaw_intl_number_format(this.locale, '1.5', false));
      return probe.length > 2 ? probe.slice(1, probe.length - 1) : '.';
    }

    _numericParts(intPart, fracPart, groupDigits) {
      const parts = [];
      const whole = this._renderDigits(intPart, groupDigits);
      const separator = groupDigits ? this._groupSeparator() : null;
      if (separator && whole.includes(separator)) {
        whole.split(separator).forEach((segment, index) => {
          if (index > 0) parts.push({ type: 'group', value: separator });
          parts.push({ type: 'integer', value: segment });
        });
      } else {
        parts.push({ type: 'integer', value: whole });
      }
      if (fracPart) {
        parts.push({ type: 'decimal', value: this._decimalSeparator() });
        parts.push({ type: 'fraction', value: this._renderDigits(fracPart, false) });
      }
      return parts;
    }

    _affixType() {
      if (this._style === 'percent') return 'percentSign';
      if (this._style === 'currency') return 'currency';
      if (this._style === 'unit') return 'unit';
      return 'literal';
    }

    // The non-numeric affixes around the number (currency symbol, unit
    // text, percent sign, punctuation/space) as typed parts.
    _affixParts(text) {
      if (!text) return [];
      const parts = [];
      let buffer = '';
      const flush = () => {
        if (buffer) parts.push({ type: this._affixType(), value: buffer });
        buffer = '';
      };
      for (const character of text) {
        if (
          character === ' ' || character === '\u00a0' || character === '\u202f' ||
          character === '\u2009' || character === '(' || character === ')'
        ) {
          flush();
          parts.push({ type: 'literal', value: character });
        } else {
          buffer += character;
        }
      }
      flush();
      return parts;
    }

    _scientificParts(text) {
      const separatorIndex = text.indexOf('E');
      const mantissa = text.slice(0, separatorIndex);
      const exponent = text.slice(separatorIndex + 1);
      const decimal = this._decimalSeparator();
      let whole = mantissa;
      let fraction = '';
      const dot = mantissa.indexOf(decimal);
      if (dot >= 0) {
        whole = mantissa.slice(0, dot);
        fraction = mantissa.slice(dot + decimal.length);
      }
      const parts = [{ type: 'integer', value: whole }];
      if (fraction) {
        parts.push({ type: 'decimal', value: decimal });
        parts.push({ type: 'fraction', value: fraction });
      }
      parts.push({ type: 'exponentSeparator', value: 'E' });
      if (exponent.startsWith('-')) {
        parts.push({ type: 'exponentMinusSign', value: '-' });
        parts.push({ type: 'exponentInteger', value: exponent.slice(1) });
      } else {
        parts.push({ type: 'exponentInteger', value: exponent });
      }
      return parts;
    }

    _localizedDigitSet() {
      if (!this._useRealLocaleData) return new Set('0123456789');
      const rendered = String(__thaw_intl_number_format(this.locale, '0123456789', false));
      return new Set([...rendered]);
    }

    // Splits an already-rendered compact string (`"1.2K"`, `"123.456"`,
    // `"1.2 thousand"`, `"1.5万"`) into numeric + compact parts.
    _compactParts(text) {
      const digits = this._localizedDigitSet();
      const decimal = this._decimalSeparator();
      const group = this._groupSeparator();
      let split = 0;
      while (split < text.length) {
        const character = text[split];
        if (digits.has(character) || character === decimal || character === group) {
          split += 1;
        } else {
          break;
        }
      }
      const numeric = text.slice(0, split);
      let suffix = text.slice(split);
      let whole = numeric;
      let fraction = '';
      const dot = numeric.indexOf(decimal);
      if (dot >= 0) {
        whole = numeric.slice(0, dot);
        fraction = numeric.slice(dot + decimal.length);
      }
      const parts = [];
      if (group && whole.includes(group)) {
        whole.split(group).forEach((segment, index) => {
          if (index > 0) parts.push({ type: 'group', value: group });
          parts.push({ type: 'integer', value: segment });
        });
      } else {
        parts.push({ type: 'integer', value: whole });
      }
      if (fraction) {
        parts.push({ type: 'decimal', value: decimal });
        parts.push({ type: 'fraction', value: fraction });
      }
      if (suffix) {
        if (/^\s/.test(suffix)) {
          parts.push({ type: 'literal', value: suffix[0] });
          suffix = suffix.slice(1);
        }
        if (suffix) parts.push({ type: 'compact', value: suffix });
      }
      return parts;
    }

    // `Intl.NumberFormat.prototype.formatToParts` (ECMA-402): standard
    // notation is rebuilt from the localized numeric core plus typed
    // affixes; `compact` parses the rendered compact string into numeric +
    // `compact` parts; `scientific`/`engineering` emit mantissa +
    // `exponentSeparator`/`exponentInteger` parts.
    formatToParts(value) {
      const exact = this._exactNotationDigits(value);
      const input = exact !== null ? value : Number(value);
      const full = this.format(input);
      const number = this._style === 'percent' ? Number(input) * 100 : Number(input);
      if (!Number.isFinite(number) && exact === null) {
        const type = Number.isNaN(number) ? 'nan' : 'infinity';
        const sign = this._signFor(number, exact);
        const marker = type === 'nan' ? 'NaN' : 'Infinity';
        const index = full.indexOf(marker);
        if (index < 0) return [{ type, value: full }];
        const prefix = full.slice(0, index);
        const parts = [];
        const signIndex = sign ? prefix.indexOf(sign) : -1;
        if (signIndex < 0) parts.push(...this._affixParts(prefix));
        else {
          parts.push(...this._affixParts(prefix.slice(0, signIndex)));
          parts.push({ type: sign === '-' ? 'minusSign' : 'plusSign', value: sign });
          parts.push(...this._affixParts(prefix.slice(signIndex + sign.length)));
        }
        parts.push({ type, value: marker });
        parts.push(...this._affixParts(full.slice(index + marker.length)));
        return parts;
      }
      const scientific = this._notation === 'scientific' || this._notation === 'engineering'
        ? this._formatScientific(number, input) : null;
      const standardDigits = this._notation === 'standard'
        ? this._standardDigits(number, input) : null;
      const sign = this._signFor(number, exact,
        scientific ? scientific.nonzero : standardDigits ? standardDigits.nonzero : null);
      if (this._notation === 'compact') {
        const parts = [];
        let text = full;
        if (text.startsWith('-')) {
          parts.push({ type: 'minusSign', value: '-' });
          text = text.slice(1);
        }
        parts.push(...this._compactParts(text));
        return parts;
      }
      if (this._notation === 'scientific' || this._notation === 'engineering') {
        const parts = [];
        if (sign) parts.push({ type: sign === '-' ? 'minusSign' : 'plusSign', value: sign });
        parts.push(...this._scientificParts(scientific.text));
        return parts;
      }
      const { intPart, fracPart, groupDigits } = standardDigits;
      const numeric = this._numericParts(intPart, fracPart, groupDigits);
      const core = numeric.map(part => part.value).join('');
      const index = full.indexOf(core);
      if (index < 0) {
        return [{ type: 'literal', value: full }];
      }
      let prefix = full.slice(0, index);
      const suffix = full.slice(index + core.length);
      const parts = [];
      if (sign && prefix.startsWith(sign)) {
        parts.push({ type: sign === '-' ? 'minusSign' : 'plusSign', value: sign });
        prefix = prefix.slice(sign.length);
      }
      parts.push(...this._affixParts(prefix));
      parts.push(...numeric);
      parts.push(...this._affixParts(suffix));
      return parts;
    }

    // ICU number skeleton for `formatRange` (style/notation/sign/
    // grouping/precision), matching the digit options `format` applies.
    _rangeSkeleton() {
      const tokens = [];
      if (this._style === 'percent') {
        tokens.push('percent');
        // The endpoint strings have already been scaled by 100.
        tokens.push('scale/0.01');
      } else if (this._style === 'currency') {
        tokens.push(`currency/${this._currency}`);
        if (this._currencyDisplay === 'code') tokens.push('unit-width-iso-code');
        else if (this._currencyDisplay === 'name') tokens.push('unit-width-full-name');
        else if (this._currencyDisplay === 'narrowSymbol') tokens.push('unit-width-narrow');
      } else if (this._style === 'unit') {
        tokens.push(`unit/${this._unit}`);
        if (this._unitDisplay === 'narrow') tokens.push('unit-width-narrow');
        else if (this._unitDisplay === 'long') tokens.push('unit-width-full-name');
      }
      if (this._notation === 'compact') {
        tokens.push(this._compactDisplay === 'long' ? 'compact-long' : 'compact-short');
      } else if (this._notation === 'scientific' || this._notation === 'engineering') {
        tokens.push(this._notation);
      }
      if (this._useGrouping === false) tokens.push('group-off');
      if (this._minimumIntegerDigits > 1) tokens.push(`integer-width/*${'0'.repeat(this._minimumIntegerDigits)}`);
      if (this._signDisplay === 'always') tokens.push('sign-always');
      else if (this._signDisplay === 'never') tokens.push('sign-never');
      else if (this._signDisplay === 'exceptZero') tokens.push('sign-except-zero');
      else if (this._signDisplay === 'negative') tokens.push('sign-negative');
      if (this._significant) {
        let precision = '@'.repeat(this._minimumSignificantDigits);
        if (this._maximumSignificantDigits > this._minimumSignificantDigits) {
          precision += '#'.repeat(this._maximumSignificantDigits - this._minimumSignificantDigits);
        }
        tokens.push(precision);
      } else {
        let minFrac = this._minimumFractionDigits;
        let maxFrac = this._maximumFractionDigits;
        if (minFrac === undefined && maxFrac === undefined) {
          let currencyDigits = 2;
          if (this._style === 'currency' && typeof __thaw_intl_currency_fraction_digits === 'function') {
            const resolved = __thaw_intl_currency_fraction_digits(this._currency);
            if (resolved !== null && resolved !== undefined) currencyDigits = resolved;
          }
          minFrac = this._style === 'currency' ? currencyDigits : 0;
          maxFrac = this._style === 'currency' ? minFrac : (this._style === 'percent' ? 0 : 3);
        } else if (minFrac === undefined) {
          minFrac = 0;
        } else if (maxFrac === undefined) {
          maxFrac = Math.max(minFrac, 3);
        }
        if (maxFrac < minFrac) maxFrac = minFrac;
        if (maxFrac > 0) {
          tokens.push('.' + '0'.repeat(minFrac) + '#'.repeat(Math.max(0, maxFrac - minFrac)));
        }
      }
      return tokens.join(' ');
    }

    // `Intl.NumberFormat.prototype.formatRange` (ECMA-402) via ICU4C's
    // number range formatter under the opt-in `--icu4c`; without it, a
    // documented approximation joining the two `format()` results with an
    // en dash (`~n` for equal endpoints).
    formatRange(start, end) {
      const startNumber = this._style === 'percent' ? Number(start) * 100 : Number(start);
      const endNumber = this._style === 'percent' ? Number(end) * 100 : Number(end);
      const startExact = this._exactNotationDigits(start);
      const endExact = this._exactNotationDigits(end);
      if ((!Number.isFinite(startNumber) && startExact === null) ||
          (!Number.isFinite(endNumber) && endExact === null)) {
        throw new RangeError('Invalid value');
      }
      const scientific = this._notation === 'scientific' || this._notation === 'engineering';
      const nativeExact = intlNativeRangeInputFits(start) && intlNativeRangeInputFits(end) &&
        (!scientific || (this._exactStandardDigits(start) !== null && this._exactStandardDigits(end) !== null));
      if (nativeExact && typeof __thaw_intl_number_range_icu4c === 'function') {
        const asText = (number, exact, { intPart, fracPart }) =>
          `${(exact ? exact.negative : number < 0 || Object.is(number, -0)) ? '-' : ''}${intPart}${fracPart ? `.${fracPart}` : ''}`;
        const startText = asText(startNumber, startExact, this._standardDigits(startNumber, start));
        const endText = asText(endNumber, endExact, this._standardDigits(endNumber, end));
        const formatted = __thaw_intl_number_range_icu4c(
          this.locale,
          this._rangeSkeleton(),
          startText,
          endText,
        );
        if (formatted) return formatted;
      }
      const same = startExact && endExact
        ? (scientific
          ? startExact.mantissa === endExact.mantissa && startExact.exponent === endExact.exponent
          : startExact.intPart === endExact.intPart && startExact.fracPart === endExact.fracPart) &&
          this._signFor(startNumber, startExact) === this._signFor(endNumber, endExact)
        : startNumber === endNumber;
      if (same) return `~${this.format(start)}`;
      const separator = this._numberRangeSeparator();
      if (!this._rangeSharesAffixes()) {
        return `${this.format(start)}${separator}${this.format(end)}`;
      }
      const { shared, startMiddle, endMiddle, trailing } = intlPartitionRange(
        this.formatToParts(start),
        this.formatToParts(end),
      );
      return [...shared, ...startMiddle, { value: separator }, ...endMiddle, ...trailing]
        .map(part => part.value)
        .join('');
    }

    _numberRangeSeparator() {
      return this._notation === 'compact' || this._style === 'currency' || this._style === 'percent'
        ? ' \u2013 '
        : '\u2013';
    }

    // Currency/percent symbols (and compact suffixes) repeat on *both*
    // endpoints, so they aren't shared range text; every other style's
    // common affixes (a unit's `" days"`, decimal digits) are.
    _rangeSharesAffixes() {
      return !(
        this._notation === 'compact' ||
        this._style === 'currency' ||
        this._style === 'percent'
      );
    }

    // `Intl.NumberFormat.prototype.formatRangeToParts` -- ICU4C number
    // range spans give the real shared/startRange/endRange sources.
    formatRangeToParts(start, end) {
      const startNumber = this._style === 'percent' ? Number(start) * 100 : Number(start);
      const endNumber = this._style === 'percent' ? Number(end) * 100 : Number(end);
      const startExact = this._exactNotationDigits(start);
      const endExact = this._exactNotationDigits(end);
      if ((!Number.isFinite(startNumber) && startExact === null) ||
          (!Number.isFinite(endNumber) && endExact === null)) {
        throw new RangeError('Invalid value');
      }
      const scientific = this._notation === 'scientific' || this._notation === 'engineering';
      const nativeExact = intlNativeRangeInputFits(start) && intlNativeRangeInputFits(end) &&
        (!scientific || (this._exactStandardDigits(start) !== null && this._exactStandardDigits(end) !== null));
      if (nativeExact && typeof __thaw_intl_number_range_parts_icu4c === 'function') {
        const asText = (number, exact, { intPart, fracPart }) =>
          `${(exact ? exact.negative : number < 0 || Object.is(number, -0)) ? '-' : ''}${intPart}${fracPart ? `.${fracPart}` : ''}`;
        const parts = JSON.parse(
          __thaw_intl_number_range_parts_icu4c(
            this.locale,
            this._rangeSkeleton(),
            asText(startNumber, startExact, this._standardDigits(startNumber, start)),
            asText(endNumber, endExact, this._standardDigits(endNumber, end)),
          ),
        );
        if (parts.length) return parts;
      }
      const same = startExact && endExact
        ? (scientific
          ? startExact.mantissa === endExact.mantissa && startExact.exponent === endExact.exponent
          : startExact.intPart === endExact.intPart && startExact.fracPart === endExact.fracPart) &&
          this._signFor(startNumber, startExact) === this._signFor(endNumber, endExact)
        : startNumber === endNumber;
      if (same) {
        return [
          { type: 'approximatelySign', value: '~', source: 'shared' },
          ...this.formatToParts(start).map(part => ({ ...part, source: 'shared' })),
        ];
      }
      const separator = this._numberRangeSeparator();
      if (!this._rangeSharesAffixes()) {
        return [
          ...this.formatToParts(start).map(part => ({ ...part, source: 'startRange' })),
          { type: 'literal', value: separator, source: 'shared' },
          ...this.formatToParts(end).map(part => ({ ...part, source: 'endRange' })),
        ];
      }
      const { shared, startMiddle, endMiddle, trailing } = intlPartitionRange(
        this.formatToParts(start),
        this.formatToParts(end),
      );
      return [
        ...shared.map(part => ({ ...part, source: 'shared' })),
        ...startMiddle.map(part => ({ ...part, source: 'startRange' })),
        { type: 'literal', value: separator, source: 'shared' },
        ...endMiddle.map(part => ({ ...part, source: 'endRange' })),
        ...trailing.map(part => ({ ...part, source: 'shared' })),
      ];
    }
  }

  class ListFormat {
    constructor(locale, options) {
      const opts = options || {};
      this._useRealLocaleData = typeof __thaw_intl_list_format === 'function';
      this.locale = this._useRealLocaleData ? intlRequestedLocale(locale) : 'en-US';
      this._type = intlEnumOption(opts.type, ['conjunction', 'disjunction', 'unit'], 'conjunction', 'type');
      this._style = intlEnumOption(opts.style, ['long', 'short', 'narrow'], 'long', 'style');
    }

    format(list) {
      const items = Array.from(list);
      if (items.some(item => typeof item !== 'string')) throw new TypeError('ListFormat items must be strings');
      if (this._useRealLocaleData) {
        const kind = this._type === 'disjunction' ? 'or' : this._type === 'unit' ? 'unit' : 'and';
        return __thaw_intl_list_format(this.locale, kind, this._style, JSON.stringify(items));
      }
      if (items.length === 0) return '';
      if (items.length === 1) return items[0];
      const conjunction = this._type === 'disjunction' ? 'or' : 'and';
      if (items.length === 2) return `${items[0]} ${conjunction} ${items[1]}`;
      return `${items.slice(0, -1).join(', ')}, ${conjunction} ${items[items.length - 1]}`;
    }
  }

  // `Intl.Locale` -- real BCP-47 identity backed by `icu4x`
  // (`intl_locale.rs`, docs/design/intl-polyfill.md's "Real CLDR data
  // via icu4x" section). Only defined when the `intl` Cargo feature is
  // compiled in (`__thaw_intl_locale_parse` exists) -- a program that
  // never references `Intl.Locale`/`PluralRules`/`Collator`/`Segmenter`/
  // `RelativeTimeFormat` doesn't link `thaw-icu-data` at all
  // (`crates/thaw-cli/src/build.rs`'s `source_uses_intl`), so this class
  // simply doesn't exist for it -- matching real `Intl.Locale` not
  // existing at all is the wrong shape (it always exists in real Node),
  // but no program can observe the difference unless it actually names
  // `Intl.Locale`, which is exactly what turns the feature on.
  // Declared unconditionally (not gated by the `if` below) so
  // `DateTimeFormat`'s own `calendar`/`numberingSystem` constructor
  // option handling can reuse it internally even when `Intl.Locale`
  // itself isn't meant to be publicly exposed yet -- its methods only
  // ever get called from a code path that already checked the native
  // global exists (`_useRealLocaleData`), so the class body itself
  // never touches anything unavailable.
  class Locale {
      constructor(tag, options) {
        if (tag === undefined || tag === null) {
          throw new TypeError("First argument to Intl.Locale constructor can't be empty or missing");
        }
        const base = INTL_LOCALE_TAGS.has(tag) ? INTL_LOCALE_TAGS.get(tag) : String(tag);
        const opts = options || {};
        const original = JSON.parse(__thaw_intl_locale_parse(base));
        if (!original.valid) throw new RangeError(`Invalid language tag: ${tag}`);
        let effectiveTag = original.tag;
        const overrides = [['ca', opts.calendar], ['nu', opts.numberingSystem], ['co', opts.collation]]
          .filter(([, value]) => value !== undefined);
        if (overrides.length) {
          const subtags = effectiveTag.split('-');
          const unicode = subtags.findIndex(part => part === 'u');
          const end = unicode < 0
            ? subtags.findIndex((part, index) => index > 0 && part.length === 1 && part > 'u')
            : subtags.findIndex((part, index) => index > unicode && part.length === 1);
          const stop = end < 0 ? subtags.length : end;
          const body = unicode < 0 ? [] : subtags.slice(unicode + 1, stop);
          const attributes = [];
          const keywords = new Map();
          let index = 0;
          while (index < body.length && body[index].length !== 2) attributes.push(body[index++]);
          while (index < body.length) {
            const key = body[index++];
            const values = [];
            while (index < body.length && body[index].length !== 2) values.push(body[index++]);
            keywords.set(key, values.join('-'));
          }
          for (const [key, value] of overrides) keywords.set(key, String(value));
          const updated = ['u', ...attributes, ...[...keywords].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
            .flatMap(([key, value]) => [key, ...(value ? value.split('-') : [])])];
          subtags.splice(unicode < 0 ? stop : unicode, unicode < 0 ? 0 : stop - unicode, ...updated);
          effectiveTag = subtags.join('-');
        }
        const parsed = JSON.parse(__thaw_intl_locale_parse(effectiveTag));
        if (!parsed.valid) throw new RangeError(`Invalid language tag: ${tag}`);
        this._tag = parsed.tag;
        INTL_LOCALE_TAGS.set(this, parsed.tag);
        this._baseName = parsed.baseName;
        this._language = parsed.language;
        this._script = parsed.script;
        this._region = parsed.region;
        this._calendar = parsed.calendar;
        this._numberingSystem = parsed.numberingSystem;
        this._collation = parsed.collation;
      }

      get language() {
        return this._language;
      }

      get script() {
        return this._script === null ? undefined : this._script;
      }

      get region() {
        return this._region === null ? undefined : this._region;
      }

      get calendar() {
        return this._calendar === null ? undefined : this._calendar;
      }

      get numberingSystem() {
        return this._numberingSystem === null ? undefined : this._numberingSystem;
      }

      get collation() {
        return this._collation === null ? undefined : this._collation;
      }

      get baseName() {
        return this._baseName;
      }

      toString() {
        return this._tag;
      }

      // `maximize()`/`minimize()` preserve the original instance's own
      // Unicode extension keywords (confirmed against real Node) --
      // only the language/script/region subtags themselves transform.
      _withTransformedSubtags(transformed) {
        return new Locale(transformed.tag);
      }

      maximize() {
        return this._withTransformedSubtags(JSON.parse(__thaw_intl_locale_maximize(this._tag)));
      }

      minimize() {
        return this._withTransformedSubtags(JSON.parse(__thaw_intl_locale_minimize(this._tag)));
      }
  }

  // `Intl.PluralRules` (M8) -- entirely new, no prior English-only
  // version existed. `minimum`/`maximumFractionDigits` and
  // `minimum`/`maximumSignificantDigits` are honored by formatting the
  // operand the same way `Intl.NumberFormat` would (so the ICU plural
  // rules see the real visible-fraction-digit count, which is what
  // distinguishes e.g. `en` `one` for `1` from `other` for `1.0`);
  // `resolvedOptions().pluralCategories` returns the locale's real
  // categories via `__thaw_intl_plural_categories`.
  class PluralRules {
    constructor(locale, options) {
      const opts = options || {};
      this.locale = intlRequestedLocale(locale);
      this._type = intlEnumOption(opts.type, ['cardinal', 'ordinal'], 'cardinal', 'type');
      this._significant = opts.minimumSignificantDigits !== undefined || opts.maximumSignificantDigits !== undefined;
      this._minimumIntegerDigits = intlIntegerOption(opts.minimumIntegerDigits, 1, 1, 21);
      this._minimumFractionDigits = intlIntegerOption(opts.minimumFractionDigits, 0, 0, 100);
      this._maximumFractionDigits = intlIntegerOption(opts.maximumFractionDigits, Math.max(this._minimumFractionDigits, 3), this._minimumFractionDigits, 100);
      this._minimumSignificantDigits = intlIntegerOption(opts.minimumSignificantDigits, 1, 1, 21);
      this._maximumSignificantDigits = intlIntegerOption(opts.maximumSignificantDigits, Math.max(this._minimumSignificantDigits, 21), this._minimumSignificantDigits, 21);
      this._roundingMode = intlEnumOption(opts.roundingMode, INTL_ROUNDING_MODES, 'halfExpand', 'roundingMode');
    }

    _operandString(value) {
      const number = Number(value);
      if (!Number.isFinite(number)) throw new RangeError(`Invalid value: ${String(value)}`);
      return intlPluralOperand(
        number,
        this._significant,
        this._minimumFractionDigits,
        this._maximumFractionDigits,
        this._minimumSignificantDigits,
        this._maximumSignificantDigits,
        this._roundingMode,
      );
    }

    select(value) {
      const number = Number(value);
      if (!Number.isFinite(number) || typeof __thaw_intl_plural_category !== 'function') return 'other';
      return __thaw_intl_plural_category(this.locale, this._type, this._operandString(number));
    }

    // `Intl.PluralRules.prototype.selectRange` (ECMA-402) with real
    // ICU4C semantics (opt-in `--icu4c`). ICU4X's vendored plural-range
    // data diverges from ICU4C/Node, so without the backend this
    // degrades to the always-valid `"other"` category.
    selectRange(start, end) {
      const startNumber = Number(start);
      const endNumber = Number(end);
      if (!Number.isFinite(startNumber) || !Number.isFinite(endNumber)) {
        throw new RangeError('selectRange requires finite start and end values');
      }
      if (typeof __thaw_intl_plural_range_icu4c === 'function') {
        return __thaw_intl_plural_range_icu4c(
          this.locale,
          this._type,
          this._operandString(startNumber),
          this._operandString(endNumber),
        );
      }
      return 'other';
    }

    resolvedOptions() {
      const pluralCategories = typeof __thaw_intl_plural_categories === 'function'
        ? JSON.parse(__thaw_intl_plural_categories(this.locale, this._type))
        : ['other'];
      const result = {
        locale: this.locale,
        type: this._type,
        minimumIntegerDigits: this._minimumIntegerDigits,
      };
      if (this._significant) {
        result.minimumSignificantDigits = this._minimumSignificantDigits;
        result.maximumSignificantDigits = this._maximumSignificantDigits;
      } else {
        result.minimumFractionDigits = this._minimumFractionDigits;
        result.maximumFractionDigits = this._maximumFractionDigits;
      }
      result.pluralCategories = pluralCategories;
      result.roundingIncrement = 1;
      result.roundingMode = this._roundingMode;
      result.roundingPriority = 'auto';
      result.trailingZeroDisplay = 'auto';
      return result;
    }
  }

  // `Intl.Collator` (M9) -- entirely new. `usage: 'search'` isn't
  // supported (falls back to plain `'sort'` behavior, see
  // `intl_collator.rs`'s own doc comment).
  class Collator {
    constructor(locale, options) {
      const opts = options || {};
      this.locale = intlRequestedLocale(locale);
      this._sensitivity = intlEnumOption(opts.sensitivity, ['base', 'accent', 'case', 'variant'], 'variant', 'sensitivity');
      this._usage = intlEnumOption(opts.usage, ['sort', 'search'], 'sort', 'usage');
      this._ignorePunctuation = Boolean(opts.ignorePunctuation);
      this._numeric = Boolean(opts.numeric);
      this._caseFirst = intlEnumOption(opts.caseFirst, ['upper', 'lower', 'false'], 'false', 'caseFirst');
      this.compare = this.compare.bind(this);
    }

    compare(a, b) {
      // `usage: 'search'` needs ICU4C's search collation (ICU4X has no
      // search-collation concept); only present with the opt-in `icu4c`
      // feature, otherwise `usage: 'search'` keeps sort behavior.
      if (this._usage === 'search' && typeof __thaw_intl_collator_compare_search === 'function') {
        return __thaw_intl_collator_compare_search(
          this.locale,
          this._sensitivity,
          this._ignorePunctuation,
          this._numeric,
          this._caseFirst,
          String(a),
          String(b),
        );
      }
      return __thaw_intl_collator_compare(
        this.locale,
        this._sensitivity,
        this._ignorePunctuation,
        this._numeric,
        this._caseFirst,
        String(a),
        String(b),
      );
    }

    resolvedOptions() {
      return {
        locale: this.locale,
        usage: this._usage,
        sensitivity: this._sensitivity,
        ignorePunctuation: this._ignorePunctuation,
        numeric: this._numeric,
        caseFirst: this._caseFirst,
        collation: 'default',
      };
    }
  }

  // `Intl.Segmenter` (M10) -- entirely new. The iterable protocol
  // (`for (const s of segmenter.segment(text))`, by far the common
  // usage) and `.containing(index)` random access are both implemented.
  class Segmenter {
    constructor(locale, options) {
      const opts = options || {};
      this.locale = intlRequestedLocale(locale);
      this._granularity = intlEnumOption(opts.granularity, ['grapheme', 'word', 'sentence'], 'grapheme', 'granularity');
    }

    resolvedOptions() {
      return { locale: this.locale, granularity: this._granularity };
    }

    segment(text) {
      const input = String(text);
      const parts = JSON.parse(__thaw_intl_segment(this.locale, this._granularity, input));
      const segments = parts.map(p => ({
        segment: p.segment,
        index: p.index,
        input,
        ...(p.isWordLike === null ? {} : { isWordLike: p.isWordLike }),
      }));
      return {
        [Symbol.iterator]() {
          return segments.map(entry => ({ ...entry }))[Symbol.iterator]();
        },
        // `Segments.prototype.containing(index)` -- real ECMA-402 random
        // access, previously a documented gap. Returns the segment whose
        // `[index, index + segment.length)` range contains `index`, or
        // `undefined` for an out-of-range index.
        containing(index) {
          const n = Number(index);
          const i = Number.isNaN(n) ? 0 : Math.trunc(n);
          if (!Number.isFinite(i) || i < 0 || i >= input.length) return undefined;
          for (const entry of segments) {
            if (entry.index <= i && i < entry.index + entry.segment.length) return { ...entry };
          }
          return undefined;
        },
      };
    }
  }

  // `Intl.RelativeTimeFormat` (M11) -- entirely new, backed by the one
  // deliberately-unstable icu4x dependency in this whole effort
  // (`icu_experimental`, see `intl_relative_time.rs`'s own doc comment).
  const INTL_RELATIVE_TIME_UNITS = {
    __proto__: null,
    year: 'year', years: 'year',
    quarter: 'quarter', quarters: 'quarter',
    month: 'month', months: 'month',
    week: 'week', weeks: 'week',
    day: 'day', days: 'day',
    hour: 'hour', hours: 'hour',
    minute: 'minute', minutes: 'minute',
    second: 'second', seconds: 'second',
  };

  class RelativeTimeFormat {
    constructor(locale, options) {
      const opts = options || {};
      const localeTag = intlRequestedLocale(locale);
      const numberingSystem = opts.numberingSystem;
      this.locale = numberingSystem === undefined
        ? localeTag : new Locale(localeTag, { numberingSystem }).toString();
      this._style = opts.style === undefined ? 'long' : String(opts.style);
      this._numeric = opts.numeric === undefined ? 'always' : String(opts.numeric);
      if (!['long', 'short', 'narrow'].includes(this._style)) throw new RangeError(`Invalid style: ${this._style}`);
      if (!['always', 'auto'].includes(this._numeric)) throw new RangeError(`Invalid numeric: ${this._numeric}`);
    }

    format(value, unit) {
      const number = Number(value);
      if (!Number.isFinite(number)) throw new RangeError('value must be finite');
      const resolvedUnit = INTL_RELATIVE_TIME_UNITS[unit];
      if (!resolvedUnit) {
        throw new RangeError(`Invalid unit argument for Intl.RelativeTimeFormat.prototype.format() '${unit}'`);
      }
      return __thaw_intl_relative_time_format(this.locale, resolvedUnit, this._style, this._numeric, number);
    }

    resolvedOptions() {
      const resolved = typeof __thaw_intl_datetime_resolved_options === 'function'
        ? JSON.parse(__thaw_intl_datetime_resolved_options(this.locale)) : null;
      return { locale: this.locale, style: this._style, numeric: this._numeric,
        numberingSystem: resolved && resolved.numberingSystem ? resolved.numberingSystem : 'latn' };
    }
  }

  // `Intl.Locale`/`Intl.PluralRules`/`Intl.Collator`/`Intl.Segmenter`/
  // `Intl.RelativeTimeFormat` are only ever *publicly exposed* when the
  // native primitives actually exist (see each class's own doc comment
  // above) -- the class declarations themselves stay unconditional.
  if (typeof __thaw_intl_locale_parse === 'function') {
    globalThis.Intl = globalThis.Intl || {};
    globalThis.Intl.Locale = Locale;
    globalThis.Intl.PluralRules = PluralRules;
    globalThis.Intl.Collator = Collator;
    globalThis.Intl.Segmenter = Segmenter;
    globalThis.Intl.RelativeTimeFormat = RelativeTimeFormat;
  }

  // `Intl.DisplayNames`/`Intl.DurationFormat`. Neither is provided by the
  // native icu4x primitives this file wraps, so both are pure-JS tables.
  // The tables are keyed by *language* so a new language is added by
  // dropping another entry into `DISPLAY_NAMES_TABLES`/`DURATION_TABLES`
  // (an unknown language falls back to `en`).
  const DISPLAY_NAMES_TABLES = {
    en: {
      language: {
        en: 'English', fr: 'French', de: 'German', es: 'Spanish',
        ja: 'Japanese', zh: 'Chinese', ko: 'Korean', ru: 'Russian',
        ar: 'Arabic', pt: 'Portuguese', it: 'Italian',
      },
      region: {
        US: 'United States', GB: 'United Kingdom', JP: 'Japan',
        FR: 'France', DE: 'Germany', CN: 'China', KR: 'South Korea',
        CA: 'Canada', AU: 'Australia', IN: 'India',
      },
      script: {
        Latn: 'Latin', Cyrl: 'Cyrillic', Hans: 'Simplified Han',
        Hant: 'Traditional Han', Jpan: 'Japanese', Kore: 'Korean',
        Arab: 'Arabic', Grek: 'Greek', Hebr: 'Hebrew',
      },
      currency: {
        USD: 'US Dollar', EUR: 'Euro', JPY: 'Japanese Yen',
        GBP: 'British Pound', CNY: 'Chinese Yuan',
      },
    },
    ja: {
      language: {
        en: '英語', fr: 'フランス語', de: 'ドイツ語', es: 'スペイン語',
        ja: '日本語', zh: '中国語', ko: '韓国語', ru: 'ロシア語',
        ar: 'アラビア語', pt: 'ポルトガル語', it: 'イタリア語',
      },
      region: {
        US: 'アメリカ合衆国', GB: 'イギリス', JP: '日本', FR: 'フランス',
        DE: 'ドイツ', CN: '中国', KR: '韓国', CA: 'カナダ',
        AU: 'オーストラリア', IN: 'インド',
      },
      script: {
        Latn: 'ラテン文字', Cyrl: 'キリル文字', Hans: '簡体字',
        Hant: '繁体字', Jpan: '日本語', Kore: 'ハングル', Arab: 'アラビア文字',
      },
      currency: {
        USD: '米ドル', EUR: 'ユーロ', JPY: '日本円', GBP: '英ポンド',
        CNY: '中国人民元',
      },
    },
  };
  const DISPLAY_NAMES_TYPES = ['language', 'region', 'script', 'currency', 'calendar', 'dateTimeField'];

  function primarylanguage(locale) {
    const language = String(locale || 'en').split('-')[0].toLowerCase();
    return DISPLAY_NAMES_TABLES[language] ? language : 'en';
  }

  class DisplayNames {
    constructor(locales, options) {
      const opts = options || {};
      this.locale = intlRequestedLocale(locales, 'en');
      this._language = primarylanguage(this.locale);
      this._type = opts.type === undefined ? 'language' : String(opts.type);
      this._style = opts.style === undefined ? 'long' : String(opts.style);
      this._fallback = opts.fallback === undefined ? 'code' : String(opts.fallback);
      if (!DISPLAY_NAMES_TYPES.includes(this._type)) {
        throw new RangeError(`Invalid type: ${this._type}`);
      }
    }

    of(code) {
      const table = DISPLAY_NAMES_TABLES[this._language][this._type];
      const raw = String(code);
      const normalized = this._type === 'region' || this._type === 'currency' ? raw.toUpperCase()
        : this._type === 'language' ? raw.toLowerCase()
        : this._type === 'script' ? raw[0].toUpperCase() + raw.slice(1).toLowerCase() : raw;
      const value = table !== undefined && Object.prototype.hasOwnProperty.call(table, normalized) ? table[normalized] : undefined;
      if (value !== undefined) return value;
      return this._fallback === 'none' ? undefined : normalized;
    }

    resolvedOptions() {
      return {
        locale: this.locale,
        style: this._style,
        type: this._type,
        fallback: this._fallback,
      };
    }
  }

  const DURATION_UNITS = [
    'years', 'months', 'weeks', 'days', 'hours',
    'minutes', 'seconds', 'milliseconds', 'microseconds', 'nanoseconds',
  ];
  const DURATION_TABLES = {
    en: {
      long: {
        years: 'year', months: 'month', weeks: 'week', days: 'day',
        hours: 'hour', minutes: 'minute', seconds: 'second',
        milliseconds: 'millisecond', microseconds: 'microsecond',
        nanoseconds: 'nanosecond',
      },
      short: {
        years: 'yr', months: 'mth', weeks: 'wk', days: 'day',
        hours: 'hr', minutes: 'min', seconds: 'sec',
        milliseconds: 'ms', microseconds: 'μs', nanoseconds: 'ns',
      },
      narrow: {
        years: 'y', months: 'm', weeks: 'w', days: 'd',
        hours: 'h', minutes: 'm', seconds: 's',
        milliseconds: 'ms', microseconds: 'μs', nanoseconds: 'ns',
      },
      separator: ', ',
      unit: ' ',
    },
    ja: {
      long: {
        years: '年', months: 'か月', weeks: '週間', days: '日',
        hours: '時間', minutes: '分', seconds: '秒',
        milliseconds: 'ミリ秒', microseconds: 'マイクロ秒', nanoseconds: 'ナノ秒',
      },
      short: {
        years: '年', months: 'か月', weeks: '週', days: '日',
        hours: '時間', minutes: '分', seconds: '秒',
        milliseconds: 'ミリ秒', microseconds: 'マイクロ秒', nanoseconds: 'ナノ秒',
      },
      narrow: {
        years: '年', months: 'か月', weeks: '週', days: '日',
        hours: '時間', minutes: '分', seconds: '秒',
        milliseconds: 'ミリ秒', microseconds: 'マイクロ秒', nanoseconds: 'ナノ秒',
      },
      separator: ' ',
      unit: ' ',
    },
  };

  class DurationFormat {
    constructor(locales, options) {
      const opts = options || {};
      this.locale = intlRequestedLocale(locales, 'en');
      this._language = primarylanguage(this.locale);
      this._style = opts.style === undefined ? 'short' : String(opts.style);
      if (!['long', 'short', 'narrow', 'digital'].includes(this._style)) {
        throw new RangeError(`Invalid style: ${this._style}`);
      }
    }

    format(duration) {
      const table = DURATION_TABLES[this._language];
      const style = this._style === 'digital' ? 'short' : this._style;
      if (duration === null || (typeof duration !== 'object' && typeof duration !== 'function')) {
        if (typeof duration === 'string') throw new RangeError('Invalid duration');
        throw new TypeError('Duration must be an object');
      }
      const value = duration;
      const numbers = {};
      let supplied = false;
      let sign = 0;
      for (const unit of DURATION_UNITS) {
        const raw = value[unit];
        if (raw === undefined) continue;
        supplied = true;
        const number = Number(raw);
        if (!Number.isFinite(number) || !Number.isInteger(number)) throw new RangeError(`Invalid duration ${unit}`);
        if (number !== 0) {
          if (sign && Math.sign(number) !== sign) throw new RangeError('Duration fields have mixed signs');
          sign = Math.sign(number);
        }
        numbers[unit] = number;
      }
      if (!supplied) throw new TypeError('Duration has no fields');
      const parts = [];
      for (const unit of DURATION_UNITS) {
        const number = numbers[unit];
        if (number === undefined) continue;
        if (number === 0) continue;
        if (this._style === 'digital' && ['hours', 'minutes', 'seconds'].includes(unit)) continue;
        let name = table[style][unit];
        // English long form pluralizes a non-unit count ("30 minutes").
        if (this._language === 'en' && style === 'long' && number !== 1) {
          name += 's';
        }
        parts.push(`${number}${table.unit}${name}`);
      }
      if (this._style === 'digital') {
        const hours = Math.abs(numbers.hours || 0);
        const minutes = Math.abs(numbers.minutes || 0);
        const seconds = Math.abs(numbers.seconds || 0);
        if (numbers.hours !== undefined || numbers.minutes !== undefined || numbers.seconds !== undefined) {
          const fields = numbers.hours !== undefined
            ? [String(hours), String(minutes).padStart(2, '0'), String(seconds).padStart(2, '0')]
            : numbers.minutes !== undefined
              ? [String(minutes), String(seconds).padStart(2, '0')]
              : [String(seconds)];
          parts.push(`${sign < 0 ? '-' : ''}${fields.join(':')}`);
        }
      }
      return parts.join(table.separator);
    }

    resolvedOptions() {
      return { locale: this.locale, style: this._style, numberingSystem: 'latn' };
    }
  }

  globalThis.Intl = Object.assign(
    { DateTimeFormat, NumberFormat, ListFormat, DisplayNames, DurationFormat },
    globalThis.Intl,
  );
})();
