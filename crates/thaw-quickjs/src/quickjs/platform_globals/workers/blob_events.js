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
        const bytes = this.__thawBytes.slice(); let done = false;
        return { getReader() { return { read() { if (done) return Promise.resolve({ value: undefined, done: true }); done = true; return Promise.resolve({ value: bytes, done: false }); }, releaseLock() {} }; }, [Symbol.asyncIterator]() { return { next() { if (done) return Promise.resolve({ value: undefined, done: true }); done = true; return Promise.resolve({ value: bytes, done: false }); }, [Symbol.asyncIterator]() { return this; } }; } };
      }
    };
    globalThis.File = class File extends Blob {
      constructor(parts, name, options = {}) { super(parts, options); this.name = String(name); this.lastModified = options.lastModified === undefined ? Date.now() : Number(options.lastModified); this.webkitRelativePath = ''; }
    };
  }
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
      }
      preventDefault() {
        if (this.cancelable) this.defaultPrevented = true;
      }
      stopPropagation() {}
      stopImmediatePropagation() { this.__thawImmediateStopped = true; }
    };
  }
  if (typeof globalThis.EventTarget !== 'function') {
    globalThis.EventTarget = class EventTarget {
      constructor() { this.__thawListeners = new Map(); }
      addEventListener(type, callback, options = {}) {
        if (callback == null) return;
        if (typeof callback !== 'function'
            && typeof callback.handleEvent !== 'function') {
          throw new TypeError('event listener must be callable');
        }
        const name = String(type);
        const listeners = this.__thawListeners.get(name) || [];
        if (!listeners.some(entry => entry.callback === callback)) {
          listeners.push({ callback, once: Boolean(options && options.once) });
          this.__thawListeners.set(name, listeners);
        }
      }
      removeEventListener(type, callback) {
        const name = String(type);
        const listeners = this.__thawListeners.get(name);
        if (listeners) {
          this.__thawListeners.set(name,
            listeners.filter(entry => entry.callback !== callback));
        }
      }
      dispatchEvent(event) {
        if (!(event instanceof Event)) throw new TypeError('expected an Event');
        event.target = this;
        event.currentTarget = this;
        event.__thawImmediateStopped = false;
        const listeners = (this.__thawListeners.get(event.type) || []).slice();
        for (const entry of listeners) {
          if (entry.once) this.removeEventListener(event.type, entry.callback);
          if (typeof entry.callback === 'function') entry.callback.call(this, event);
          else entry.callback.handleEvent(event);
          if (event.__thawImmediateStopped) break;
        }
        event.currentTarget = null;
        return !event.defaultPrevented;
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
