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
  const __thawTextDataViewBuffer = __thawTextGetter(DataView.prototype, 'buffer');
  const __thawTextDataViewOffset = __thawTextGetter(DataView.prototype, 'byteOffset');
  const __thawTextDataViewByteLength = __thawTextGetter(DataView.prototype, 'byteLength');
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
    if (__thawTextApply(__thawTextIsView, __thawTextArrayBuffer, [input])) {
      try { return { kind: 'dataview', buffer: __thawTextApply(__thawTextDataViewBuffer, input, []), view: input }; }
      catch (_) {
        try { return { kind: 'typedarray', buffer: __thawTextApply(__thawTextViewBuffer, input, []), view: input }; }
        catch (_) { throw new __thawTextTypeError('input must be an ArrayBuffer or view'); }
      }
    }
    throw new __thawTextTypeError('input must be an ArrayBuffer or view');
  };
  const __thawTextSnapshot = source => {
    if (source.kind === 'empty' || __thawTextDetached(source.buffer)) return new __thawTextUint8Array();
    let offset = 0, length;
    if (source.kind === 'dataview') {
      offset = __thawTextApply(__thawTextDataViewOffset, source.view, []);
      length = __thawTextApply(__thawTextDataViewByteLength, source.view, []);
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

  const encodeUtf8 = input => {
    const bytes = [];
    for (const character of String(input)) {
      let code = character.codePointAt(0);
      if (code >= 0xd800 && code <= 0xdfff) code = 0xfffd;
      if (code <= 0x7f) bytes.push(code);
      else if (code <= 0x7ff) bytes.push(0xc0 | code >> 6, 0x80 | code & 0x3f);
      else if (code <= 0xffff) bytes.push(0xe0 | code >> 12,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
      else bytes.push(0xf0 | code >> 18, 0x80 | code >> 12 & 0x3f,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
    }
    return bytes;
  };
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') { return Uint8Array.from(encodeUtf8(`${input}`)); }
    encodeInto(input, destination) {
      if (!__thawTextApply(__thawTextIsView, __thawTextArrayBuffer, [destination]) ||
          __thawTextApply(__thawTextViewTag, destination, []) !== 'Uint8Array') {
        throw new __thawTextTypeError('destination must be a Uint8Array');
      }
      const text = `${input}`;
      const capacity = __thawTextApply(__thawTextViewByteLength, destination, []);
      let read = 0;
      let written = 0;
      for (const character of text) {
        const bytes = encodeUtf8(character);
        if (written + bytes.length > capacity) break;
        const encoded = new __thawTextUint8Array(bytes.length);
        for (let index = 0; index < bytes.length; index++) encoded[index] = bytes[index];
        __thawTextApply(__thawTextSet, destination, [encoded, written]);
        written += bytes.length;
        read += character.length;
      }
      return { read, written };
    }
  };

  const replacement = '\ufffd';
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = 'utf-8', options = {}) {
      const normalized = String(label).trim().toLowerCase().replace(/[_\s]/g, '-');
      if (!['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(normalized)) {
        throw new RangeError(`unsupported encoding: ${label}`);
      }
      this.fatal = Boolean(options.fatal);
      this.ignoreBOM = Boolean(options.ignoreBOM);
      this.encoding = 'utf-8';
      this._pending = new __thawTextUint8Array();
      this._bomSeen = false;
      this._previousStreaming = false;
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
      const bytes = new __thawTextUint8Array(byteLength);
      __thawTextApply(__thawTextSet, bytes, [this._pending]);
      __thawTextApply(__thawTextSet, bytes, [chunk, pendingLength]);
      this._pending = new __thawTextUint8Array();
      let output = '';
      let index = 0;
      const invalid = () => {
        if (this.fatal) throw new TypeError('invalid UTF-8 data');
        output += replacement;
      };
      while (index < byteLength) {
        const first = bytes[index++];
        if (first <= 0x7f) { output += String.fromCodePoint(first); continue; }
        let length, code, minimum;
        if (first >= 0xc2 && first <= 0xdf) { length = 1; code = first & 0x1f; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { length = 2; code = first & 0x0f; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { length = 3; code = first & 7; minimum = 0x10000; }
        else { invalid(); continue; }
        let valid = true, consumed = 0;
        for (let offset = 0; offset < length && index + offset < byteLength; offset++) {
          const continuation = bytes[index + offset];
          const lower = offset === 0 && first === 0xe0 ? 0xa0
            : offset === 0 && first === 0xf0 ? 0x90 : 0x80;
          const upper = offset === 0 && first === 0xed ? 0x9f
            : offset === 0 && first === 0xf4 ? 0x8f : 0xbf;
          if (continuation < lower || continuation > upper) { valid = false; break; }
          code = code << 6 | continuation & 0x3f;
          consumed++;
        }
        if (index + length > byteLength) {
          if (!valid) { index += consumed; invalid(); continue; }
          if (streaming) {
            const remainingLength = byteLength - index + 1;
            this._pending = new __thawTextUint8Array(remainingLength);
            for (let pendingIndex = 0; pendingIndex < remainingLength; pendingIndex++) {
              this._pending[pendingIndex] = bytes[index - 1 + pendingIndex];
            }
            break;
          }
          invalid(); break;
        }
        if (!valid || code < minimum || code > 0x10ffff ||
            (code >= 0xd800 && code <= 0xdfff)) { index += consumed; invalid(); continue; }
        index += length;
        output += String.fromCodePoint(code);
      }
      if (!this._bomSeen && output.length) {
        this._bomSeen = true;
        if (!this.ignoreBOM && output.charCodeAt(0) === 0xfeff) output = output.slice(1);
      }
      if (!streaming) { this._pending = new __thawTextUint8Array(); this._bomSeen = false; }
      return output;
    }
  };
