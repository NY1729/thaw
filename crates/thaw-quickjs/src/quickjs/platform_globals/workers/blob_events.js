  if (typeof globalThis.Blob !== 'function') {
    globalThis.Blob = class Blob {
      constructor(parts = [], options = {}) {
        const chunks = Array.from(parts, part => {
          if (part instanceof Blob) return part.__thawBytes;
          if (part instanceof ArrayBuffer) return new Uint8Array(part);
          if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
          return new TextEncoder().encode(String(part));
        });
        const size = chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
        this.__thawBytes = new Uint8Array(size);
        let offset = 0;
        for (const chunk of chunks) { this.__thawBytes.set(chunk, offset); offset += chunk.byteLength; }
        this.size = size;
        this.type = String(options.type || '').toLowerCase();
      }
      arrayBuffer() { const copy = this.__thawBytes.slice(); return Promise.resolve(copy.buffer); }
      bytes() { return Promise.resolve(this.__thawBytes.slice()); }
      text() { return Promise.resolve(new TextDecoder().decode(this.__thawBytes)); }
      slice(start = 0, end = this.size, type = '') {
        const from = start < 0 ? Math.max(this.size + Number(start), 0) : Math.min(Number(start), this.size);
        const to = end < 0 ? Math.max(this.size + Number(end), 0) : Math.min(Number(end), this.size);
        return new Blob([this.__thawBytes.slice(from, Math.max(from, to))], { type });
      }
      stream() {
        const bytes = this.__thawBytes.slice();
        return new ReadableStream({
          type: 'bytes',
          pull(controller) {
            if (bytes.byteLength) controller.enqueue(bytes);
            controller.close();
          },
        });
      }
    };
    globalThis.File = class File extends Blob {
      constructor(parts, name, options = {}) { super(parts, options); this.name = String(name); this.lastModified = options.lastModified === undefined ? Date.now() : Number(options.lastModified); this.webkitRelativePath = ''; }
    };
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
            } finally { state.inPassiveListener = false; }
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
