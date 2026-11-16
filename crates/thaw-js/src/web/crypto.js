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
// Node's classes are not constructible by users: only the instances built below pass the private token.
var token = {};
function illegalCtor(args) {
  if (args[0] === token) return;
  var e = new TypeError('Illegal constructor');
  e.code = 'ERR_ILLEGAL_CONSTRUCTOR';
  throw e;
}
class Crypto { constructor(...args) { illegalCtor(args); } }
class SubtleCrypto { constructor(...args) { illegalCtor(args); } }
class CryptoKey { constructor(...args) { illegalCtor(args); } }
for (const C of [Crypto, SubtleCrypto, CryptoKey]) {
  Object.defineProperty(C.prototype, Symbol.toStringTag, { value: C.name, configurable: true });
  def(globalThis, C.name, C);
}
def(Crypto.prototype, 'getRandomValues', getRandomValues);
def(Crypto.prototype, 'randomUUID', randomUUID);
def(SubtleCrypto.prototype, 'digest', function digest(algorithm, data) {
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
var subtleInstance = new SubtleCrypto(token);
Object.defineProperty(Crypto.prototype, 'subtle', { get: function () { return subtleInstance; }, enumerable: true, configurable: true });
def(globalThis, 'crypto', new Crypto(token));

