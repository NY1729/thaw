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
    encode(input = '') { return Uint8Array.from(encodeUtf8(input)); }
    encodeInto(input, destination) {
      if (!(destination instanceof Uint8Array)) {
        throw new TypeError('destination must be a Uint8Array');
      }
      let read = 0;
      let written = 0;
      for (const character of String(input)) {
        const bytes = encodeUtf8(character);
        if (written + bytes.length > destination.length) break;
        destination.set(bytes, written);
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
      this._pending = new Uint8Array();
      this._bomSeen = false;
    }
    decode(input = new Uint8Array(), options = {}) {
      const chunk = input instanceof ArrayBuffer
        ? new Uint8Array(input)
        : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      const streaming = Boolean(options.stream), bytes = new Uint8Array(this._pending.byteLength + chunk.byteLength);
      bytes.set(this._pending); bytes.set(chunk, this._pending.byteLength); this._pending = new Uint8Array();
      let output = '';
      let index = 0;
      const invalid = () => {
        if (this.fatal) throw new TypeError('invalid UTF-8 data');
        output += replacement;
      };
      while (index < bytes.length) {
        const first = bytes[index++];
        if (first <= 0x7f) { output += String.fromCodePoint(first); continue; }
        let length, code, minimum;
        if (first >= 0xc2 && first <= 0xdf) { length = 1; code = first & 0x1f; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { length = 2; code = first & 0x0f; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { length = 3; code = first & 7; minimum = 0x10000; }
        else { invalid(); continue; }
        let valid = true, consumed = 0;
        for (let offset = 0; offset < length && index + offset < bytes.length; offset++) {
          const continuation = bytes[index + offset];
          if ((continuation & 0xc0) !== 0x80) { valid = false; break; }
          code = code << 6 | continuation & 0x3f;
          consumed++;
        }
        if (index + length > bytes.length) {
          if (!valid) { index += consumed; invalid(); continue; }
          if (streaming) { this._pending = bytes.slice(index - 1); break; }
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
      if (!streaming) { this._pending = new Uint8Array(); this._bomSeen = false; }
      return output;
    }
  };

