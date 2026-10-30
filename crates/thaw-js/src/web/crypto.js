function def(o, name, v) {
  Object.defineProperty(o, name, { value: v, writable: true, enumerable: false, configurable: true });
}
var fill = __thaw_random_fill;
function getRandomValues(a) {
  if (!ArrayBuffer.isView(a) || a instanceof Float32Array || a instanceof Float64Array || a instanceof DataView) {
    var e = new TypeError("The data argument must be an integer-type TypedArray");
    e.name = 'TypeMismatchError';
    throw e;
  }
  if (a.byteLength > 65536) {
    var q = new Error("The ArrayBufferView's byte length (" + a.byteLength + ") exceeds the number of bytes of entropy available via this API (65536)");
    q.name = 'QuotaExceededError';
    throw q;
  }
  fill(a);
  return a;
}
function randomUUID() {
  var b = new Uint8Array(16);
  fill(b);
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  var h = '';
  for (var i = 0; i < 16; i++) {
    h += (b[i] + 256).toString(16).slice(1);
    if (i === 3 || i === 5 || i === 7 || i === 9) h += '-';
  }
  return h;
}
var webcrypto = {};
def(webcrypto, 'getRandomValues', getRandomValues);
def(webcrypto, 'randomUUID', randomUUID);
def(globalThis, 'crypto', webcrypto);

function randomBytes(n, cb) {
  var b = Buffer.alloc(n);
  fill(b);
  if (typeof cb === 'function') { setImmediate(function () { cb(null, b); }); return undefined; }
  return b;
}
function randomFillSync(buf, offset, size) {
  var view = ArrayBuffer.isView(buf) ? new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength) : new Uint8Array(buf);
  offset = offset === undefined ? 0 : offset;
  fill(view.subarray(offset, size === undefined ? view.length : offset + size));
  return buf;
}
function randomInt(min, max, cb) {
  if (max === undefined || typeof max === 'function') { cb = max; max = min; min = 0; }
  var range = max - min, r = new Uint32Array(1), limit = 4294967296 - (4294967296 % range), v;
  do { fill(r); v = r[0]; } while (v >= limit);
  v = min + (v % range);
  if (typeof cb === 'function') { setImmediate(function () { cb(null, v); }); return undefined; }
  return v;
}
def(globalThis, '__thaw_crypto_module', {
  getRandomValues: getRandomValues, randomUUID: randomUUID, randomBytes: randomBytes,
  randomFillSync: randomFillSync, randomInt: randomInt, webcrypto: webcrypto,
});
