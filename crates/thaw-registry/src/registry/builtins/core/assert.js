// The module name is built at run time so the CommonJS bundler does not pull node:util into every assert/events consumer.
function quote(text) { var q = text.indexOf("'") < 0 ? "'" : text.indexOf('"') < 0 ? '"' : "'"; var body = JSON.stringify(text).slice(1, -1).replace(/\\"/g, '"'); if (q === "'") body = body.replace(/'/g, "\\'"); return q + body + q; }
function isObj(value) { return value !== null && typeof value === 'object'; }
function fmt(value, pad, seen) {
if (typeof value === 'string') return quote(value);
if (!isObj(value) || value instanceof Date || value instanceof RegExp || value instanceof Error || value instanceof Map || value instanceof Set || ArrayBuffer.isView(value)) { try { return require(['node', 'util'].join(':')).inspect(value); } catch (_) { return String(value); } }
seen = seen || []; if (seen.indexOf(value) >= 0) return '[Circular]'; seen = seen.concat([value]);
var inner = pad + '  ', isArray = Array.isArray(value);
var parts = isArray ? Array.prototype.map.call(value, function(item) { return inner + fmt(item, inner, seen); }) : Object.keys(value).sort().map(function(key) { return inner + (/^[A-Za-z_$][\w$]*$/.test(key) ? key : quote(key)) + ': ' + fmt(value[key], inner, seen); });
var open = isArray ? '[' : '{', close = isArray ? ']' : '}';
return parts.length ? open + '\n' + parts.join(',\n') + '\n' + pad + close : open + close;
}
function show(value) { return fmt(value, ''); }
// ponytail: plain LCS line diff without Node's "..." elision of long unchanged runs.
function diff(actual, expected) {
var a = show(actual).split('\n'), e = show(expected).split('\n'), table = [], i, j, out = [];
for (i = 0; i <= a.length; i++) { table.push([]); for (j = 0; j <= e.length; j++) table[i].push(0); }
for (i = a.length - 1; i >= 0; i--) for (j = e.length - 1; j >= 0; j--) table[i][j] = a[i] === e[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
i = 0; j = 0;
while (i < a.length || j < e.length) {
if (i < a.length && j < e.length && a[i] === e[j]) { out.push('  ' + a[i]); i++; j++; }
else if (i < a.length && (j >= e.length || table[i + 1][j] >= table[i][j + 1])) out.push('+ ' + a[i++]);
else out.push('- ' + e[j++]);
}
return '+ actual - expected\n\n' + out.join('\n') + '\n';
}
function plain(left, right) { return !isObj(left) && !isObj(right) && typeof left !== 'function' && typeof right !== 'function'; }
function strictText(actual, expected) {
var left = show(actual), right = show(expected);
if (left === right && isObj(actual)) return 'Values have same structure but are not reference-equal:\n\n' + left + '\n';
if (plain(actual, expected)) return 'Expected values to be strictly equal:\n\n' + left + ' !== ' + right + '\n';
return 'Expected "actual" to be reference-equal to "expected":\n' + diff(actual, expected);
}
function deepText(actual, expected) { return plain(actual, expected) ? 'Expected values to be strictly deep-equal:\n\n' + show(actual) + ' !== ' + show(expected) + '\n' : 'Expected values to be strictly deep-equal:\n' + diff(actual, expected); }
function notText(actual, single, multi) { var text = show(actual); return text.indexOf('\n') < 0 ? single + ': ' + text : multi + ':\n\n' + text + '\n'; }
function AssertionError(options) {
  options = options || {}; this.name = 'AssertionError'; this.code = 'ERR_ASSERTION';
  this.actual = options.actual; this.expected = options.expected; this.operator = options.operator;
  this.generatedMessage = options.generatedMessage !== undefined ? options.generatedMessage : options.message === undefined;
  this.message = options.message !== undefined ? String(options.message) : options.operator === 'strictEqual' ? strictText(options.actual, options.expected) : options.operator === 'deepStrictEqual' ? deepText(options.actual, options.expected) : 'Expected values to satisfy ' + (options.operator || 'assertion');
  if (Error.captureStackTrace) Error.captureStackTrace(this, options.stackStartFn || AssertionError);
}
AssertionError.prototype = Object.create(Error.prototype); AssertionError.prototype.constructor = AssertionError;
function failure(actual, expected, message, operator, start, text, custom) {
if (message instanceof Error) throw message;
var final = message === undefined ? text : message;
throw new AssertionError({ actual: actual, expected: expected, message: final, operator: operator, stackStartFn: start, generatedMessage: message === undefined && !custom });
}
var looseDeep = false;
function deep(actual, expected, seen) {
if (Object.is(actual, expected)) return true;
if (looseDeep && (actual === null || typeof actual !== 'object')) return (expected === null || typeof expected !== 'object') && (actual == expected || (actual !== actual && expected !== expected));
if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;
if (!looseDeep && Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;
seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);
if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();
if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;
if ((actual instanceof Number || actual instanceof String || actual instanceof Boolean) && !Object.is(actual.valueOf(), expected.valueOf())) return false;
if (actual instanceof Error) { if (actual.name !== expected.name || actual.message !== expected.message || ('cause' in actual) !== ('cause' in expected) || !deep(actual.cause, expected.cause, seen) || !deep(actual.errors, expected.errors, seen)) return false; }
if (Array.isArray(actual) && actual.length !== expected.length) return false;
if (actual instanceof ArrayBuffer || actual instanceof DataView) { if (actual.byteLength !== expected.byteLength) return false; var left = actual instanceof DataView ? new Uint8Array(actual.buffer, actual.byteOffset, actual.byteLength) : new Uint8Array(actual), right = expected instanceof DataView ? new Uint8Array(expected.buffer, expected.byteOffset, expected.byteLength) : new Uint8Array(expected); for (var at = 0; at < left.length; at++) if (left[at] !== right[at]) return false; return true; }
if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }
if (actual instanceof Map) { if (actual.size !== expected.size) return false; var usedKeys = new Set(); for (var entry of actual) { if (expected.has(entry[0]) && deep(entry[1], expected.get(entry[0]), seen)) { usedKeys.add(entry[0]); continue; } var found = false; for (var other of expected) { if (usedKeys.has(other[0])) continue; if (deep(entry[0], other[0], seen) && deep(entry[1], other[1], seen)) { usedKeys.add(other[0]); found = true; break; } } if (!found) return false; } }
if (actual instanceof Set) { if (actual.size !== expected.size) return false; var usedValues = new Set(); for (var item of actual) { if (expected.has(item)) { usedValues.add(item); continue; } var matched = false; for (var candidate of expected) { if (usedValues.has(candidate)) continue; if (deep(item, candidate, seen)) { usedValues.add(candidate); matched = true; break; } } if (!matched) return false; } }
var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;
for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !deep(actual[key], expected[key], seen)) return false; }
return true;
}
function ok(value, message) { if (!value) failure(value, true, message, '==', ok, arguments.length ? show(value) + ' == true' : 'No value argument passed to `assert.ok()`'); }
function equal(actual, expected, message) { if (actual != expected) failure(actual, expected, message, '==', equal, show(actual) + ' == ' + show(expected)); }
function notEqual(actual, expected, message) { if (actual == expected) failure(actual, expected, message, '!=', notEqual, show(actual) + ' != ' + show(expected)); }
// Node 22 appends the diff body (generated text minus its header line) to a custom string message for the strict-equal family only.
function withBody(message, text) { return typeof message === 'string' ? message + '\n' + text.slice(text.indexOf('\n') + 1) : message; }
function strictEqual(actual, expected, message) { if (!Object.is(actual, expected)) { var text = strictText(actual, expected); failure(actual, expected, withBody(message, text), 'strictEqual', strictEqual, text); } }
function notStrictEqual(actual, expected, message) { if (Object.is(actual, expected)) failure(actual, expected, message, 'notStrictEqual', notStrictEqual, notText(actual, 'Expected "actual" to be strictly unequal to', 'Expected "actual" not to be strictly unequal to')); }
function deepStrictEqual(actual, expected, message) { if (!deep(actual, expected)) { var text = deepText(actual, expected); failure(actual, expected, withBody(message, text), 'deepStrictEqual', deepStrictEqual, text); } }
function notDeepStrictEqual(actual, expected, message) { if (deep(actual, expected)) failure(actual, expected, message, 'notDeepStrictEqual', notDeepStrictEqual, notText(actual, 'Expected "actual" not to be strictly deep-equal to', 'Expected "actual" not to be strictly deep-equal to')); }
function fail(message) { failure(undefined, undefined, message, 'fail', fail, 'Failed'); }
function matches(error, expected) {
if (expected === undefined) return true;
if (expected instanceof RegExp) return expected.test(String(error && error.message || error));
if (typeof expected === 'function') { if (expected.prototype !== undefined && error instanceof expected) return true; if (Error.isPrototypeOf(expected)) return false; return expected.call({}, error) === true; }
if (expected !== null && typeof expected === 'object') { if (error === null || typeof error !== 'object') return false; var keys = Object.keys(expected); if (expected instanceof Error) keys.push('name', 'message'); return keys.every(function(key) { if (!(key in error)) return false; var want = expected[key]; if (want instanceof RegExp && typeof error[key] === 'string') return want.test(error[key]); return deep(error[key], want); }); }
return true;
}
function mismatchText(caught, expected, message) {
var text;
if (message !== undefined) text = undefined;
else if (expected instanceof RegExp) text = 'The input did not match the regular expression ' + String(expected) + '. Input:\n\n' + show(String(caught)) + '\n';
else if (typeof expected === 'function' && expected.prototype !== undefined && Error.isPrototypeOf(expected)) text = 'The error is expected to be an instance of "' + expected.name + '". Received "' + (caught && caught.constructor && caught.constructor.name) + '"\n\nError message:\n\n' + (caught && caught.message);
return text;
}
function throws(block, expected, message) {
var caught, threw = false; try { block(); } catch (error) { caught = error; threw = true; }
if (typeof expected === 'string') { message = expected; expected = undefined; }
if (!threw) failure(undefined, expected, undefined, 'throws', throws, 'Missing expected exception' + (expected && expected.name ? ' (' + expected.name + ')' : '') + (message ? ': ' + message : '.'), true);
if (!matches(caught, expected)) {
var text = mismatchText(caught, expected, message);
failure(caught, expected, message, 'throws', throws, text);
}
return caught;
}
function doesNotThrow(block, expected, message) { try { block(); } catch (error) { if (typeof expected === 'string') { message = expected; expected = undefined; } if (matches(error, expected)) failure(error, expected, undefined, 'doesNotThrow', doesNotThrow, 'Got unwanted exception' + (message ? ': ' + message : '.') + '\nActual message: "' + (error && error.message) + '"', true); throw error; } }
function rejects(promiseFn, expected, message) {
if (typeof expected === 'string') { message = expected; expected = undefined; }
return Promise.resolve().then(function() { return typeof promiseFn === 'function' ? promiseFn() : promiseFn; }).then(function() {
failure(undefined, expected, undefined, 'rejects', rejects, 'Missing expected rejection' + (expected && expected.name ? ' (' + expected.name + ')' : '') + (message ? ': ' + message : '.'), true);
}, function(error) {
if (!matches(error, expected)) failure(error, expected, message, 'rejects', rejects, mismatchText(error, expected, message));
});
}
function doesNotReject(promiseFn, expected, message) {
if (typeof expected === 'string') { message = expected; expected = undefined; }
return Promise.resolve().then(function() { return typeof promiseFn === 'function' ? promiseFn() : promiseFn; }).then(function() {}, function(error) {
if (matches(error, expected)) failure(error, expected, undefined, 'doesNotReject', doesNotReject, 'Got unwanted rejection' + (message ? ': ' + message : '.') + '\nActual message: "' + (error && error.message) + '"', true);
throw error;
});
}
ok.AssertionError = AssertionError; ok.ok = ok; ok.equal = equal; ok.notEqual = notEqual; ok.strictEqual = strictEqual; ok.notStrictEqual = notStrictEqual;
function looseDeepEqual(actual, expected) { var prev = looseDeep; looseDeep = true; try { return deep(actual, expected); } finally { looseDeep = prev; } }
function deepEqual(actual, expected, message) { if (!looseDeepEqual(actual, expected)) failure(actual, expected, message, 'deepEqual', deepEqual, 'Expected values to be loosely deep-equal:\n\n' + show(actual) + '\n\nshould loosely deep-equal\n\n' + show(expected)); }
function notDeepEqual(actual, expected, message) { if (looseDeepEqual(actual, expected)) failure(actual, expected, message, 'notDeepEqual', notDeepEqual, notText(actual, 'Expected "actual" not to be loosely deep-equal to', 'Expected "actual" not to be loosely deep-equal to')); }
ok.deepEqual = deepEqual; ok.notDeepEqual = notDeepEqual; ok.deepStrictEqual = deepStrictEqual; ok.notDeepStrictEqual = notDeepStrictEqual;
function received(value) {
if (value === null || value === undefined) return String(value);
if (typeof value === 'function') return 'function ' + value.name;
if (typeof value === 'object') return value.constructor && value.constructor.name ? 'an instance of ' + value.constructor.name : show(value);
return 'type ' + typeof value + ' (' + show(value) + ')';
}
function ifError(value) { if (value !== null && value !== undefined) failure(value, null, undefined, 'ifError', ifError, 'ifError got unwanted exception: ' + (value instanceof Error ? value.message : show(value)), true); }
function matchWith(wantMatch, operator, input, regexp, message) {
if (!(regexp instanceof RegExp)) { var bad = new TypeError('The "regexp" argument must be an instance of RegExp. Received ' + received(regexp)); bad.code = 'ERR_INVALID_ARG_TYPE'; throw bad; }
var text;
if (typeof input !== 'string') text = 'The "string" argument must be of type string. Received ' + received(input);
else if (regexp.test(input) === wantMatch) return;
else text = wantMatch ? 'The input did not match the regular expression ' + String(regexp) + '. Input:\n\n' + show(input) + '\n' : 'The input was expected to not match the regular expression ' + String(regexp) + '. Input:\n\n' + show(input) + '\n';
failure(input, regexp, message, operator, wantMatch ? match : doesNotMatch, text);
}
function match(input, regexp, message) { matchWith(true, 'match', input, regexp, message); }
function doesNotMatch(input, regexp, message) { matchWith(false, 'doesNotMatch', input, regexp, message); }
ok.ifError = ifError; ok.match = match; ok.doesNotMatch = doesNotMatch;
ok.fail = fail; ok.rejects = rejects; ok.doesNotReject = doesNotReject; ok.throws = throws; ok.doesNotThrow = doesNotThrow; ok.default = ok; ok.__esModule = true;
function strict(value, message) { if (!value) failure(value, true, message, '==', strict, arguments.length ? show(value) + ' == true' : 'No value argument passed to `assert.ok()`'); }
Object.assign(strict, ok, { equal: strictEqual, notEqual: notStrictEqual, deepEqual: deepStrictEqual, notDeepEqual: notDeepStrictEqual }); strict.strict = strict; strict.default = strict; ok.strict = strict;

