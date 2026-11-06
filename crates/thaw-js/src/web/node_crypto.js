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
  constructor(algorithm, key) { super(algorithm); this._key = Buffer.from(key instanceof KeyObject ? key.export() : key); }
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
    var name = String(algorithm).toLowerCase().replace(/^(?:id-)?aes(\d+)$/, 'aes-$1-cbc').replace(/^id-/, '');
    var match = /^aes-(128|192|256)-(cbc|ecb|ctr|gcm)$/.exec(name);
    if (!match) throw cryptoError(Error, 'ERR_CRYPTO_UNKNOWN_CIPHER', 'Unknown cipher');
    this.algorithm = name;
    this.mode = match[2];
    this.key = Buffer.from(key instanceof KeyObject ? key.export() : key);
    if (this.key.length * 8 !== Number(match[1])) throw cryptoError(RangeError, 'ERR_CRYPTO_INVALID_KEYLEN', 'Invalid key length');
    this.iv = iv === null || iv === undefined ? Buffer.alloc(0) : Buffer.from(iv);
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
function createDecipheriv(algorithm, key, iv, options) { return new Cipheriv(algorithm, key, iv, true, options); }
// timingSafeEqual: Node's messages and codes (the check is a plain XOR loop, as in QuickJS).
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
Object.assign(cryptoModule, { createCipheriv: createCipheriv, createDecipheriv: createDecipheriv, Cipheriv: Cipheriv, timingSafeEqual: timingSafeEqual });
// Asymmetric keys and signatures (natives in node_crypto/asym.rs).
// Sign/Verify side of node:crypto (lifted from buffer_crypto.js, with the same natives).
// Digest names are checked by the natives: sha1/sha256/sha384/sha512 only (RSA), ECDSA uses its curve's digest.
function normalizeSignDigestAlgorithm(algorithm) {
  return normalizeDigestAlgorithm(String(algorithm).replace(/^(rsa|ecdsa|dsa)-/i, ''));
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
  constructor(algorithm) { this.algorithm = normalizeSignDigestAlgorithm(algorithm); this._chunks = []; }
  update(data, encoding) { this._chunks.push(Buffer.from(data, encoding)); return this; }
  sign(privateKey, outputEncoding) {
    const { pem } = parseAsymmetricKeyMaterial(privateKey);
    const data = Buffer.concat(this._chunks).toString('hex');
    const usePss = privateKey && privateKey.padding === RSA_PKCS1_PSS_PADDING;
    const signer = usePss ? __thaw_crypto_asymmetric_sign_pss_hex : __thaw_crypto_asymmetric_sign_hex;
    const value = Buffer.from(signer(this.algorithm, pem, data), 'hex');
    return outputEncoding === undefined ? value : value.toString(outputEncoding);
  }
}
class Verify {
  constructor(algorithm) { this.algorithm = normalizeSignDigestAlgorithm(algorithm); this._chunks = []; }
  update(data, encoding) { this._chunks.push(Buffer.from(data, encoding)); return this; }
  verify(publicKey, signature, signatureEncoding) {
    const { pem } = parseAsymmetricKeyMaterial(publicKey);
    const data = Buffer.concat(this._chunks).toString('hex');
    const signatureHex = Buffer.from(signature, signatureEncoding).toString('hex');
    const usePss = publicKey && publicKey.padding === RSA_PKCS1_PSS_PADDING;
    const verifier = usePss ? __thaw_crypto_asymmetric_verify_pss : __thaw_crypto_asymmetric_verify;
    return verifier(this.algorithm, pem, data, signatureHex);
  }
}
const createSign = algorithm => new Sign(algorithm);
const createVerify = algorithm => new Verify(algorithm);
const sign = (algorithm, data, key) => new Sign(algorithm || 'sha256').update(data).sign(key);
const verify = (algorithm, data, key, signature) => new Verify(algorithm || 'sha256').update(data).verify(key, signature);
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
Object.assign(cryptoModule, { KeyObject: KeyObject, createSecretKey: createSecretKey, createPrivateKey: createPrivateKey, createPublicKey: createPublicKey,
  Sign: Sign, Verify: Verify, createSign: createSign, createVerify: createVerify, sign: sign, verify: verify,
  publicEncrypt: publicEncrypt, privateDecrypt: privateDecrypt, privateEncrypt: privateEncrypt, publicDecrypt: publicDecrypt,
  generateKeyPairSync: generateKeyPairSync, generateKeyPair: generateKeyPair, diffieHellman: diffieHellman, ECDH: ECDH, createECDH: createECDH,
  constants: { RSA_PKCS1_PADDING: RSA_PKCS1_PADDING, RSA_PKCS1_PSS_PADDING: RSA_PKCS1_PSS_PADDING, RSA_PKCS1_OAEP_PADDING: RSA_PKCS1_OAEP_PADDING } });
def(globalThis, '__thaw_Hash', Hash); // globals so the scan knows update/digest/copy
def(globalThis, '__thaw_Hmac', Hmac);
def(globalThis, '__thaw_Cipheriv', Cipheriv);
def(globalThis, '__thaw_crypto_module', cryptoModule);
