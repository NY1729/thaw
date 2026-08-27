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
        "events" => Some(
            "function EventEmitter() {\n\
             \x20\x20if (!(this instanceof EventEmitter)) return new EventEmitter();\n\
             \x20\x20this._events = Object.create(null); this._maxListeners = undefined;\n\
             }\n\
             EventEmitter.prototype._add = function(event, listener, prepend, once) {\n\
             \x20\x20if (typeof listener !== 'function') throw new TypeError('listener must be a function');\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name] || (this._events[name] = []);\n\
             \x20\x20var entry = { listener: listener, once: Boolean(once) };\n\
             \x20\x20if (prepend) list.unshift(entry); else list.push(entry);\n\
             \x20\x20return this;\n\
             };\n\
             EventEmitter.prototype.addListener = EventEmitter.prototype.on = function(event, listener) { return this._add(event, listener, false, false); };\n\
             EventEmitter.prototype.once = function(event, listener) { return this._add(event, listener, false, true); };\n\
             EventEmitter.prototype.prependListener = function(event, listener) { return this._add(event, listener, true, false); };\n\
             EventEmitter.prototype.prependOnceListener = function(event, listener) { return this._add(event, listener, true, true); };\n\
             EventEmitter.prototype.emit = function(event) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name];\n\
             \x20\x20if (!list || list.length === 0) {\n\
             \x20\x20\x20\x20if (name === 'error') { var error = arguments[1]; throw error instanceof Error ? error : new Error('Unhandled error event'); }\n\
             \x20\x20\x20\x20return false;\n\
             \x20\x20}\n\
             \x20\x20var args = Array.prototype.slice.call(arguments, 1);\n\
             \x20\x20list.slice().forEach(function(entry) {\n\
             \x20\x20\x20\x20if (entry.once) this.removeListener(name, entry.listener);\n\
             \x20\x20\x20\x20entry.listener.apply(this, args);\n\
             \x20\x20}, this);\n\
             \x20\x20return true;\n\
             };\n\
             EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(event, listener) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name]; if (!list) return this;\n\
             \x20\x20for (var index = list.length - 1; index >= 0; index--) if (list[index].listener === listener || list[index].listener.listener === listener) { list.splice(index, 1); break; }\n\
             \x20\x20if (list.length === 0) delete this._events[name]; return this;\n\
             };\n\
             EventEmitter.prototype.removeAllListeners = function(event) { if (event === undefined) this._events = Object.create(null); else delete this._events[typeof event === 'symbol' ? event : String(event)]; return this; };\n\
             EventEmitter.prototype.listeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener.listener || entry.listener; }); };\n\
             EventEmitter.prototype.rawListeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener; }); };\n\
             EventEmitter.prototype.listenerCount = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)]; return list ? list.length : 0; };\n\
             EventEmitter.prototype.eventNames = function() { return Reflect.ownKeys(this._events); };\n\
             EventEmitter.prototype.setMaxListeners = function(value) { value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); this._maxListeners = value; return this; }; EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? EventEmitter.defaultMaxListeners : this._maxListeners; }; EventEmitter.defaultMaxListeners = 10;\n\
             EventEmitter.listenerCount = function(emitter, event) { return emitter.listenerCount(event); };\n\
             function once(emitter, event, options) {\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } emitter.once(event, done); if (event !== 'error') emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(emitter, event, options) { options = options || {}; var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } emitter.on(event, received); if (event !== 'error') emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { return typeof emitter.listeners === 'function' ? emitter.listeners(event) : []; } function getMaxListeners(emitter) { return typeof emitter.getMaxListeners === 'function' ? emitter.getMaxListeners() : EventEmitter.defaultMaxListeners; } function setMaxListeners(value) { var emitters = Array.prototype.slice.call(arguments, 1); value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = Symbol.for('events.errorMonitor'); module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        ),
        "assert" | "assert/strict" => Some(
            "function AssertionError(options) {\n\
             \x20\x20options = options || {}; this.name = 'AssertionError'; this.code = 'ERR_ASSERTION';\n\
             \x20\x20this.actual = options.actual; this.expected = options.expected; this.operator = options.operator;\n\
             \x20\x20this.generatedMessage = options.message === undefined;\n\
             \x20\x20this.message = options.message === undefined ? 'Expected values to satisfy ' + (options.operator || 'assertion') : String(options.message);\n\
             \x20\x20if (Error.captureStackTrace) Error.captureStackTrace(this, options.stackStartFn || AssertionError);\n\
             }\n\
             AssertionError.prototype = Object.create(Error.prototype); AssertionError.prototype.constructor = AssertionError;\n\
             function failure(actual, expected, message, operator, start) { throw new AssertionError({ actual: actual, expected: expected, message: message, operator: operator, stackStartFn: start }); }\n\
             function deep(actual, expected, seen) {\n\
             \x20\x20if (Object.is(actual, expected)) return true;\n\
             \x20\x20if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;\n\
             \x20\x20if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;\n\
             \x20\x20seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);\n\
             \x20\x20if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();\n\
             \x20\x20if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;\n\
             \x20\x20if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }\n\
             \x20\x20var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;\n\
             \x20\x20for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !deep(actual[key], expected[key], seen)) return false; }\n\
             \x20\x20return true;\n\
             }\n\
             function ok(value, message) { if (!value) failure(value, true, message, '==', ok); }\n\
             function equal(actual, expected, message) { if (actual != expected) failure(actual, expected, message, '==', equal); }\n\
             function notEqual(actual, expected, message) { if (actual == expected) failure(actual, expected, message, '!=', notEqual); }\n\
             function strictEqual(actual, expected, message) { if (!Object.is(actual, expected)) failure(actual, expected, message, 'strictEqual', strictEqual); }\n\
             function notStrictEqual(actual, expected, message) { if (Object.is(actual, expected)) failure(actual, expected, message, 'notStrictEqual', notStrictEqual); }\n\
             function deepStrictEqual(actual, expected, message) { if (!deep(actual, expected)) failure(actual, expected, message, 'deepStrictEqual', deepStrictEqual); }\n\
             function notDeepStrictEqual(actual, expected, message) { if (deep(actual, expected)) failure(actual, expected, message, 'notDeepStrictEqual', notDeepStrictEqual); }\n\
             function fail(message) { failure(undefined, undefined, message || 'Failed', 'fail', fail); }\n\
             function matches(error, expected) { if (expected === undefined) return true; if (expected instanceof RegExp) return expected.test(String(error && error.message || error)); if (typeof expected === 'function') return error instanceof expected || expected(error) === true; return true; }\n\
             function throws(block, expected, message) { var caught; try { block(); } catch (error) { caught = error; } if (caught === undefined || !matches(caught, expected)) failure(caught, expected, message, 'throws', throws); return caught; }\n\
             function doesNotThrow(block, expected, message) { try { block(); } catch (error) { if (matches(error, expected)) failure(error, undefined, message, 'doesNotThrow', doesNotThrow); throw error; } }\n\
             ok.AssertionError = AssertionError; ok.ok = ok; ok.equal = equal; ok.notEqual = notEqual; ok.strictEqual = strictEqual; ok.notStrictEqual = notStrictEqual;\n\
             ok.deepEqual = deepStrictEqual; ok.notDeepEqual = notDeepStrictEqual; ok.deepStrictEqual = deepStrictEqual; ok.notDeepStrictEqual = notDeepStrictEqual;\n\
             ok.fail = fail; ok.throws = throws; ok.doesNotThrow = doesNotThrow; ok.strict = ok; ok.default = ok; ok.__esModule = true;\n\
             module.exports = ok;\n",
        ),
        _ => None,
    }
}
