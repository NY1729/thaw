  const normalizeBufferEncoding = encoding => String(encoding || 'utf8')
    .toLowerCase().replace(/[-_]/g, '');
  const bufferBytes = (value, encoding = 'utf8') => {
    const normalized = normalizeBufferEncoding(encoding);
    if (typeof value !== 'string') return Array.from(value);
    if (normalized === 'utf8' || normalized === 'utf') return encodeUtf8(value);
    if (normalized === 'hex') {
      const bytes = [];
      for (let index = 0; index + 1 < value.length && /^[0-9a-f]{2}$/i.test(value.substring(index, index + 2)); index += 2) {
        bytes.push(parseInt(value.substring(index, index + 2), 16));
      }
      return bytes;
    }
    if (normalized === 'base64' || normalized === 'base64url') {
      let encoded = value.replace(/-/g, '+').replace(/_/g, '/');
      while (encoded.length % 4) encoded += '=';
      return Array.from(atob(encoded), character => character.charCodeAt(0));
    }
    if (normalized === 'latin1' || normalized === 'binary' || normalized === 'ascii') {
      return Array.from(value, character => character.charCodeAt(0) & (normalized === 'ascii' ? 0x7f : 0xff));
    }
    if (normalized === 'utf16le' || normalized === 'ucs2') {
      const bytes = [];
      for (let index = 0; index < value.length; index++) {
        const code = value.charCodeAt(index); bytes.push(code & 255, code >> 8);
      }
      return bytes;
    }
    throw new TypeError(`Unknown encoding: ${encoding}`);
  };
  globalThis.Buffer = class Buffer extends Uint8Array {
    static from(value, encodingOrOffset, length) {
      if (typeof value === 'string') return new Buffer(bufferBytes(value, encodingOrOffset));
      if (value instanceof ArrayBuffer) {
        const offset = Number(encodingOrOffset || 0);
        return new Buffer(value, offset, length === undefined ? value.byteLength - offset : Number(length));
      }
      if (ArrayBuffer.isView(value)) return new Buffer(Array.from(new Uint8Array(value.buffer, value.byteOffset, value.byteLength)));
      if (value && value.type === 'Buffer' && Array.isArray(value.data)) return new Buffer(value.data);
      return new Buffer(Array.from(value || []));
    }
    static alloc(size, fill = 0, encoding) {
      const buffer = new Buffer(Number(size));
      if (fill !== 0) buffer.fill(fill, 0, buffer.length, encoding);
      return buffer;
    }
    static allocUnsafe(size) { return new Buffer(Number(size)); }
    static allocUnsafeSlow(size) { return new Buffer(Number(size)); }
    static isBuffer(value) { return value instanceof Buffer; }
    static isEncoding(value) {
      return ['utf8', 'utf', 'hex', 'base64', 'base64url', 'latin1', 'binary', 'ascii', 'utf16le', 'ucs2']
        .includes(normalizeBufferEncoding(value));
    }
    static byteLength(value, encoding) {
      if (ArrayBuffer.isView(value) || value instanceof ArrayBuffer) return value.byteLength;
      return bufferBytes(String(value), encoding).length;
    }
    static compare(left, right) {
      const length = Math.min(left.length, right.length);
      for (let index = 0; index < length; index++) if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
      return left.length === right.length ? 0 : left.length < right.length ? -1 : 1;
    }
    static concat(list, totalLength) {
      if (!Array.isArray(list)) throw new TypeError('list must be an Array');
      const length = totalLength === undefined ? list.reduce((sum, item) => sum + item.length, 0) : Number(totalLength);
      const output = Buffer.alloc(length); let offset = 0;
      for (const item of list) { offset += Buffer.from(item).copy(output, offset); if (offset >= length) break; }
      return output;
    }
    toString(encoding = 'utf8', start = 0, end = this.length) {
      const bytes = this.subarray(Math.max(0, Number(start)), Math.min(this.length, Number(end)));
      const normalized = normalizeBufferEncoding(encoding);
      if (normalized === 'utf8' || normalized === 'utf') return new TextDecoder().decode(bytes);
      if (normalized === 'hex') return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
      if (normalized === 'base64' || normalized === 'base64url') {
        let value = btoa(Array.from(bytes, byte => String.fromCharCode(byte)).join(''));
        if (normalized === 'base64url') value = value.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        return value;
      }
      if (normalized === 'latin1' || normalized === 'binary') return Array.from(bytes, byte => String.fromCharCode(byte)).join('');
      if (normalized === 'ascii') return Array.from(bytes, byte => String.fromCharCode(byte & 0x7f)).join('');
      if (normalized === 'utf16le' || normalized === 'ucs2') {
        let value = ''; for (let index = 0; index + 1 < bytes.length; index += 2) value += String.fromCharCode(bytes[index] | bytes[index + 1] << 8); return value;
      }
      throw new TypeError(`Unknown encoding: ${encoding}`);
    }
    toJSON() { return { type: 'Buffer', data: Array.from(this) }; }
    equals(other) { return Buffer.compare(this, other) === 0; }
    compare(other) { return Buffer.compare(this, other); }
    copy(target, targetStart = 0, sourceStart = 0, sourceEnd = this.length) {
      const bytes = this.subarray(Number(sourceStart), Number(sourceEnd));
      const count = Math.min(bytes.length, target.length - Number(targetStart));
      target.set(bytes.subarray(0, count), Number(targetStart)); return Math.max(0, count);
    }
    fill(value, start = 0, end = this.length, encoding) {
      const pattern = typeof value === 'string' ? bufferBytes(value, encoding) : [Number(value) & 255];
      if (pattern.length === 0) return this;
      for (let index = Number(start); index < Number(end); index++) this[index] = pattern[(index - Number(start)) % pattern.length];
      return this;
    }
    indexOf(value, byteOffset = 0, encoding) {
      const needle = bufferBytes(typeof value === 'number' ? [value & 255] : value, encoding);
      outer: for (let index = Math.max(0, Number(byteOffset)); index <= this.length - needle.length; index++) {
        for (let part = 0; part < needle.length; part++) if (this[index + part] !== needle[part]) continue outer;
        return index;
      }
      return -1;
    }
    lastIndexOf(value, byteOffset = this.length, encoding) {
      const needle = bufferBytes(typeof value === 'number' ? [value & 255] : value, encoding);
      outer: for (let index = Math.min(Number(byteOffset), this.length - needle.length); index >= 0; index--) {
        for (let part = 0; part < needle.length; part++) if (this[index + part] !== needle[part]) continue outer;
        return index;
      }
      return -1;
    }
    includes(value, byteOffset, encoding) { return this.indexOf(value, byteOffset, encoding) !== -1; }
    slice(start, end) { return this.subarray(start, end); }
    write(value, offset = 0, length, encoding = 'utf8') {
      if (typeof length === 'string') { encoding = length; length = undefined; }
      const bytes = bufferBytes(value, encoding); const count = Math.min(bytes.length, length === undefined ? this.length - Number(offset) : Number(length));
      this.set(bytes.slice(0, count), Number(offset)); return count;
    }
    readUInt8(offset = 0) { return this[Number(offset)]; }
    readInt8(offset = 0) { const value = this.readUInt8(offset); return value > 0x7f ? value - 0x100 : value; }
    writeUInt8(value, offset = 0) { this[Number(offset)] = Number(value) & 255; return Number(offset) + 1; }
    readUInt16LE(offset = 0) { const index = Number(offset); return this[index] | this[index + 1] << 8; }
    readUInt16BE(offset = 0) { const index = Number(offset); return this[index] << 8 | this[index + 1]; }
    readInt16LE(offset = 0) { const value = this.readUInt16LE(offset); return value > 0x7fff ? value - 0x10000 : value; }
    readInt16BE(offset = 0) { const value = this.readUInt16BE(offset); return value > 0x7fff ? value - 0x10000 : value; }
    writeUInt16LE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) & 255; this[index + 1] = Number(value) >> 8 & 255; return index + 2; }
    writeUInt16BE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) >> 8 & 255; this[index + 1] = Number(value) & 255; return index + 2; }
    writeInt16LE(value, offset = 0) { return this.writeUInt16LE(value, offset); }
    writeInt16BE(value, offset = 0) { return this.writeUInt16BE(value, offset); }
    readUInt32LE(offset = 0) { const index = Number(offset); return (this[index] | this[index + 1] << 8 | this[index + 2] << 16 | this[index + 3] << 24) >>> 0; }
    readUInt32BE(offset = 0) { const index = Number(offset); return (this[index] * 0x1000000 + (this[index + 1] << 16 | this[index + 2] << 8 | this[index + 3])) >>> 0; }
    readInt32LE(offset = 0) { const value = this.readUInt32LE(offset); return value > 0x7fffffff ? value - 0x100000000 : value; }
    readInt32BE(offset = 0) { const value = this.readUInt32BE(offset); return value > 0x7fffffff ? value - 0x100000000 : value; }
    writeUInt32LE(value, offset = 0) { const index = Number(offset), number = Number(value) >>> 0; this[index] = number; this[index + 1] = number >>> 8; this[index + 2] = number >>> 16; this[index + 3] = number >>> 24; return index + 4; }
    writeUInt32BE(value, offset = 0) { const index = Number(offset), number = Number(value) >>> 0; this[index] = number >>> 24; this[index + 1] = number >>> 16; this[index + 2] = number >>> 8; this[index + 3] = number; return index + 4; }
    writeInt32LE(value, offset = 0) { return this.writeUInt32LE(value, offset); }
    writeInt32BE(value, offset = 0) { return this.writeUInt32BE(value, offset); }
    readFloatLE(offset = 0) { return new DataView(this.buffer, this.byteOffset, this.byteLength).getFloat32(Number(offset), true); }
    readFloatBE(offset = 0) { return new DataView(this.buffer, this.byteOffset, this.byteLength).getFloat32(Number(offset), false); }
    readDoubleLE(offset = 0) { return new DataView(this.buffer, this.byteOffset, this.byteLength).getFloat64(Number(offset), true); }
    readDoubleBE(offset = 0) { return new DataView(this.buffer, this.byteOffset, this.byteLength).getFloat64(Number(offset), false); }
    writeFloatLE(value, offset = 0) { const index = Number(offset); new DataView(this.buffer, this.byteOffset, this.byteLength).setFloat32(index, Number(value), true); return index + 4; }
    writeFloatBE(value, offset = 0) { const index = Number(offset); new DataView(this.buffer, this.byteOffset, this.byteLength).setFloat32(index, Number(value), false); return index + 4; }
    writeDoubleLE(value, offset = 0) { const index = Number(offset); new DataView(this.buffer, this.byteOffset, this.byteLength).setFloat64(index, Number(value), true); return index + 8; }
    writeDoubleBE(value, offset = 0) { const index = Number(offset); new DataView(this.buffer, this.byteOffset, this.byteLength).setFloat64(index, Number(value), false); return index + 8; }
    swap16() { if (this.length % 2) throw new RangeError('Buffer size must be a multiple of 16-bits'); for (let index = 0; index < this.length; index += 2) [this[index], this[index + 1]] = [this[index + 1], this[index]]; return this; }
    swap32() { if (this.length % 4) throw new RangeError('Buffer size must be a multiple of 32-bits'); for (let index = 0; index < this.length; index += 4) { [this[index], this[index + 3]] = [this[index + 3], this[index]]; [this[index + 1], this[index + 2]] = [this[index + 2], this[index + 1]]; } return this; }
    swap64() { if (this.length % 8) throw new RangeError('Buffer size must be a multiple of 64-bits'); for (let index = 0; index < this.length; index += 8) for (let offset = 0; offset < 4; offset++) [this[index + offset], this[index + 7 - offset]] = [this[index + 7 - offset], this[index + offset]]; return this; }
  };
  Buffer.poolSize = 8192;
  for (const name of Object.getOwnPropertyNames(Buffer)) {
    if (!['length', 'name', 'prototype'].includes(name)) {
      const descriptor = Object.getOwnPropertyDescriptor(Buffer, name);
      if (descriptor.configurable) Object.defineProperty(Buffer, name, { ...descriptor, enumerable: true });
    }
  }
  globalThis.SlowBuffer = size => Buffer.alloc(Number(size));
  const normalizeHashAlgorithm = algorithm => {
    const name = String(algorithm).toLowerCase().replace(/[-_]/g, '');
    if (name !== 'sha1' && name !== 'sha256' && name !== 'sha384' && name !== 'sha512') throw new Error(`Unsupported digest: ${algorithm}`);
    return name;
  };
  // `createSign`/`createVerify`'s own `algorithm` argument is a *digest*
  // name that real Node also accepts in an `"rsa-sha256"`-style prefixed
  // form (matching OpenSSL's own digest names) -- the actual RSA-vs-
  // ECDSA choice always comes from the key passed to `.sign()`/
  // `.verify()`, never from this string, so the prefix is simply
  // stripped before normalizing.
  const normalizeSignDigestAlgorithm = algorithm => normalizeHashAlgorithm(
    String(algorithm).replace(/^(rsa|ecdsa|dsa)-/i, '')
  );
  const randomBytesSync = size => Buffer.from(__thaw_crypto_random_hex(Number(size)), 'hex');
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
  class KeyObject {
    constructor(type, material, asymmetricInfo) {
      this.type = type;
      this._material = material;
      if (asymmetricInfo) {
        this.asymmetricKeyType = asymmetricInfo.keyType;
        this.asymmetricKeyDetails = asymmetricInfo.namedCurve
          ? { namedCurve: asymmetricInfo.namedCurve }
          : asymmetricInfo.modulusLength != null
          ? { modulusLength: asymmetricInfo.modulusLength, publicExponent: BigInt(asymmetricInfo.publicExponent) }
          : {};
      }
    }
    export() {
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
  const parseAsymmetricKeyMaterial = key => {
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
  class Hash {
    constructor(algorithm) { this.algorithm = normalizeHashAlgorithm(algorithm); this._chunks = []; this._digested = false; }
    update(data, encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._chunks.push(Buffer.from(data, encoding)); return this;
    }
    digest(encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._digested = true;
      const value = Buffer.from(__thaw_crypto_hash_hex(this.algorithm, Buffer.concat(this._chunks).toString('hex')), 'hex');
      return encoding === undefined ? value : value.toString(encoding);
    }
    copy() { const copied = new Hash(this.algorithm); copied._chunks = this._chunks.map(chunk => Buffer.from(chunk)); return copied; }
  }
  class Hmac extends Hash {
    constructor(algorithm, key) {
      super(algorithm);
      this._key = Buffer.from(key instanceof KeyObject ? key.export() : key);
    }
    digest(encoding) {
      if (this._digested) throw new Error('Digest already called');
      this._digested = true;
      const value = Buffer.from(__thaw_crypto_hmac_hex(this.algorithm, this._key.toString('hex'), Buffer.concat(this._chunks).toString('hex')), 'hex');
      return encoding === undefined ? value : value.toString(encoding);
    }
  }
  const randomFillSync = (buffer, offset = 0, size = buffer.byteLength - Number(offset)) => {
    if (!ArrayBuffer.isView(buffer)) throw new TypeError('buffer must be an ArrayBuffer view');
    const bytes = new Uint8Array(buffer.buffer, buffer.byteOffset, buffer.byteLength);
    bytes.set(randomBytesSync(Number(size)), Number(offset)); return buffer;
  };
  const randomFill = (buffer, offset, size, callback) => {
    if (typeof offset === 'function') { callback = offset; offset = 0; size = buffer.byteLength; }
    else if (typeof size === 'function') { callback = size; size = buffer.byteLength - Number(offset || 0); }
    if (typeof callback !== 'function') throw new TypeError('callback must be a function');
    try { randomFillSync(buffer, offset || 0, size); queueMicrotask(() => callback(null, buffer)); }
    catch (error) { queueMicrotask(() => callback(error)); }
  };
  const randomBytes = (size, callback) => {
    const value = randomBytesSync(size);
    if (callback === undefined) return value;
    if (typeof callback !== 'function') throw new TypeError('callback must be a function');
    queueMicrotask(() => callback(null, value));
  };
  const randomInt = (min, max, callback) => {
    if (max === undefined || typeof max === 'function') { callback = typeof max === 'function' ? max : callback; max = min; min = 0; }
    min = Math.ceil(Number(min)); max = Math.floor(Number(max));
    if (!Number.isSafeInteger(min) || !Number.isSafeInteger(max) || max <= min) throw new RangeError('invalid random integer range');
    const range = max - min; const bytes = randomBytesSync(4);
    const value = min + ((bytes[0] * 0x1000000 + bytes[1] * 0x10000 + bytes[2] * 0x100 + bytes[3]) % range);
    if (callback === undefined) return value;
    queueMicrotask(() => callback(null, value));
  };
  const randomUUID = () => {
    const bytes = randomBytesSync(16); bytes[6] = bytes[6] & 0x0f | 0x40; bytes[8] = bytes[8] & 0x3f | 0x80;
    const hex = bytes.toString('hex');
    return `${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}`;
  };
  const timingSafeEqual = (left, right) => {
    const a = Buffer.from(left), b = Buffer.from(right);
    if (a.length !== b.length) throw new RangeError('input buffers must have the same length');
    let difference = 0; for (let index = 0; index < a.length; index++) difference |= a[index] ^ b[index]; return difference === 0;
  };
  const pbkdf2Sync = (password, salt, iterations, keylen, digest) => Buffer.from(
    __thaw_crypto_pbkdf2_hex(
      normalizeHashAlgorithm(digest),
      Buffer.from(password).toString('hex'),
      Buffer.from(salt).toString('hex'),
      Number(iterations),
      Number(keylen)
    ),
    'hex'
  );
  const scryptSync = (password, salt, keylen, options = {}) => Buffer.from(
    __thaw_crypto_scrypt_hex(
      Buffer.from(password).toString('hex'),
      Buffer.from(salt).toString('hex'),
      Number(keylen),
      Number(options.N === undefined ? 16384 : options.N),
      Number(options.r === undefined ? 8 : options.r),
      Number(options.p === undefined ? 1 : options.p)
    ),
    'hex'
  );
  const scrypt = (password, salt, keylen, options, callback) => {
    if (typeof options === 'function') { callback = options; options = {}; }
    if (typeof callback !== 'function') throw new TypeError('callback must be a function');
    queueMicrotask(() => {
      try { callback(null, scryptSync(password, salt, keylen, options)); }
      catch (error) { callback(error); }
    });
  };
  class Cipheriv {
    constructor(algorithm, key, iv, decrypt) {
      this.algorithm = String(algorithm).toLowerCase();
      this.key = Buffer.from(key);
      this.iv = Buffer.from(iv);
      this.decrypt = decrypt;
      this.pending = Buffer.alloc(0);
      this.finished = false;
    }
    update(value, inputEncoding, outputEncoding) {
      if (this.finished) throw new Error('Trying to add data in unsupported state');
      this.pending = Buffer.concat([this.pending, Buffer.from(value, inputEncoding)]);
      const length = this.decrypt
        ? Math.max(0, Math.floor((this.pending.length - 1) / 16) * 16)
        : Math.floor(this.pending.length / 16) * 16;
      if (!length) return outputEncoding === undefined ? Buffer.alloc(0) : '';
      const input = this.pending.subarray(0, length);
      this.pending = Buffer.from(this.pending.subarray(length));
      const output = Buffer.from(__thaw_crypto_cipher_hex(
        this.algorithm, this.key.toString('hex'), this.iv.toString('hex'),
        input.toString('hex'), this.decrypt, false
      ), 'hex');
      this.iv = Buffer.from((this.decrypt ? input : output).subarray(length - 16, length));
      return outputEncoding === undefined ? output : output.toString(outputEncoding);
    }
    final(outputEncoding) {
      if (this.finished) throw new Error('Invalid state');
      this.finished = true;
      const value = Buffer.from(__thaw_crypto_cipher_hex(
        this.algorithm,
        this.key.toString('hex'),
        this.iv.toString('hex'),
        this.pending.toString('hex'),
        this.decrypt,
        true
      ), 'hex');
      return outputEncoding === undefined ? value : value.toString(outputEncoding);
    }
  }
  const createCipheriv = (algorithm, key, iv) => new Cipheriv(algorithm, key, iv, false);
  const createDecipheriv = (algorithm, key, iv) => new Cipheriv(algorithm, key, iv, true);
  const createSecretKey = key => new KeyObject('secret', Buffer.from(key));
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
  const RSA_PKCS1_OAEP_PADDING = 4;
  // `publicEncrypt(key, buffer)` / `privateDecrypt(key, buffer)` --
  // real Node's default padding for *these* (unlike `Sign`/`Verify`'s
  // own PKCS1v15 default) is OAEP, defaulting to a SHA-1 `oaepHash`
  // (OpenSSL's own long-standing default) unless `padding:
  // RSA_PKCS1_PADDING` is explicitly requested. RSA only -- an EC/
  // Ed25519 key throws a clear error, matching real Node. `publicDecrypt`/
  // `privateEncrypt` (the rarer raw-RSA "encrypt with private key"
  // operations) are not implemented.
  const resolveEncryptionPadding = key => (key && key.padding === RSA_PKCS1_PADDING)
    ? '' : normalizeHashAlgorithm((key && key.oaepHash) || 'sha1');
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
    const privateType = (options.privateKeyEncoding && options.privateKeyEncoding.type) || '';
    const publicType = (options.publicKeyEncoding && options.publicKeyEncoding.type) || '';
    const generated = JSON.parse(__thaw_crypto_generate_key_pair_json(type, arg || '', publicExponent, privateType, publicType));
    return {
      publicKey: encodeGeneratedKeyHalf(generated.publicPem, false, options.publicKeyEncoding),
      privateKey: encodeGeneratedKeyHalf(generated.privatePem, true, options.privateKeyEncoding)
    };
  };
  const generateKeyPair = (type, options, callback) => {
    if (typeof options === 'function') { callback = options; options = {}; }
    queueMicrotask(() => {
      try {
        const result = generateKeyPairSync(type, options);
        callback(null, result.publicKey, result.privateKey);
      } catch (error) { callback(error); }
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
      return encoding === undefined ? value : value.toString(encoding);
    }
    getPublicKey(encoding) {
      const value = Buffer.from(this._publicKeyHex, 'hex');
      return encoding === undefined ? value : value.toString(encoding);
    }
    computeSecret(otherPublicKey, inputEncoding, outputEncoding) {
      const publicKeyHex = Buffer.from(otherPublicKey, inputEncoding).toString('hex');
      const value = Buffer.from(
        __thaw_crypto_ecdh_compute_secret_hex(this.curveName, this._privateKeyHex, publicKeyHex),
        'hex'
      );
      return outputEncoding === undefined ? value : value.toString(outputEncoding);
    }
  }
  const createECDH = curveName => new ECDH(curveName);
  const cryptoModule = {
    createHash: algorithm => new Hash(algorithm),
    createHmac: (algorithm, key) => new Hmac(algorithm, key),
    createCipheriv, createDecipheriv, Cipheriv,
    Hash, Hmac, KeyObject, createSecretKey, createPrivateKey, createPublicKey, pbkdf2Sync, scrypt, scryptSync, randomBytes, randomFill, randomFillSync, randomInt, randomUUID,
    Sign, Verify, createSign, createVerify, sign, verify, publicEncrypt, privateDecrypt,
    generateKeyPairSync, generateKeyPair,
    diffieHellman, ECDH, createECDH,
    constants: { RSA_PKCS1_PADDING, RSA_PKCS1_PSS_PADDING, RSA_PKCS1_OAEP_PADDING },
    timingSafeEqual, getHashes: () => ['sha256', 'sha512']
  };
  const subtle = {
    digest(algorithm, data) {
      const name = typeof algorithm === 'string' ? algorithm : algorithm.name;
      const value = cryptoModule.createHash(name).update(Buffer.from(data.buffer || data,
        data.byteOffset || 0, data.byteLength)).digest();
      return Promise.resolve(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    },
    importKey(format, keyData, algorithm, extractable, usages) {
      if (format !== 'raw') return Promise.reject(new TypeError(`Unsupported key format: ${format}`));
      return Promise.resolve({
        __thawRaw: Buffer.from(keyData.buffer || keyData, keyData.byteOffset || 0, keyData.byteLength),
        algorithm: typeof algorithm === 'string' ? { name: algorithm } : algorithm,
        extractable: Boolean(extractable),
        usages: Array.from(usages || [])
      });
    },
    sign(algorithm, key, data) {
      const name = typeof algorithm === 'string' ? algorithm : algorithm.name;
      const hash = key.algorithm && key.algorithm.hash;
      const digest = typeof hash === 'string' ? hash : hash && hash.name;
      if (String(name).toUpperCase() !== 'HMAC' || !key.__thawRaw) return Promise.reject(new TypeError('Unsupported signing key'));
      const value = Buffer.from(__thaw_crypto_hmac_hex(normalizeHashAlgorithm(digest), key.__thawRaw.toString('hex'), Buffer.from(data.buffer || data, data.byteOffset || 0, data.byteLength).toString('hex')), 'hex');
      return Promise.resolve(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    },
    deriveBits(algorithm, key, length) {
      const name = typeof algorithm === 'string' ? algorithm : algorithm.name;
      const hash = algorithm && algorithm.hash;
      const digest = typeof hash === 'string' ? hash : hash && hash.name;
      if (String(name).toUpperCase() !== 'PBKDF2' || !key.__thawRaw || Number(length) % 8 !== 0) return Promise.reject(new TypeError('Unsupported key derivation'));
      const salt = Buffer.from(algorithm.salt.buffer || algorithm.salt, algorithm.salt.byteOffset || 0, algorithm.salt.byteLength);
      const value = Buffer.from(__thaw_crypto_pbkdf2_hex(normalizeHashAlgorithm(digest), key.__thawRaw.toString('hex'), salt.toString('hex'), Number(algorithm.iterations), Number(length) / 8), 'hex');
      return Promise.resolve(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength));
    }
  };
  const webCrypto = globalThis.crypto || {};
  webCrypto.getRandomValues = value => {
    if (!ArrayBuffer.isView(value) || value.byteLength > 65536) throw new DOMException('invalid random value target', 'QuotaExceededError');
    return randomFillSync(value);
  };
  webCrypto.randomUUID = randomUUID;
  webCrypto.subtle = webCrypto.subtle || subtle;
  globalThis.crypto = webCrypto;
  cryptoModule.webcrypto = webCrypto;
  cryptoModule.subtle = webCrypto.subtle;
  globalThis.__thaw_crypto_module = cryptoModule;
