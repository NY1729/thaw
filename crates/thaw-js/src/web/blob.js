// `Blob` / `File` (Node 22 shape): bytes and metadata live in closure-private WeakMaps, never on the object, and every
// intrinsic the algorithms use is captured here, so user-patched typed-array / TextDecoder / WeakMap members and own
// properties on a Blob cannot observe or change them. Conversion order and error codes follow lib/internal/blob.js.
const apply = Reflect.apply;
const U8 = Uint8Array;
const TypedArrayProto = Object.getPrototypeOf(U8.prototype);
const getter = (proto, key) => Object.getOwnPropertyDescriptor(proto, key).get;
const taBuffer = getter(TypedArrayProto, 'buffer');
const taOffset = getter(TypedArrayProto, 'byteOffset');
const taByteLength = getter(TypedArrayProto, 'byteLength');
const u8Set = U8.prototype.set;
const u8Slice = U8.prototype.slice;
const abByteLength = getter(ArrayBuffer.prototype, 'byteLength');
const wmGet = WeakMap.prototype.get, wmSet = WeakMap.prototype.set, wmHas = WeakMap.prototype.has;
const brand = __thaw_brand;
const decode = TextDecoder.prototype.decode, encode = TextEncoder.prototype.encode;
const decoder = new TextDecoder(), encoder = new TextEncoder();
const toLower = String.prototype.toLowerCase, charCodeAt = String.prototype.charCodeAt, replace = String.prototype.replace;
const now = Date.now;
const nativeNewline = typeof process === 'object' && process !== null && process.platform === 'win32' ? '\r\n' : '\n';
const ReadableStreamCtor = () => globalThis.ReadableStream;
const kInspect = Symbol.for('nodejs.util.inspect.custom');
const blobs = new WeakMap(), files = new WeakMap();
const VIEWS = ['Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array', 'Int32Array', 'Uint32Array',
  'Float16Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array'];

function nodeError(Ctor, code, message) { const e = new Ctor(message); e.code = code; return e; }
function received(value) {
  if (value === undefined || value === null) return 'Received ' + value;
  if (typeof value === 'function') return 'Received function ' + value.name;
  if (typeof value === 'object') return 'Received an instance of ' + ((value.constructor && value.constructor.name) || 'Object');
  if (typeof value === 'string') return "Received type string ('" + value + "')";
  return 'Received type ' + typeof value + ' (' + String(value) + ')';
}
const invalidThis = () => nodeError(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type Blob');
const stateOf = self => {
  if (!apply(wmHas, blobs, [self])) throw invalidThis();
  return apply(wmGet, blobs, [self]);
};
function normalizeType(type) {
  for (let i = 0; i < type.length; i++) {
    const c = apply(charCodeAt, type, [i]);
    if (c < 0x20 || c > 0x7e) return '';
  }
  return apply(toLower, type, []);
}
function make(proto, bytes, type) {
  const blob = Object.create(proto);
  apply(wmSet, blobs, [blob, { bytes, type }]);
  return blob;
}
function copyOf(bytes) { return apply(u8Slice, bytes, []); }
function viewBytes(v) {
  const len = apply(taByteLength, v, []);
  return new U8(apply(taBuffer, v, []), apply(taOffset, v, []), len);
}
function sourceBytes(source, native) {
  if (typeof source === 'object' && source !== null) {
    if (apply(wmHas, blobs, [source])) return apply(wmGet, blobs, [source]).bytes;
    const kind = brand(source);
    if (kind === 'ArrayBuffer' || kind === 'SharedArrayBuffer') return new U8(source, 0, apply(abByteLength, source, []));
    if (kind === 'DataView') return new U8(source.buffer, source.byteOffset, source.byteLength);
    if (VIEWS.indexOf(kind) !== -1) return viewBytes(source);
  }
  let text = `${source}`;
  if (native) text = apply(replace, text, [/\r\n/g, nativeNewline]);
  return apply(encode, encoder, [text]);
}
function sequence(sources) {
  if (sources === null || typeof sources === 'string' || (sources !== undefined && typeof sources[Symbol.iterator] !== 'function')) {
    throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "sources" argument must be a sequence. ' + received(sources));
  }
  return sources;
}
class Blob {
  constructor(sources = [], options) {
    sequence(sources);
    if (options !== undefined && options !== null && typeof options !== 'object' && typeof options !== 'function') {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be a dictionary. ' + received(options));
    }
    let type = '', endings = 'transparent';
    if (options !== undefined && options !== null) {
      const t = options.type, e = options.endings;
      if (t !== undefined) type = t;
      if (e !== undefined) endings = e;
    }
    endings = `${endings}`;
    if (endings !== 'transparent' && endings !== 'native') {
      throw nodeError(TypeError, 'ERR_INVALID_ARG_VALUE', "The property 'options.endings' is invalid. Received '" + endings + "'");
    }
    const native = endings === 'native';
    const chunks = [];
    let length = 0;
    const iterator = apply(sources[Symbol.iterator], sources, []);
    const next = iterator.next;
    for (;;) {
      const step = apply(next, iterator, []);
      if (step === null || (typeof step !== 'object' && typeof step !== 'function')) throw new TypeError('Iterator result ' + step + ' is not an object');
      if (step.done) break;
      try {
        const chunk = sourceBytes(step.value, native);
        chunks.push(chunk);
        length += chunk.length;
      } catch (error) {
        try { const close = iterator.return; if (close !== undefined && close !== null) apply(close, iterator, []); } catch (_) {}
        throw error;
      }
    }
    const bytes = new U8(length);
    let at = 0;
    for (let i = 0; i < chunks.length; i++) { apply(u8Set, bytes, [chunks[i], at]); at += chunks[i].length; }
    apply(wmSet, blobs, [this, { bytes, type: normalizeType(`${type}`) }]);
  }
  get size() { return stateOf(this).bytes.length; }
  get type() { return stateOf(this).type; }
  slice(start = 0, end, contentType = '') {
    const state = stateOf(this), len = state.bytes.length;
    start |= 0;
    start = start < 0 ? Math.max(len + start, 0) : Math.min(start, len);
    end = end === undefined ? len : end | 0;
    end = end < 0 ? Math.max(len + end, 0) : Math.min(end, len);
    const type = normalizeType(`${contentType}`);
    return make(Blob.prototype, end > start ? apply(u8Slice, state.bytes, [start, end]) : new U8(0), type);
  }
  arrayBuffer() {
    let state;
    try { state = stateOf(this); } catch (error) { return Promise.reject(error); }
    return Promise.resolve(copyOf(state.bytes).buffer);
  }
  text() {
    let state;
    try { state = stateOf(this); } catch (error) { return Promise.reject(error); }
    return Promise.resolve(apply(decode, decoder, [state.bytes]));
  }
  bytes() {
    let state;
    try { state = stateOf(this); } catch (error) { return Promise.reject(error); }
    return Promise.resolve(copyOf(state.bytes));
  }
  stream() {
    const bytes = stateOf(this).bytes;
    return new (ReadableStreamCtor())({ start(controller) { if (bytes.length) controller.enqueue(copyOf(bytes)); controller.close(); } });
  }
  [kInspect](depth, options, inspect) {
    if (!apply(wmHas, blobs, [this]) || depth < 0) return this;
    const fields = { size: this.size, type: this.type };
    if (typeof inspect !== 'function') return `Blob { size: ${fields.size}, type: '${fields.type}' }`;
    const opts = { ...options, depth: options.depth == null ? null : options.depth - 1 };
    return `${this.constructor.name} ${inspect(fields, opts)}`;
  }
}
class File extends Blob {
  constructor(fileBits, fileName, options) {
    if (arguments.length < 2) throw nodeError(TypeError, 'ERR_MISSING_ARGS', 'The "fileBits" and "fileName" arguments must be specified');
    super(fileBits, options);
    let lastModified;
    if (options !== undefined && options !== null) lastModified = options.lastModified;
    if (lastModified !== undefined) { lastModified = +lastModified; if (lastModified !== lastModified) lastModified = 0; }
    else lastModified = apply(now, Date, []);
    const name = `${fileName}`;
    apply(wmSet, files, [this, { name: name.toWellFormed(), lastModified }]);
  }
  get name() { return fileState(this).name; }
  get lastModified() { return fileState(this).lastModified; }
  [kInspect](depth, options, inspect) {
    if (!apply(wmHas, files, [this]) || depth < 0) return this;
    if (typeof inspect !== 'function') return `File { size: ${this.size}, type: '${this.type}', name: '${this.name}', lastModified: ${this.lastModified} }`;
    const opts = { ...options, depth: options.depth == null ? null : options.depth - 1 };
    return `File ${inspect({ size: this.size, type: this.type, name: this.name, lastModified: this.lastModified }, opts)}`;
  }
}
function fileState(self) {
  if (!apply(wmHas, files, [self])) throw new TypeError('Cannot read private member #name from an object whose class did not declare it');
  return apply(wmGet, files, [self]);
}
// the accessors and methods are enumerable, as in Node
for (const [proto, keys] of [[Blob.prototype, ['size', 'type', 'slice', 'arrayBuffer', 'text', 'bytes', 'stream']], [File.prototype, ['name', 'lastModified']]]) {
  for (const key of keys) Object.defineProperty(proto, key, { ...Object.getOwnPropertyDescriptor(proto, key), enumerable: true });
}
Object.defineProperty(Blob.prototype, Symbol.toStringTag, { value: 'Blob', configurable: true });
Object.defineProperty(File.prototype, Symbol.toStringTag, { value: 'File', configurable: true });
// structuredClone / postMessage of a Blob (a File too) yields a plain Blob of the same bytes and type
Object.defineProperty(globalThis, '__thaw_blob_clone', {
  value: function (v) {
    if (typeof v !== 'object' || v === null || !apply(wmHas, blobs, [v])) return undefined;
    const state = apply(wmGet, blobs, [v]);
    return make(Blob.prototype, copyOf(state.bytes), state.type);
  }, writable: false, enumerable: false, configurable: false
});
Object.defineProperty(globalThis, 'Blob', { value: Blob, writable: true, enumerable: false, configurable: true });
Object.defineProperty(globalThis, 'File', { value: File, writable: true, enumerable: false, configurable: true });
