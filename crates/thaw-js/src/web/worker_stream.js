// Private stream shim for worker stdio (W5): a push-style Readable and a sink-style Writable, just enough for
// worker.stdin/stdout/stderr and the worker's process.stdin. Not node:stream (no dependency). Installed as the frozen,
// non-enumerable global `__thaw_worker_stream` in every realm; worker_threads.js and worker_bootstrap.js use it.
(function () {
  // ponytail: a copy of worker_threads.js's Emitter (kept there too, so the W3 files stay untouched)
  class Emitter {
    constructor() {
      Object.defineProperty(this, '__thawListeners', { value: new Map() });
    }
    __thawList(name) {
      if (!this.__thawListeners.has(name)) this.__thawListeners.set(name, []);
      return this.__thawListeners.get(name);
    }
    on(name, listener) { this.__thawList(name).push({ listener, once: false }); return this; }
    addListener(name, listener) { return this.on(name, listener); }
    once(name, listener) { this.__thawList(name).push({ listener, once: true }); return this; }
    off(name, listener) {
      const list = this.__thawList(name);
      const index = list.findIndex(entry => entry.listener === listener);
      if (index >= 0) list.splice(index, 1);
      return this;
    }
    removeListener(name, listener) { return this.off(name, listener); }
    emit(name, ...args) {
      const list = this.__thawList(name);
      if (list.length === 0) {
        if (name === 'error') throw args[0];
        return false;
      }
      for (const entry of list.slice()) {
        if (entry.once) list.splice(list.indexOf(entry), 1);
        entry.listener.apply(this, args);
      }
      return true;
    }
  }

  const bytesToHex = bytes => Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
  const hexToBytes = hex => {
    const bytes = new Uint8Array(hex.length >> 1);
    for (let index = 0; index < bytes.length; index++) bytes[index] = parseInt(hex.slice(index * 2, index * 2 + 2), 16);
    return bytes;
  };

  // Readable: chunks are pushed by the owner (__thawPush / __thawEnd, then __thawFlush). Chunks are Uint8Array
  // unless setEncoding was called (then strings, decoded as UTF-8). `data` is delivered only while flowing and
  // re-checked per chunk, so a handler's pause() holds back the rest; `end` only after the buffer drains.
  class Readable extends Emitter {
    constructor() {
      super();
      this.buf = [];
      this.flowing = false;
      this.ended = false;
      this.endEmitted = false;
      this.destroyed = false;
      this.encoding = null;
      this.decoder = null;
      // ponytail: UTF-8 only; other encodings are treated as utf8
      this.onChange = null;
    }
    get readable() { return !this.endEmitted && !this.destroyed; }
    get readableFlowing() { return this.flowing; }
    changed() { if (this.onChange) this.onChange(); }
    setEncoding(encoding) {
      this.encoding = encoding;
      this.decoder = new TextDecoder('utf-8');
      return this;
    }
    on(name, listener) {
      super.on(name, listener);
      if (name === 'data') this.resume();
      return this;
    }
    once(name, listener) {
      super.once(name, listener);
      if (name === 'data') this.resume();
      return this;
    }
    resume() {
      this.flowing = true;
      this.changed();
      queueMicrotask(() => this.__thawFlush());
      return this;
    }
    pause() {
      this.flowing = false;
      this.changed();
      return this;
    }
    isPaused() { return !this.flowing; }
    read() {
      if (this.buf.length === 0) return null;
      const bytes = this.buf.shift();
      return this.encoding ? this.decoder.decode(bytes, { stream: true }) : bytes;
    }
    pipe(dest) {
      this.on('data', chunk => dest.write(chunk));
      this.on('end', () => { if (typeof dest.end === 'function') dest.end(); });
      return dest;
    }
    destroy() {
      this.destroyed = true;
      this.buf = [];
      this.flowing = false;
      this.changed();
      return this;
    }
    __thawPush(bytes) {
      if (!this.destroyed && !this.endEmitted) this.buf.push(bytes);
    }
    __thawEnd() {
      this.ended = true;
    }
    __thawFlush() {
      while (this.flowing && this.buf.length > 0) {
        const bytes = this.buf.shift();
        const chunk = this.encoding ? this.decoder.decode(bytes, { stream: true }) : bytes;
        if (chunk.length !== 0) this.emit('data', chunk);
      }
      if (this.flowing && this.ended && this.buf.length === 0 && !this.endEmitted) {
        this.endEmitted = true;
        const tail = this.encoding ? this.decoder.decode() : '';
        if (tail.length !== 0) this.emit('data', tail);
        this.emit('end');
        this.emit('close');
      }
      this.changed();
    }
  }

  // Writable: every write goes to `sink(hex, end)` as lower-case hex bytes (UTF-8 for strings).
  // ponytail: unbounded, write() always returns true and never emits 'drain'; add back-pressure when a package needs it
  class Writable extends Emitter {
    constructor(sink) {
      super();
      this.sink = sink;
      this.writable = true;
      this.writableEnded = false;
      this.destroyed = false;
    }
    write(chunk, encoding, callback) {
      if (typeof encoding === 'function') callback = encoding;
      if (this.writableEnded || this.destroyed) {
        const error = new Error('write after end');
        error.code = 'ERR_STREAM_WRITE_AFTER_END';
        queueMicrotask(() => {
          if (typeof callback === 'function') callback(error);
          this.emit('error', error);
        });
        return false;
      }
      const bytes = typeof chunk === 'string' ? new TextEncoder().encode(chunk) : chunk;
      this.sink(bytesToHex(bytes), false);
      if (typeof callback === 'function') queueMicrotask(callback);
      return true;
    }
    end(chunk, encoding, callback) {
      if (typeof chunk === 'function') callback = chunk;
      else if (chunk !== undefined && chunk !== null) this.write(chunk, typeof encoding === 'function' ? undefined : encoding);
      if (!this.writableEnded && !this.destroyed) {
        this.writableEnded = true;
        this.writable = false;
        this.sink('', true);
        queueMicrotask(() => {
          this.emit('finish');
          this.emit('close');
          if (typeof callback === 'function') callback();
        });
      }
      return this;
    }
    destroy() {
      this.destroyed = true;
      this.writable = false;
      return this;
    }
    cork() {}
    uncork() {}
  }

  Object.defineProperty(globalThis, '__thaw_worker_stream', {
    value: Object.freeze({ Readable, Writable, bytesToHex, hexToBytes }),
    configurable: false, enumerable: false, writable: false,
  });
})();
