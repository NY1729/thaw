macro_rules! assert_source {
    ($tail:literal) => {
        concat!(include_str!("assert.js"), $tail)
    };
}

pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "domain" => Some(
            r#"var EventEmitter = require('node:events'), stack = [];
             function Domain() { if (!(this instanceof Domain)) return new Domain(); Object.defineProperty(this, 'domain', { value: null, writable: true, configurable: true, enumerable: false }); EventEmitter.call(this); this.members = []; this.on('removeListener', function() {}); this.on('newListener', function() {}); }
             Domain.prototype = Object.create(EventEmitter.prototype); Domain.prototype.constructor = Domain;
             Domain.prototype.enter = function() { stack.push(module.exports.active); module.exports.active = this; if (globalThis.process) process.domain = this; return this; };
             Domain.prototype.exit = function() { if (module.exports.active !== this) return this; module.exports.active = stack.length ? stack.pop() : null; if (globalThis.process) process.domain = module.exports.active || null; return this; };
             Domain.prototype.add = function(emitter) { if (!emitter || (typeof emitter !== 'object' && typeof emitter !== 'function')) throw new TypeError('emitter must be an object'); if (emitter.domain && emitter.domain !== this && emitter.domain.remove) emitter.domain.remove(emitter); if (this.members.indexOf(emitter) < 0) this.members.push(emitter); emitter.domain = this; return this; };
             Domain.prototype.remove = function(emitter) { var index = this.members.indexOf(emitter); if (index >= 0) this.members.splice(index, 1); if (emitter && emitter.domain === this) emitter.domain = null; return this; };
             Domain.prototype._handle = function(error) { if (!(error instanceof Error)) error = new Error(String(error)); error.domain = this; error.domainThrown = true; if (this.listenerCount('error')) { this.emit('error', error); return; } throw error; };
             Domain.prototype.run = function(fn) { var args = Array.prototype.slice.call(arguments, 1); this.enter(); try { return fn.apply(undefined, args); } catch (error) { return this._handle(error); } finally { this.exit(); } };
             Domain.prototype.bind = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function bound() { var args = arguments, receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } bound.domain = domain; return bound; };
             Domain.prototype.intercept = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function intercepted(error) { if (error) return domain._handle(error); var args = Array.prototype.slice.call(arguments, 1), receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } intercepted.domain = domain; return intercepted; };
             Domain.prototype.dispose = function() { this.exit(); this.members.slice().forEach(this.remove, this); this.removeAllListeners(); return this; };
             function create() { return new Domain(); }
             module.exports = { _stack: stack, Domain: Domain, createDomain: create, create: create, active: null }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        // Every prototype method below tolerates `this._events` being
        // `undefined` -- not just the `EventEmitter()` constructor path,
        // which sets it up front. Real Node's own `EventEmitter.prototype`
        // does the same (`emit` treats a missing `_events` as "no
        // listeners", never indexes into it directly), because a real,
        // common pattern *mixes in* the prototype onto a plain object via
        // `Object.setPrototypeOf(obj, EventEmitter.prototype)` and never
        // calls the constructor at all -- real example: pino's logger
        // (`Object.setPrototypeOf(prototype, EventEmitter.prototype)` in
        // its own `lib/proto.js`, no `EventEmitter.call(this)` anywhere).
        // Before this, every read here assumed `this._events` already
        // existed, so `logger.emit('level-change', ...)` (pino sets its
        // level during construction) crashed with `cannot read property
        // 'level-change' of undefined` on a logger that had never called
        // `.on()` first (the one path, `addEventListener`, that *did*
        // already lazily create `_events`).
        "events" => Some(concat!(
            "var errorMonitor = Symbol.for('events.errorMonitor');\n",
            include_str!("events_head.js"),
            "// ponytail: tiny inspect for the ERR_UNHANDLED_ERROR message only (strings, plain arrays/objects); a static require of node:util would make the bundler pull it into every events consumer.
             function inspectError(value, depth) { if (typeof value === 'string') return \"'\" + value + \"'\"; if (!value || typeof value !== 'object' || depth > 2) return String(value); if (Array.isArray(value)) return value.length ? '[ ' + value.map(function(item) { return inspectError(item, depth + 1); }).join(', ') + ' ]' : '[]'; if (Object.getPrototypeOf(value) !== Object.prototype) return String(value); var keys = Object.keys(value); return keys.length ? '{ ' + keys.map(function(key) { return key + ': ' + inspectError(value[key], depth + 1); }).join(', ') + ' }' : '{}'; }\n",
            include_str!("events_core.js"),
            "function isEventTarget(target) { return !!target && typeof target.addEventListener === 'function' && typeof target.on !== 'function'; }\n\
             function listenerTarget(source) { return isEventTarget(source) ? { on: function(name, listener) { source.addEventListener(name, listener); }, once: function(name, listener) { source.addEventListener(name, listener, { once: true }); }, removeListener: function(name, listener) { source.removeEventListener(name, listener); } } : source; }\n\
             var targetMaxListeners = new WeakMap();\n\
             function addAbortListener(signal, listener) { if (!signal || typeof signal.aborted !== 'boolean') throw argTypeError('signal', 'an instance of AbortSignal', signal); checkListener(listener); var dispose = typeof Symbol.dispose === 'symbol' ? Symbol.dispose : Symbol.for('nodejs.dispose'); if (signal.aborted) queueMicrotask(function() { listener(); }); else signal.addEventListener('abort', listener, { once: true }); var disposable = Object.create(null); disposable[dispose] = function() { signal.removeEventListener('abort', listener); }; return disposable; }\n\
             function once(source, event) { var options = arguments[2]; var emitter = listenerTarget(source);\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } // shortcut: stream end is emitted synchronously, Node emits it on a later tick, so awaiting it afterwards must still resolve.
 if (event === 'end' && emitter._endEmitted === true) { resolve([]); return; } emitter.once(event, done); if (event !== 'error' && !isEventTarget(source)) emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(source, event) { var options = arguments[2] === undefined ? {} : arguments[2]; if (options === null || typeof options !== 'object' || Array.isArray(options)) throw argTypeError('options', 'of type object', options); if (!isEventTarget(source) && !(source !== null && source !== undefined && typeof source.on === 'function')) throw argTypeError('emitter', 'an instance of EventEmitter', source); var emitter = listenerTarget(source); var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; closeNames.forEach(function(name) { emitter.removeListener(name, finish); }); emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } var closeNames = Array.isArray(options.close) ? options.close : []; closeNames.forEach(function(name) { emitter.on(name, finish); }); emitter.on(event, received); if (event !== 'error' && !isEventTarget(source)) emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { if (isEventTarget(emitter)) { var records = emitter.__thawListeners && emitter.__thawListeners.get(String(event)); return records ? records.map(function(entry) { return entry.callback; }) : []; } if (typeof emitter.listeners === 'function') return emitter.listeners(event); throw argTypeError('emitter', 'an instance of EventEmitter or EventTarget', emitter); } function getMaxListeners(emitter) { if (isEventTarget(emitter)) return targetMaxListeners.has(emitter) ? targetMaxListeners.get(emitter) : EventEmitter.defaultMaxListeners; if (emitter !== null && emitter !== undefined && typeof emitter.getMaxListeners === 'function') return emitter.getMaxListeners(); throw argTypeError('emitter', 'an instance of EventEmitter or EventTarget', emitter); } function setMaxListeners() { var value = arguments[0], emitters = Array.prototype.slice.call(arguments, 1); validateNumber(value, 'setMaxListeners', 0); emitters.forEach(function(emitter) { if (!isEventTarget(emitter) && !(emitter !== null && emitter !== undefined && typeof emitter.setMaxListeners === 'function')) throw argTypeError('eventTargets', 'an instance of EventEmitter or EventTarget', emitter); }); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { if (isEventTarget(emitter)) targetMaxListeners.set(emitter, value); else emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.addAbortListener = addAbortListener;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = errorMonitor; module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             EventEmitter.prototype._events = undefined; EventEmitter.prototype._eventsCount = 0; EventEmitter.prototype._maxListeners = undefined;\n\
             function EventEmitterAsyncResource(options) { if (!(this instanceof EventEmitterAsyncResource)) throw new TypeError(\"Class constructor EventEmitterAsyncResource cannot be invoked without 'new'\"); if (!options || typeof options.name !== 'string') { var nameError = new TypeError('The \"options.name\" property must be of type string.'); nameError.code = 'ERR_INVALID_ARG_TYPE'; throw nameError; } EventEmitter.call(this, options); this._asyncId = ++asyncResourceCount; this._triggerAsyncId = options.triggerAsyncId === undefined ? 0 : options.triggerAsyncId; }\n\
             var asyncResourceCount = 1;\n\
             EventEmitterAsyncResource.prototype = Object.create(EventEmitter.prototype); EventEmitterAsyncResource.prototype.constructor = EventEmitterAsyncResource;\n\
             Object.defineProperty(EventEmitterAsyncResource.prototype, 'asyncId', { get: function() { return this._asyncId; }, configurable: true }); Object.defineProperty(EventEmitterAsyncResource.prototype, 'triggerAsyncId', { get: function() { return this._triggerAsyncId; }, configurable: true }); EventEmitterAsyncResource.prototype.emitDestroy = function() { return this; };\n\
             module.exports.EventEmitterAsyncResource = EventEmitterAsyncResource;\n\
             module.exports.usingDomains = false; Object.defineProperty(module.exports, 'kMaxEventTargetListeners', { value: Symbol('events.maxEventTargetListeners'), writable: true, configurable: true }); Object.defineProperty(module.exports, 'kMaxEventTargetListenersWarned', { value: Symbol('events.maxEventTargetListenersWarned'), writable: true, configurable: true });\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        )),
        "assert" => Some(assert_source!("module.exports = ok;\n")),
        "assert/strict" => Some(assert_source!("module.exports = ok.strict;\n")),
        _ => None,
    }
}
