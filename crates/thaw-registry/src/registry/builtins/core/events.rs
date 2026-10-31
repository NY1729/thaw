macro_rules! assert_source {
    ($tail:literal) => {
        concat!(include_str!("assert.js"), $tail)
    };
}

pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "domain" => Some(
            r#"var EventEmitter = require('node:events'), stack = [];
             function Domain() { if (!(this instanceof Domain)) return new Domain(); EventEmitter.call(this); this.members = []; }
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
             module.exports = { Domain: Domain, create: create, createDomain: create, active: null }; module.exports.default = module.exports; module.exports.__esModule = true;
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
        "events" => Some(
            "var errorMonitor = Symbol.for('events.errorMonitor');\n\
             function EventEmitter(options) {\n\
             \x20\x20if (!(this instanceof EventEmitter)) return new EventEmitter(options);\n\
             \x20\x20this._events = Object.create(null); this._maxListeners = undefined; if (options && options.captureRejections) Object.defineProperty(this, '_captureRejections', { value: true, writable: true, configurable: true });\n\
             }\n\
             function addEventListener(emitter, event, listener, prepend, once) {\n\
             \x20\x20if (typeof listener !== 'function') throw new TypeError('listener must be a function');\n\
             \x20\x20if (!emitter._events) emitter._events = Object.create(null);\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); if (emitter._events.newListener && emitter._events.newListener.length) emitter.emit('newListener', name, listener); var list = emitter._events[name] || (emitter._events[name] = []);\n\
             \x20\x20var entry = { listener: listener, once: Boolean(once) };\n\
             \x20\x20if (prepend) list.unshift(entry); else list.push(entry);\n\
             \x20\x20return emitter;\n\
             };\n\
             EventEmitter.prototype.addListener = EventEmitter.prototype.on = function(event, listener) { return addEventListener(this, event, listener, false, false); };\n\
             EventEmitter.prototype.once = function(event, listener) { return addEventListener(this, event, listener, false, true); };\n\
             EventEmitter.prototype.prependListener = function(event, listener) { return addEventListener(this, event, listener, true, false); };\n\
             EventEmitter.prototype.prependOnceListener = function(event, listener) { return addEventListener(this, event, listener, true, true); };\n\
             function removeEntry(emitter, name, entry) { var events = emitter._events; var list = events ? events[name] : undefined; var at = list ? list.indexOf(entry) : -1; if (at < 0) return; list.splice(at, 1); if (list.length === 0) delete events[name]; if (events.removeListener && events.removeListener.length) emitter.emit('removeListener', name, entry.listener); }\n\
             // ponytail: tiny inspect for the ERR_UNHANDLED_ERROR message only (strings, plain arrays/objects); a static require of node:util would make the bundler pull it into every events consumer.
             function inspectError(value, depth) { if (typeof value === 'string') return \"'\" + value + \"'\"; if (!value || typeof value !== 'object' || depth > 2) return String(value); if (Array.isArray(value)) return value.length ? '[ ' + value.map(function(item) { return inspectError(item, depth + 1); }).join(', ') + ' ]' : '[]'; if (Object.getPrototypeOf(value) !== Object.prototype) return String(value); var keys = Object.keys(value); return keys.length ? '{ ' + keys.map(function(key) { return key + ': ' + inspectError(value[key], depth + 1); }).join(', ') + ' }' : '{}'; }\n\
             EventEmitter.prototype.emit = function(event) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var events = this._events; var list = events ? events[name] : undefined;\n\
             \x20\x20if (name === 'error' && events && events[errorMonitor] && events[errorMonitor].length) this.emit.apply(this, [errorMonitor].concat(Array.prototype.slice.call(arguments, 1)));\n\
             \x20\x20if (!list || list.length === 0) {\n\
             \x20\x20\x20\x20if (name === 'error') { var error = arguments[1]; if (error instanceof Error) throw error; var unhandled = new Error('Unhandled error. (' + inspectError(error, 0) + ')'); unhandled.code = 'ERR_UNHANDLED_ERROR'; unhandled.context = error; throw unhandled; }\n\
             \x20\x20\x20\x20return false;\n\
             \x20\x20}\n\
             \x20\x20var args = Array.prototype.slice.call(arguments, 1);\n\
             \x20\x20list.slice().forEach(function(entry) {\n\
             \x20\x20\x20\x20if (entry.once) { if (entry.fired) return; entry.fired = true; removeEntry(this, name, entry); }\n\
             \x20\x20\x20\x20var result = entry.listener.apply(this, args); if (this._captureRejections && result && typeof result.then === 'function') { var emitter = this; result.then(undefined, function(error) { var handler = emitter[Symbol.for('nodejs.rejection')]; if (typeof handler === 'function') handler.apply(emitter, [error, name].concat(args)); else { emitter._captureRejections = false; try { emitter.emit('error', error); } finally { emitter._captureRejections = true; } } }); }\n\
             \x20\x20}, this);\n\
             \x20\x20return true;\n\
             };\n\
             EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(event, listener) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var events = this._events; var list = events ? events[name] : undefined; if (!list) return this;\n\
             \x20\x20for (var index = list.length - 1; index >= 0; index--) if (list[index].listener === listener || list[index].listener.listener === listener) { var removed = list[index].listener.listener || list[index].listener; list.splice(index, 1); if (list.length === 0) delete events[name]; if (events.removeListener && events.removeListener.length) this.emit('removeListener', name, removed); break; } return this;\n\
             };\n\
             EventEmitter.prototype.removeAllListeners = function(event) { var emitter = this; var events = this._events; if (!events) return this; if (!events.removeListener || !events.removeListener.length) { if (event === undefined) this._events = Object.create(null); else delete events[typeof event === 'symbol' ? event : String(event)]; return this; } if (event !== undefined) { var name = typeof event === 'symbol' ? event : String(event), list = (events[name] || []).slice(); for (var index = list.length - 1; index >= 0; index--) this.removeListener(name, list[index].listener); return this; } Reflect.ownKeys(events).filter(function(name) { return name !== 'removeListener'; }).forEach(function(name) { emitter.removeAllListeners(name); }); this.removeAllListeners('removeListener'); return this; };\n\
             EventEmitter.prototype.listeners = function(event) { var events = this._events; var list = (events ? events[typeof event === 'symbol' ? event : String(event)] : undefined) || []; return list.map(function(entry) { return entry.listener.listener || entry.listener; }); };\n\
             EventEmitter.prototype.rawListeners = function(event) { var events = this._events; var list = (events ? events[typeof event === 'symbol' ? event : String(event)] : undefined) || []; var emitter = this; return list.map(function(entry) { if (!entry.once) return entry.listener; if (!entry.wrapper) { entry.wrapper = function() { if (entry.fired) return; entry.fired = true; removeEntry(emitter, typeof event === 'symbol' ? event : String(event), entry); return entry.listener.apply(emitter, arguments); }; entry.wrapper.listener = entry.listener; } return entry.wrapper; }); };\n\
             EventEmitter.prototype.listenerCount = function(event) { var events = this._events; var list = events ? events[typeof event === 'symbol' ? event : String(event)] : undefined; return list ? list.length : 0; };\n\
             EventEmitter.prototype.eventNames = function() { return this._events ? Reflect.ownKeys(this._events) : []; };\n\
             EventEmitter.prototype.setMaxListeners = function(value) { value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); this._maxListeners = value; return this; }; EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? EventEmitter.defaultMaxListeners : this._maxListeners; }; EventEmitter.defaultMaxListeners = 10;\n\
             EventEmitter.listenerCount = function(emitter, event) { return emitter.listenerCount(event); };\n\
             function once(emitter, event, options) {\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } // shortcut: stream end is emitted synchronously, Node emits it on a later tick, so awaiting it afterwards must still resolve.
 if (event === 'end' && emitter._endEmitted === true) { resolve([]); return; } emitter.once(event, done); if (event !== 'error') emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(emitter, event, options) { options = options || {}; var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; closeNames.forEach(function(name) { emitter.removeListener(name, finish); }); emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } var closeNames = Array.isArray(options.close) ? options.close : []; closeNames.forEach(function(name) { emitter.on(name, finish); }); emitter.on(event, received); if (event !== 'error') emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { return typeof emitter.listeners === 'function' ? emitter.listeners(event) : []; } function getMaxListeners(emitter) { return typeof emitter.getMaxListeners === 'function' ? emitter.getMaxListeners() : EventEmitter.defaultMaxListeners; } function setMaxListeners(value) { var emitters = Array.prototype.slice.call(arguments, 1); value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = errorMonitor; module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        ),
        "assert" => Some(assert_source!("module.exports = ok;\n")),
        "assert/strict" => Some(assert_source!("module.exports = ok.strict;\n")),
        _ => None,
    }
}
