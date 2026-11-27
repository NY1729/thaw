  const __thawTextApply = Reflect.apply;
  const __thawTextUint8Array = Uint8Array;
  const __thawTextTypeError = TypeError;
  const __thawTextArrayBuffer = ArrayBuffer;
  const __thawTextIsView = ArrayBuffer.isView;
  const __thawTextSharedArrayBuffer = typeof SharedArrayBuffer === 'undefined' ? null : SharedArrayBuffer;
  const __thawTextTypedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
  const __thawTextGetter = (object, key) => {
    while (object) {
      const descriptor = Object.getOwnPropertyDescriptor(object, key);
      if (descriptor) return descriptor.get;
      object = Object.getPrototypeOf(object);
    }
    return undefined;
  };
  const __thawTextViewTag = __thawTextGetter(__thawTextTypedArrayPrototype, Symbol.toStringTag);
  const __thawTextViewBuffer = __thawTextGetter(__thawTextTypedArrayPrototype, 'buffer');
  const __thawTextViewOffset = __thawTextGetter(__thawTextTypedArrayPrototype, 'byteOffset');
  const __thawTextViewByteLength = __thawTextGetter(__thawTextTypedArrayPrototype, 'byteLength');
  const __thawTextSet = Uint8Array.prototype.set;
  const __thawTextArrayBufferLength = __thawTextGetter(ArrayBuffer.prototype, 'byteLength');
  const __thawTextSharedBufferLength = __thawTextSharedArrayBuffer
    ? __thawTextGetter(__thawTextSharedArrayBuffer.prototype, 'byteLength') : undefined;
  const __thawTextErr = (Ctor, code, message) => { const e = new Ctor(message); e.code = code; return e; };
  const __thawTextCp = code => String.fromCodePoint(code);
  const __thawTextNativeTypeError = (error) => error instanceof __thawTextTypeError;
  const __thawTextIsArrayBuffer = value => {
    try { __thawTextApply(__thawTextArrayBufferLength, value, []); return true; }
    catch (_) { return false; }
  };
  const __thawTextIsSharedBuffer = value => {
    if (!__thawTextSharedArrayBuffer) return false;
    try { __thawTextApply(__thawTextSharedBufferLength, value, []); return true; }
    catch (_) { return false; }
  };
  const __thawTextDetached = buffer => {
    try { new __thawTextUint8Array(buffer, 0, 0); return false; }
    catch (error) { if (__thawTextNativeTypeError(error)) return true; throw error; }
  };
  const __thawTextDescribeSource = input => {
    if (input === undefined) return { kind: 'empty' };
    if (__thawTextIsArrayBuffer(input) || __thawTextIsSharedBuffer(input)) {
      return { kind: 'buffer', buffer: input };
    }
    // thaw-js DataView keeps buffer/byteOffset/byteLength as own data properties
    if (typeof DataView === 'function' && input instanceof DataView) {
      return { kind: 'dataview', buffer: input.buffer, view: input };
    }
    if (__thawTextApply(__thawTextIsView, __thawTextArrayBuffer, [input])) {
      try { return { kind: 'typedarray', buffer: __thawTextApply(__thawTextViewBuffer, input, []), view: input }; }
      catch (_) { throw __thawTextErr(__thawTextTypeError, 'ERR_INVALID_ARG_TYPE', 'The "input" argument must be an instance of ArrayBuffer or ArrayBufferView.'); }
    }
    throw __thawTextErr(__thawTextTypeError, 'ERR_INVALID_ARG_TYPE', 'The "input" argument must be an instance of ArrayBuffer or ArrayBufferView.');
  };
  const __thawTextSnapshot = source => {
    if (source.kind === 'empty' || __thawTextDetached(source.buffer)) return new __thawTextUint8Array();
    let offset = 0, length;
    if (source.kind === 'dataview') {
      offset = source.view.byteOffset;
      length = source.view.byteLength;
    } else if (source.kind === 'typedarray') {
      offset = __thawTextApply(__thawTextViewOffset, source.view, []);
      length = __thawTextApply(__thawTextViewByteLength, source.view, []);
    } else if (__thawTextIsArrayBuffer(source.buffer)) {
      length = __thawTextApply(__thawTextArrayBufferLength, source.buffer, []);
    } else {
      length = __thawTextApply(__thawTextSharedBufferLength, source.buffer, []);
    }
    const visible = new __thawTextUint8Array(source.buffer, offset, length);
    const copy = new __thawTextUint8Array(length);
    __thawTextApply(__thawTextSet, copy, [visible]);
    return copy;
  };

  const __thawEncode = __thaw_utf8_encode, __thawDecode = __thaw_utf8_decode, __thawTail = __thaw_utf8_tail;
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') { return __thawEncode(`${input}`); }
    encodeInto(input, destination) {
      if (!__thawTextApply(__thawTextIsView, __thawTextArrayBuffer, [destination]) ||
          __thawTextApply(__thawTextViewTag, destination, []) !== 'Uint8Array') {
        throw __thawTextErr(__thawTextTypeError, 'ERR_INVALID_ARG_TYPE', 'The "dest" argument must be an instance of Uint8Array.');
      }
      if (typeof input !== 'string') throw __thawTextErr(__thawTextTypeError, 'ERR_INVALID_ARG_TYPE', 'The "src" argument must be of type string.');
      const text = input;
      const capacity = __thawTextApply(__thawTextViewByteLength, destination, []);
      const bytes = __thawEncode(text);
      let written = Math.min(bytes.length, capacity);
      // never split a code point: back off to a character boundary
      while (written > 0 && written < bytes.length && (bytes[written] & 0xc0) === 0x80) written--;
      __thawTextApply(__thawTextSet, destination, [new __thawTextUint8Array(bytes.buffer, 0, written)]);
      return { read: __thawDecode(bytes, 0, written).length, written };
    }
  };

  const replacement = '\ufffd';
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = 'utf-8', options = {}) {
      const normalized = String(label).trim().toLowerCase().replace(/[_\s]/g, '-');
      const encoding = ['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(normalized) ? 'utf-8'
        : ['utf-16le', 'utf-16', 'ucs-2', 'unicode', 'csunicode', 'iso-10646-ucs-2', 'unicodefeff'].includes(normalized) ? 'utf-16le'
        : ['latin1', 'iso-8859-1', 'iso8859-1', 'l1', 'ascii', 'us-ascii', 'windows-1252', 'cp1252', 'x-cp1252'].includes(normalized) ? 'windows-1252'
        : null;
      if (encoding === null) {
        throw __thawTextErr(RangeError, 'ERR_ENCODING_NOT_SUPPORTED', `The "${label}" encoding is not supported`);
      }
      // non-enumerable: Node's TextDecoder instance has no own enumerable properties
      const hidden = (key, value) => Object.defineProperty(this, key, { value, writable: true, configurable: true });
      hidden('_fatal', Boolean(options.fatal));
      hidden('_ignoreBOM', Boolean(options.ignoreBOM));
      hidden('_encoding', encoding);
      hidden('_pending', new __thawTextUint8Array());
      hidden('_bomSeen', false);
      hidden('_previousStreaming', false);
    }
    decode(input = new __thawTextUint8Array(), options = {}) {
      const source = __thawTextDescribeSource(input);
      const streaming = Boolean(options.stream);
      const chunk = __thawTextSnapshot(source);
      if (!this._previousStreaming) { this._pending = new __thawTextUint8Array(); this._bomSeen = false; }
      this._previousStreaming = streaming;
      const pendingLength = __thawTextApply(__thawTextViewByteLength, this._pending, []);
      const chunkLength = __thawTextApply(__thawTextViewByteLength, chunk, []);
      const byteLength = pendingLength + chunkLength;
      let bytes = chunk;
      if (pendingLength) {
        bytes = new __thawTextUint8Array(byteLength);
        __thawTextApply(__thawTextSet, bytes, [this._pending]);
        __thawTextApply(__thawTextSet, bytes, [chunk, pendingLength]);
      }
      this._pending = new __thawTextUint8Array();
      let output = '';
      let index = 0;
      const invalid = () => {
        if (this._fatal) throw __thawTextErr(TypeError, 'ERR_ENCODING_INVALID_ENCODED_DATA', `The encoded data was not valid for encoding ${this._encoding}`);
        output += replacement;
      };
      if (this._encoding === 'windows-1252') {
        const high = [0x20ac, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160, 0x2039, 0x152, 0x8d, 0x17d, 0x8f,
          0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014, 0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x17e, 0x178];
        for (; index < byteLength; index++) {
          const byte = bytes[index];
          output += String.fromCharCode(byte >= 0x80 && byte <= 0x9f ? high[byte - 0x80] : byte);
        }
      } else if (this._encoding === 'utf-16le') {
        let end = byteLength - byteLength % 2;
        if (streaming && end >= 2) {
          const last = bytes[end - 2] | bytes[end - 1] << 8;
          if (last >= 0xd800 && last <= 0xdbff) end -= 2;
        }
        if (streaming) {
          this._pending = new __thawTextUint8Array(byteLength - end);
          for (let pendingIndex = end; pendingIndex < byteLength; pendingIndex++) this._pending[pendingIndex - end] = bytes[pendingIndex];
        }
        for (; index + 1 < end; index += 2) {
          const unit = bytes[index] | bytes[index + 1] << 8;
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const next = index + 3 < end ? bytes[index + 2] | bytes[index + 3] << 8 : 0;
            if (next >= 0xdc00 && next <= 0xdfff) { output += __thawTextCp(((unit - 0xd800) << 10) + (next - 0xdc00) + 0x10000); index += 2; }
            else invalid();
          } else if (unit >= 0xdc00 && unit <= 0xdfff) invalid();
          else output += String.fromCharCode(unit);
        }
        if (!streaming && byteLength % 2) invalid();
        index = byteLength;
      }
      else {
        const tail = streaming ? __thawTail(bytes) : 0;
        if (tail) {
          this._pending = new __thawTextUint8Array(tail);
          for (let pendingIndex = 0; pendingIndex < tail; pendingIndex++) this._pending[pendingIndex] = bytes[byteLength - tail + pendingIndex];
        }
        const text = __thawDecode(bytes, 0, byteLength - tail, this._fatal);
        if (text === undefined) invalid();
        else output = text;
      }
      if (!this._bomSeen && output.length) {
        this._bomSeen = true;
        if (!this._ignoreBOM && output.charCodeAt(0) === 0xfeff) output = output.slice(1);
      }
      if (!streaming) { this._pending = new __thawTextUint8Array(); this._bomSeen = false; }
      return output;
    }
  };
  // Node: encoding, fatal and ignoreBOM are prototype getters, enumerable and configurable
  Object.defineProperties(TextDecoder.prototype, Object.getOwnPropertyDescriptors({
    get encoding() { return this._encoding; },
    get fatal() { return this._fatal; },
    get ignoreBOM() { return this._ignoreBOM; }
  }));
