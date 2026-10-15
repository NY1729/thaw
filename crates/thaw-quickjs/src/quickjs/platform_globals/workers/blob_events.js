  const blobState = new WeakMap();
  const blobWeakMapGet = WeakMap.prototype.get;
  const blobWeakMapSet = WeakMap.prototype.set;
  const blobApply = Reflect.apply;
  const blobNativeUint8Array = Uint8Array;
  const blobTypedArrayPrototype = Object.getPrototypeOf(blobNativeUint8Array.prototype);
  const blobNativeByteLength = Object.getOwnPropertyDescriptor(blobTypedArrayPrototype, 'byteLength').get;
  const blobNativeBuffer = Object.getOwnPropertyDescriptor(blobTypedArrayPrototype, 'buffer').get;
  const blobStateGet = value => blobApply(blobWeakMapGet, blobState, [value]);
  const blobByteLength = value => blobApply(blobNativeByteLength, value, []);
  const blobWriteBytes = (destination, destinationOffset, source, sourceOffset, count) => {
    for (let index = 0; index < count; index++) destination[destinationOffset + index] = source[sourceOffset + index];
  };
  const blobCopyBytes = (source, start, end) => {
    const length = blobByteLength(source);
    const from = start === undefined ? 0 : start;
    const to = end === undefined ? length : end;
    const copy = new blobNativeUint8Array(to - from);
    blobWriteBytes(copy, 0, source, from, to - from);
    return copy;
  };
  const blobTypeError = TypeError;
  const blobArrayFrom = Array.from;
  const blobIteratorSymbol = Symbol.iterator;
  const blobNativeNewline = hostInfo.platform === 'win32' ? '\r\n' : '\n';
  const blobDomString = value => `${value}`;
  const blobUsvString = value => blobDomString(value).replace(/[\uD800-\uDFFF]/gu, '\uFFFD');
  const blobLongLong = value => {
    const number = +value;
    if (!Number.isFinite(number) || number === 0) return 0;
    return Number(BigInt.asIntN(64, BigInt(Math.trunc(number))));
  };
  const blobClampLongLong = value => {
    const number = +value;
    if (Number.isNaN(number)) return 0;
    const limit = 9007199254740991;
    if (number <= -limit) return -limit;
    if (number >= limit) return limit;
    const lower = Math.floor(number), fraction = number - lower;
    const rounded = fraction < 0.5 ? lower
      : fraction > 0.5 ? lower + 1
      : lower % 2 === 0 ? lower : lower + 1;
    return rounded === 0 ? 0 : rounded;
  };
  const blobNormalizeType = value => {
    for (let index = 0; index < value.length; index++) {
      const code = value.charCodeAt(index);
      if (code < 0x20 || code > 0x7e) return '';
    }
    return value.toLowerCase();
  };
  const blobDictionary = value => {
    if (value === undefined || value === null) return {};
    if (typeof value !== 'object' && typeof value !== 'function') {
      throw new blobTypeError('Blob options must be a dictionary');
    }
    return value;
  };
  const blobOptions = (value, isFile) => {
    const dictionary = blobDictionary(value);
    const rawEndings = dictionary.endings;
    const endings = rawEndings === undefined ? 'transparent' : blobDomString(rawEndings);
    if (endings !== 'transparent' && endings !== 'native') {
      throw new blobTypeError('endings must be transparent or native');
    }
    const rawType = dictionary.type;
    const type = blobNormalizeType(rawType === undefined ? '' : blobDomString(rawType));
    let lastModified;
    if (isFile) {
      const rawLastModified = dictionary.lastModified;
      lastModified = rawLastModified === undefined ? undefined : blobLongLong(rawLastModified);
    }
    return { endings, type, lastModified };
  };
  const blobConvertPart = part => {
    const state = blobStateGet(part);
    if (state) return { kind: 'blob', value: part };
    if (part instanceof ArrayBuffer || ArrayBuffer.isView(part)) {
      return { kind: 'buffer', value: part };
    }
    return { kind: 'string', value: blobUsvString(part) };
  };
  const blobConvertParts = parts => {
    if (parts === null || (typeof parts !== 'object' && typeof parts !== 'function')) {
      throw new blobTypeError('Blob parts must be a sequence');
    }
    const method = parts[blobIteratorSymbol];
    if (typeof method !== 'function') throw new blobTypeError('Blob parts must be iterable');
    const iterable = {
      [blobIteratorSymbol]() { return blobApply(method, parts, []); },
    };
    return blobApply(blobArrayFrom, Array, [iterable, blobConvertPart]);
  };
  const blobBytes = (parts, endings) => {
    const chunks = parts.map(part => {
      if (part.kind === 'blob') return blobCopyBytes(blobStateGet(part.value).bytes);
      if (part.kind === 'buffer') {
        const value = part.value;
        const source = value instanceof ArrayBuffer
          ? new blobNativeUint8Array(value)
          : new blobNativeUint8Array(value.buffer, value.byteOffset, value.byteLength);
        return blobCopyBytes(source);
      }
      const text = endings === 'native'
        ? part.value.replace(/\r\n|\r|\n/g, blobNativeNewline)
        : part.value;
      return blobCopyBytes(new TextEncoder().encode(text));
    });
    let size = 0;
    for (const chunk of chunks) size += blobByteLength(chunk);
    const bytes = new blobNativeUint8Array(size);
    let offset = 0;
    for (const chunk of chunks) {
      const length = blobByteLength(chunk);
      blobWriteBytes(bytes, offset, chunk, 0, length);
      offset += length;
    }
    return bytes;
  };
  const blobFileCloneSnapshot = value => {
    const state = blobStateGet(value);
    if (!state) return null;
    const snapshot = {
      __thawBytes: blobCopyBytes(state.bytes),
      size: blobByteLength(state.bytes),
      type: state.type,
    };
    if (state.file) {
      snapshot.name = state.file.name;
      snapshot.lastModified = state.file.lastModified;
      snapshot.webkitRelativePath = state.file.webkitRelativePath;
    }
    return snapshot;
  };
  if (typeof globalThis.Blob !== 'function') {
    globalThis.Blob = class Blob {
      constructor(parts = [], options = {}) {
        const converted = blobConvertParts(parts);
        const settings = blobOptions(options, false);
        blobApply(blobWeakMapSet, blobState, [this, {
          bytes: blobBytes(converted, settings.endings), type: settings.type, file: null,
        }]);
      }
      get size() { return blobByteLength(blobStateGet(this).bytes); }
      get type() { return blobStateGet(this).type; }
      arrayBuffer() {
        let state;
        try { state = blobStateGet(this); if (!state) throw new blobTypeError('invalid Blob receiver'); }
        catch (error) { return Promise.reject(error); }
        const copy = blobCopyBytes(state.bytes);
        return Promise.resolve(blobApply(blobNativeBuffer, copy, []));
      }
      bytes() {
        let state;
        try { state = blobStateGet(this); if (!state) throw new blobTypeError('invalid Blob receiver'); }
        catch (error) { return Promise.reject(error); }
        return Promise.resolve(blobCopyBytes(state.bytes));
      }
      text() {
        let state;
        try { state = blobStateGet(this); if (!state) throw new blobTypeError('invalid Blob receiver'); }
        catch (error) { return Promise.reject(error); }
        return Promise.resolve(new TextDecoder().decode(blobCopyBytes(state.bytes)));
      }
      slice(start, end, type) {
        const state = blobStateGet(this);
        if (!state) throw new blobTypeError('invalid Blob receiver');
        const size = blobByteLength(state.bytes);
        const fromIndex = arguments.length === 0 || start === undefined ? 0 : blobClampLongLong(start);
        const toIndex = arguments.length < 2 || end === undefined ? size : blobClampLongLong(end);
        const contentType = arguments.length < 3 || type === undefined ? '' : blobNormalizeType(blobDomString(type));
        const from = fromIndex < 0 ? Math.max(size + fromIndex, 0) : Math.min(fromIndex, size);
        const to = toIndex < 0 ? Math.max(size + toIndex, 0) : Math.min(toIndex, size);
        const bytes = blobCopyBytes(state.bytes, from, Math.max(from, to));
        const result = new Blob();
        blobApply(blobWeakMapSet, blobState, [result, { bytes, type: contentType, file: null }]);
        return result;
      }
      stream() {
        const state = blobStateGet(this);
        if (!state) throw new blobTypeError('invalid Blob receiver');
        const bytes = blobCopyBytes(state.bytes);
        return new ReadableStream({
          type: 'bytes',
          pull(controller) {
            if (bytes.byteLength) controller.enqueue(bytes);
            controller.close();
          },
        });
      }
    };
    const blobPrototype = globalThis.Blob.prototype;
    for (const key of ['size', 'type']) {
      const descriptor = Object.getOwnPropertyDescriptor(blobPrototype, key);
      Object.defineProperty(blobPrototype, key, { ...descriptor, enumerable: true });
    }
    globalThis.File = class File extends globalThis.Blob {
      constructor(parts, name, options = {}) {
        if (arguments.length < 2) throw new blobTypeError('File requires fileBits and fileName');
        const converted = blobConvertParts(parts);
        const fileName = blobUsvString(name);
        const settings = blobOptions(options, true);
        const lastModified = settings.lastModified === undefined ? Date.now() : settings.lastModified;
        const bytes = blobBytes(converted, settings.endings);
        super();
        blobApply(blobWeakMapSet, blobState, [this, {
          bytes, type: settings.type,
          file: { name: fileName, lastModified, webkitRelativePath: '' },
        }]);
      }
      get name() { const state = blobStateGet(this); if (!state || !state.file) throw new blobTypeError('invalid File receiver'); return state.file.name; }
      get lastModified() { const state = blobStateGet(this); if (!state || !state.file) throw new blobTypeError('invalid File receiver'); return state.file.lastModified; }
      get webkitRelativePath() { const state = blobStateGet(this); if (!state || !state.file) throw new blobTypeError('invalid File receiver'); return state.file.webkitRelativePath; }
    };
    const filePrototype = globalThis.File.prototype;
    for (const key of ['name', 'lastModified', 'webkitRelativePath']) {
      const descriptor = Object.getOwnPropertyDescriptor(filePrototype, key);
      Object.defineProperty(filePrototype, key, { ...descriptor, enumerable: true });
    }
  }
  let eventListenerSignalBridge = null;
  const NativeWeakMap = WeakMap;
  const eventState = new NativeWeakMap();
  const entrySignals = new NativeWeakMap();
  const weakMapGet = NativeWeakMap.prototype.get;
  const weakMapSet = NativeWeakMap.prototype.set;
  const weakMapDelete = NativeWeakMap.prototype.delete;
  const apply = Reflect.apply;
  const eventStateOf = event => apply(weakMapGet, eventState, [event]);
  const listenerSignalOf = entry => apply(weakMapGet, entrySignals, [entry]);
  const NativeDOMException = DOMException;
  if (typeof globalThis.Event !== 'function') {
    globalThis.Event = class Event {
      constructor(type, options = {}) {
        this.type = String(type);
        this.bubbles = Boolean(options.bubbles);
        this.cancelable = Boolean(options.cancelable);
        this.composed = Boolean(options.composed);
        this.target = null;
        this.currentTarget = null;
        this.defaultPrevented = false;
        this.__thawImmediateStopped = false;
        apply(weakMapSet, eventState, [this, { dispatching: false, inPassiveListener: false }]);
      }
      preventDefault() {
        if (this.cancelable && !eventStateOf(this).inPassiveListener) this.defaultPrevented = true;
      }
      stopPropagation() {}
      stopImmediatePropagation() { this.__thawImmediateStopped = true; }
    };
  }
  if (typeof globalThis.EventTarget !== 'function') {
    const mapGet = Map.prototype.get;
    const mapSet = Map.prototype.set;
    const arraySlice = Array.prototype.slice;
    const arrayIncludes = Array.prototype.includes;
    const arrayIndexOf = Array.prototype.indexOf;
    const arraySplice = Array.prototype.splice;
    const arrayPush = Array.prototype.push;
    const arraySome = Array.prototype.some;
    const arrayFind = Array.prototype.find;
    const mapRead = (map, key) => apply(mapGet, map, [key]);
    const mapWrite = (map, key, value) => apply(mapSet, map, [key, value]);
    const liveRecords = (target, type) => mapRead(target.__thawListeners, type) || [];
    const removeEventListenerRecord = (target, type, entry) => {
      const list = liveRecords(target, type);
      const index = apply(arrayIndexOf, list, [entry]);
      if (index === -1) return;
      apply(arraySplice, list, [index, 1]);
      const binding = listenerSignalOf(entry);
      if (binding) {
        apply(weakMapDelete, entrySignals, [entry]);
        eventListenerSignalBridge.remove(binding.state, binding.algorithm);
      }
      if (eventListenerSignalBridge) eventListenerSignalBridge.listenersChanged(target);
    };
    globalThis.EventTarget = class EventTarget {
      constructor() { this.__thawListeners = new Map(); }
      addEventListener(type, callback, options = {}) {
        const callable = callback == null || typeof callback === 'function'
          || typeof callback.handleEvent === 'function';
        if (!callable) {
          throw new TypeError('event listener must be callable');
        }
        const name = String(type);
        let capture = false, once = false, passive = false, signal;
        if (typeof options === 'boolean') capture = options;
        else if (options != null) {
          capture = Boolean(options.capture);
          once = Boolean(options.once);
          passive = Boolean(options.passive);
          signal = options.signal;
        }
        let signalState;
        if (signal != null) {
          signalState = eventListenerSignalBridge && eventListenerSignalBridge.stateOf(signal);
          if (!signalState) throw new TypeError('signal must be an AbortSignal');
          if (signalState.aborted) return;
        }
        if (callback == null) return;
        const listeners = liveRecords(this, name);
        if (apply(arraySome, listeners,
          [entry => entry.callback === callback && entry.capture === capture])) return;
        const entry = { callback, capture, once, passive };
        if (!mapRead(this.__thawListeners, name)) mapWrite(this.__thawListeners, name, listeners);
        apply(arrayPush, listeners, [entry]);
        if (signalState) {
          const algorithm = () => removeEventListenerRecord(this, name, entry);
          apply(weakMapSet, entrySignals, [entry, { state: signalState, algorithm }]);
          eventListenerSignalBridge.add(signalState, algorithm);
        }
        if (eventListenerSignalBridge) eventListenerSignalBridge.listenersChanged(this);
      }
      removeEventListener(type, callback, options = false) {
        const name = String(type);
        const capture = typeof options === 'boolean' ? options
          : options == null ? false : Boolean(options.capture);
        const listeners = liveRecords(this, name);
        const entry = apply(arrayFind, listeners,
          [item => item.callback === callback && item.capture === capture]);
        if (entry) removeEventListenerRecord(this, name, entry);
      }
      dispatchEvent(event) {
        if (!(event instanceof Event)) throw new TypeError('expected an Event');
        const state = eventStateOf(event);
        if (state.dispatching) {
          throw new NativeDOMException('The event is already being dispatched', 'InvalidStateError');
        }
        state.dispatching = true;
        let result;
        let thrown = false;
        let firstError;
        try {
          const type = event.type;
          event.target = this;
          event.currentTarget = this;
          event.__thawImmediateStopped = false;
          const listeners = apply(arraySlice, liveRecords(this, type), []);
          for (const entry of listeners) {
            // ponytail: scan the live list; use removal flags if dispatch lists grow large.
            if (!apply(arrayIncludes, liveRecords(this, type), [entry])) continue;
            if (entry.once) removeEventListenerRecord(this, type, entry);
            state.inPassiveListener = entry.passive;
            try {
              if (typeof entry.callback === 'function') apply(entry.callback, this, [event]);
              else apply(entry.callback.handleEvent, entry.callback, [event]);
            } catch (error) { globalThis.__thaw_report_uncaught(error); }
            finally { state.inPassiveListener = false; }
            if (event.__thawImmediateStopped) break;
          }
          result = !event.defaultPrevented;
        } catch (error) {
          thrown = true;
          firstError = error;
        } finally {
          try { event.currentTarget = null; }
          catch (error) { if (!thrown) { thrown = true; firstError = error; } }
          try { event.__thawImmediateStopped = false; }
          catch (error) { if (!thrown) { thrown = true; firstError = error; } }
          finally {
            state.inPassiveListener = false;
            state.dispatching = false;
          }
        }
        if (thrown) throw firstError;
        return result;
      }
    };
  }
  if (typeof globalThis.MessageEvent !== 'function') {
    globalThis.MessageEvent = class MessageEvent extends Event {
      constructor(type, options = {}) {
        super(type, options);
        this.data = options.data === undefined ? null : options.data;
        this.origin = options.origin === undefined ? '' : String(options.origin);
        this.lastEventId = options.lastEventId === undefined ? '' : String(options.lastEventId);
        this.source = options.source === undefined ? null : options.source;
        this.ports = options.ports === undefined ? [] : Array.from(options.ports);
      }
    };
  }
