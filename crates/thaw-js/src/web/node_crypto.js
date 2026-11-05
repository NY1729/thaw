// node:crypto (hashing, HMAC, random): lifted from thaw-quickjs buffer_crypto.js (the oracle).
// Runs after web/crypto.js, which owns globalThis.crypto; this file only adds the node half.
function def(o, name, v) {
  Object.defineProperty(o, name, { value: v, writable: true, enumerable: false, configurable: true });
}
// Digests createHash/createHmac accept; the names match thaw-quickjs exactly.
function normalizeDigestAlgorithm(algorithm) {
  var name = String(algorithm).toLowerCase().replace(/[-_]/g, '');
  if (name === 'rmd160' || name === 'ripemd') name = 'ripemd160';
  if (['md5', 'ripemd160', 'sha1', 'sha224', 'sha256', 'sha384', 'sha512', 'sha3224', 'sha3256', 'sha3384', 'sha3512'].indexOf(name) < 0) {
    throw new Error('Digest method not supported');
  }
  return name;
}
function hashFinalized() {
  var e = new Error('Digest already called'); e.code = 'ERR_CRYPTO_HASH_FINALIZED'; return e;
}
class Hash {
  constructor(algorithm) { this.algorithm = normalizeDigestAlgorithm(algorithm); this._chunks = []; this._digested = false; }
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
  constructor(algorithm, key) { super(algorithm); this._key = Buffer.from(key); }
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
    var value = new Hash(algorithm).update(data).digest();
    if (outputEncoding === undefined) outputEncoding = 'hex';
    return outputEncoding === 'buffer' ? value : value.toString(outputEncoding);
  }
};
// KDFs (lifted from buffer_crypto.js; same natives and messages as QuickJS).
function checkedPbkdf2Number(value, minimum, name) {
  var number = Number(value);
  if (!Number.isInteger(number) || number < minimum || number > 0xffffffff) throw new RangeError(name + ' out of range');
  return number;
}
function pbkdf2Sync(password, salt, iterations, keylen, digest) {
  return Buffer.from(__thaw_crypto_pbkdf2_hex(
    normalizeDigestAlgorithm(digest),
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
  if (typeof callback !== 'function') throw new TypeError('callback must be a function');
  var value;
  try { value = pbkdf2Sync(password, salt, iterations, keylen, digest); } catch (error) { if (error.code === undefined && error instanceof TypeError) throw error; queueMicrotask(function () { callback(error); }); return; }
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
def(globalThis, '__thaw_Hash', Hash); // globals so the scan knows update/digest/copy
def(globalThis, '__thaw_Hmac', Hmac);
def(globalThis, '__thaw_crypto_module', cryptoModule);
