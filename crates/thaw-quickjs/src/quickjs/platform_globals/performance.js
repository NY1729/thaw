  const base64Alphabet =
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = input => {
    const text = String(input);
    let output = '';
    for (let offset = 0; offset < text.length; offset += 3) {
      const first = text.charCodeAt(offset);
      const second = offset + 1 < text.length ? text.charCodeAt(offset + 1) : 0;
      const third = offset + 2 < text.length ? text.charCodeAt(offset + 2) : 0;
      if (first > 255 || second > 255 || third > 255) {
        throw new DOMException('btoa input must contain only Latin-1 characters',
                               'InvalidCharacterError');
      }
      const bits = first << 16 | second << 8 | third;
      output += base64Alphabet[bits >> 18 & 63];
      output += base64Alphabet[bits >> 12 & 63];
      output += offset + 1 < text.length ? base64Alphabet[bits >> 6 & 63] : '=';
      output += offset + 2 < text.length ? base64Alphabet[bits & 63] : '=';
    }
    return output;
  };
  globalThis.atob = input => {
    const encoded = String(input).replace(/[\t\n\f\r ]/g, '');
    if (encoded.length % 4 === 1 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) {
      throw new DOMException('invalid base64 input', 'InvalidCharacterError');
    }
    const unpadded = encoded.replace(/=+$/, '');
    if (encoded.includes('=') && encoded.length % 4 !== 0) {
      throw new DOMException('invalid base64 padding', 'InvalidCharacterError');
    }
    let output = '';
    let bits = 0;
    let bitCount = 0;
    for (const character of unpadded) {
      bits = bits << 6 | base64Alphabet.indexOf(character);
      bitCount += 6;
      if (bitCount >= 8) {
        bitCount -= 8;
        output += String.fromCharCode(bits >> bitCount & 255);
      }
    }
    return output;
  };
  const performanceTimeOrigin = Date.now();
  let lastPerformanceNow = 0;
  const performanceNow = () => {
    lastPerformanceNow = Math.max(lastPerformanceNow,
                                  Date.now() - performanceTimeOrigin);
    return lastPerformanceNow;
  };
  if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = { timeOrigin: performanceTimeOrigin,
                               now: performanceNow };
  } else {
    if (typeof globalThis.performance.now !== 'function') {
      globalThis.performance.now = performanceNow;
    }
    if (typeof globalThis.performance.timeOrigin !== 'number') {
      globalThis.performance.timeOrigin = performanceTimeOrigin;
    }
  }
  const performanceEntries = [];
  const performanceObservers = new Set();
  class PerformanceEntry {
    constructor(name, entryType, startTime, duration = 0, detail = null) {
      this.name = String(name); this.entryType = String(entryType);
      this.startTime = Number(startTime); this.duration = Number(duration); this.detail = detail;
    }
    toJSON() { return { name: this.name, entryType: this.entryType, startTime: this.startTime, duration: this.duration, detail: this.detail }; }
  }
  class PerformanceMark extends PerformanceEntry {
    constructor(name, options = {}) { super(name, 'mark', options.startTime === undefined ? performanceNow() : options.startTime, 0, options.detail); }
  }
  class PerformanceMeasure extends PerformanceEntry {
    constructor(name, startTime, duration, detail) { super(name, 'measure', startTime, duration, detail); }
  }
  class PerformanceObserverEntryList {
    constructor(entries) { this._entries = entries; }
    getEntries() { return this._entries.slice().sort((left, right) => left.startTime - right.startTime); }
    getEntriesByType(type) { return this.getEntries().filter(entry => entry.entryType === String(type)); }
    getEntriesByName(name, type) { return this.getEntries().filter(entry => entry.name === String(name) && (type === undefined || entry.entryType === String(type))); }
  }
  const queuePerformanceEntry = entry => {
    performanceEntries.push(entry);
    for (const observer of performanceObservers) {
      if (!observer._types.has(entry.entryType)) continue;
      observer._records.push(entry);
      if (!observer._queued) {
        observer._queued = true;
        queueMicrotask(() => {
          observer._queued = false;
          const records = observer.takeRecords();
          if (records.length) observer._callback(new PerformanceObserverEntryList(records), observer);
        });
      }
    }
    return entry;
  };
  class PerformanceObserver {
    constructor(callback) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); this._callback = callback; this._types = new Set(); this._records = []; this._queued = false; }
    observe(options = {}) {
      if (options.type) this._types.add(String(options.type));
      if (options.entryTypes) for (const type of options.entryTypes) this._types.add(String(type));
      performanceObservers.add(this);
      if (options.buffered && options.type) this._records.push(...performanceEntries.filter(entry => entry.entryType === String(options.type)));
      if (this._records.length && !this._queued) {
        this._queued = true;
        queueMicrotask(() => { this._queued = false; const records = this.takeRecords(); if (records.length) this._callback(new PerformanceObserverEntryList(records), this); });
      }
    }
    disconnect() { performanceObservers.delete(this); this._records = []; this._types.clear(); }
    takeRecords() { const records = this._records; this._records = []; return records; }
  }
  PerformanceObserver.supportedEntryTypes = ['function', 'mark', 'measure'];
  const entryTime = (value, fallback) => {
    if (value === undefined) return fallback;
    if (typeof value === 'number') return value;
    const entry = performanceEntries.slice().reverse().find(candidate => candidate.name === String(value));
    if (!entry) throw new SyntaxError(`Unknown performance mark: ${value}`);
    return entry.startTime;
  };
  Object.assign(globalThis.performance, {
    mark(name, options = {}) { return queuePerformanceEntry(new PerformanceMark(name, options)); },
    measure(name, startOrOptions, endMark) {
      let start, end, detail;
      if (startOrOptions && typeof startOrOptions === 'object') {
        start = entryTime(startOrOptions.start, 0); detail = startOrOptions.detail;
        end = startOrOptions.duration === undefined ? entryTime(startOrOptions.end, performanceNow()) : start + Number(startOrOptions.duration);
      } else { start = entryTime(startOrOptions, 0); end = entryTime(endMark, performanceNow()); }
      return queuePerformanceEntry(new PerformanceMeasure(name, start, end - start, detail));
    },
    clearMarks(name) { for (let index = performanceEntries.length - 1; index >= 0; index--) if (performanceEntries[index].entryType === 'mark' && (name === undefined || performanceEntries[index].name === String(name))) performanceEntries.splice(index, 1); },
    clearMeasures(name) { for (let index = performanceEntries.length - 1; index >= 0; index--) if (performanceEntries[index].entryType === 'measure' && (name === undefined || performanceEntries[index].name === String(name))) performanceEntries.splice(index, 1); },
    getEntries() { return new PerformanceObserverEntryList(performanceEntries).getEntries(); },
    getEntriesByType(type) { return this.getEntries().filter(entry => entry.entryType === String(type)); },
    getEntriesByName(name, type) { return this.getEntries().filter(entry => entry.name === String(name) && (type === undefined || entry.entryType === String(type))); },
    timerify(callback) {
      return function(...args) { const start = performanceNow(); try { const result = callback.apply(this, args); if (result && typeof result.then === 'function') return Promise.resolve(result).finally(() => queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start))); queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start)); return result; } catch (error) { queuePerformanceEntry(new PerformanceEntry(callback.name || 'anonymous', 'function', start, performanceNow() - start)); throw error; } };
    },
    toJSON() { return { timeOrigin: this.timeOrigin }; }
  });
  globalThis.PerformanceEntry = PerformanceEntry;
  globalThis.PerformanceMark = PerformanceMark;
  globalThis.PerformanceMeasure = PerformanceMeasure;
  globalThis.PerformanceObserver = PerformanceObserver;
  globalThis.PerformanceObserverEntryList = PerformanceObserverEntryList;
  if (typeof globalThis.structuredClone !== 'function') {
    globalThis.structuredClone = (value, options = {}) => {
      const seen = new Map();
      const transfer = new Set();
      for (const item of options.transfer || []) {
        if (transfer.has(item)) {
          throw new DOMException('transfer list contains duplicate values',
                                 'DataCloneError');
        }
        if (!(item instanceof ArrayBuffer)
            && !(typeof globalThis.MessagePort === 'function'
                 && item instanceof globalThis.MessagePort)) {
          throw new DOMException('value is not transferable', 'DataCloneError');
        }
        transfer.add(item);
      }
      const clone = input => {
        if (input === null || typeof input !== 'object') {
          if (typeof input === 'function' || typeof input === 'symbol') {
            throw new DOMException('value cannot be structured-cloned',
                                   'DataCloneError');
          }
          return input;
        }
        if (seen.has(input)) return seen.get(input);
        if (typeof globalThis.MessagePort === 'function'
            && input instanceof globalThis.MessagePort) {
          if (!transfer.has(input)) {
            throw new DOMException('MessagePort requires a transfer list',
                                   'DataCloneError');
          }
          const output = input.__thawTransfer();
          seen.set(input, output);
          return output;
        }
        let output;
        if (input instanceof Date) {
          output = new Date(input.getTime());
        } else if (input instanceof RegExp) {
          output = new RegExp(input.source, input.flags);
          output.lastIndex = input.lastIndex;
        } else if (input instanceof Map) {
          output = new Map();
          seen.set(input, output);
          for (const [key, entry] of input) output.set(clone(key), clone(entry));
          return output;
        } else if (input instanceof Set) {
          output = new Set();
          seen.set(input, output);
          for (const entry of input) output.add(clone(entry));
          return output;
        } else if (input instanceof ArrayBuffer) {
          output = input.slice(0);
        } else if (ArrayBuffer.isView(input)) {
          if (input instanceof DataView) {
            output = new DataView(input.buffer.slice(input.byteOffset,
              input.byteOffset + input.byteLength));
          } else {
            output = new input.constructor(input);
          }
        } else {
          output = Array.isArray(input) ? [] : {};
        }
        seen.set(input, output);
        if (!(input instanceof Date) && !(input instanceof RegExp)
            && !(input instanceof ArrayBuffer) && !ArrayBuffer.isView(input)) {
          for (const key of Object.keys(input)) output[key] = clone(input[key]);
        }
        return output;
      };
      const output = clone(value);
      for (const item of transfer) {
        if (item instanceof ArrayBuffer) __thaw_detach_array_buffer(item);
      }
      return output;
    };
  }
