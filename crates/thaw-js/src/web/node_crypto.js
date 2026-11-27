// node:crypto (hashing, HMAC, random):
// Runs after web/crypto.js, which owns globalThis.crypto; this file only adds the node half.
function def(o, name, v) {
  Object.defineProperty(o, name, { value: v, writable: true, enumerable: false, configurable: true });
}
// Node checks each argument's type before any algorithm lookup. `received` and argTypeError follow
// web/performance.js; that copy is IIFE-scoped, so this file keeps its own.
function received(value) {
  if (value === undefined || value === null) return 'Received ' + value;
  if (typeof value === 'function') return 'Received function ' + value.name;
  if (typeof value === 'object') return 'Received an instance of ' + ((value.constructor && value.constructor.name) || 'Object');
  if (typeof value === 'string') return "Received type string ('" + value + "')";
  return 'Received type ' + typeof value + ' (' + String(value) + ')';
}
function argTypeError(name, expected, value) {
  return cryptoError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "' + name + '" argument must be ' + expected + '. ' + received(value));
}
function requireArg(ok, name, expected, value) { if (!ok) throw argTypeError(name, expected, value); }
var TYPE_NAME = 'of type string', TYPE_CALLBACK = 'of type function';
var TYPE_DATA = 'of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView';
var TYPE_INPUT = 'of type string or an instance of Buffer, TypedArray, or DataView';
var TYPE_KEY = 'of type string or an instance of ArrayBuffer, Buffer, TypedArray, DataView, KeyObject, or CryptoKey';
var TYPE_SIGNATURE = 'an instance of Buffer, TypedArray, or DataView';
function isView(value) { return ArrayBuffer.isView(value) || (typeof DataView === 'function' && value instanceof DataView); }
function isData(value) { return typeof value === 'string' || value instanceof ArrayBuffer || isView(value); }
function isKeyInput(value) {
  return isData(value) || value instanceof KeyObject || (typeof CryptoKey === 'function' && value instanceof CryptoKey);
}
// Unknown digest names: createHash keeps a plain Error; the others carry Node's codes and messages.
function plainDigestError() { return new Error('Digest method not supported'); }
function unsupportedHashDigest(algorithm) { return new Error('Digest method ' + algorithm + ' is not supported'); }
function invalidDigest(algorithm) { return cryptoError(TypeError, 'ERR_CRYPTO_INVALID_DIGEST', 'Invalid digest: ' + algorithm); }
function invalidSignDigest() { return cryptoError(TypeError, 'ERR_CRYPTO_INVALID_DIGEST', 'Invalid digest'); }
// Digests createHash/createHmac accept.
function normalizeDigestAlgorithm(algorithm, unknown) {
  var name = String(algorithm).toLowerCase().replace(/[-_]/g, '');
  if (name === 'rmd160' || name === 'ripemd') name = 'ripemd160';
  if (['md5', 'ripemd160', 'sha1', 'sha224', 'sha256', 'sha384', 'sha512', 'sha3224', 'sha3256', 'sha3384', 'sha3512'].indexOf(name) < 0) {
    throw (unknown || plainDigestError)(algorithm);
  }
  return name;
}
function hashFinalized() {
  var e = new Error('Digest already called'); e.code = 'ERR_CRYPTO_HASH_FINALIZED'; return e;
}
class Hash {
  constructor(algorithm) { requireArg(typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm); this.algorithm = normalizeDigestAlgorithm(algorithm); this._chunks = []; this._digested = false; }
  update(data, encoding) {
    if (this._digested) throw hashFinalized();
    this._chunks.push(Buffer.from(data, encoding)); return this;
  }
  digest(encoding) {
    if (this._digested) throw hashFinalized();
    this._digested = true;
    var value = Buffer.from(__thaw_crypto_hash_hex(this.algorithm, Buffer.concat(this._chunks).toString('hex')), 'hex');
    return encoding === undefined || encoding === 'buffer' ? value : value.toString(encoding);
  }
  copy() {
    if (this._digested) throw hashFinalized();
    var copied = new Hash(this.algorithm);
    copied._chunks = this._chunks.map(function (chunk) { return Buffer.from(chunk); });
    return copied;
  }
}
class Hmac extends Hash {
  constructor(algorithm, key) {
    requireArg(typeof algorithm === 'string', 'hmac', TYPE_NAME, algorithm);
    requireArg(isKeyInput(key), 'key', TYPE_KEY, key);
    super(normalizeDigestAlgorithm(algorithm, invalidDigest));
    this._key = Buffer.from(key instanceof KeyObject ? key.export() : key);
  }
  digest(encoding) {
    if (this._digested) throw hashFinalized();
    this._digested = true;
    var value = Buffer.from(__thaw_crypto_hmac_hex(this.algorithm, this._key.toString('hex'), Buffer.concat(this._chunks).toString('hex')), 'hex');
    return encoding === undefined || encoding === 'buffer' ? value : value.toString(encoding);
  }
  copy() {
    if (this._digested) throw hashFinalized();
    var copied = new Hmac(this.algorithm, this._key);
    copied._chunks = this._chunks.map(function (chunk) { return Buffer.from(chunk); });
    return copied;
  }
}
function outOfRange(name, value) {
  var e = new RangeError('The value of "' + name + '" is out of range. Received ' + value);
  e.code = 'ERR_OUT_OF_RANGE';
  return e;
}
function randomBytesSync(size) {
  size = Number(size);
  if (!Number.isInteger(size) || size < 0 || size > 0x7fffffff) throw outOfRange('size', size);
  return Buffer.from(__thaw_crypto_random_hex(size), 'hex');
}
function randomFillSync(buffer, offset, size) {
  if (!ArrayBuffer.isView(buffer)) throw new TypeError('buffer must be an ArrayBuffer view');
  offset = offset === undefined ? 0 : Number(offset);
  size = size === undefined ? buffer.byteLength - offset : Number(size);
  var bytes = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  bytes.set(randomBytesSync(size), offset);
  return buffer;
}
function randomFill(buffer, offset, size, callback) {
  if (typeof offset === 'function') { callback = offset; offset = 0; size = buffer.byteLength; }
  else if (typeof size === 'function') { callback = size; size = buffer.byteLength - Number(offset || 0); }
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  try { randomFillSync(buffer, offset || 0, size); queueMicrotask(function () { callback(null, buffer); }); }
  catch (error) { queueMicrotask(function () { callback(error); }); }
}
function randomBytes(size, callback) {
  var value = randomBytesSync(size);
  if (callback === undefined) return value;
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  queueMicrotask(function () { callback(null, value); });
}
function randomInt(min, max, callback) {
  if (max === undefined || typeof max === 'function') { callback = typeof max === 'function' ? max : callback; max = min; min = 0; }
  if (typeof min !== 'number' || typeof max !== 'number' || !Number.isSafeInteger(min) || !Number.isSafeInteger(max) || max <= min) {
    throw new RangeError('invalid random integer range');
  }
  var range = max - min;
  if (range >= 0x1000000000000) throw new RangeError('random integer range must be less than 2^48');
  var limit = 0x1000000000000 - (0x1000000000000 % range), sample;
  do {
    var bytes = randomBytesSync(6);
    sample = bytes[0] * 0x10000000000 + bytes[1] * 0x100000000 + bytes[2] * 0x1000000 + bytes[3] * 0x10000 + bytes[4] * 0x100 + bytes[5];
  } while (sample >= limit);
  var value = min + sample % range;
  if (callback === undefined) return value;
  queueMicrotask(function () { callback(null, value); });
}
function randomUUID() {
  var bytes = randomBytesSync(16); bytes[6] = bytes[6] & 0x0f | 0x40; bytes[8] = bytes[8] & 0x3f | 0x80;
  var hex = bytes.toString('hex');
  return hex.substring(0, 8) + '-' + hex.substring(8, 12) + '-' + hex.substring(12, 16) + '-' + hex.substring(16, 20) + '-' + hex.substring(20);
}
var webCrypto = globalThis.crypto;
var cryptoModule = {
  createHash: function (algorithm) { return new Hash(algorithm); },
  createHmac: function (algorithm, key) { return new Hmac(algorithm, key); },
  Hash: Hash, Hmac: Hmac,
  randomBytes: randomBytes, randomFill: randomFill, randomFillSync: randomFillSync, randomInt: randomInt, randomUUID: randomUUID,
  getRandomValues: webCrypto.getRandomValues, webcrypto: webCrypto,
    getHashes: () => ["RSA-MD5","RSA-RIPEMD160","RSA-SHA1","RSA-SHA1-2","RSA-SHA224","RSA-SHA256","RSA-SHA3-224","RSA-SHA3-256","RSA-SHA3-384","RSA-SHA3-512","RSA-SHA384","RSA-SHA512","RSA-SHA512/224","RSA-SHA512/256","RSA-SM3","blake2b512","blake2s256","id-rsassa-pkcs1-v1_5-with-sha3-224","id-rsassa-pkcs1-v1_5-with-sha3-256","id-rsassa-pkcs1-v1_5-with-sha3-384","id-rsassa-pkcs1-v1_5-with-sha3-512","md5","md5-sha1","md5WithRSAEncryption","ripemd","ripemd160","ripemd160WithRSA","rmd160","sha1","sha1WithRSAEncryption","sha224","sha224WithRSAEncryption","sha256","sha256WithRSAEncryption","sha3-224","sha3-256","sha3-384","sha3-512","sha384","sha384WithRSAEncryption","sha512","sha512-224","sha512-224WithRSAEncryption","sha512-256","sha512-256WithRSAEncryption","sha512WithRSAEncryption","shake128","shake256","sm3","sm3WithRSAEncryption","ssl3-md5","ssl3-sha1"].slice(),
    getCiphers: () => ["aes-128-cbc","aes-128-cbc-hmac-sha1","aes-128-cbc-hmac-sha256","aes-128-ccm","aes-128-cfb","aes-128-cfb1","aes-128-cfb8","aes-128-ctr","aes-128-ecb","aes-128-gcm","aes-128-ocb","aes-128-ofb","aes-128-xts","aes-192-cbc","aes-192-ccm","aes-192-cfb","aes-192-cfb1","aes-192-cfb8","aes-192-ctr","aes-192-ecb","aes-192-gcm","aes-192-ocb","aes-192-ofb","aes-256-cbc","aes-256-cbc-hmac-sha1","aes-256-cbc-hmac-sha256","aes-256-ccm","aes-256-cfb","aes-256-cfb1","aes-256-cfb8","aes-256-ctr","aes-256-ecb","aes-256-gcm","aes-256-ocb","aes-256-ofb","aes-256-xts","aes128","aes128-wrap","aes128-wrap-pad","aes192","aes192-wrap","aes192-wrap-pad","aes256","aes256-wrap","aes256-wrap-pad","aria-128-cbc","aria-128-ccm","aria-128-cfb","aria-128-cfb1","aria-128-cfb8","aria-128-ctr","aria-128-ecb","aria-128-gcm","aria-128-ofb","aria-192-cbc","aria-192-ccm","aria-192-cfb","aria-192-cfb1","aria-192-cfb8","aria-192-ctr","aria-192-ecb","aria-192-gcm","aria-192-ofb","aria-256-cbc","aria-256-ccm","aria-256-cfb","aria-256-cfb1","aria-256-cfb8","aria-256-ctr","aria-256-ecb","aria-256-gcm","aria-256-ofb","aria128","aria192","aria256","camellia-128-cbc","camellia-128-cfb","camellia-128-cfb1","camellia-128-cfb8","camellia-128-ctr","camellia-128-ecb","camellia-128-ofb","camellia-192-cbc","camellia-192-cfb","camellia-192-cfb1","camellia-192-cfb8","camellia-192-ctr","camellia-192-ecb","camellia-192-ofb","camellia-256-cbc","camellia-256-cfb","camellia-256-cfb1","camellia-256-cfb8","camellia-256-ctr","camellia-256-ecb","camellia-256-ofb","camellia128","camellia192","camellia256","chacha20","chacha20-poly1305","des-ede","des-ede-cbc","des-ede-cfb","des-ede-ecb","des-ede-ofb","des-ede3","des-ede3-cbc","des-ede3-cfb","des-ede3-cfb1","des-ede3-cfb8","des-ede3-ecb","des-ede3-ofb","des3","des3-wrap","id-aes128-CCM","id-aes128-GCM","id-aes128-wrap","id-aes128-wrap-pad","id-aes192-CCM","id-aes192-GCM","id-aes192-wrap","id-aes192-wrap-pad","id-aes256-CCM","id-aes256-GCM","id-aes256-wrap","id-aes256-wrap-pad","id-smime-alg-CMS3DESwrap"].slice(),
  hash: function (algorithm, data, outputEncoding) {
    requireArg(typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm);
    requireArg(typeof data === 'string' || isView(data), 'input', TYPE_INPUT, data);
    normalizeDigestAlgorithm(algorithm, unsupportedHashDigest);
    var value = new Hash(algorithm).update(data).digest();
    if (outputEncoding === undefined) outputEncoding = 'hex';
    return outputEncoding === 'buffer' ? value : value.toString(outputEncoding);
  }
};
// KDFs (lifted from buffer_crypto.js; same natives and messages as retired-engine).
function checkedPbkdf2Number(value, minimum, name) {
  var number = Number(value);
  if (!Number.isInteger(number) || number < minimum || number > 0xffffffff) throw new RangeError(name + ' out of range');
  return number;
}
// Node's order: password, salt, iterations, keylen, digest type; the digest name is looked up after that.
function checkPbkdf2Args(password, salt, iterations, keylen, digest) {
  requireArg(isData(password), 'password', TYPE_DATA, password);
  requireArg(isData(salt), 'salt', TYPE_DATA, salt);
  checkedPbkdf2Number(iterations, 1, 'iterations');
  checkedPbkdf2Number(keylen, 0, 'keylen');
  requireArg(typeof digest === 'string', 'digest', TYPE_NAME, digest);
}
function pbkdf2Sync(password, salt, iterations, keylen, digest) {
  checkPbkdf2Args(password, salt, iterations, keylen, digest);
  return Buffer.from(__thaw_crypto_pbkdf2_hex(
    normalizeDigestAlgorithm(digest, invalidDigest),
    Buffer.from(password).toString('hex'),
    Buffer.from(salt).toString('hex'),
    checkedPbkdf2Number(iterations, 1, 'iterations'),
    checkedPbkdf2Number(keylen, 0, 'keylen')
  ), 'hex');
}
function scryptSync(password, salt, keylen, options) {
  options = options || {};
  return Buffer.from(__thaw_crypto_scrypt_hex(
    Buffer.from(password).toString('hex'),
    Buffer.from(salt).toString('hex'),
    Number(keylen),
    Number(options.N === undefined ? 16384 : options.N),
    Number(options.r === undefined ? 8 : options.r),
    Number(options.p === undefined ? 1 : options.p)
  ), 'hex');
}
function pbkdf2(password, salt, iterations, keylen, digest, callback) {
  checkPbkdf2Args(password, salt, iterations, keylen, digest);
  if (typeof callback !== 'function') throw argTypeError('callback', TYPE_CALLBACK, callback);
  // An unknown digest throws synchronously, as in Node; only the KDF itself reports through the callback.
  var name = normalizeDigestAlgorithm(digest, invalidDigest), value;
  try { value = pbkdf2Sync(password, salt, iterations, keylen, name); } catch (error) { if (error.code === undefined && error instanceof TypeError) throw error; queueMicrotask(function () { callback(error); }); return; }
  queueMicrotask(function () { callback(null, value); });
}
function scrypt(password, salt, keylen, options, callback) {
  if (typeof options === 'function') { callback = options; options = {}; }
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  queueMicrotask(function () {
    var value;
    try { value = scryptSync(password, salt, keylen, options); }
    catch (error) { callback(error); return; }
    callback(null, value);
  });
}
// ponytail: Node returns an ArrayBuffer; a Buffer (also a Uint8Array) keeps typed callers' Buffer.from/new Uint8Array working.
function hkdfSync(digest, ikm, salt, info, keylen) {
  var key = Buffer.from(ikm);
  var hmac = function (k, d) { return new Hmac(digest, k).update(d).digest(); };
  var prk = hmac(Buffer.from(salt), key);
  var infoBuffer = Buffer.from(info), blocks = [];
  var previous = Buffer.alloc(0), length = 0;
  for (var counter = 1; length < keylen; counter++) {
    previous = hmac(prk, Buffer.concat([previous, infoBuffer, Buffer.from([counter])]));
    blocks.push(previous); length += previous.length;
  }
  return Buffer.concat(blocks).subarray(0, keylen);
}
function hkdf(digest, ikm, salt, info, keylen, callback) {
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  var value;
  try { value = hkdfSync(digest, ikm, salt, info, keylen); } catch (error) { queueMicrotask(function () { callback(error); }); return; }
  queueMicrotask(function () { callback(null, value); });
}
Object.assign(cryptoModule, { pbkdf2Sync: pbkdf2Sync, pbkdf2: pbkdf2, scryptSync: scryptSync, scrypt: scrypt, hkdfSync: hkdfSync, hkdf: hkdf });
// Ciphers (AES cbc/ecb/ctr/gcm; natives in node_crypto/cipher.rs). Lifted from buffer_crypto.js. Stream (Transform) API is a known gap.
function cryptoError(Type, code, message) { var e = new Type(message); e.code = code; return e; }
// Natives return hex, or `!reason` on failure; map the reasons to the OpenSSL-style errors Node throws.
function cipherChecked(result) {
  if (result.charCodeAt(0) === 33) {
    var reason = result.slice(1);
    if (reason === 'bad decrypt') throw Object.assign(new Error('error:1C800064:Provider routines::bad decrypt'), { library: 'Provider routines', reason: 'bad decrypt', code: 'ERR_OSSL_BAD_DECRYPT' });
    if (reason === 'wrong final block length' || reason === 'data not multiple of block length') throw Object.assign(new Error('error:1C80006B:Provider routines::wrong final block length'), { library: 'Provider routines', reason: 'wrong final block length', code: 'ERR_OSSL_WRONG_FINAL_BLOCK_LENGTH' });
    throw new Error(reason);
  }
  return Buffer.from(result, 'hex');
}
class Cipheriv {
  constructor(algorithm, key, iv, decrypt, options) {
    requireArg(typeof algorithm === 'string', 'cipher', TYPE_NAME, algorithm);
    requireArg(isKeyInput(key), 'key', TYPE_KEY, key);
    requireArg(iv === null || isData(iv), 'iv', TYPE_DATA, iv);
    var name = String(algorithm).toLowerCase().replace(/^(?:id-)?aes(\d+)$/, 'aes-$1-cbc').replace(/^id-/, '');
    var match = /^aes-(128|192|256)-(cbc|ecb|ctr|gcm)$/.exec(name);
    if (!match) throw cryptoError(Error, 'ERR_CRYPTO_UNKNOWN_CIPHER', 'Unknown cipher');
    this.algorithm = name;
    this.mode = match[2];
    this.key = Buffer.from(key instanceof KeyObject ? key.export() : key);
    if (this.key.length * 8 !== Number(match[1])) throw cryptoError(RangeError, 'ERR_CRYPTO_INVALID_KEYLEN', 'Invalid key length');
    this.iv = iv === null ? Buffer.alloc(0) : Buffer.from(iv);
    var ivOk = this.mode === 'ecb' ? this.iv.length === 0 : this.mode === 'gcm' ? this.iv.length > 0 : this.iv.length === 16;
    if (!ivOk) throw cryptoError(TypeError, 'ERR_CRYPTO_INVALID_IV', 'Invalid initialization vector');
    this.initialIv = this.iv;
    this.decrypt = decrypt;
    this.pending = Buffer.alloc(0);
    this.finished = false;
    this.autoPadding = true;
    this.processed = 0;
    this.aad = Buffer.alloc(0);
    this.ciphertext = [];
    this.tagLength = options && options.authTagLength !== undefined ? Number(options.authTagLength) : 16;
    this.tag = null;
  }
  _out(value, outputEncoding) { return outputEncoding === undefined || outputEncoding === 'buffer' ? value : value.toString(outputEncoding); }
  setAutoPadding(value) { this.autoPadding = value === undefined ? true : Boolean(value); return this; }
  setAAD(buffer, options) {
    if (this.mode !== 'gcm') throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation setAAD');
    this.aad = Buffer.concat([this.aad, Buffer.from(buffer)]);
    return this;
  }
  getAuthTag() {
    if (this.decrypt || this.mode !== 'gcm' || this.tag === null) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation getAuthTag');
    return Buffer.from(this.tag);
  }
  setAuthTag(tag) {
    if (!this.decrypt || this.mode !== 'gcm' || this.finished) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation setAuthTag');
    this.tag = Buffer.from(tag);
    return this;
  }
  update(value, inputEncoding, outputEncoding) {
    if (this.finished) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation update');
    var input = Buffer.from(value, inputEncoding);
    if (this.mode === 'ctr' || this.mode === 'gcm') {
      var output = cipherChecked(__thaw_crypto_ctr_hex(this.mode === 'gcm', this.key.toString('hex'), this.iv.toString('hex'), this.processed, input.toString('hex')));
      this.processed += input.length;
      if (this.mode === 'gcm') this.ciphertext.push(this.decrypt ? input : output);
      return this._out(output, outputEncoding);
    }
    this.pending = Buffer.concat([this.pending, input]);
    var keepLast = this.decrypt && this.autoPadding;
    var length = Math.max(0, Math.floor((this.pending.length - (keepLast ? 1 : 0)) / 16) * 16);
    if (!length) return this._out(Buffer.alloc(0), outputEncoding);
    var chunk = this.pending.subarray(0, length);
    this.pending = Buffer.from(this.pending.subarray(length));
    var blockOutput = cipherChecked(__thaw_crypto_cipher_hex(this.algorithm, this.key.toString('hex'), this.iv.toString('hex'), chunk.toString('hex'), this.decrypt, false, false));
    this.iv = Buffer.from((this.decrypt ? chunk : blockOutput).subarray(length - 16, length));
    return this._out(blockOutput, outputEncoding);
  }
  final(outputEncoding) {
    if (this.finished) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation final');
    this.finished = true;
    if (this.mode === 'ctr') return this._out(Buffer.alloc(0), outputEncoding);
    if (this.mode === 'gcm') {
      var computed = cipherChecked(__thaw_crypto_gcm_tag_hex(this.key.toString('hex'), this.iv.toString('hex'), this.aad.toString('hex'), Buffer.concat(this.ciphertext).toString('hex')));
      if (this.decrypt) {
        if (this.tag === null || this.tag.length === 0 || !computed.subarray(0, this.tag.length).equals(this.tag)) throw new Error('Unsupported state or unable to authenticate data');
      } else this.tag = computed.subarray(0, this.tagLength);
      return this._out(Buffer.alloc(0), outputEncoding);
    }
    var value = cipherChecked(__thaw_crypto_cipher_hex(this.algorithm, this.key.toString('hex'), this.iv.toString('hex'), this.pending.toString('hex'), this.decrypt, true, this.autoPadding));
    return this._out(value, outputEncoding);
  }
}
function createCipheriv(algorithm, key, iv, options) { return new Cipheriv(algorithm, key, iv, false, options); }
class Decipheriv extends Cipheriv { constructor(algorithm, key, iv, options) { super(algorithm, key, iv, true, options); } }
function createDecipheriv(algorithm, key, iv, options) { return new Decipheriv(algorithm, key, iv, options); }
// timingSafeEqual: Node's messages and codes (the check is a plain XOR loop, as in retired-engine).
function timingSafeBytes(value, name) {
  // thaw-js's DataView is not an ArrayBuffer view (ArrayBuffer.isView is false for it), so test it explicitly.
  if (ArrayBuffer.isView(value) || (typeof DataView === 'function' && value instanceof DataView)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  throw cryptoError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "' + name + '" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.');
}
function timingSafeEqual(left, right) {
  var a = timingSafeBytes(left, 'buf1'), b = timingSafeBytes(right, 'buf2');
  if (a.length !== b.length) throw cryptoError(RangeError, 'ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH', 'Input buffers must have the same byte length');
  var difference = 0;
  for (var i = 0; i < a.length; i++) difference |= a[i] ^ b[i];
  return difference === 0;
}
Object.assign(cryptoModule, { createCipheriv: createCipheriv, createDecipheriv: createDecipheriv, Cipheriv: Cipheriv, Decipheriv: Decipheriv, timingSafeEqual: timingSafeEqual });
// Asymmetric keys and signatures (natives in node_crypto/asym.rs).
// Sign/Verify side of node:crypto (lifted from buffer_crypto.js, with the same natives).
// Digest names are checked by the natives: sha1/sha256/sha384/sha512 only (RSA), ECDSA uses its curve's digest.
function normalizeSignDigestAlgorithm(algorithm) {
  return normalizeDigestAlgorithm(String(algorithm).replace(/^(rsa|ecdsa|dsa)-/i, ''), invalidSignDigest);
}
// A minimal stand-in for real Node's `crypto.KeyObject` -- thaw's own
// crypto shim has no asymmetric-key support at all (no
// `createPrivateKey`/`createPublicKey`), but real packages commonly do
// `x instanceof KeyObject` purely to tell a wrapped key apart from a
// plain string/Buffer secret -- real example: jsonwebtoken's own
// `sign.js`/`verify.js`, `secretOrPrivateKey instanceof KeyObject`.
// Without *something* named `KeyObject` reachable here, that check
// threw outright ("invalid 'instanceof' right operand" -- the
// right-hand side of `instanceof` was `undefined`), aborting every
// call regardless of the key's actual shape.
//
// `type`/`export()` mirror real `KeyObject`'s own shape closely enough
// for the *symmetric* (HMAC) case real packages actually exercise
// through this: jsonwebtoken's own fallback chain, real Node's
// `createPrivateKey(secret)` throwing on a plain HMAC secret (thaw
// doesn't implement it at all -- calling it throws "not a function",
// caught the same way) then `createSecretKey(secret)` succeeding and
// producing a `KeyObject` whose `.type` must read back `"secret"` for
// the rest of jsonwebtoken's own logic to accept it.
//
// Also backs a real asymmetric (RSA/EC/Ed25519) key now: `_material`
// for those is always a plain, unencrypted, normalized PKCS8/SPKI PEM
// string -- regardless of what format/encryption the *original* input
// was in (PEM, DER, or passphrase-protected PKCS8; see
// `parseAsymmetricKeyMaterial` and `crypto_import_key_json`,
// `asymmetric_crypto.rs`) -- reparsed fresh on every `sign`/`verify`,
// no persistent native key object. `asymmetricKeyType`/
// `asymmetricKeyDetails` are populated from the same import call
// (`namedCurve` for EC, `modulusLength`/`publicExponent` -- a
// `BigInt`, matching real Node -- for RSA), matching real
// `KeyObject`'s own fields closely enough for `x.asymmetricKeyType
// === 'rsa'`-style feature checks. `export()`
// for an asymmetric key returns the normalized PEM text as a string
// (real Node's own default `format: 'pem'`) -- `format: 'der'`/`jwk`
// output are not supported, matching this shim's existing PEM-only
// *output* scope (DER *input* is supported, see above).
function rejectEncryptedPrivateKeyOutput(encoding) {
  if (encoding && (encoding.cipher !== undefined || encoding.passphrase !== undefined)) {
    throw new TypeError('Encrypted private key output is not supported');
  }
};
class KeyObject {
  constructor(type, material, asymmetricInfo) {
    this.type = type;
    this._material = material;
    if (asymmetricInfo) {
      this.asymmetricKeyType = asymmetricInfo.keyType;
      this.asymmetricKeyDetails = asymmetricInfo.namedCurve
        ? { namedCurve: { 'P-256': 'prime256v1', 'P-384': 'secp384r1', 'P-521': 'secp521r1' }[asymmetricInfo.namedCurve] || asymmetricInfo.namedCurve }
        : asymmetricInfo.modulusLength != null
        ? { modulusLength: asymmetricInfo.modulusLength, publicExponent: BigInt(asymmetricInfo.publicExponent) }
        : {};
    }
  }
  export(options) {
    if (this.type === 'private') rejectEncryptedPrivateKeyOutput(options);
    return this.type === 'secret' ? Buffer.from(this._material) : this._material;
  }
}
// Extracts a PEM string plus its parsed key-info from whatever shape
// `createPrivateKey`/`createPublicKey`/`.sign()`/`.verify()` accepts:
// a bare PEM string/Buffer, a `{key, format, type, passphrase}`
// options object, or an existing `KeyObject` (already normalized at
// construction time -- returned as-is, no need to reimport). `format:
// 'der'` reads `key`'s raw bytes as DER instead of PEM text;
// `passphrase` decrypts a modern PKCS8 `ENCRYPTED PRIVATE KEY` (the
// legacy OpenSSL "Proc-Type: 4,ENCRYPTED" PKCS#1/SEC1 header format
// is not supported -- real Node's own modern default is PKCS8 too).
// Whatever shape/encoding the input was in, the *output* is always a
// plain, unencrypted, normalized PEM string (`crypto_import_key_json`
// re-encodes it) -- every other caller of this function only ever
// sees that.
function parseAsymmetricKeyMaterial(key) {
  if (key instanceof KeyObject) return { pem: key._material, info: {
    valid: true, keyType: key.asymmetricKeyType, isPrivate: key.type === 'private',
    namedCurve: key.asymmetricKeyDetails && key.asymmetricKeyDetails.namedCurve
  } };
  const rawKey = key && key.key instanceof KeyObject ? key.key
    : key && (typeof key.key === 'string' || key.key instanceof Buffer || ArrayBuffer.isView(key.key)) ? key.key
    : key;
  if (rawKey instanceof KeyObject) return parseAsymmetricKeyMaterial(rawKey);
  const material = typeof rawKey === 'string' ? Buffer.from(rawKey)
    : rawKey instanceof Buffer || ArrayBuffer.isView(rawKey) ? Buffer.from(rawKey)
    : null;
  if (material === null) throw new TypeError('Only a PEM/DER string/Buffer or a KeyObject is supported');
  const isDer = Boolean(key && key.format === 'der');
  const passphrase = (key && key.passphrase) || '';
  const info = JSON.parse(__thaw_crypto_import_key_json(material.toString('hex'), isDer, String(passphrase)));
  if (!info.valid) throw new Error('Invalid or unsupported key (only RSA/EC/Ed25519 PEM or DER keys are supported)');
  return { pem: info.pem, info };
};
function createSecretKey(key) { return new KeyObject('secret', Buffer.from(key)); }
// Real Node's `createPrivateKey`/`createPublicKey`, now backed by
// real RSA/EC/Ed25519 PEM *and DER* parsing, including passphrase-
// protected PKCS8 private keys (`asymmetric_crypto.rs`,
// `parseAsymmetricKeyMaterial`). `createPublicKey` deriving a public
// key *from* a private key/PEM (real Node supports this) is still
// not implemented -- pass the public key PEM directly instead.
const createPrivateKey = key => {
  const { pem, info } = parseAsymmetricKeyMaterial(key);
  if (!info.isPrivate) throw new Error('Expected a private key');
  return new KeyObject('private', pem, info);
};
const createPublicKey = key => {
  if (key instanceof KeyObject) {
    if (key.type === 'public') return key;
    throw new Error('Deriving a public key from a private key is not supported here -- pass the public key PEM directly');
  }
  const { pem, info } = parseAsymmetricKeyMaterial(key);
  if (info.isPrivate) throw new Error('Deriving a public key from a private key is not supported here -- pass the public key PEM directly');
  return new KeyObject('public', pem, info);
};
const RSA_PKCS1_PADDING = 1;
const RSA_PKCS1_PSS_PADDING = 6;
const RSA_PKCS1_OAEP_PADDING = 4;
// `createSign(algorithm).update(data).sign(privateKey)` /
// `createVerify(algorithm).update(data).verify(publicKey, signature)`
// -- `algorithm` is a *digest* name (real Node accepts an
// `"RSA-SHA256"`-style prefixed form too, see
// `normalizeSignDigestAlgorithm`); the actual RSA-vs-ECDSA choice
// always comes from the key passed to `.sign()`/`.verify()`, not from
// this string. ECDSA signatures are DER-encoded, matching real Node's
// own default (`dsaEncoding: 'der'`) -- no `ieee-p1363` raw-format
// support (out of scope; real packages that need JOSE's raw r||s
// format, e.g. jsonwebtoken's own `ecdsa-sig-formatter` dependency,
// already convert DER<->raw entirely in JS, so this needs no special
// handling here).
class Sign {
  constructor(algorithm) { requireArg(typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm); this.algorithm = normalizeSignDigestAlgorithm(algorithm); this._chunks = []; }
  update(data, encoding) { this._chunks.push(Buffer.from(data, encoding)); return this; }
  sign(privateKey, outputEncoding) {
    const { pem } = parseAsymmetricKeyMaterial(privateKey);
    const data = Buffer.concat(this._chunks).toString('hex');
    const usePss = privateKey && privateKey.padding === RSA_PKCS1_PSS_PADDING;
    const signer = usePss ? __thaw_crypto_asymmetric_sign_pss_hex : __thaw_crypto_asymmetric_sign_hex;
    const value = Buffer.from(usePss ? signer(this.algorithm, pem, data, privateKey.saltLength) : signer(this.algorithm, pem, data), 'hex');
    return outputEncoding === undefined ? value : value.toString(outputEncoding);
  }
}
class Verify {
  constructor(algorithm) { requireArg(typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm); this.algorithm = normalizeSignDigestAlgorithm(algorithm); this._chunks = []; }
  update(data, encoding) { this._chunks.push(Buffer.from(data, encoding)); return this; }
  verify(publicKey, signature, signatureEncoding) {
    const { pem } = parseAsymmetricKeyMaterial(publicKey);
    const data = Buffer.concat(this._chunks).toString('hex');
    const signatureHex = Buffer.from(signature, signatureEncoding).toString('hex');
    const usePss = publicKey && publicKey.padding === RSA_PKCS1_PSS_PADDING;
    const verifier = usePss ? __thaw_crypto_asymmetric_verify_pss : __thaw_crypto_asymmetric_verify;
    return usePss ? verifier(this.algorithm, pem, data, signatureHex, publicKey.saltLength) : verifier(this.algorithm, pem, data, signatureHex);
  }
}
const createSign = algorithm => new Sign(algorithm);
const createVerify = algorithm => new Verify(algorithm);
// The top-level forms take a null/undefined algorithm (the default digest) and check data and signature types
// before the algorithm name.
const sign = (algorithm, data, key) => {
  requireArg(algorithm == null || typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm);
  requireArg(isData(data), 'data', TYPE_DATA, data);
  return new Sign(algorithm || 'sha256').update(data).sign(key);
};
const verify = (algorithm, data, key, signature) => {
  requireArg(algorithm == null || typeof algorithm === 'string', 'algorithm', TYPE_NAME, algorithm);
  requireArg(isData(data), 'data', TYPE_DATA, data);
  requireArg(isView(signature), 'signature', TYPE_SIGNATURE, signature);
  return new Verify(algorithm || 'sha256').update(data).verify(key, signature);
};
  // `publicEncrypt(key, buffer)` / `privateDecrypt(key, buffer)` --
  // real Node's default padding for *these* (unlike `Sign`/`Verify`'s
  // own PKCS1v15 default) is OAEP, defaulting to a SHA-1 `oaepHash`
  // (OpenSSL's own long-standing default) unless `padding:
  // RSA_PKCS1_PADDING` is explicitly requested. RSA only -- an EC/
  // Ed25519 key throws a clear error, matching real Node.
  const resolveEncryptionPadding = key => (key && key.padding === RSA_PKCS1_PADDING)
    ? '' : normalizeDigestAlgorithm((key && key.oaepHash) || 'sha1');
  const publicEncrypt = (key, buffer) => {
    const { pem } = parseAsymmetricKeyMaterial(key);
    const oaepDigest = resolveEncryptionPadding(key);
    return Buffer.from(__thaw_crypto_asymmetric_encrypt_hex(pem, oaepDigest, Buffer.from(buffer).toString('hex')), 'hex');
  };
  const privateDecrypt = (key, buffer) => {
    const { pem } = parseAsymmetricKeyMaterial(key);
    const oaepDigest = resolveEncryptionPadding(key);
    return Buffer.from(__thaw_crypto_asymmetric_decrypt_hex(pem, oaepDigest, Buffer.from(buffer).toString('hex')), 'hex');
  };
  // `privateEncrypt(key, buffer)` / `publicDecrypt(key, buffer)` --
  // Node's rarer raw-RSA pair (real use: producing something only the
  // matching *public* key can recover, not confidentiality). Always
  // PKCS#1 v1.5 padding (real Node's own default and only supported
  // choice for this direction -- OAEP is not defined for it, and
  // there is no `oaepHash`/`padding` option to resolve). RSA only,
  // same as `publicEncrypt`/`privateDecrypt` above.
  const privateEncrypt = (key, buffer) => {
    const { pem } = parseAsymmetricKeyMaterial(key);
    return Buffer.from(__thaw_crypto_asymmetric_private_encrypt_hex(pem, Buffer.from(buffer).toString('hex')), 'hex');
  };
  const publicDecrypt = (key, buffer) => {
    const { pem } = parseAsymmetricKeyMaterial(key);
    return Buffer.from(__thaw_crypto_asymmetric_public_decrypt_hex(pem, Buffer.from(buffer).toString('hex')), 'hex');
  };
  // A PEM block is just base64(DER) wrapped in `-----BEGIN/END-----`
  // header/footer lines with fixed-width line wrapping -- no crypto
  // needed to recover the raw DER bytes, just strip the non-base64
  // lines and decode.
  const pemToDer = pem => Buffer.from(
    pem.split('\n').filter(line => line && !line.startsWith('-----')).join(''),
    'base64'
  );
  // `generateKeyPairSync(type, options)` / `generateKeyPair(type,
  // options, callback)` -- `type` is `'rsa'`/`'ec'`/`'ed25519'`;
  // `options.modulusLength` (RSA) or `options.namedCurve` (EC, either
  // Node/JOSE's own names or the common OpenSSL aliases) picks the key
  // size/curve; `options.publicExponent` (RSA only, default 65537)
  // picks a custom public exponent. `options.privateKeyEncoding`/
  // `publicKeyEncoding` (`{type, format}`) control the return shape per
  // half: omitted -> a real `KeyObject` (matching real Node's own
  // default, always normalized as PKCS8/SPKI internally regardless of
  // `type` below); `format: 'pem'` -> the PEM string as-is; `format:
  // 'der'` -> raw DER bytes as a `Buffer`. `type` is `'pkcs8'`/`'spki'`
  // (the default) or `'pkcs1'` (RSA)/`'sec1'` (EC) for the native
  // format real OpenSSL itself would produce; Ed25519 has no PKCS1/SEC1
  // equivalent (RSA/EC only, matching real Node).
  const encodeGeneratedKeyHalf = (pem, isPrivate, encoding) => {
    if (!encoding) {
      const info = JSON.parse(__thaw_crypto_import_key_json(Buffer.from(pem).toString('hex'), false, ''));
      return new KeyObject(isPrivate ? 'private' : 'public', pem, info);
    }
    return encoding.format === 'der' ? pemToDer(pem) : pem;
  };
  const generateKeyPairSync = (type, options) => {
    options = options || {};
    const arg = type === 'rsa' ? String(options.modulusLength)
      : type === 'ec' ? options.namedCurve
      : '';
    const publicExponent = type === 'rsa' && options.publicExponent != null ? String(options.publicExponent) : '';
    const privateEncoding = options.privateKeyEncoding;
    const publicEncoding = options.publicKeyEncoding;
    rejectEncryptedPrivateKeyOutput(privateEncoding);
    const privateType = (privateEncoding && privateEncoding.type) || '';
    const publicType = (publicEncoding && publicEncoding.type) || '';
    const generated = JSON.parse(__thaw_crypto_generate_key_pair_json(type, arg || '', publicExponent, privateType, publicType));
    return {
      publicKey: encodeGeneratedKeyHalf(generated.publicPem, false, publicEncoding),
      privateKey: encodeGeneratedKeyHalf(generated.privatePem, true, privateEncoding)
    };
  };
  const generateKeyPair = (type, options, callback) => {
    if (typeof options === 'function') { callback = options; options = {}; }
    queueMicrotask(() => {
      let result;
      try { result = generateKeyPairSync(type, options); }
      catch (error) { callback(error); return; }
      callback(null, result.publicKey, result.privateKey);
    });
  };
  // `crypto.diffieHellman({privateKey, publicKey})` -- the modern,
  // `KeyObject`-based one-shot key-agreement API real Node itself
  // recommends over the classic `ECDH` class below. Both keys must be
  // the same key-agreement type: a matching EC curve, or both X25519
  // (`generateKeyPairSync('x25519')`); RSA/Ed25519 keys aren't
  // key-agreement keys and are rejected (see `crypto_diffie_hellman_
  // hex`, `asymmetric_crypto.rs`).
  const diffieHellman = ({ privateKey, publicKey }) => {
    const { pem: privatePem } = parseAsymmetricKeyMaterial(privateKey);
    const { pem: publicPem } = parseAsymmetricKeyMaterial(publicKey);
    return Buffer.from(__thaw_crypto_diffie_hellman_hex(privatePem, publicPem), 'hex');
  };
  // `crypto.createECDH(curveName)` -- the classic, raw-byte EC-only
  // key-agreement API (no `KeyObject`/PEM involved at all, unlike
  // `diffieHellman` above -- it exchanges raw SEC1 point/scalar bytes
  // end to end, matching real Node exactly). `curveName` accepts both
  // Node/JOSE's own names and the common OpenSSL aliases (see
  // `normalize_curve_name`, `asymmetric_crypto.rs`). X25519 has no
  // classic `ECDH` form in real Node either -- use `diffieHellman` +
  // `generateKeyPairSync('x25519')` for that.
  class ECDH {
    constructor(curveName) { this.curveName = curveName; }
    generateKeys(encoding, format) {
      const generated = JSON.parse(__thaw_crypto_ecdh_generate_keys_hex(this.curveName));
      this._privateKeyHex = generated.privateKeyHex;
      this._publicKeyHex = generated.publicKeyHex;
      return this.getPublicKey(encoding, format);
    }
    setPrivateKey(privateKey, encoding) {
      this._privateKeyHex = Buffer.from(privateKey, encoding).toString('hex');
      this._publicKeyHex = __thaw_crypto_ecdh_public_from_private_hex(this.curveName, this._privateKeyHex);
    }
    getPrivateKey(encoding) {
      const value = Buffer.from(this._privateKeyHex, 'hex');
      return !encoding ? value : value.toString(encoding);
    }
    getPublicKey(encoding, format = 'uncompressed') {
      const value = Buffer.from(this._publicKeyHex, 'hex');
      if (format !== 'uncompressed') {
        if (format !== 'compressed' && format !== 'hybrid') throw new TypeError('invalid public key format');
        const coordinateLength = (value.length - 1) / 2;
        const odd = value[value.length - 1] & 1;
        value[0] = (format === 'compressed' ? 2 : 6) + odd;
        if (format === 'compressed') return !encoding ? value.subarray(0, coordinateLength + 1) : value.subarray(0, coordinateLength + 1).toString(encoding);
      }
      return !encoding ? value : value.toString(encoding);
    }
    computeSecret(otherPublicKey, inputEncoding, outputEncoding) {
      const publicKeyHex = Buffer.from(otherPublicKey, inputEncoding).toString('hex');
      const value = Buffer.from(
        __thaw_crypto_ecdh_compute_secret_hex(this.curveName, this._privateKeyHex, publicKeyHex),
        'hex'
      );
      return !outputEncoding ? value : value.toString(outputEncoding);
    }
  }
  const createECDH = curveName => new ECDH(curveName);
// crypto.ECDH.convertKey: re-encode an EC public point (compressed/uncompressed/hybrid) with BigInt field math.
// ponytail: the five curves node:crypto's ECDH offers by default; others throw ERR_CRYPTO_INVALID_CURVE.
var P256 = (1n << 256n) - (1n << 224n) + (1n << 192n) + (1n << 96n) - 1n, P384 = (1n << 384n) - (1n << 128n) - (1n << 96n) + (1n << 32n) - 1n, P521 = (1n << 521n) - 1n, K256 = (1n << 256n) - (1n << 32n) - 977n;
var ecCurves = {
  prime256v1: { p: P256, a: P256 - 3n, b: 0x5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604bn, size: 32 },
  secp384r1: { p: P384, a: P384 - 3n, b: 0xb3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aefn, size: 48 },
  secp521r1: { p: P521, a: P521 - 3n, b: 0x0051953eb9618e1c9a1f929a21a0b68540eea2da725b99b315f3b8b489918ef109e156193951ec7e937b1652c0bd3bb1bf073573df883d2c34f1ef451fd46b503f00n, size: 66 },
  secp256k1: { p: K256, a: 0n, b: 7n, size: 32 }
};
ecCurves.secp256r1 = ecCurves.prime256v1;
function powMod(base, exp, mod) { var r = 1n; base %= mod; while (exp > 0n) { if (exp & 1n) r = r * base % mod; base = base * base % mod; exp >>= 1n; } return r; }
function convertEcKey(key, curve, inputEncoding, outputEncoding, format) {
  requireArg(typeof key === 'string' || key instanceof ArrayBuffer || isView(key), 'key', 'of type string or an instance of ArrayBuffer, Buffer, TypedArray, or DataView', key);
  var c = ecCurves[String(curve)];
  if (!c) throw cryptoError(TypeError, 'ERR_CRYPTO_INVALID_CURVE', 'Invalid EC curve name');
  if (format === undefined) format = 'uncompressed';
  if (format !== 'uncompressed' && format !== 'compressed' && format !== 'hybrid') throw cryptoError(TypeError, 'ERR_CRYPTO_ECDH_INVALID_FORMAT', 'Invalid ECDH format: ' + format);
  var raw = typeof key === 'string' ? Buffer.from(key, inputEncoding) : Buffer.from(key instanceof ArrayBuffer ? new Uint8Array(key) : new Uint8Array(key.buffer, key.byteOffset, key.byteLength));
  var bad = function () { return cryptoError(Error, 'ERR_CRYPTO_OPERATION_FAILED', 'Failed to convert Buffer to EC_POINT'); };
  var n = c.size, hex = function (b) { return BigInt('0x' + (b.length ? b.toString('hex') : '0')); }, x, y;
  if ((raw[0] === 2 || raw[0] === 3) && raw.length === n + 1) {
    x = hex(raw.subarray(1));
    if (x >= c.p) throw bad();
    var rhs = (x * x * x + c.a * x + c.b) % c.p; y = powMod(rhs, (c.p + 1n) / 4n, c.p);
    if (y * y % c.p !== rhs) throw bad();
    if ((y & 1n) !== BigInt(raw[0] & 1)) y = c.p - y;
  } else if ((raw[0] === 4 || raw[0] === 6 || raw[0] === 7) && raw.length === 2 * n + 1) {
    x = hex(raw.subarray(1, n + 1)); y = hex(raw.subarray(n + 1));
    if (raw[0] !== 4 && (y & 1n) !== BigInt(raw[0] & 1)) throw bad();
  } else throw bad();
  if (x >= c.p || y >= c.p || (y * y - (x * x * x + c.a * x + c.b)) % c.p !== 0n) throw bad();
  var pad = function (v) { return Buffer.from(v.toString(16).padStart(2 * n, '0'), 'hex'); };
  var out = format === 'compressed' ? Buffer.concat([Buffer.from([2 + Number(y & 1n)]), pad(x)])
    : Buffer.concat([Buffer.from([format === 'hybrid' ? 6 + Number(y & 1n) : 4]), pad(x), pad(y)]);
  return outputEncoding ? out.toString(outputEncoding) : out;
}
def(ECDH, 'convertKey', function convertKey(key, curve, inputEncoding, outputEncoding, format) { return convertEcKey(key, curve, inputEncoding, outputEncoding, format); });
// crypto.KeyObject.from(cryptoKey): only secret CryptoKeys exist in thaw (web/crypto.js).
def(KeyObject, 'from', function from(key) {
  if (!(typeof CryptoKey === 'function' && key instanceof CryptoKey)) throw argTypeError('key', 'an instance of CryptoKey', key);
  var bytes = globalThis.__thaw_cryptokey_secret(key);
  if (bytes === undefined) throw cryptoError(TypeError, 'ERR_INVALID_ARG_VALUE', "The argument 'key' is not a supported CryptoKey");
  return new KeyObject('secret', Buffer.from(bytes));
});
// crypto.Certificate: SPKAC (signed public key and challenge) helpers over a minimal DER reader.
function derRead(buf, off) {
  var tag = buf[off], len = buf[off + 1], at = off + 2;
  if (len & 0x80) { var count = len & 0x7f; len = 0; for (var i = 0; i < count; i++) len = len * 256 + buf[at++]; }
  if (tag === undefined || at + len > buf.length) throw new Error('bad DER');
  return { tag: tag, start: off, body: at, end: at + len };
}
function spkacParts(spkac, encoding) {
  requireArg(typeof spkac === 'string' || spkac instanceof ArrayBuffer || isView(spkac), 'spkac', TYPE_DATA, spkac);
  var text = typeof spkac === 'string' ? Buffer.from(spkac, encoding) : Buffer.from(spkac instanceof ArrayBuffer ? new Uint8Array(spkac) : new Uint8Array(spkac.buffer, spkac.byteOffset, spkac.byteLength));
  try {
    var der = Buffer.from(text.toString('latin1').trim(), 'base64');
    var outer = derRead(der, 0), pkac = derRead(der, outer.body), spki = derRead(der, pkac.body), challenge = derRead(der, spki.end);
    var alg = derRead(der, pkac.end), oid = derRead(der, alg.body), sig = derRead(der, alg.end);
    return { der: der, pkac: der.subarray(pkac.start, pkac.end), spki: der.subarray(spki.start, spki.end), challenge: der.subarray(challenge.body, challenge.end), oid: der.subarray(oid.body, oid.end).toString('hex'), sig: der.subarray(sig.body + 1, sig.end) };
  } catch (_) { return null; }
}
var spkacHash = { '2a864886f70d010104': 'md5', '2a864886f70d010105': 'sha1', '2a864886f70d01010b': 'sha256', '2a864886f70d01010c': 'sha384', '2a864886f70d01010d': 'sha512' };
function Certificate() { if (!(this instanceof Certificate)) return new Certificate(); }
def(Certificate, 'exportChallenge', function exportChallenge(spkac, encoding) { var p = spkacParts(spkac, encoding); return p ? Buffer.from(p.challenge) : Buffer.alloc(0); });
def(Certificate, 'exportPublicKey', function exportPublicKey(spkac, encoding) {
  var p = spkacParts(spkac, encoding); if (!p) return Buffer.alloc(0);
  var b64 = p.spki.toString('base64').replace(/(.{64})/g, '$1\n').replace(/\n$/, '');
  return Buffer.from('-----BEGIN PUBLIC KEY-----\n' + b64 + '\n-----END PUBLIC KEY-----\n');
});
def(Certificate, 'verifySpkac', function verifySpkac(spkac, encoding) {
  var p = spkacParts(spkac, encoding); if (!p || !spkacHash[p.oid]) return false;
  try { return new Verify(spkacHash[p.oid]).update(p.pkac).verify(Certificate.exportPublicKey(spkac, encoding), p.sig); } catch (_) { return false; }
});
for (var certMethod of ['exportChallenge', 'exportPublicKey', 'verifySpkac']) def(Certificate.prototype, certMethod, Certificate[certMethod]);
// crypto.checkPrime / generatePrime / DiffieHellman / getCipherInfo / generateKey, on BigInt (Miller-Rabin).
// ponytail: generatePrime/createDiffieHellman(bits) search with Miller-Rabin in JS; fine for the sizes tests use (<= ~1024 bits), slow beyond.
var MODP_PRIMES = {"modp1":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a63a3620ffffffffffffffff","modp2":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece65381ffffffffffffffff","modp5":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca237327ffffffffffffffff","modp14":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aacaa68ffffffffffffffff","modp15":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a93ad2caffffffffffffffff","modp16":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c934063199ffffffffffffffff","modp17":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c93402849236c3fab4d27c7026c1d4dcb2602646dec9751e763dba37bdf8ff9406ad9e530ee5db382f413001aeb06a53ed9027d831179727b0865a8918da3edbebcf9b14ed44ce6cbaced4bb1bdb7f1447e6cc254b332051512bd7af426fb8f401378cd2bf5983ca01c64b92ecf032ea15d1721d03f482d7ce6e74fef6d55e702f46980c82b5a84031900b1c9e59e7c97fbec7e8f323a97a7e36cc88be0f1d45b7ff585ac54bd407b22b4154aacc8f6d7ebf48e1d814cc5ed20f8037e0a79715eef29be32806a1d58bb7c5da76f550aa3d8a1fbff0eb19ccb1a313d55cda56c9ec2ef29632387fe8d76e3c0468043e8f663f4860ee12bf2d5b0b7474d6e694f91e6dcc4024ffffffffffffffff","modp18":"ffffffffffffffffc90fdaa22168c234c4c6628b80dc1cd129024e088a67cc74020bbea63b139b22514a08798e3404ddef9519b3cd3a431b302b0a6df25f14374fe1356d6d51c245e485b576625e7ec6f44c42e9a637ed6b0bff5cb6f406b7edee386bfb5a899fa5ae9f24117c4b1fe649286651ece45b3dc2007cb8a163bf0598da48361c55d39a69163fa8fd24cf5f83655d23dca3ad961c62f356208552bb9ed529077096966d670c354e4abc9804f1746c08ca18217c32905e462e36ce3be39e772c180e86039b2783a2ec07a28fb5c55df06f4c52c9de2bcbf6955817183995497cea956ae515d2261898fa051015728e5a8aaac42dad33170d04507a33a85521abdf1cba64ecfb850458dbef0a8aea71575d060c7db3970f85a6e1e4c7abf5ae8cdb0933d71e8c94e04a25619dcee3d2261ad2ee6bf12ffa06d98a0864d87602733ec86a64521f2b18177b200cbbe117577a615d6c770988c0bad946e208e24fa074e5ab3143db5bfce0fd108e4b82d120a92108011a723c12a787e6d788719a10bdba5b2699c327186af4e23c1a946834b6150bda2583e9ca2ad44ce8dbbbc2db04de8ef92e8efc141fbecaa6287c59474e6bc05d99b2964fa090c3a2233ba186515be7ed1f612970cee2d7afb81bdd762170481cd0069127d5b05aa993b4ea988d8fddc186ffb7dc90a6c08f4df435c93402849236c3fab4d27c7026c1d4dcb2602646dec9751e763dba37bdf8ff9406ad9e530ee5db382f413001aeb06a53ed9027d831179727b0865a8918da3edbebcf9b14ed44ce6cbaced4bb1bdb7f1447e6cc254b332051512bd7af426fb8f401378cd2bf5983ca01c64b92ecf032ea15d1721d03f482d7ce6e74fef6d55e702f46980c82b5a84031900b1c9e59e7c97fbec7e8f323a97a7e36cc88be0f1d45b7ff585ac54bd407b22b4154aacc8f6d7ebf48e1d814cc5ed20f8037e0a79715eef29be32806a1d58bb7c5da76f550aa3d8a1fbff0eb19ccb1a313d55cda56c9ec2ef29632387fe8d76e3c0468043e8f663f4860ee12bf2d5b0b7474d6e694f91e6dbe115974a3926f12fee5e438777cb6a932df8cd8bec4d073b931ba3bc832b68d9dd300741fa7bf8afc47ed2576f6936ba424663aab639c5ae4f5683423b4742bf1c978238f16cbe39d652de3fdb8befc848ad922222e04a4037c0713eb57a81a23f0c73473fc646cea306b4bcbc8862f8385ddfa9d4b7fa2c087e879683303ed5bdd3a062b3cf5b3a278a66d2a13f83f44f82ddf310ee074ab6a364597e899a0255dc164f31cc50846851df9ab48195ded7ea1b1d510bd7ee74d73faf36bc31ecfa268359046f4eb879f924009438b481c6cd7889a002ed5ee382bc9190da6fc026e479558e4475677e9aa9e3050e2765694dfc81f56e880b96e7160c980dd98edd3dfffffffffffffffff"};
var CIPHER_INFO = {"aes-128-cbc":[419,"cbc",16,16,16,"aes-128-cbc"],"aes-128-cbc-hmac-sha1":[916,"cbc",16,16,16,"aes-128-cbc-hmac-sha1"],"aes-128-cbc-hmac-sha256":[948,"cbc",16,16,16,"aes-128-cbc-hmac-sha256"],"aes-128-ccm":[896,"ccm",1,12,16,"id-aes128-ccm"],"aes-128-cfb":[421,"cfb",1,16,16,"aes-128-cfb"],"aes-128-cfb1":[650,"cfb",1,16,16,"aes-128-cfb1"],"aes-128-cfb8":[653,"cfb",1,16,16,"aes-128-cfb8"],"aes-128-ctr":[904,"ctr",1,16,16,"aes-128-ctr"],"aes-128-ecb":[418,"ecb",16,null,16,"aes-128-ecb"],"aes-128-gcm":[895,"gcm",1,12,16,"id-aes128-gcm"],"aes-128-ocb":[958,"ocb",16,12,16,"aes-128-ocb"],"aes-128-ofb":[420,"ofb",1,16,16,"aes-128-ofb"],"aes-128-xts":[913,"xts",1,16,32,"aes-128-xts"],"aes-192-cbc":[423,"cbc",16,16,24,"aes-192-cbc"],"aes-192-ccm":[899,"ccm",1,12,24,"id-aes192-ccm"],"aes-192-cfb":[425,"cfb",1,16,24,"aes-192-cfb"],"aes-192-cfb1":[651,"cfb",1,16,24,"aes-192-cfb1"],"aes-192-cfb8":[654,"cfb",1,16,24,"aes-192-cfb8"],"aes-192-ctr":[905,"ctr",1,16,24,"aes-192-ctr"],"aes-192-ecb":[422,"ecb",16,null,24,"aes-192-ecb"],"aes-192-gcm":[898,"gcm",1,12,24,"id-aes192-gcm"],"aes-192-ocb":[959,"ocb",16,12,24,"aes-192-ocb"],"aes-192-ofb":[424,"ofb",1,16,24,"aes-192-ofb"],"aes-256-cbc":[427,"cbc",16,16,32,"aes-256-cbc"],"aes-256-cbc-hmac-sha1":[918,"cbc",16,16,32,"aes-256-cbc-hmac-sha1"],"aes-256-cbc-hmac-sha256":[950,"cbc",16,16,32,"aes-256-cbc-hmac-sha256"],"aes-256-ccm":[902,"ccm",1,12,32,"id-aes256-ccm"],"aes-256-cfb":[429,"cfb",1,16,32,"aes-256-cfb"],"aes-256-cfb1":[652,"cfb",1,16,32,"aes-256-cfb1"],"aes-256-cfb8":[655,"cfb",1,16,32,"aes-256-cfb8"],"aes-256-ctr":[906,"ctr",1,16,32,"aes-256-ctr"],"aes-256-ecb":[426,"ecb",16,null,32,"aes-256-ecb"],"aes-256-gcm":[901,"gcm",1,12,32,"id-aes256-gcm"],"aes-256-ocb":[960,"ocb",16,12,32,"aes-256-ocb"],"aes-256-ofb":[428,"ofb",1,16,32,"aes-256-ofb"],"aes-256-xts":[914,"xts",1,16,64,"aes-256-xts"],"aes128":[419,"cbc",16,16,16,"aes-128-cbc"],"aes128-wrap":[788,"wrap",8,8,16,"id-aes128-wrap"],"aes128-wrap-pad":[897,"wrap",8,4,16,"id-aes128-wrap-pad"],"aes192":[423,"cbc",16,16,24,"aes-192-cbc"],"aes192-wrap":[789,"wrap",8,8,24,"id-aes192-wrap"],"aes192-wrap-pad":[900,"wrap",8,4,24,"id-aes192-wrap-pad"],"aes256":[427,"cbc",16,16,32,"aes-256-cbc"],"aes256-wrap":[790,"wrap",8,8,32,"id-aes256-wrap"],"aes256-wrap-pad":[903,"wrap",8,4,32,"id-aes256-wrap-pad"],"aria-128-cbc":[1066,"cbc",16,16,16,"aria-128-cbc"],"aria-128-ccm":[1120,"ccm",1,12,16,"aria-128-ccm"],"aria-128-cfb":[1067,"cfb",1,16,16,"aria-128-cfb"],"aria-128-cfb1":[1080,"cfb",1,16,16,"aria-128-cfb1"],"aria-128-cfb8":[1083,"cfb",1,16,16,"aria-128-cfb8"],"aria-128-ctr":[1069,"ctr",1,16,16,"aria-128-ctr"],"aria-128-ecb":[1065,"ecb",16,null,16,"aria-128-ecb"],"aria-128-gcm":[1123,"gcm",1,12,16,"aria-128-gcm"],"aria-128-ofb":[1068,"ofb",1,16,16,"aria-128-ofb"],"aria-192-cbc":[1071,"cbc",16,16,24,"aria-192-cbc"],"aria-192-ccm":[1121,"ccm",1,12,24,"aria-192-ccm"],"aria-192-cfb":[1072,"cfb",1,16,24,"aria-192-cfb"],"aria-192-cfb1":[1081,"cfb",1,16,24,"aria-192-cfb1"],"aria-192-cfb8":[1084,"cfb",1,16,24,"aria-192-cfb8"],"aria-192-ctr":[1074,"ctr",1,16,24,"aria-192-ctr"],"aria-192-ecb":[1070,"ecb",16,null,24,"aria-192-ecb"],"aria-192-gcm":[1124,"gcm",1,12,24,"aria-192-gcm"],"aria-192-ofb":[1073,"ofb",1,16,24,"aria-192-ofb"],"aria-256-cbc":[1076,"cbc",16,16,32,"aria-256-cbc"],"aria-256-ccm":[1122,"ccm",1,12,32,"aria-256-ccm"],"aria-256-cfb":[1077,"cfb",1,16,32,"aria-256-cfb"],"aria-256-cfb1":[1082,"cfb",1,16,32,"aria-256-cfb1"],"aria-256-cfb8":[1085,"cfb",1,16,32,"aria-256-cfb8"],"aria-256-ctr":[1079,"ctr",1,16,32,"aria-256-ctr"],"aria-256-ecb":[1075,"ecb",16,null,32,"aria-256-ecb"],"aria-256-gcm":[1125,"gcm",1,12,32,"aria-256-gcm"],"aria-256-ofb":[1078,"ofb",1,16,32,"aria-256-ofb"],"aria128":[1066,"cbc",16,16,16,"aria-128-cbc"],"aria192":[1071,"cbc",16,16,24,"aria-192-cbc"],"aria256":[1076,"cbc",16,16,32,"aria-256-cbc"],"camellia-128-cbc":[751,"cbc",16,16,16,"camellia-128-cbc"],"camellia-128-cfb":[757,"cfb",1,16,16,"camellia-128-cfb"],"camellia-128-cfb1":[760,"cfb",1,16,16,"camellia-128-cfb1"],"camellia-128-cfb8":[763,"cfb",1,16,16,"camellia-128-cfb8"],"camellia-128-ctr":[963,"ctr",1,16,16,"camellia-128-ctr"],"camellia-128-ecb":[754,"ecb",16,null,16,"camellia-128-ecb"],"camellia-128-ofb":[766,"ofb",1,16,16,"camellia-128-ofb"],"camellia-192-cbc":[752,"cbc",16,16,24,"camellia-192-cbc"],"camellia-192-cfb":[758,"cfb",1,16,24,"camellia-192-cfb"],"camellia-192-cfb1":[761,"cfb",1,16,24,"camellia-192-cfb1"],"camellia-192-cfb8":[764,"cfb",1,16,24,"camellia-192-cfb8"],"camellia-192-ctr":[967,"ctr",1,16,24,"camellia-192-ctr"],"camellia-192-ecb":[755,"ecb",16,null,24,"camellia-192-ecb"],"camellia-192-ofb":[767,"ofb",1,16,24,"camellia-192-ofb"],"camellia-256-cbc":[753,"cbc",16,16,32,"camellia-256-cbc"],"camellia-256-cfb":[759,"cfb",1,16,32,"camellia-256-cfb"],"camellia-256-cfb1":[762,"cfb",1,16,32,"camellia-256-cfb1"],"camellia-256-cfb8":[765,"cfb",1,16,32,"camellia-256-cfb8"],"camellia-256-ctr":[971,"ctr",1,16,32,"camellia-256-ctr"],"camellia-256-ecb":[756,"ecb",16,null,32,"camellia-256-ecb"],"camellia-256-ofb":[768,"ofb",1,16,32,"camellia-256-ofb"],"camellia128":[751,"cbc",16,16,16,"camellia-128-cbc"],"camellia192":[752,"cbc",16,16,24,"camellia-192-cbc"],"camellia256":[753,"cbc",16,16,32,"camellia-256-cbc"],"chacha20":[1019,"stream",null,16,32,"chacha20"],"chacha20-poly1305":[1018,"stream",null,12,32,"chacha20-poly1305"],"des-ede":[32,"ecb",8,null,16,"des-ede"],"des-ede-cbc":[43,"cbc",8,8,16,"des-ede-cbc"],"des-ede-cfb":[60,"cfb",1,8,16,"des-ede-cfb"],"des-ede-ecb":[32,"ecb",8,null,16,"des-ede"],"des-ede-ofb":[62,"ofb",1,8,16,"des-ede-ofb"],"des-ede3":[33,"ecb",8,null,24,"des-ede3"],"des-ede3-cbc":[44,"cbc",8,8,24,"des-ede3-cbc"],"des-ede3-cfb":[61,"cfb",1,8,24,"des-ede3-cfb"],"des-ede3-cfb1":[658,"cfb",1,8,24,"des-ede3-cfb1"],"des-ede3-cfb8":[659,"cfb",1,8,24,"des-ede3-cfb8"],"des-ede3-ecb":[33,"ecb",8,null,24,"des-ede3"],"des-ede3-ofb":[63,"ofb",1,8,24,"des-ede3-ofb"],"des3":[44,"cbc",8,8,24,"des-ede3-cbc"],"des3-wrap":[246,"wrap",8,null,24,"id-smime-alg-cms3deswrap"],"id-aes128-CCM":[896,"ccm",1,12,16,"id-aes128-ccm"],"id-aes128-GCM":[895,"gcm",1,12,16,"id-aes128-gcm"],"id-aes128-wrap":[788,"wrap",8,8,16,"id-aes128-wrap"],"id-aes128-wrap-pad":[897,"wrap",8,4,16,"id-aes128-wrap-pad"],"id-aes192-CCM":[899,"ccm",1,12,24,"id-aes192-ccm"],"id-aes192-GCM":[898,"gcm",1,12,24,"id-aes192-gcm"],"id-aes192-wrap":[789,"wrap",8,8,24,"id-aes192-wrap"],"id-aes192-wrap-pad":[900,"wrap",8,4,24,"id-aes192-wrap-pad"],"id-aes256-CCM":[902,"ccm",1,12,32,"id-aes256-ccm"],"id-aes256-GCM":[901,"gcm",1,12,32,"id-aes256-gcm"],"id-aes256-wrap":[790,"wrap",8,8,32,"id-aes256-wrap"],"id-aes256-wrap-pad":[903,"wrap",8,4,32,"id-aes256-wrap-pad"],"id-smime-alg-CMS3DESwrap":[246,"wrap",8,null,24,"id-smime-alg-cms3deswrap"]};
function bigFromBytes(bytes) { var hex = Buffer.from(bytes).toString('hex'); return hex ? BigInt('0x' + hex) : 0n; }
function bigToBytes(n, size) { var hex = n.toString(16); if (hex.length % 2) hex = '0' + hex; var b = Buffer.from(hex, 'hex'); return size && b.length < size ? Buffer.concat([Buffer.alloc(size - b.length), b]) : b; }
function bigToArrayBuffer(n) { var b = bigToBytes(n); return b.buffer.slice(b.byteOffset, b.byteOffset + b.length); }
function bigBitLength(n) { return n === 0n ? 0 : n.toString(2).length; }
function modPow(b, e, m) { var r = 1n; b %= m; while (e > 0n) { if (e & 1n) r = r * b % m; b = b * b % m; e >>= 1n; } return r; }
function randomBig(bits) { var bytes = randomBytesSync(Math.ceil(bits / 8)); var n = bigFromBytes(bytes); return n & ((1n << BigInt(bits)) - 1n); }
var SMALL_PRIMES = [2n, 3n, 5n, 7n, 11n, 13n, 17n, 19n, 23n, 29n, 31n, 37n, 41n, 43n, 47n, 53n, 59n, 61n, 67n, 71n, 73n, 79n, 83n, 89n, 97n];
function isProbablePrime(n, rounds) {
  if (n < 2n) return false;
  for (var sp of SMALL_PRIMES) { if (n === sp) return true; if (n % sp === 0n) return false; }
  var d = n - 1n, r = 0; while ((d & 1n) === 0n) { d >>= 1n; r++; }
  var bits = bigBitLength(n);
  for (var i = 0; i < rounds; i++) {
    var a = i < SMALL_PRIMES.length ? SMALL_PRIMES[i] : 2n + randomBig(bits) % (n - 3n);
    var x = modPow(a, d, n);
    if (x === 1n || x === n - 1n) continue;
    var composite = true;
    for (var j = 1; j < r; j++) { x = x * x % n; if (x === n - 1n) { composite = false; break; } }
    if (composite) return false;
  }
  return true;
}
function bigCandidate(candidate) {
  if (typeof candidate === 'bigint') return candidate;
  if (candidate instanceof ArrayBuffer) return bigFromBytes(new Uint8Array(candidate));
  if (isView(candidate)) return bigFromBytes(new Uint8Array(candidate.buffer, candidate.byteOffset, candidate.byteLength));
  throw argTypeError('candidate', 'of type bigint or an instance of ArrayBuffer, TypedArray, Buffer, or DataView', candidate);
}
function checkPrimeSync(candidate, options) {
  var n = bigCandidate(candidate);
  if (options !== undefined && (typeof options !== 'object' || options === null)) throw argTypeError('options', 'of type object', options);
  var checks = options && options.checks !== undefined ? options.checks : 0;
  if (typeof checks !== 'number') throw argTypeError('options.checks', 'of type number', checks);
  if (!Number.isInteger(checks) || checks < 0 || checks > 2147483647) throw outOfRange('options.checks', checks);
  return isProbablePrime(n, checks > 0 ? checks : 20);
}
function checkPrime(candidate, options, callback) {
  if (typeof options === 'function') { callback = options; options = undefined; }
  var n = bigCandidate(candidate);
  requireArg(typeof callback === 'function', 'callback', TYPE_CALLBACK, callback);
  var result = checkPrimeSync(n, options);
  setImmediate(function () { callback(undefined, result); });
}
function generatePrimeSync(size, options) {
  requireArg(typeof size === 'number', 'size', 'of type number', size);
  if (!Number.isInteger(size) || size < 1 || size > 2147483647) throw cryptoError(RangeError, 'ERR_OUT_OF_RANGE', 'The value of "size" is out of range. It must be >= 1 && <= 2147483647. Received ' + size);
  if (options !== undefined) requireArg(typeof options === 'object' && options !== null, 'options', 'of type object', options);
  options = options || {};
  var add = options.add === undefined ? null : bigCandidate(options.add), rem = options.rem === undefined ? null : bigCandidate(options.rem);
  var safe = !!options.safe, wantBig = !!options.bigint;
  if (add !== null && rem !== null && add <= rem) throw cryptoError(RangeError, 'ERR_OUT_OF_RANGE', 'The value of "options.rem" is out of range. It must be < options.add');
  if (add !== null && bigBitLength(add) > size) throw cryptoError(RangeError, 'ERR_OUT_OF_RANGE', 'invalid options.add');
  var top = 1n << BigInt(size - 1);
  for (;;) {
    var n = randomBig(size) | top | 1n;
    if (add !== null) { var r = rem === null ? (safe ? 3n : 1n) : rem; n = n - n % add + r; if (bigBitLength(n) !== size) continue; }
    if (size === 1) n = 2n;
    if (!isProbablePrime(n, 20)) continue;
    if (safe && !isProbablePrime((n - 1n) >> 1n, 20)) continue;
    return wantBig ? n : bigToArrayBuffer(n);
  }
}
function generatePrime(size, options, callback) {
  if (typeof options === 'function') { callback = options; options = undefined; }
  requireArg(typeof callback === 'function', 'callback', TYPE_CALLBACK, callback);
  var result = generatePrimeSync(size, options);
  setImmediate(function () { callback(undefined, result); });
}
function dhBytes(value, encoding, name) {
  if (typeof value === 'string') return Buffer.from(value, encoding);
  if (value instanceof ArrayBuffer) return Buffer.from(new Uint8Array(value));
  if (isView(value)) return Buffer.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength));
  throw argTypeError(name, TYPE_DATA, value);
}
function dhOut(buf, encoding) { return encoding && encoding !== 'buffer' ? buf.toString(encoding) : buf; }
function DiffieHellman(sizeOrKey, keyEncoding, generator, genEncoding, group) {
  if (!(this instanceof DiffieHellman)) return new DiffieHellman(sizeOrKey, keyEncoding, generator, genEncoding);
  Object.defineProperties(this, { _p: { value: 0n, writable: true }, _g: { value: 0n, writable: true } });
  if (group) { this._p = BigInt('0x' + MODP_PRIMES[group]); this._g = 2n; }
  else {
    if (typeof keyEncoding !== 'string' && keyEncoding !== undefined && keyEncoding !== null) { genEncoding = generator; generator = keyEncoding; keyEncoding = undefined; }
    if (typeof sizeOrKey === 'number') {
      if (!Number.isInteger(sizeOrKey) || sizeOrKey < 2) throw cryptoError(RangeError, 'ERR_OUT_OF_RANGE', 'The value of "sizeOrKey" is out of range. Received ' + sizeOrKey);
      this._p = generatePrimeSync(sizeOrKey, { safe: true, bigint: true });
    } else this._p = bigFromBytes(dhBytes(sizeOrKey, keyEncoding, 'sizeOrKey'));
    this._g = generator === undefined ? 2n : typeof generator === 'number' ? BigInt(generator) : bigFromBytes(dhBytes(generator, genEncoding, 'generator'));
    if (this._g <= 1n) throw cryptoError(Error, 'ERR_OSSL_DH_BAD_GENERATOR', 'error:02800080:Diffie-Hellman routines::bad generator');
  }
  Object.defineProperty(this, '_priv', { value: null, writable: true });
  Object.defineProperty(this, '_pub', { value: null, writable: true });
  this.verifyError = group ? 0 : dhVerifyError(this._p, this._g);
}
function dhVerifyError(p, g) {
  var flags = 0;
  if (!isProbablePrime(p, 20)) flags |= 1; else if (!isProbablePrime((p - 1n) >> 1n, 20)) flags |= 2;
  if (g <= 1n || g >= p - 1n) flags |= 8;
  return flags;
}
var dhProto = {
  generateKeys: function generateKeys(encoding) {
    var bits = bigBitLength(this._p);
    var x; do { x = randomBig(bits - 1); } while (x < 2n);
    this._priv = x; this._pub = modPow(this._g, x, this._p);
    return this.getPublicKey(encoding);
  },
  computeSecret: function computeSecret(other, inputEncoding, outputEncoding) {
    if (this._priv === null) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'Invalid state for operation computeSecret');
    var y = bigFromBytes(dhBytes(other, inputEncoding, 'otherPublicKey'));
    if (y <= 1n || y >= this._p - 1n) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_KEYLEN', 'Supplied key is too small');
    return dhOut(bigToBytes(modPow(y, this._priv, this._p), Math.ceil(bigBitLength(this._p) / 8)), outputEncoding);
  },
  getPrime: function getPrime(encoding) { return dhOut(bigToBytes(this._p), encoding); },
  getGenerator: function getGenerator(encoding) { return dhOut(bigToBytes(this._g), encoding); },
  getPublicKey: function getPublicKey(encoding) { if (this._pub === null) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'No public key - did you specify a key?'); return dhOut(bigToBytes(this._pub), encoding); },
  getPrivateKey: function getPrivateKey(encoding) { if (this._priv === null) throw cryptoError(Error, 'ERR_CRYPTO_INVALID_STATE', 'No private key - did you specify a key?'); return dhOut(bigToBytes(this._priv), encoding); },
  setPublicKey: function setPublicKey(key, encoding) { this._pub = bigFromBytes(dhBytes(key, encoding, 'publicKey')); return this; },
  setPrivateKey: function setPrivateKey(key, encoding) { this._priv = bigFromBytes(dhBytes(key, encoding, 'privateKey')); this._pub = modPow(this._g, this._priv, this._p); return this; }
};
for (var dhName of Object.keys(dhProto)) def(DiffieHellman.prototype, dhName, dhProto[dhName]);
function DiffieHellmanGroup(name) {
  if (!(this instanceof DiffieHellmanGroup)) return new DiffieHellmanGroup(name);
  requireArg(typeof name === 'string', 'name', 'of type string', name);
  if (!Object.prototype.hasOwnProperty.call(MODP_PRIMES, name)) throw cryptoError(Error, 'ERR_CRYPTO_UNKNOWN_DH_GROUP', 'Unknown DH group');
  DiffieHellman.call(this, 0, undefined, undefined, undefined, name);
}
DiffieHellmanGroup.prototype = Object.create(DiffieHellman.prototype, { constructor: { value: DiffieHellmanGroup, writable: true, configurable: true } });
function createDiffieHellman(sizeOrKey, keyEncoding, generator, genEncoding) { return new DiffieHellman(sizeOrKey, keyEncoding, generator, genEncoding); }
function createDiffieHellmanGroup(name) { return new DiffieHellmanGroup(name); }
function getCipherInfo(nameOrNid, options) {
  requireArg(typeof nameOrNid === 'string' || typeof nameOrNid === 'number', 'nameOrNid', 'of type string or number', nameOrNid);
  if (options !== undefined) requireArg(typeof options === 'object' && options !== null, 'options', 'of type object', options);
  var row = null;
  if (typeof nameOrNid === 'string') row = CIPHER_INFO[nameOrNid.toLowerCase()] || null;
  else for (var k of Object.keys(CIPHER_INFO)) if (CIPHER_INFO[k][0] === nameOrNid) { row = CIPHER_INFO[k]; break; }
  if (!row) return undefined;
  var info = { mode: row[1], name: row[5], nid: row[0] };
  if (row[2] !== null) info.blockSize = row[2];
  if (row[3] !== null) info.ivLength = row[3];
  info.keyLength = row[4];
  if (options && options.keyLength !== undefined && options.keyLength !== info.keyLength) return undefined;
  if (options && options.ivLength !== undefined && options.ivLength !== info.ivLength) return undefined;
  return info;
}
function generateKeySync(type, options) {
  requireArg(typeof type === 'string', 'type', 'of type string', type);
  requireArg(typeof options === 'object' && options !== null, 'options', 'of type object', options);
  if (type !== 'hmac' && type !== 'aes') throw cryptoError(TypeError, 'ERR_INVALID_ARG_VALUE', "The argument 'type' must be one of: 'hmac', 'aes'. Received '" + type + "'");
  var length = options.length;
  requireArg(typeof length === 'number', 'options.length', 'of type number', length);
  if (type === 'aes' ? length !== 128 && length !== 192 && length !== 256 : !Number.isInteger(length) || length < 8 || length > 2147483647) throw cryptoError(TypeError, 'ERR_INVALID_ARG_VALUE', "The property 'options.length' is invalid. Received " + length);
  return new KeyObject('secret', randomBytesSync(length >> 3));
}
function generateKey(type, options, callback) {
  requireArg(typeof callback === 'function', 'callback', TYPE_CALLBACK, callback);
  var key = generateKeySync(type, options);
  setImmediate(function () { callback(null, key); });
}
Object.defineProperty(KeyObject.prototype, 'symmetricKeySize', { get: function () { return this.type === 'secret' ? this._material.length : undefined; }, configurable: true });
Object.assign(cryptoModule, { checkPrime: checkPrime, checkPrimeSync: checkPrimeSync, generatePrime: generatePrime, generatePrimeSync: generatePrimeSync,
  DiffieHellman: DiffieHellman, DiffieHellmanGroup: DiffieHellmanGroup, createDiffieHellman: createDiffieHellman, createDiffieHellmanGroup: createDiffieHellmanGroup,
  getDiffieHellman: createDiffieHellmanGroup, getCipherInfo: getCipherInfo, generateKeySync: generateKeySync, generateKey: generateKey });

// crypto.X509Certificate: DER walk over the certificate (subject/issuer/validity/SAN/CA/fingerprints, verify via createVerify).
// ponytail: keyUsage lists extended key usage OIDs, infoAccess/issuerCertificate/toLegacyObject are not implemented.
var DN_NAMES = { '2.5.4.3': 'CN', '2.5.4.4': 'SN', '2.5.4.5': 'serialNumber', '2.5.4.6': 'C', '2.5.4.7': 'L', '2.5.4.8': 'ST', '2.5.4.9': 'street', '2.5.4.10': 'O', '2.5.4.11': 'OU', '2.5.4.12': 'title', '2.5.4.42': 'GN', '1.2.840.113549.1.9.1': 'emailAddress', '0.9.2342.19200300.100.1.25': 'DC', '0.9.2342.19200300.100.1.1': 'UID' };
var X509_SIG_HASH = { '1.2.840.113549.1.1.4': 'md5', '1.2.840.113549.1.1.5': 'sha1', '1.2.840.113549.1.1.14': 'sha224', '1.2.840.113549.1.1.11': 'sha256', '1.2.840.113549.1.1.12': 'sha384', '1.2.840.113549.1.1.13': 'sha512', '1.2.840.10045.4.1': 'sha1', '1.2.840.10045.4.3.1': 'sha224', '1.2.840.10045.4.3.2': 'sha256', '1.2.840.10045.4.3.3': 'sha384', '1.2.840.10045.4.3.4': 'sha512' };
var MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
function derOid(buf, tlv) {
  var bytes = buf.subarray(tlv.body, tlv.end), parts = [Math.floor(bytes[0] / 40), bytes[0] % 40], n = 0;
  for (var i = 1; i < bytes.length; i++) { n = n * 128 + (bytes[i] & 0x7f); if (!(bytes[i] & 0x80)) { parts.push(n); n = 0; } }
  return parts.join('.');
}
function derChildren(buf, tlv) { var out = [], at = tlv.body; while (at < tlv.end) { var c = derRead(buf, at); out.push(c); at = c.end; } return out; }
function derTime(buf, tlv) {
  var t = buf.subarray(tlv.body, tlv.end).toString('latin1'), y, rest;
  if (tlv.tag === 0x17) { y = Number(t.slice(0, 2)); y += y >= 50 ? 1900 : 2000; rest = t.slice(2); } else { y = Number(t.slice(0, 4)); rest = t.slice(4); }
  return new Date(Date.UTC(y, Number(rest.slice(0, 2)) - 1, Number(rest.slice(2, 4)), Number(rest.slice(4, 6)), Number(rest.slice(6, 8)), Number(rest.slice(8, 10))));
}
function x509Name(buf, tlv) {
  var lines = [];
  for (var rdn of derChildren(buf, tlv)) for (var atv of derChildren(buf, rdn)) {
    var kids = derChildren(buf, atv), oid = derOid(buf, kids[0]), raw = buf.subarray(kids[1].body, kids[1].end);
    lines.push((DN_NAMES[oid] || oid) + '=' + (kids[1].tag === 0x1e ? raw.swap16 ? Buffer.from(raw).swap16().toString('utf16le') : raw.toString('latin1') : kids[1].tag === 0x0c ? raw.toString('utf8') : raw.toString('latin1')));
  }
  return lines.join('\n');
}
function x509Parse(der) {
  var cert = derRead(der, 0), parts = derChildren(der, cert), tbs = parts[0], f = derChildren(der, tbs), i = 0;
  if (f[0].tag === 0xa0) i++;
  var out = { tbs: der.subarray(tbs.start, tbs.end), serial: der.subarray(f[i].body, f[i].end), sigOid: derOid(der, derChildren(der, parts[1])[0]), sig: der.subarray(parts[2].body + 1, parts[2].end), issuer: x509Name(der, f[i + 2]), subject: x509Name(der, f[i + 4]), spki: der.subarray(f[i + 5].start, f[i + 5].end), san: undefined, ca: false, eku: undefined };
  var validity = derChildren(der, f[i + 3]); out.notBefore = derTime(der, validity[0]); out.notAfter = derTime(der, validity[1]);
  var ext = f.find(function (t) { return t.tag === 0xa3; });
  if (ext) for (var e of derChildren(der, derChildren(der, ext)[0])) {
    var ek = derChildren(der, e), oid = derOid(der, ek[0]), val = ek[ek.length - 1], inner = derRead(der, val.body);
    if (oid === '2.5.29.17') {
      var names = [];
      for (var g of derChildren(der, inner)) { var gv = der.subarray(g.body, g.end); if (g.tag === 0x82) names.push('DNS:' + gv.toString('latin1')); else if (g.tag === 0x81) names.push('email:' + gv.toString('latin1')); else if (g.tag === 0x86) names.push('URI:' + gv.toString('latin1')); else if (g.tag === 0x87) names.push('IP Address:' + (gv.length === 4 ? Array.from(gv).join('.') : gv.toString('hex').match(/.{4}/g).map(function (h) { return h.replace(/^0+(?=.)/, ''); }).join(':').toUpperCase())); }
      out.san = names.join(', ');
    } else if (oid === '2.5.29.19') { var bc = derChildren(der, inner); out.ca = bc.length > 0 && bc[0].tag === 0x01 && der[bc[0].body] !== 0; }
    else if (oid === '2.5.29.15') out.noCertSign = !(der[inner.body + 1] & 0x04);
    else if (oid === '1.3.6.1.5.5.7.1.1') {
      out.aia = [];
      for (var ad of derChildren(der, inner)) {
        var adk = derChildren(der, ad), method = derOid(der, adk[0]), label = method === '1.3.6.1.5.5.7.48.1' ? 'OCSP' : method === '1.3.6.1.5.5.7.48.2' ? 'CA Issuers' : method;
        out.aia.push([label + ' - ' + (adk[1].tag === 0x86 ? 'URI' : adk[1].tag === 0x82 ? 'DNS' : adk[1].tag === 0x81 ? 'email' : 'unknown'), der.subarray(adk[1].body, adk[1].end).toString('latin1')]);
      }
    }
    else if (oid === '2.5.29.37') out.eku = derChildren(der, inner).map(function (o) { return derOid(der, o); });
  }
  return out;
}
function x509Date(d) { function p(n) { return String(n).padStart(2, '0'); } return MONTHS[d.getUTCMonth()] + ' ' + String(d.getUTCDate()).padStart(2, ' ') + ' ' + p(d.getUTCHours()) + ':' + p(d.getUTCMinutes()) + ':' + p(d.getUTCSeconds()) + ' ' + d.getUTCFullYear() + ' GMT'; }
function x509Fingerprint(der, alg) { return cryptoModule.createHash(alg).update(der).digest('hex').toUpperCase().match(/../g).join(':'); }
function x509HostMatch(pattern, host) {
  pattern = pattern.toLowerCase(); host = host.toLowerCase();
  if (pattern.indexOf('*.') === 0) { var dot = host.indexOf('.'); return dot > 0 && host.slice(dot + 1) === pattern.slice(2); }
  return pattern === host;
}
class X509Certificate {
  constructor(buffer) {
    requireArg(typeof buffer === 'string' || isView(buffer) || buffer instanceof ArrayBuffer, 'buffer', 'of type string or an instance of Buffer, TypedArray, or DataView', buffer);
    var raw = typeof buffer === 'string' ? Buffer.from(buffer) : Buffer.from(buffer instanceof ArrayBuffer ? new Uint8Array(buffer) : new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength));
    var text = raw.toString('latin1'), m = /-----BEGIN CERTIFICATE-----([\s\S]*?)-----END CERTIFICATE-----/.exec(text), der = m ? Buffer.from(m[1].replace(/\s+/g, ''), 'base64') : raw;
    var parsed;
    try { parsed = x509Parse(der); } catch (_) { throw cryptoError(Error, 'ERR_OSSL_PEM_NO_START_LINE', 'error:0480006C:PEM routines::no start line'); }
    Object.defineProperties(this, { _der: { value: der }, _x: { value: parsed } });
  }
  get subject() { return this._x.subject; }
  get issuer() { return this._x.issuer; }
  get subjectAltName() { return this._x.san; }
  get infoAccess() { var a = this._x.aia; return a ? a.map(function (e) { return e[0] + ':' + e[1]; }).join('\n') : undefined; }
  get issuerCertificate() { return undefined; }
  get validFrom() { return x509Date(this._x.notBefore); }
  get validTo() { return x509Date(this._x.notAfter); }
  get validFromDate() { return this._x.notBefore; }
  get validToDate() { return this._x.notAfter; }
  get fingerprint() { return x509Fingerprint(this._der, 'sha1'); }
  get fingerprint256() { return x509Fingerprint(this._der, 'sha256'); }
  get fingerprint512() { return x509Fingerprint(this._der, 'sha512'); }
  get keyUsage() { return this._x.eku; }
  get serialNumber() { var h = this._x.serial.toString('hex').toUpperCase().replace(/^(00)+(?=.)/, ''); return h; }
  get raw() { return Buffer.from(this._der); }
  get publicKey() { return createPublicKey(spkiPem(this._x.spki)); }
  get ca() { return this._x.ca && !this._x.noCertSign; }
  toLegacyObject() {
    var x = this._x, obj = function (text) { var o = {}; for (var line of text.split('\n')) { var eq = line.indexOf('='), k = line.slice(0, eq), v = line.slice(eq + 1); o[k] = k in o ? [].concat(o[k], v) : v; } return o; };
    var out = { subject: obj(x.subject), issuer: obj(x.issuer) };
    if (x.san !== undefined) out.subjectaltname = x.san;
    if (x.aia) { var info = {}; for (var e of x.aia) (info[e[0]] = info[e[0]] || []).push(e[1]); out.infoAccess = info; }
    out.ca = this.ca;
    var ecDone = false;
    try {
      var spki = derChildren(derBytes(x.spki), derRead(derBytes(x.spki), 0)), bits = derBytes(x.spki).subarray(spki[1].body + 1, spki[1].end), key = derRead(bits, 0), kids = derChildren(bits, key);
      if (kids.length === 2 && kids[0].tag === 0x02) {
        var n = bits.subarray(kids[0].body, kids[0].end), ex = bits.subarray(kids[1].body, kids[1].end);
        while (n.length > 1 && n[0] === 0) n = n.subarray(1);
        out.modulus = n.toString('hex').toUpperCase(); out.bits = (n.length - 1) * 8 + (32 - Math.clz32(n[0]));
        out.exponent = '0x' + BigInt('0x' + ex.toString('hex')).toString(16);
      }
    } catch (_) {}
    try {
      var sp = derBytes(x.spki), top = derChildren(sp, derRead(sp, 0)), alg = derChildren(sp, top[0]);
      if (derOid(sp, alg[0]) === '1.2.840.10045.2.1') {
        var curveOid = derOid(sp, alg[1]), curves = { '1.2.840.10045.3.1.7': ['prime256v1', 'P-256', 256], '1.3.132.0.34': ['secp384r1', 'P-384', 384], '1.3.132.0.35': ['secp521r1', 'P-521', 521], '1.3.132.0.10': ['secp256k1', undefined, 256] }, info = curves[curveOid];
        out.bits = info ? info[2] : undefined;
        out.pubkey = Buffer.from(sp.subarray(top[1].body + 1, top[1].end));
        if (info) { out.asn1Curve = info[0]; if (info[1]) out.nistCurve = info[1]; }
        ecDone = true;
      }
    } catch (_) {}
    if (!ecDone) out.pubkey = Buffer.from(x.spki);
    out.valid_from = this.validFrom; out.valid_to = this.validTo;
    out.fingerprint = this.fingerprint; out.fingerprint256 = this.fingerprint256; out.fingerprint512 = this.fingerprint512;
    if (x.eku) out.ext_key_usage = x.eku;
    out.serialNumber = this.serialNumber; out.raw = this.raw;
    return out;
  }
  toString() { var b64 = this._der.toString('base64').replace(/(.{64})/g, '$1\n').replace(/\n$/, ''); return '-----BEGIN CERTIFICATE-----\n' + b64 + '\n-----END CERTIFICATE-----\n'; }
  toJSON() { return this.toString(); }
  checkHost(name) {
    requireArg(typeof name === 'string', 'name', 'of type string', name);
    var x = this._x, dns = x.san ? x.san.split(', ').filter(function (e) { return e.indexOf('DNS:') === 0; }).map(function (e) { return e.slice(4); }) : [];
    if (!dns.length) { var cn = /(?:^|\n)CN=([^\n]*)/.exec(x.subject); if (cn) dns = [cn[1]]; }
    for (var d of dns) if (x509HostMatch(d, name)) return d;
    return undefined;
  }
  checkEmail(email) { requireArg(typeof email === 'string', 'email', 'of type string', email); var x = this._x, list = x.san ? x.san.split(', ').filter(function (e) { return e.indexOf('email:') === 0; }).map(function (e) { return e.slice(6); }) : []; if (!list.length) { var m = /(?:^|\n)emailAddress=([^\n]*)/.exec(x.subject); if (m) list = [m[1]]; } return list.indexOf(email) >= 0 ? email : undefined; }
  checkIP(ip) { requireArg(typeof ip === 'string', 'ip', 'of type string', ip); var x = this._x; return x.san && x.san.split(', ').indexOf('IP Address:' + ip) >= 0 ? ip : undefined; }
  checkIssued(other) { if (!(other instanceof X509Certificate)) throw argTypeError('otherCert', 'an instance of X509Certificate', other); return this._x.issuer === other._x.subject; }
  checkPrivateKey(key) { var pub = key instanceof KeyObject ? createPublicKey(key) : createPublicKey(key); return pub.export({ type: 'spki', format: 'der' }).equals(this._x.spki); }
  verify(key) {
    if (!(key instanceof KeyObject)) throw argTypeError('publicKey', 'an instance of KeyObject', key);
    var alg = X509_SIG_HASH[this._x.sigOid]; if (!alg) return false;
    try { return new Verify(alg).update(this._x.tbs).verify(key, this._x.sig); } catch (_) { return false; }
  }
}
function derBytes(b) { return b; }
function spkiPem(spki) { return '-----BEGIN PUBLIC KEY-----\n' + spki.toString('base64').replace(/(.{64})/g, '$1\n').replace(/\n$/, '') + '\n-----END PUBLIC KEY-----\n'; }
Object.assign(cryptoModule, { KeyObject: KeyObject, X509Certificate: X509Certificate, Certificate: Certificate, createSecretKey: createSecretKey, createPrivateKey: createPrivateKey, createPublicKey: createPublicKey,
  Sign: Sign, Verify: Verify, createSign: createSign, createVerify: createVerify, sign: sign, verify: verify,
  publicEncrypt: publicEncrypt, privateDecrypt: privateDecrypt, privateEncrypt: privateEncrypt, publicDecrypt: publicDecrypt,
  generateKeyPairSync: generateKeyPairSync, generateKeyPair: generateKeyPair, diffieHellman: diffieHellman, ECDH: ECDH, createECDH: createECDH,
  constants: { RSA_PKCS1_PADDING: RSA_PKCS1_PADDING, RSA_PKCS1_PSS_PADDING: RSA_PKCS1_PSS_PADDING, RSA_PKCS1_OAEP_PADDING: RSA_PKCS1_OAEP_PADDING } });
def(globalThis, '__thaw_Hash', Hash); // globals so the scan knows update/digest/copy
def(globalThis, '__thaw_Hmac', Hmac);
def(globalThis, '__thaw_Cipheriv', Cipheriv);
Object.defineProperty(cryptoModule, 'subtle', { get: function () { return webCrypto.subtle; }, enumerable: true }); // Node 22: module-level subtle is webcrypto.subtle
def(globalThis, '__thaw_crypto_module', cryptoModule);
