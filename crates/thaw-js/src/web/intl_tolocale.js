// Locale-sensitive built-ins routed through the Intl polyfill (web/intl.js). Installed by intl.rs only when
// the `intl` feature is on, so `Intl` always exists here. Without the feature the native en-US versions stay.
// Method shorthand + rest parameters give the builtin table's shapes (no `prototype`; length 0, `localeCompare` 1).
(function () {
  'use strict';
  const NumberFormat = Intl.NumberFormat;
  const DateTimeFormat = Intl.DateTimeFormat;
  const Collator = Intl.Collator;
  const numberValueOf = Number.prototype.valueOf;
  // thaw's Number.prototype.valueOf accepts a string receiver, so the brand check is explicit here.
  function thisNumber(v) {
    if (typeof v === 'number') return v;
    if (v instanceof Number) return numberValueOf.call(v);
    throw new TypeError('Number.prototype.toLocaleString requires that \'this\' be a Number');
  }
  const bigintValueOf = BigInt.prototype.valueOf;
  const dateGetTime = Date.prototype.getTime;

  // ECMA-402 ToDateTimeOptions: `required` is 'date' | 'time' | 'any', `defaults` is 'date' | 'time' | 'all'.
  function dateTimeOptions(options, required, defaults) {
    if (options === null) throw new TypeError('Cannot convert undefined or null to object');
    const o = Object.assign({}, options);
    let needDefaults = true;
    if (required !== 'time' && ['weekday', 'year', 'month', 'day'].some(k => o[k] !== undefined)) needDefaults = false;
    if (required !== 'date' && ['dayPeriod', 'hour', 'minute', 'second', 'fractionalSecondDigits'].some(k => o[k] !== undefined)) needDefaults = false;
    if (o.dateStyle !== undefined || o.timeStyle !== undefined) needDefaults = false;
    if (required === 'date' && o.timeStyle !== undefined) throw new TypeError('Invalid option : timeStyle');
    if (required === 'time' && o.dateStyle !== undefined) throw new TypeError('Invalid option : dateStyle');
    if (needDefaults && (defaults === 'date' || defaults === 'all')) Object.assign(o, { year: 'numeric', month: 'numeric', day: 'numeric' });
    if (needDefaults && (defaults === 'time' || defaults === 'all')) Object.assign(o, { hour: 'numeric', minute: 'numeric', second: 'numeric' });
    return o;
  }

  function formatDate(self, args, required, defaults) {
    const t = dateGetTime.call(self);
    if (Number.isNaN(t)) return 'Invalid Date';
    return new DateTimeFormat(args[0], dateTimeOptions(args[1], required, defaults)).format(t);
  }

  // Language subtag of the first canonical locale (`undefined` -> root casing).
  function casingLanguage(locales) {
    if (locales === undefined) return '';
    const first = Intl.getCanonicalLocales(locales)[0];
    return first === undefined ? '' : first.split('-')[0];
  }

  // Lithuanian: an explicit dot above on i/j/į before an above-accent (grave, acute, tilde, dot above);
  // ICU's More_Above is approximated by the combining marks U+0300-U+0314.
  function lowerLt(s) {
    return s
      .replace(/([IJĮ])(?=[̀-̔])/g, '$1̇')
      .replace(/Ì/g, 'İ̀')
      .replace(/Í/g, 'İ́')
      .replace(/Ĩ/g, 'İ̃')
      .toLowerCase();
  }

  function upperLt(s) {
    return s.replace(/([ijį])̇/g, '$1').toUpperCase();
  }

  // Turkish and Azeri: dotted/dotless i pairs; a dot above after I is dropped when lowercasing.
  function lowerTr(s) {
    return s.replace(/İ/g, 'i').replace(/İ/g, 'i').replace(/I/g, 'ı').toLowerCase();
  }

  function upperTr(s) {
    return s.replace(/i/g, 'İ').toUpperCase();
  }

  function casing(self, name, locales, lower) {
    if (self === null || self === undefined) throw new TypeError('String.prototype.' + name + ' called on null or undefined');
    const s = `${self}`;
    const lang = casingLanguage(locales);
    if (lang === 'tr' || lang === 'az') return lower ? lowerTr(s) : upperTr(s);
    if (lang === 'lt') return lower ? lowerLt(s) : upperLt(s);
    return lower ? s.toLowerCase() : s.toUpperCase();
  }

  // The default collator is shared: `localeCompare()` with no arguments is the common hot path (sorting).
  let defaultCollator;

  const numberMethods = {
    toLocaleString(...args) {
      const x = thisNumber(this);
      return new NumberFormat(args[0], args[1]).format(x);
    },
  };
  const bigintMethods = {
    toLocaleString(...args) {
      const x = bigintValueOf.call(this);
      return new NumberFormat(args[0], args[1]).format(x);
    },
  };
  const dateMethods = {
    toLocaleString(...args) { return formatDate(this, args, 'any', 'all'); },
    toLocaleDateString(...args) { return formatDate(this, args, 'date', 'date'); },
    toLocaleTimeString(...args) { return formatDate(this, args, 'time', 'time'); },
  };
  const stringMethods = {
    localeCompare(that, ...args) {
      if (this === null || this === undefined) throw new TypeError('String.prototype.localeCompare called on null or undefined');
      // ToString (a template literal), not String(): a Symbol must throw TypeError like Node
      const s = `${this}`;
      const t = `${that}`;
      if (args[0] === undefined && args[1] === undefined) {
        if (defaultCollator === undefined) defaultCollator = new Collator();
        return defaultCollator.compare(s, t);
      }
      return new Collator(args[0], args[1]).compare(s, t);
    },
    toLocaleLowerCase(...args) { return casing(this, 'toLocaleLowerCase', args[0], true); },
    toLocaleUpperCase(...args) { return casing(this, 'toLocaleUpperCase', args[0], false); },
  };
  // Same cycle guard as the prelude version in builtins.rs, forwarding the locale arguments to each element.
  const arrayLocaleStack = [];
  const arrayMethods = {
    toLocaleString(...args) {
      if (this === null || this === undefined) throw new TypeError('Array.prototype.toLocaleString called on null or undefined');
      const o = Object(this);
      if (arrayLocaleStack.indexOf(o) !== -1) return '';
      arrayLocaleStack.push(o);
      try {
        let s = '';
        const n = o.length >>> 0;
        for (let i = 0; i < n; i++) {
          if (i > 0) s += ',';
          const e = o[i];
          if (e !== undefined && e !== null) s += e.toLocaleString(...args);
        }
        return s;
      } finally {
        arrayLocaleStack.pop();
      }
    },
  };

  function install(proto, methods) {
    for (const k of Object.keys(methods)) {
      Object.defineProperty(proto, k, { value: methods[k], writable: true, enumerable: false, configurable: true });
    }
  }
  install(Number.prototype, numberMethods);
  install(BigInt.prototype, bigintMethods);
  install(Date.prototype, dateMethods);
  install(String.prototype, stringMethods);
  install(Array.prototype, arrayMethods);
})();
