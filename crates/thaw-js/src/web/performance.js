// `Performance`, `performance` and the user-timing classes (Node 22 shape). Algorithms follow the
// QuickJS port (thaw-quickjs platform_globals/performance.js); the shape is Node's: prototype accessors,
// Symbol.toStringTag, illegal constructors, private state in WeakMaps (so Object.keys(performance) is []).
// Known gaps: resource timing never produces entries (no fetch timing), timerify entries carry no detail,
// observer inspect shows no buffered records, Performance itself has no custom inspect output.
const kToken = {};
const kInspect = Symbol.for('nodejs.util.inspect.custom');
const kSupported = Object.freeze(['dns', 'function', 'gc', 'http', 'http2', 'mark', 'measure', 'net', 'resource']);
function nodeError(Ctor, code, message) {
  const e = new Ctor(message);
  e.code = code;
  return e;
}
function illegal() { return nodeError(TypeError, 'ERR_ILLEGAL_CONSTRUCTOR', 'Illegal constructor'); }
function received(value) {
  if (value === undefined || value === null) return 'Received ' + value;
  if (typeof value === 'function') return 'Received function ' + value.name;
  if (typeof value === 'object') return 'Received an instance of ' + ((value.constructor && value.constructor.name) || 'Object');
  if (typeof value === 'string') return "Received type string ('" + value + "')";
  return 'Received type ' + typeof value + ' (' + String(value) + ')';
}
function argTypeError(name, type, value) {
  return nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "' + name + '" argument must be of type ' + type + '. ' + received(value));
}
function invalidThis(type) {
  return nodeError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type ' + type);
}
function timestampError(value) {
  return nodeError(TypeError, 'ERR_PERFORMANCE_INVALID_TIMESTAMP', value + ' is not a valid timestamp');
}
function checkTimestamp(value) {
  if (typeof value === 'number' && value < 0) throw timestampError(value);
  return value;
}
function cloneDetail(detail) {
  return detail === undefined || detail === null ? null : globalThis.structuredClone(detail);
}

const entryData = new WeakMap();    // entry -> { name, entryType, startTime, duration, detail }
const observerData = new WeakMap(); // observer -> { callback, types, mode, buffer, connected }
const resultData = new WeakMap();   // PerformanceObserverEntryList -> entries
const timeline = [];                // performance timeline, in creation order
const observers = new Set();        // observers with a registration, in observe() order
const timeOrigin = Date.now() - __thaw_perf_now();
let flushScheduled = false;
let performanceObject;

function entryOf(value, type) {
  const data = entryData.get(value);
  if (data === undefined) throw invalidThis(type);
  return data;
}
function makeEntry(proto, data) {
  const entry = Object.create(proto);
  entryData.set(entry, data);
  return entry;
}
function sortByStart(entries) {
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => entryData.get(a.entry).startTime - entryData.get(b.entry).startTime || a.index - b.index)
    .map(item => item.entry);
}
function filterEntries(entries, name, type) {
  return sortByStart(entries).filter(entry => {
    const data = entryData.get(entry);
    return (name === undefined || data.name === name) && (type === undefined || data.entryType === type);
  });
}
function latestMark(name) {
  for (let index = timeline.length - 1; index >= 0; index--) {
    const data = entryData.get(timeline[index]);
    if (data.entryType === 'mark' && data.name === name) return data.startTime;
  }
  return undefined;
}
function resolveTime(value) {
  if (typeof value === 'number') return checkTimestamp(value);
  const name = String(value);
  const time = latestMark(name);
  if (time === undefined) {
    throw new DOMException('The "' + name + '" performance mark has not been set', 'SyntaxError');
  }
  return time;
}
function fmtValue(value) {
  if (typeof value === 'string') return "'" + value + "'";
  if (value !== null && typeof value === 'object') {
    const parts = Object.keys(value).map(key => key + ': ' + fmtValue(value[key]));
    return Array.isArray(value) ? '[ ' + value.map(fmtValue).join(', ') + ' ]' : '{ ' + parts.join(', ') + ' }';
  }
  return String(value);
}
function inspectEntry(entry, type) {
  const data = entryData.get(entry);
  const fields = ['name', 'entryType', 'startTime', 'duration', 'detail'];
  return type + ' {\n' + fields.map(key => '  ' + key + ': ' + fmtValue(data[key])).join(',\n') + '\n}';
}

function scheduleFlush() {
  if (flushScheduled) return;
  flushScheduled = true;
  setImmediate(flush);
}
function flush() {
  flushScheduled = false;
  for (const observer of [...observers]) {
    const data = observerData.get(observer);
    if (data.buffer.length === 0) continue;
    const records = data.buffer;
    data.buffer = [];
    data.callback(makeList(records), observer);
  }
}
function makeList(entries) {
  const list = Object.create(PerformanceObserverEntryList.prototype);
  resultData.set(list, entries);
  return list;
}
function record(entry) {
  timeline.push(entry);
  const type = entryData.get(entry).entryType;
  for (const observer of observers) {
    const data = observerData.get(observer);
    if (data.types.has(type)) {
      data.buffer.push(entry);
      scheduleFlush();
    }
  }
  return entry;
}

class PerformanceEntry {
  constructor(...args) {
    if (args[0] !== kToken) throw illegal();
  }
  get name() { return entryOf(this, 'PerformanceEntry').name; }
  get entryType() { return entryOf(this, 'PerformanceEntry').entryType; }
  get startTime() { return entryOf(this, 'PerformanceEntry').startTime; }
  get duration() { return entryOf(this, 'PerformanceEntry').duration; }
  toJSON() {
    const data = entryOf(this, 'PerformanceEntry');
    return { name: data.name, entryType: data.entryType, startTime: data.startTime, duration: data.duration, detail: data.detail };
  }
  [kInspect]() { return inspectEntry(this, this.constructor.name); }
}

class PerformanceMark extends PerformanceEntry {
  constructor(name, ...rest) {
    super(kToken);
    if (name === undefined) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "name" argument must be specified');
    let options = rest[0];
    if (options === undefined || options === null) options = {};
    else if (typeof options !== 'object') throw argTypeError('options', 'object', options);
    let startTime = options.startTime;
    if (startTime === undefined) startTime = __thaw_perf_now();
    else if (typeof startTime !== 'number') throw argTypeError('startTime', 'number', startTime);
    else checkTimestamp(startTime);
    entryData.set(this, { name: String(name), entryType: 'mark', startTime, duration: 0, detail: cloneDetail(options.detail) });
  }
  get detail() { return entryOf(this, 'PerformanceMark').detail; }
}

class PerformanceMeasure extends PerformanceEntry {
  constructor() { throw illegal(); }
  get detail() { return entryOf(this, 'PerformanceMeasure').detail; }
}

class PerformanceResourceTiming extends PerformanceEntry {
  constructor() { throw illegal(); }
}
for (const name of ['workerStart', 'redirectStart', 'redirectEnd', 'fetchStart', 'domainLookupStart', 'domainLookupEnd',
  'connectStart', 'connectEnd', 'secureConnectionStart', 'requestStart', 'responseStart', 'responseEnd',
  'encodedBodySize', 'decodedBodySize', 'transferSize']) {
  Object.defineProperty(PerformanceResourceTiming.prototype, name, {
    get() { entryOf(this, 'PerformanceResourceTiming'); return 0; }, enumerable: true, configurable: true,
  });
}
for (const name of ['initiatorType', 'nextHopProtocol', 'deliveryType']) {
  Object.defineProperty(PerformanceResourceTiming.prototype, name, {
    get() { entryOf(this, 'PerformanceResourceTiming'); return ''; }, enumerable: true, configurable: true,
  });
}
Object.defineProperty(PerformanceResourceTiming.prototype, 'responseStatus', {
  get() { entryOf(this, 'PerformanceResourceTiming'); return 0; }, enumerable: true, configurable: true,
});

class PerformanceObserverEntryList {
  constructor() { throw illegal(); }
  getEntries() { return sortByStart(resultOf(this)); }
  getEntriesByType(type) {
    if (type === undefined) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "type" argument must be specified');
    return filterEntries(resultOf(this), undefined, String(type));
  }
  getEntriesByName(name, type) {
    if (name === undefined) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "name" argument must be specified');
    return filterEntries(resultOf(this), String(name), type === undefined ? undefined : String(type));
  }
}
function resultOf(list) {
  const entries = resultData.get(list);
  if (entries === undefined) throw invalidThis('PerformanceObserverEntryList');
  return entries;
}

class PerformanceObserver {
  constructor(callback) {
    if (typeof callback !== 'function') throw argTypeError('callback', 'function', callback);
    observerData.set(this, { callback, types: new Set(), mode: undefined, buffer: [], connected: false });
  }
  observe(options) {
    const data = observerOf(this);
    const opts = options === undefined || options === null ? {} : options;
    const hasType = opts.type !== undefined;
    const hasEntryTypes = opts.entryTypes !== undefined;
    if (!hasType && !hasEntryTypes) {
      throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "options.entryTypes" and "options.type" arguments must be specified');
    }
    if (hasType && hasEntryTypes) {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_VALUE', "The property 'options.entryTypes' options.entryTypes can not set with options.type together. Received " + fmtValue(opts.entryTypes));
    }
    const mode = hasType ? 'type' : 'entryTypes';
    let names;
    if (hasType) {
      names = [String(opts.type)];
    } else {
      if (!Array.isArray(opts.entryTypes)) {
        throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options.entryTypes" property must be string[]. ' + received(opts.entryTypes));
      }
      names = opts.entryTypes.map(String);
    }
    names = names.filter(name => kSupported.includes(name));
    if (names.length === 0) return;
    if (data.mode !== undefined && data.mode !== mode) {
      throw new DOMException('PerformanceObserver can not change to ' + (mode === 'type' ? 'single observation' : 'multiple observations'), 'InvalidModificationError');
    }
    data.mode = mode;
    data.connected = true;
    for (const name of names) data.types.add(name);
    observers.add(this);
    if (hasType && opts.buffered) {
      for (const entry of timeline) if (entryData.get(entry).entryType === names[0]) data.buffer.push(entry);
      if (data.buffer.length > 0) scheduleFlush();
    }
  }
  disconnect() {
    const data = observerOf(this);
    observers.delete(this);
    data.types.clear();
    data.buffer = [];
    data.mode = undefined;
    data.connected = false;
  }
  takeRecords() {
    const data = observerOf(this);
    const records = data.buffer;
    data.buffer = [];
    return records;
  }
  [kInspect]() {
    const data = observerOf(this);
    const types = [...data.types];
    const list = types.length === 0 ? '[]' : '[ ' + types.map(type => "'" + type + "'").join(', ') + ' ]';
    const fields = ['connected: ' + data.connected, 'pending: ' + (data.buffer.length > 0), 'entryTypes: ' + list, 'buffer: []'];
    const line = fields.join(', ');
    return line.length <= 72 ? 'PerformanceObserver { ' + line + ' }' : 'PerformanceObserver {\n  ' + fields.join(',\n  ') + '\n}';
  }
  static get supportedEntryTypes() { return kSupported; }
}
function observerOf(observer) {
  const data = observerData.get(observer);
  if (data === undefined) throw invalidThis('PerformanceObserver');
  return data;
}

class Performance extends EventTarget {
  constructor(...args) {
    if (args[0] !== kToken) throw illegal();
    super();
    // EventTarget keeps its listener map as an own property; Node has no own keys here
    Object.defineProperty(this, '__thawListeners', { enumerable: false });
  }
  get timeOrigin() { requirePerformance(this); return timeOrigin; }
  now() { requirePerformance(this); return __thaw_perf_now(); }
  mark(name, options) {
    requirePerformance(this);
    const entry = new PerformanceMark(name, options);
    return record(makeEntry(PerformanceMark.prototype, entryData.get(entry)));
  }
  measure(name, startOrOptions, endMark) {
    requirePerformance(this);
    if (typeof name !== 'string') throw argTypeError('name', 'string', name);
    let startValue;
    let endValue = endMark;
    let durationValue;
    let detail = null;
    if (startOrOptions !== null && typeof startOrOptions === 'object') {
      const options = startOrOptions;
      const populated = options.detail !== undefined || options.start !== undefined
        || options.end !== undefined || options.duration !== undefined;
      if (populated && endMark !== undefined) {
        throw nodeError(TypeError, 'ERR_PERFORMANCE_MEASURE_INVALID_OPTIONS', 'endMark must not be specified');
      }
      if (options.start !== undefined && options.end !== undefined && options.duration !== undefined) {
        throw nodeError(TypeError, 'ERR_PERFORMANCE_MEASURE_INVALID_OPTIONS', 'Must not have options.start, options.end, and options.duration specified');
      }
      startValue = options.start;
      durationValue = options.duration;
      endValue = options.end !== undefined ? options.end : endMark;
      detail = cloneDetail(options.detail);
    } else if (typeof startOrOptions === 'string') {
      startValue = startOrOptions;
    }
    const start = startValue === undefined ? undefined : resolveTime(startValue);
    const end = endValue === undefined ? undefined : resolveTime(endValue);
    const duration = durationValue === undefined ? undefined : checkTimestamp(durationValue);
    let startTime;
    let endTime;
    if (end !== undefined) {
      startTime = start !== undefined ? start : duration !== undefined ? end - duration : 0;
      endTime = end;
    } else if (start !== undefined) {
      startTime = start;
      endTime = duration !== undefined ? start + duration : __thaw_perf_now();
    } else {
      startTime = 0;
      endTime = __thaw_perf_now();
    }
    return record(makeEntry(PerformanceMeasure.prototype, { name, entryType: 'measure', startTime, duration: endTime - startTime, detail }));
  }
  clearMarks(name) { requirePerformance(this); clearEntries('mark', name); }
  clearMeasures(name) { requirePerformance(this); clearEntries('measure', name); }
  clearResourceTimings() { requirePerformance(this); }
  setResourceTimingBufferSize() { requirePerformance(this); }
  markResourceTiming() { requirePerformance(this); }
  getEntries() { requirePerformance(this); return sortByStart(timeline); }
  getEntriesByType(type) {
    requirePerformance(this);
    if (type === undefined) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "type" argument must be specified');
    return filterEntries(timeline, undefined, String(type));
  }
  getEntriesByName(name, type) {
    requirePerformance(this);
    if (name === undefined) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "name" argument must be specified');
    return filterEntries(timeline, String(name), type === undefined ? undefined : String(type));
  }
  timerify(fn) {
    requirePerformance(this);
    if (typeof fn !== 'function') throw argTypeError('fn', 'function', fn);
    const label = fn.name;
    const timed = (start) => record(makeEntry(PerformanceEntry.prototype, { name: label, entryType: 'function', startTime: start, duration: __thaw_perf_now() - start, detail: null }));
    return function timerified(...args) {
      const start = __thaw_perf_now();
      let result;
      try {
        result = Reflect.apply(fn, this, args);
      } catch (error) {
        timed(start);
        throw error;
      }
      if (result && typeof result.then === 'function') {
        return Promise.resolve(result).finally(() => timed(start));
      }
      timed(start);
      return result;
    };
  }
  eventLoopUtilization(util1, util2) {
    requirePerformance(this);
    const utilization = (idle, active) => (idle + active === 0 ? 0 : active / (idle + active));
    const stats = __thaw_loop_stats();
    let active = stats[0];
    let idle = stats[1];
    if (util2) {
      idle = util1.idle - util2.idle;
      active = util1.active - util2.active;
      return { idle, active, utilization: utilization(idle, active) };
    }
    if (util1) {
      idle -= util1.idle;
      active -= util1.active;
    }
    return { idle, active, utilization: utilization(idle, active) };
  }
  get nodeTiming() {
    requirePerformance(this);
    return nodeTimingObject;
  }
  toJSON() {
    requirePerformance(this);
    return { nodeTiming: nodeTimingObject, timeOrigin, eventLoopUtilization: this.eventLoopUtilization() };
  }
}
Object.defineProperty(Performance.prototype, 'onresourcetimingbufferfull', { value: null, writable: true, enumerable: true, configurable: true });
function requirePerformance(value) {
  if (value !== performanceObject) throw invalidThis('Performance');
}
function clearEntries(type, name) {
  for (let index = timeline.length - 1; index >= 0; index--) {
    const data = entryData.get(timeline[index]);
    if (data.entryType === type && (name === undefined || data.name === String(name))) timeline.splice(index, 1);
  }
}
const nodeTimingObject = {
  name: 'node', entryType: 'node', startTime: 0, duration: 0, nodeStart: 0, v8Start: 0,
  bootstrapComplete: 0, environment: 0, loopStart: 0, loopExit: -1, idleTime: 0,
};
Object.defineProperty(nodeTimingObject, 'toJSON', { value() { return { ...this }; }, enumerable: false, configurable: true, writable: true });

for (const [name, value] of [
  ['Performance', Performance], ['PerformanceEntry', PerformanceEntry], ['PerformanceMark', PerformanceMark],
  ['PerformanceMeasure', PerformanceMeasure], ['PerformanceResourceTiming', PerformanceResourceTiming],
  ['PerformanceObserver', PerformanceObserver], ['PerformanceObserverEntryList', PerformanceObserverEntryList],
]) {
  Object.defineProperty(globalThis, name, { value, writable: true, enumerable: false, configurable: true });
}
for (const [proto, tag] of [
  [Performance.prototype, 'Performance'], [PerformanceMark.prototype, 'PerformanceMark'],
  [PerformanceMeasure.prototype, 'PerformanceMeasure'], [PerformanceResourceTiming.prototype, 'PerformanceResourceTiming'],
  [PerformanceObserver.prototype, 'PerformanceObserver'], [PerformanceObserverEntryList.prototype, 'PerformanceObserverEntryList'],
]) {
  Object.defineProperty(proto, Symbol.toStringTag, { value: tag, configurable: true });
}
performanceObject = new Performance(kToken);
Object.defineProperty(globalThis, 'performance', { value: performanceObject, writable: true, enumerable: true, configurable: true });
