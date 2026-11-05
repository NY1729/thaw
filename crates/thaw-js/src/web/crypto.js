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
var hashSizes = { md5: 16, sha1: 20, sha256: 32, sha384: 48, sha512: 64 };
var subtle = {};
def(subtle, 'digest', function digest(algorithm, data) {
  try {
    var name = String(typeof algorithm === 'string' ? algorithm : algorithm && algorithm.name).toLowerCase().replace('-', '');
    if (name === 'md5' || !hashSizes[name]) throw new DOMException('Unrecognized algorithm name', 'NotSupportedError');
    if (!ArrayBuffer.isView(data) && !(data instanceof ArrayBuffer)) throw new TypeError('The data argument must be an instance of ArrayBuffer or ArrayBufferView');
    var bytes = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);
    var out = new Uint8Array(hashSizes[name]);
    __thaw_hash(name, bytes, out);
    return Promise.resolve(out.buffer);
  } catch (e) { return Promise.reject(e); }
});
var webcrypto = {};
def(webcrypto, 'subtle', subtle);
def(webcrypto, 'getRandomValues', getRandomValues);
def(webcrypto, 'randomUUID', randomUUID);
def(globalThis, 'crypto', webcrypto);

