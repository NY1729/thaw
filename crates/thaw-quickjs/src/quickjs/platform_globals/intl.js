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
    if (style === 'long' || style === 'longGeneric') {
      // No real per-zone English long names (e.g. "Eastern Daylight
      // Time") without bundling full ICU data -- documented, honest
      // simplification.
      return intlOffsetString(zoned.offsetMinutes, true);
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
      this.locale = 'en-US';
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
        locale: 'en-US',
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
      if (this._useGrouping) {
        intPart = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
      }
      const rendered = fracPart ? `${intPart}.${fracPart}` : intPart;
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

  globalThis.Intl = { DateTimeFormat, NumberFormat, ListFormat };
})();
