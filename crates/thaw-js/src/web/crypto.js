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
// ponytail: secret keys only (HMAC and AES raw/jwk, generate). Asymmetric keys, spki/pkcs8 and every
// operation (encrypt/sign/derive/wrap...) reject NotSupportedError until those parts land; never resolve.
var keyAlgs = ['HMAC', 'AES-CBC', 'AES-CTR', 'AES-GCM', 'AES-KW'];
var knownAlgs = keyAlgs.concat(['RSASSA-PKCS1-v1_5', 'RSA-PSS', 'RSA-OAEP', 'ECDSA', 'ECDH', 'Ed25519', 'Ed448', 'X25519', 'X448', 'HKDF', 'PBKDF2']);
var hashNames = ['SHA-1', 'SHA-256', 'SHA-384', 'SHA-512'];
var hashBlockBits = { 'SHA-1': 512, 'SHA-256': 512, 'SHA-384': 1024, 'SHA-512': 1024 };
var jwkHmacAlg = { 'SHA-1': 'HS1', 'SHA-256': 'HS256', 'SHA-384': 'HS384', 'SHA-512': 'HS512' };
var jwkAesSuffix = { 'AES-CBC': 'CBC', 'AES-CTR': 'CTR', 'AES-GCM': 'GCM', 'AES-KW': 'KW' };
var keySlots = new WeakMap();
function canonIn(list, v) {
  var n = String(v !== null && typeof v === 'object' ? v.name : v).toUpperCase();
  return list.find(function (x) { return x.toUpperCase() === n; });
}
function domError(name, msg) { return new DOMException(msg, name); }
function unimplementedError(algorithm, op) {
  var n = canonIn(knownAlgs, algorithm);
  return n ? domError('NotSupportedError', n + ' ' + op + ' is not implemented') : domError('NotSupportedError', 'Unrecognized algorithm name');
}
function settle(f) { try { return Promise.resolve(f()); } catch (e) { return Promise.reject(e); } }
function badFormat(method, format) {
  return new TypeError("Failed to execute '" + method + "' on 'SubtleCrypto': 1st argument value '" + format + "' is not a valid enum value of type KeyFormat.");
}
function hashOf(algorithm) {
  var h = canonIn(hashNames, algorithm.hash);
  if (!h) throw domError('NotSupportedError', 'Unrecognized algorithm name');
  return h;
}
function secretAlgorithm(name, algorithm, bits) {
  return name === 'HMAC' ? { name: name, hash: { name: hashOf(algorithm) }, length: bits } : { name: name, length: bits };
}
function checkUsages(name, usages, creating) {
  if (!Array.isArray(usages)) throw new TypeError('keyUsages must be an array');
  if (!usages.length) throw new SyntaxError(creating ? 'Usages cannot be empty when creating a key.' : 'Usages cannot be empty when importing a secret key.');
  var allowed = name === 'HMAC' ? ['sign', 'verify'] : name === 'AES-KW' ? ['wrapKey', 'unwrapKey'] : ['encrypt', 'decrypt', 'wrapKey', 'unwrapKey'];
  for (var i = 0; i < usages.length; i++) {
    if (allowed.indexOf(usages[i]) < 0) throw new SyntaxError('Unsupported key usage for an ' + (name === 'HMAC' ? 'HMAC' : 'AES') + ' key');
  }
  return usages.slice();
}
function makeKey(alg, bytes, extractable, usages) {
  var k = new CryptoKey(token);
  keySlots.set(k, { alg: alg, bytes: bytes, extractable: !!extractable, usages: usages });
  return k;
}
function slotOf(k) {
  var s = keySlots.get(k);
  if (!s) throw new TypeError('Value of "this" must be of type CryptoKey');
  return s;
}
var keyAccessors = {
  get type() { slotOf(this); return 'secret'; },
  get algorithm() { return slotOf(this).alg; },
  get extractable() { return slotOf(this).extractable; },
  get usages() { return slotOf(this).usages; }
};
// Closures synthesize `name`/`length`; defining them makes them real own properties (Node's shape).
function ownNameLength(f, name) {
  Object.defineProperty(f, 'name', { value: name, configurable: true });
  Object.defineProperty(f, 'length', { value: f.length, configurable: true });
  return f;
}
for (var ka of ['type', 'algorithm', 'extractable', 'usages']) {
  var getter = ownNameLength(Object.getOwnPropertyDescriptor(keyAccessors, ka).get, 'get ' + ka);
  Object.defineProperty(CryptoKey.prototype, ka, { get: getter, enumerable: true, configurable: true });
}
function generateSecret(algorithm, extractable, usages) {
  var name = canonIn(keyAlgs, algorithm);
  if (!name) throw unimplementedError(algorithm, 'generateKey');
  var list = checkUsages(name, usages, true);
  var bits;
  if (name === 'HMAC') {
    var hash = hashOf(algorithm);
    bits = algorithm.length === undefined ? hashBlockBits[hash] : algorithm.length;
    if (bits === 0) throw domError('DataError', 'Zero-length key is not supported');
    if (bits % 8) throw domError('NotSupportedError', 'Unsupported algorithm.length');
  } else {
    bits = algorithm.length;
    if ([128, 192, 256].indexOf(bits) < 0) throw domError('OperationError', 'AES key length must be 128, 192, or 256 bits');
  }
  var bytes = new Uint8Array(bits / 8);
  fill(bytes);
  return makeKey(secretAlgorithm(name, algorithm, bits), bytes, extractable, list);
}
function importSecret(format, data, algorithm, extractable, usages) {
  var name = canonIn(keyAlgs, algorithm);
  if (!name) throw unimplementedError(algorithm, 'importKey');
  var list = checkUsages(name, usages, false);
  var bytes;
  if (format === 'raw') {
    if (!ArrayBuffer.isView(data) && !(data instanceof ArrayBuffer)) throw new TypeError('keyData must be an ArrayBuffer or ArrayBufferView');
    var view = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);
    bytes = view.slice();
  } else {
    if (!data || typeof data !== 'object' || data.kty !== 'oct') throw domError('DataError', 'Invalid JWK "kty" Parameter');
    if (typeof data.k !== 'string') throw domError('DataError', 'Invalid keyData');
    bytes = new Uint8Array(Buffer.from(data.k, 'base64url'));
  }
  if (name !== 'HMAC' && [16, 24, 32].indexOf(bytes.length) < 0) throw domError('DataError', 'Invalid key length');
  if (!bytes.length) throw domError('DataError', 'Zero-length key is not supported');
  var alg = secretAlgorithm(name, algorithm, bytes.length * 8);
  if (format === 'jwk') {
    var want = name === 'HMAC' ? jwkHmacAlg[alg.hash.name] : 'A' + alg.length + jwkAesSuffix[name];
    if (data.ext === false && extractable) throw domError('DataError', 'JWK "ext" Parameter and extractable mismatch');
    if (data.alg !== undefined && data.alg !== want) throw domError('DataError', 'JWK "alg" does not match the requested algorithm');
    if (data.key_ops && list.some(function (u) { return data.key_ops.indexOf(u) < 0; })) throw domError('DataError', 'Key operations and usage mismatch');
  }
  return makeKey(alg, bytes, extractable, list);
}
function exportSecret(format, key) {
  if (['raw', 'jwk', 'spki', 'pkcs8'].indexOf(format) < 0) throw badFormat('exportKey', format);
  var s = keySlots.get(key);
  if (!s) throw new TypeError("Failed to execute 'exportKey' on 'SubtleCrypto': 2nd argument is not of type CryptoKey.");
  if (!s.extractable) throw domError('InvalidAccessException', 'key is not extractable');
  if (format === 'raw') return s.bytes.slice().buffer;
  if (format === 'jwk') {
    var alg = s.alg.name === 'HMAC' ? jwkHmacAlg[s.alg.hash.name] : 'A' + s.alg.length + jwkAesSuffix[s.alg.name];
    return { key_ops: s.usages.slice(), ext: true, alg: alg, kty: 'oct', k: Buffer.from(s.bytes).toString('base64url') };
  }
  throw domError('NotSupportedError', 'Unable to export ' + s.alg.name + ' secret key using ' + format + ' format');
}
// Method shorthand: no `prototype` own property, as in Node's method shape.
var subtleMethods = {
  encrypt(algorithm, key, data) { return Promise.reject(unimplementedError(algorithm, 'encrypt')); },
  decrypt(algorithm, key, data) { return Promise.reject(unimplementedError(algorithm, 'decrypt')); },
  sign(algorithm, key, data) { return Promise.reject(unimplementedError(algorithm, 'sign')); },
  verify(algorithm, key, signature, data) { return Promise.reject(unimplementedError(algorithm, 'verify')); },
  deriveBits(algorithm, baseKey) { return Promise.reject(unimplementedError(algorithm, 'deriveBits')); },
  deriveKey(algorithm, baseKey, derivedKeyType, extractable, keyUsages) { return Promise.reject(unimplementedError(algorithm, 'deriveKey')); },
  wrapKey(format, key, wrappingKey, wrapAlgorithm) { return Promise.reject(unimplementedError(wrapAlgorithm, 'wrapKey')); },
  unwrapKey(format, wrappedKey, unwrappingKey, unwrapAlgorithm, unwrappedKeyAlgorithm, extractable, keyUsages) { return Promise.reject(unimplementedError(unwrapAlgorithm, 'unwrapKey')); },
  generateKey(algorithm, extractable, keyUsages) { return settle(() => generateSecret(algorithm, extractable, keyUsages)); },
  importKey(format, keyData, algorithm, extractable, keyUsages) {
    return settle(() => {
      if (['raw', 'jwk', 'spki', 'pkcs8'].indexOf(format) < 0) throw badFormat('importKey', format);
      if (format !== 'raw' && format !== 'jwk') throw unimplementedError(algorithm, 'importKey ' + format);
      return importSecret(format, keyData, algorithm, extractable, keyUsages);
    });
  },
  exportKey(format, key) { return settle(() => exportSecret(format, key)); },
};
for (var m of Object.keys(subtleMethods)) {
  Object.defineProperty(SubtleCrypto.prototype, m, { value: ownNameLength(subtleMethods[m], m), writable: true, enumerable: true, configurable: true });
}
var subtleInstance = new SubtleCrypto(token);
Object.defineProperty(Crypto.prototype, 'subtle', { get: function () { return subtleInstance; }, enumerable: true, configurable: true });
def(globalThis, 'crypto', new Crypto(token));

