// node:test `mock.timers` host side, Node 22 MockTimers semantics. The shared node:test wrapper (thaw-registry
// builtins/testing.rs) checks the state and the arguments, then calls the `__thaw_test_mock_timers_*` globals below.
// Virtual timers live in their own queue, never in the native loop. While a clock mocks an API the global is a
// scheduler that reads the clock; when no clock mocks it the native function is put back.
const realSetTimeout = globalThis.setTimeout;
const realClearTimeout = globalThis.clearTimeout;
const realSetInterval = globalThis.setInterval;
const realClearInterval = globalThis.clearInterval;
const realSetImmediate = globalThis.setImmediate;
const realClearImmediate = globalThis.clearImmediate;
const realDate = globalThis.Date;
const nativeDateClock = globalThis.__thaw_test_mock_clock;
const TIMEOUT_MAX = 2147483647;
const SUPPORTED = ['setTimeout', 'setInterval', 'setImmediate', 'Date'];
const SCHEDULERS = ['setTimeout', 'setInterval', 'setImmediate'];
const CLEARERS = { setTimeout: 'clearTimeout', setInterval: 'clearInterval', setImmediate: 'clearImmediate' };
const NATIVE = { setTimeout: realSetTimeout, setInterval: realSetInterval, setImmediate: realSetImmediate, clearTimeout: realClearTimeout, clearInterval: realClearInterval, clearImmediate: realClearImmediate };

const clocks = [];
const queue = new Map();
const records = new WeakMap();
let nextId = 1;

function nodeError(Ctor, code, message) {
  const error = new Ctor(message);
  error.code = code;
  return error;
}
function received(value) {
  if (value === undefined || value === null) return 'Received ' + value;
  if (typeof value === 'function') return 'Received function ' + value.name;
  if (typeof value === 'object') return 'Received an instance of ' + ((value.constructor && value.constructor.name) || 'Object');
  if (typeof value === 'string') return "Received type string ('" + value + "')";
  return 'Received type ' + typeof value + ' (' + String(value) + ')';
}
function notEnabled() {
  return nodeError(Error, 'ERR_INVALID_STATE', 'Invalid state: You should enable MockTimers first by calling the .enable function');
}
function negativeTime(value) {
  return nodeError(TypeError, 'ERR_INVALID_ARG_VALUE', "The argument 'time' " + value + ". Received 'positive integer'");
}
function requireClock(clock) {
  if (!clock || !clock.enabled) throw notEnabled();
}
function activeClock(api) {
  for (let index = clocks.length - 1; index >= 0; index--) {
    if (clocks[index].apis.has(api)) return clocks[index];
  }
  return null;
}

// Node's mock handles, for timers and immediates alike, are Timeout objects with their own prototype (no native
// toPrimitive, so the shape is Node's).
function Timeout() {}
const handleMethods = {
  ref() { return this; },
  unref() { return this; },
  // Node 22 keeps reporting a ref under mock timers, whatever unref() did.
  hasRef() { return true; },
};
Object.assign(Timeout.prototype, handleMethods, {
  refresh() {
    const record = records.get(this);
    if (record && !record.cancelled && record.clock.enabled) {
      record.due = record.clock.now + record.ms;
      queue.set(record.id, record);
    }
    return this;
  },
});

// Node clamps a delay outside [1, TIMEOUT_MAX] (and NaN) to 1 ms.
function normalizeDelay(delay) {
  const after = Number(delay);
  return after >= 1 && after <= TIMEOUT_MAX ? after : 1;
}
function schedule(api, callback, delay, args, repeat) {
  if (typeof callback !== 'function') {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "callback" argument must be of type function. ' + received(callback));
  }
  const clock = activeClock(api);
  const ms = api === 'setImmediate' ? 0 : normalizeDelay(delay);
  const record = { id: nextId++, api, clock, callback, args, repeat, ms, due: clock.now + ms, cancelled: false };
  queue.set(record.id, record);
  const handle = Object.create(Timeout.prototype);
  records.set(handle, record);
  return handle;
}
function mockClear(real) {
  return function (value) {
    const record = typeof value === 'object' && value !== null ? records.get(value) : queue.get(Number(value));
    if (record) {
      record.cancelled = true;
      queue.delete(record.id);
      return;
    }
    real(value);
  };
}

// Date(...) with no arguments reads the clock; `new Date(...)` builds a plain Date (so a subclass gets a Date, as in
// Node). The statics (UTC, parse) are inherited from the native Date.
function MockDate(...args) {
  const clock = activeClock('Date');
  const now = clock ? clock.now : realDate.now();
  if (new.target === undefined) return new realDate(now).toString();
  return args.length ? new realDate(...args) : new realDate(now);
}
MockDate.prototype = realDate.prototype;
Object.setPrototypeOf(MockDate, realDate);
MockDate.now = () => {
  const clock = activeClock('Date');
  return clock ? clock.now : realDate.now();
};

const scheduled = {
  setTimeout: (callback, delay, ...args) => schedule('setTimeout', callback, delay, args, false),
  setInterval: (callback, delay, ...args) => schedule('setInterval', callback, delay, args, true),
  setImmediate: (callback, ...args) => schedule('setImmediate', callback, 0, args, false),
};
// The compiled lane reads the clock from the runtime's mock cell, so it follows the mocked Date.
function syncNativeDateClock() {
  const clock = activeClock('Date');
  nativeDateClock(clock === null ? undefined : clock.now);
}
function refreshGlobals() {
  for (const api of SCHEDULERS) {
    const mocked = activeClock(api) !== null;
    globalThis[api] = mocked ? scheduled[api] : NATIVE[api];
    const clear = CLEARERS[api];
    globalThis[clear] = mocked ? mockClear(NATIVE[clear]) : NATIVE[clear];
  }
  globalThis.Date = activeClock('Date') !== null ? MockDate : realDate;
  syncNativeDateClock();
}

// Runs every virtual timer due by `through` in (due, creation) order, with the clock held at `through` (Node
// reports the tick's end time inside callbacks). An interval re-arms by its period, so one tick can fire it twice.
function fireDue(clock, through) {
  for (;;) {
    let next = null;
    for (const record of queue.values()) {
      if (record.clock !== clock || record.due > through) continue;
      if (next === null || record.due < next.due || (record.due === next.due && record.id < next.id)) next = record;
    }
    if (next === null) return;
    if (next.repeat) next.due += next.ms;
    else queue.delete(next.id);
    next.callback(...next.args);
  }
}
function advance(clock, amount) {
  clock.now += amount;
  syncNativeDateClock();
  fireDue(clock, clock.now);
}

globalThis.__thaw_test_mock_timers_enable = options => {
  if (typeof options !== 'object' || options === null) options = {};
  const apis = options.apis === undefined ? SUPPORTED : options.apis;
  if (!Array.isArray(apis)) {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options.apis" property must be an instance of Array. ' + received(apis));
  }
  for (const api of apis) {
    if (!SUPPORTED.includes(api)) {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_VALUE', "The property 'options.apis' option " + api + " is not supported. Received '" + api + "'");
    }
  }
  const initial = options.now === undefined ? 0 : options.now;
  let now = initial;
  if (initial instanceof realDate) now = initial.getTime();
  else if (typeof initial !== 'number') {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "initialTime" argument must be of type number. ' + received(initial));
  }
  if (Number.isNaN(now)) {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_VALUE', "The argument 'now' epoch must be a positive integer received " + now + '. Received ' + now);
  }
  if (now < 0) throw negativeTime(now);
  const clock = { enabled: true, now, apis: new Set(apis) };
  clocks.push(clock);
  refreshGlobals();
  return clock;
};
globalThis.__thaw_test_mock_timers_tick = (clock, milliseconds) => {
  requireClock(clock);
  const amount = milliseconds === undefined ? 1 : Number(milliseconds);
  if (amount < 0) throw negativeTime(amount);
  // NaN passes no time; a non-finite amount would never settle an interval, so it is ignored too.
  if (Number.isFinite(amount)) advance(clock, amount);
};
globalThis.__thaw_test_mock_timers_run_all = clock => {
  requireClock(clock);
  let last = -Infinity;
  for (const record of queue.values()) {
    if (record.clock === clock && record.due > last) last = record.due;
  }
  if (last !== -Infinity) advance(clock, Math.max(0, last - clock.now));
};
globalThis.__thaw_test_mock_timers_set_time = (clock, milliseconds) => {
  requireClock(clock);
  if (milliseconds !== undefined && typeof milliseconds !== 'number') {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "time" argument must be of type number. ' + received(milliseconds));
  }
  const now = milliseconds === undefined ? 0 : milliseconds;
  if (now < 0) throw negativeTime(now);
  clock.now = now;
  syncNativeDateClock();
};
globalThis.__thaw_test_mock_timers_reset = clock => {
  if (!clock || !clock.enabled) return;
  clock.enabled = false;
  for (const [id, record] of queue) {
    if (record.clock === clock) {
      record.cancelled = true;
      queue.delete(id);
    }
  }
  clocks.splice(clocks.indexOf(clock), 1);
  refreshGlobals();
};
