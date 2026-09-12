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

  function intlZonedParts(timeZone, epochMs) {
    return JSON.parse(__thaw_intl_zoned_parts(String(timeZone), Number(epochMs)));
  }

  function intlPad(value, width) {
    const text = String(Math.trunc(value));
    return text.length >= width ? text : '0'.repeat(width - text.length) + text;
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

  function intlTimeZoneName(style, zoned) {
    if (style === 'longOffset') {
      return intlOffsetString(zoned.offsetMinutes, true);
    }
    if (style === 'long') {
      // Real per-zone English long name (e.g. "Eastern Daylight
      // Time"/"Eastern Standard Time", DST-aware) -- `intl_zoned_
      // parts_json`'s `longName`, mechanically extracted from a real
      // `node`/ICU run (`intl_time_zone_names.rs`). Falls back to the
      // synthesized numeric offset for a zone the table has no entry
      // for (shouldn't happen for a real IANA zone, but keeps this
      // honestly degrading rather than throwing either way).
      return zoned.longName !== null ? zoned.longName : intlOffsetString(zoned.offsetMinutes, true);
    }
    if (style === 'longGeneric') {
      // Same table, DST-independent form (e.g. "Eastern Time" rather
      // than "Eastern Standard/Daylight Time").
      return zoned.longGenericName !== null ? zoned.longGenericName : intlOffsetString(zoned.offsetMinutes, true);
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
      const localeTag = String(locale === undefined ? 'en-US' : locale);
      this.locale = this._useRealLocaleData
        ? (opts.calendar !== undefined || opts.numberingSystem !== undefined
            ? new Locale(localeTag, { calendar: opts.calendar, numberingSystem: opts.numberingSystem }).toString()
            : localeTag)
        : 'en-US';
      this._timeZone = opts.timeZone ? String(opts.timeZone) : 'UTC';
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
      this._timeZoneName = opts.timeZoneName;
      // Kept separate from `this._hourCycle` below (which always
      // forces a concrete value for the legacy English-only path's own
      // internal am/pm logic): the *real* per-locale path must leave
      // hour12/hourCycle unset when the caller didn't request either,
      // so the native call can apply the requested locale's own actual
      // default hour cycle (e.g. most of Europe defaults to h23, not
      // en-US's h12) instead of a hardcoded English default.
      this._explicitHour12 = opts.hour12;
      this._explicitHourCycle = opts.hourCycle;
      if (opts.hourCycle) {
        this._hourCycle = opts.hourCycle;
      } else if (opts.hour12 === true) {
        this._hourCycle = 'h12';
      } else if (opts.hour12 === false) {
        this._hourCycle = 'h23';
      } else {
        this._hourCycle = 'h12';
      }
    }

    resolvedOptions() {
      const result = {
        locale: this.locale,
        calendar: 'gregory',
        numberingSystem: 'latn',
        timeZone: this._timeZone,
      };
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
      if (this._timeZoneName) result.timeZoneName = this._timeZoneName;
      return result;
    }

    formatToParts(date) {
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
      const options = {
        weekday: this._weekday,
        era: this._era,
        year: this._year,
        month: this._month,
        day: this._day,
        hour: this._hour,
        minute: this._minute,
        second: this._second,
        hour12: this._explicitHour12,
        hourCycle: this._explicitHourCycle,
      };
      const parts = JSON.parse(
        __thaw_intl_datetime_format_parts(this.locale, JSON.stringify(options), JSON.stringify(zoned)),
      );
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
      if (this._timeZoneName) {
        if (parts.length) parts.push({ type: 'literal', value: ' ' });
        parts.push({ type: 'timeZoneName', value: intlTimeZoneName(this._timeZoneName, zoned) });
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
      const hasTime = Boolean(this._hour || this._minute || this._second);
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
        if (this._second) fields.push(['second', this._second === '2-digit' ? intlPad(zoned.second, 2) : String(zoned.second)]);
        fields.forEach(([type, value], index) => {
          if (index > 0) trailing.push({ type: 'literal', value: ':' });
          trailing.push({ type, value });
        });
        if (dayPeriod) {
          trailing.push({ type: 'literal', value: ' ' });
          trailing.push({ type: 'dayPeriod', value: dayPeriod });
        }
      }
      if (this._timeZoneName) {
        if (trailing.length) trailing.push({ type: 'literal', value: ' ' });
        trailing.push({ type: 'timeZoneName', value: intlTimeZoneName(this._timeZoneName, zoned) });
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
  }

  class NumberFormat {
    constructor(locale, options) {
      const opts = options || {};
      // Same real-per-locale-or-English-fast-path split as
      // `DateTimeFormat` above (M6): only the digit/grouping rendering
      // changes here -- rounding, sign, and `style: 'unit'`'s English
      // forms are locale-independent arithmetic already correct as-is.
      this._useRealLocaleData = typeof __thaw_intl_number_format === 'function';
      this.locale = this._useRealLocaleData ? String(locale === undefined ? 'en-US' : locale) : 'en-US';
      this._style = opts.style || 'decimal';
      this._unit = opts.unit;
      this._unitDisplay = opts.unitDisplay || 'short';
      if (this._style === 'unit' && !INTL_UNITS[this._unit]) {
        throw new RangeError(`Invalid unit argument for Intl.NumberFormat() '${this._unit}'`);
      }
      this._useGrouping = opts.useGrouping === undefined ? true : Boolean(opts.useGrouping);
      this._minimumIntegerDigits = opts.minimumIntegerDigits || 1;
      this._minimumFractionDigits = opts.minimumFractionDigits;
      this._maximumFractionDigits = opts.maximumFractionDigits;
    }

    resolvedOptions() {
      const result = {
        locale: this.locale,
        numberingSystem: 'latn',
        style: this._style,
        useGrouping: this._useGrouping,
        minimumIntegerDigits: this._minimumIntegerDigits,
      };
      if (this._style === 'unit') {
        result.unit = this._unit;
        result.unitDisplay = this._unitDisplay;
      }
      return result;
    }

    format(value) {
      const number = Number(value);
      const negative = number < 0 || Object.is(number, -0);
      let minFrac = this._minimumFractionDigits;
      let maxFrac = this._maximumFractionDigits;
      if (minFrac === undefined && maxFrac === undefined) {
        minFrac = 0;
        maxFrac = 3;
      } else if (minFrac === undefined) {
        minFrac = 0;
      } else if (maxFrac === undefined) {
        maxFrac = Math.max(minFrac, 3);
      }
      if (maxFrac < minFrac) maxFrac = minFrac;
      const fixed = Math.abs(number).toFixed(maxFrac);
      const [wholePart, fracPartRaw = ''] = fixed.split('.');
      let fracPart = fracPartRaw;
      while (fracPart.length > minFrac && fracPart.endsWith('0')) {
        fracPart = fracPart.slice(0, -1);
      }
      let intPart = wholePart;
      while (intPart.length < this._minimumIntegerDigits) intPart = `0${intPart}`;
      const digits = fracPart ? `${intPart}.${fracPart}` : intPart;
      // Sign is handled here, not passed to the native call: it's
      // applied identically across every curated locale (confirmed),
      // so there's no need to push it through a locale-data lookup.
      const rendered = this._useRealLocaleData
        ? __thaw_intl_number_format(this.locale, digits, this._useGrouping)
        : (this._useGrouping ? intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ',') : intPart) +
          (fracPart ? `.${fracPart}` : '');
      const signed = negative ? `-${rendered}` : rendered;
      if (this._style !== 'unit') return signed;
      const forms = INTL_UNITS[this._unit][this._unitDisplay] || INTL_UNITS[this._unit].short;
      const unitText = forms[Math.abs(number) === 1 ? 0 : 1];
      // 'narrow' has no space between the number and unit (`"3d"`);
      // 'long'/'short' both do (`"3 days"`/`"3 days"`) -- confirmed
      // against real Node's own output for every unit above.
      return this._unitDisplay === 'narrow' ? `${signed}${unitText}` : `${signed} ${unitText}`;
    }
  }

  class ListFormat {
    constructor(locale, options) {
      const opts = options || {};
      this._type = opts.type === 'disjunction' ? 'disjunction' : 'conjunction';
    }

    format(list) {
      const items = Array.from(list, String);
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
        const base = tag instanceof Locale ? tag.toString() : String(tag);
        const opts = options || {};
        const overrides = [];
        if (opts.calendar !== undefined) overrides.push(`ca-${opts.calendar}`);
        if (opts.numberingSystem !== undefined) overrides.push(`nu-${opts.numberingSystem}`);
        if (opts.collation !== undefined) overrides.push(`co-${opts.collation}`);
        // A `-u-` Unicode extension always starts a new subtag boundary
        // right after language/script/region/variants and right before
        // any `-x-` private-use section -- stripping from the first
        // `-u-` onward and re-appending is enough for the common case
        // (a plain tag with at most one `-u-` extension); real Node's
        // own semantics let explicit options fully replace whatever the
        // tag itself specified, confirmed against real Node.
        let effectiveTag = base;
        if (overrides.length > 0) {
          const unicodeExtensionStart = effectiveTag.search(/-u(-|$)/);
          const withoutExtension =
            unicodeExtensionStart === -1 ? effectiveTag : effectiveTag.slice(0, unicodeExtensionStart);
          effectiveTag = `${withoutExtension}-u-${overrides.join('-')}`;
        }
        const parsed = JSON.parse(__thaw_intl_locale_parse(effectiveTag));
        if (!parsed.valid) throw new RangeError(`Invalid language tag: ${tag}`);
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
        let name = this._language;
        if (this._script) name += `-${this._script}`;
        if (this._region) name += `-${this._region}`;
        return name;
      }

      toString() {
        let name = this.baseName;
        const extension = [];
        if (this._calendar) extension.push(`ca-${this._calendar}`);
        if (this._numberingSystem) extension.push(`nu-${this._numberingSystem}`);
        if (this._collation) extension.push(`co-${this._collation}`);
        if (extension.length > 0) name += `-u-${extension.join('-')}`;
        return name;
      }

      // `maximize()`/`minimize()` preserve the original instance's own
      // Unicode extension keywords (confirmed against real Node) --
      // only the language/script/region subtags themselves transform.
      _withTransformedSubtags(transformed) {
        const result = Object.create(Locale.prototype);
        result._language = transformed.language;
        result._script = transformed.script;
        result._region = transformed.region;
        result._calendar = this._calendar;
        result._numberingSystem = this._numberingSystem;
        result._collation = this._collation;
        return result;
      }

      maximize() {
        return this._withTransformedSubtags(JSON.parse(__thaw_intl_locale_maximize(this.baseName)));
      }

      minimize() {
        return this._withTransformedSubtags(JSON.parse(__thaw_intl_locale_minimize(this.baseName)));
      }
  }

  // `Intl.Locale` itself is only ever *publicly exposed* when the
  // native primitives actually exist (see the class's own doc comment
  // above) -- the class declaration itself stays unconditional.
  if (typeof __thaw_intl_locale_parse === 'function') {
    globalThis.Intl = globalThis.Intl || {};
    globalThis.Intl.Locale = Locale;
  }

  globalThis.Intl = Object.assign({ DateTimeFormat, NumberFormat, ListFormat }, globalThis.Intl);
})();
