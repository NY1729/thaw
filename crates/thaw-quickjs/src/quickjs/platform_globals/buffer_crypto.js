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
    writeUInt8(value, offset = 0) { this[Number(offset)] = Number(value) & 255; return Number(offset) + 1; }
    readUInt16LE(offset = 0) { const index = Number(offset); return this[index] | this[index + 1] << 8; }
    readUInt16BE(offset = 0) { const index = Number(offset); return this[index] << 8 | this[index + 1]; }
    writeUInt16LE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) & 255; this[index + 1] = Number(value) >> 8 & 255; return index + 2; }
    writeUInt16BE(value, offset = 0) { const index = Number(offset); this[index] = Number(value) >> 8 & 255; this[index + 1] = Number(value) & 255; return index + 2; }
  };
  Buffer.poolSize = 8192;
  globalThis.SlowBuffer = size => Buffer.alloc(Number(size));
  const normalizeHashAlgorithm = algorithm => {
    const name = String(algorithm).toLowerCase().replace(/[-_]/g, '');
    if (name !== 'sha256' && name !== 'sha512') throw new TypeError(`Unsupported digest: ${algorithm}`);
    return name;
  };
  const randomBytesSync = size => Buffer.from(__thaw_crypto_random_hex(Number(size)), 'hex');
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
    constructor(algorithm, key) { super(algorithm); this._key = Buffer.from(key); }
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
  const cryptoModule = {
    createHash: algorithm => new Hash(algorithm),
    createHmac: (algorithm, key) => new Hmac(algorithm, key),
    Hash, Hmac, randomBytes, randomFill, randomFillSync, randomInt, randomUUID,
    timingSafeEqual, getHashes: () => ['sha256', 'sha512']
  };
  const subtle = {
    digest(algorithm, data) {
      const name = typeof algorithm === 'string' ? algorithm : algorithm.name;
      const value = cryptoModule.createHash(name).update(Buffer.from(data.buffer || data,
        data.byteOffset || 0, data.byteLength)).digest();
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
})();
