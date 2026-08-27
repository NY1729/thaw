  if (typeof globalThis.Headers !== 'function') {
    const headerLists = new WeakMap();
    const headerName = value => {
      const name = String(value).toLowerCase();
      if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(name)) throw new TypeError(`invalid header name: ${value}`);
      return name;
    };
    const headerValue = value => {
      const text = String(value);
      if (/[\0\r\n]/.test(text)) throw new TypeError(`invalid header value: ${text}`);
      return text.replace(/^[\t ]+|[\t ]+$/g, '');
    };
    const headersList = value => { const list = headerLists.get(value); if (!list) throw new TypeError('invalid Headers receiver'); return list; };
    const normalizedHeaderEntries = value => {
      const grouped = new Map();
      for (const [name, item] of headersList(value)) { const values = grouped.get(name) || []; values.push(item); grouped.set(name, values); }
      const entries = [];
      for (const name of Array.from(grouped.keys()).sort()) {
        const values = grouped.get(name);
        if (name === 'set-cookie') for (const item of values) entries.push([name, item]);
        else entries.push([name, values.join(', ')]);
      }
      return entries;
    };
    class Headers {
      constructor(init = undefined) {
        headerLists.set(this, []);
        if (init === undefined) return;
        if (headerLists.has(init)) { for (const [name, value] of headersList(init)) this.append(name, value); return; }
        if (init !== null && typeof init[Symbol.iterator] === 'function') {
          for (const entry of init) { const pair = Array.from(entry); if (pair.length !== 2) throw new TypeError('header pair must contain exactly two items'); this.append(pair[0], pair[1]); }
          return;
        }
        if (init === null || (typeof init !== 'object' && typeof init !== 'function')) throw new TypeError('Headers init must be an object');
        for (const name of Object.keys(init)) this.append(name, init[name]);
      }
      append(name, value) { headersList(this).push([headerName(name), headerValue(value)]); }
      delete(name) { const normalized = headerName(name), list = headersList(this); headerLists.set(this, list.filter(entry => entry[0] !== normalized)); }
      get(name) { const normalized = headerName(name), values = headersList(this).filter(entry => entry[0] === normalized).map(entry => entry[1]); return values.length ? values.join(', ') : null; }
      has(name) { const normalized = headerName(name); return headersList(this).some(entry => entry[0] === normalized); }
      set(name, value) { const normalized = headerName(name), text = headerValue(value), list = headersList(this).filter(entry => entry[0] !== normalized); list.push([normalized, text]); headerLists.set(this, list); }
      getSetCookie() { return headersList(this).filter(entry => entry[0] === 'set-cookie').map(entry => entry[1]); }
      *keys() { for (const entry of normalizedHeaderEntries(this)) yield entry[0]; }
      *values() { for (const entry of normalizedHeaderEntries(this)) yield entry[1]; }
      *entries() { yield* normalizedHeaderEntries(this); }
      forEach(callback, thisArg = undefined) { if (typeof callback !== 'function') throw new TypeError('callback must be a function'); for (const [name, value] of normalizedHeaderEntries(this)) callback.call(thisArg, value, name, this); }
      [Symbol.iterator]() { return this.entries(); }
      get [Symbol.toStringTag]() { return 'Headers'; }
    }
    for (const name of ['append', 'delete', 'get', 'has', 'set', 'getSetCookie', 'keys', 'values', 'entries', 'forEach']) {
      const descriptor = Object.getOwnPropertyDescriptor(Headers.prototype, name); descriptor.enumerable = true; Object.defineProperty(Headers.prototype, name, descriptor);
    }
    globalThis.Headers = Headers;
  }
  if (typeof globalThis.ReadableStream !== 'function') {
    const webInvalidState = message => { const error = new TypeError(message); error.code = 'ERR_INVALID_STATE'; return error; };
    class ReadableStreamDefaultController {
      constructor(stream) { this._stream = stream; }
      enqueue(chunk) {
        const stream = this._stream;
        if (stream._state !== 'readable' || stream._closeRequested) throw webInvalidState('Controller is already closed');
        if (stream._reads.length) stream._reads.shift().resolve({ value: chunk, done: false });
        else { let size; try { size = stream._chunkSize(chunk); } catch (error) { this.error(error); throw error; } stream._queue.push(chunk); stream._queueSizes.push(size); stream._queueTotalSize += size; }
        stream._notifyCapacity(); stream._callPullIfNeeded();
      }
      close() {
        const stream = this._stream; if (stream._state !== 'readable' || stream._closeRequested) throw webInvalidState('Controller is already closed');
        stream._closeRequested = true; stream._finishCloseIfReady();
      }
      error(error) {
        const stream = this._stream; if (stream._state !== 'readable') return;
        stream._state = 'errored'; stream._error = error; stream._queue.length = 0; stream._queueSizes.length = 0; stream._queueTotalSize = 0; stream._rejectCapacity(error); while (stream._reads.length) stream._reads.shift().reject(error); stream._rejectClosed(error);
      }
      get desiredSize() { return this._stream._state === 'readable' ? this._stream._highWaterMark - this._stream._queueTotalSize : null; }
    }
    const byobResultView = (view, byteLength) => view instanceof DataView
      ? new DataView(view.buffer, view.byteOffset, byteLength)
      : new view.constructor(view.buffer, view.byteOffset, Math.floor(byteLength / view.BYTES_PER_ELEMENT));
    class ReadableStreamBYOBRequest {
      constructor(stream, read) { this._stream = stream; this._read = read; this.view = read.view; }
      respond(bytesWritten) {
        const stream = this._stream, read = this._read, length = Number(bytesWritten);
        if (!stream || stream._byobRequest !== this) throw webInvalidState('This BYOB request has been invalidated');
        if (!Number.isInteger(length) || length < 0 || length > this.view.byteLength) { const error = new RangeError('invalid BYOB response length'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        if (length === 0 && stream._state === 'readable') throw new TypeError('BYOB response must write bytes');
        stream._byobRequest = null; this._stream = null; this._read = null; this.view = null;
        read.filled += length;
        if (read.filled >= read.minBytes || stream._state !== 'readable') {
          stream._byobReads.shift(); read.resolve({ value: byobResultView(read.view, read.filled), done: read.filled === 0 });
          stream._prepareByobRequest();
        } else {
          stream._prepareByobRequest();
        }
      }
      respondWithNewView(view) {
        if (!this._stream || this._stream._byobRequest !== this) throw webInvalidState('This BYOB request has been invalidated');
        if (!ArrayBuffer.isView(view) || view.buffer !== this.view.buffer || view.byteOffset !== this.view.byteOffset || view.byteLength > this.view.byteLength) { const error = new RangeError('invalid BYOB response view'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        this.respond(view.byteLength);
      }
    }
    class ReadableByteStreamController {
      constructor(stream) { this._stream = stream; }
      enqueue(chunk) {
        const stream = this._stream;
        if (stream._state !== 'readable' || stream._closeRequested) throw webInvalidState('Controller is already closed');
        if (!ArrayBuffer.isView(chunk) || chunk.byteLength === 0) throw new TypeError('byte stream chunks must be non-empty ArrayBuffer views');
        stream._queue.push(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength)); stream._queueSizes.push(chunk.byteLength); stream._queueTotalSize += chunk.byteLength; stream._drainByob();
        if (stream._reads.length && stream._queue.length) stream._reads.shift().resolve({ value: stream._dequeue(), done: false });
        stream._notifyCapacity(); stream._callPullIfNeeded();
      }
      close() {
        const stream = this._stream; if (stream._state !== 'readable' || stream._closeRequested) throw webInvalidState('Controller is already closed');
        if (stream._byobReads.length && stream._byobReads[0].filled % stream._byobReads[0].elementSize !== 0) throw webInvalidState('Partial read');
        stream._closeRequested = true; stream._drainByob(); stream._finishCloseIfReady();
      }
      error(error) {
        const stream = this._stream; if (stream._state !== 'readable') return;
        stream._state = 'errored'; stream._error = error; stream._queue.length = 0; stream._queueSizes.length = 0; stream._queueTotalSize = 0; stream._byobRequest = null; stream._rejectCapacity(error); while (stream._reads.length) stream._reads.shift().reject(error); while (stream._byobReads.length) stream._byobReads.shift().reject(error); stream._rejectClosed(error);
      }
      get byobRequest() { return this._stream._byobRequest; }
      get desiredSize() { return this._stream._state === 'readable' ? this._stream._highWaterMark - this._stream._queueTotalSize : null; }
    }
    class ReadableStreamDefaultReader {
      constructor(stream) { if (stream.locked) throw webInvalidState('ReadableStream is locked'); this._stream = stream; stream._reader = this; this.closed = stream._closed; }
      read() {
        const stream = this._stream; if (!stream) return Promise.reject(webInvalidState('Reader is released'));
        stream._disturbed = true;
        if (stream._byteStream && stream._autoAllocateChunkSize) return stream._readInto(new Uint8Array(stream._autoAllocateChunkSize), this, 1);
        if (stream._queue.length) { const value = stream._dequeue(); stream._finishCloseIfReady(); stream._callPullIfNeeded(); return Promise.resolve({ value, done: false }); }
        if (stream._state === 'closed') return Promise.resolve({ value: undefined, done: true });
        if (stream._state === 'errored') return Promise.reject(stream._error);
        const result = new Promise((resolve, reject) => stream._reads.push({ reader: this, resolve, reject }));
        stream._notifyCapacity(); stream._callPullIfNeeded();
        return result;
      }
      cancel(reason) { if (this._stream) this._stream._disturbed = true; return this._stream ? this._stream._cancel(reason) : Promise.reject(webInvalidState('Reader is released')); }
      releaseLock() { if (!this._stream) return; const stream = this._stream, error = webInvalidState('Reader was released'); stream._reads = stream._reads.filter(read => { if (read.reader !== this) return true; read.reject(error); return false; }); stream._byobReads = stream._byobReads.filter(read => { if (read.reader !== this) return true; read.reject(error); return false; }); if (stream._byobRequest && stream._byobRequest._read.reader === this) stream._byobRequest = null; stream._reader = null; this._stream = null; this.closed = Promise.reject(error); this.closed.catch(() => {}); }
    }
    class ReadableStreamBYOBReader {
      constructor(stream) { if (stream.locked) throw webInvalidState('ReadableStream is locked'); if (!stream._byteStream) { const error = new TypeError('stream must be a byte stream'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } this._stream = stream; stream._reader = this; this.closed = stream._closed; }
      read(view, options = {}) {
        const stream = this._stream; if (!stream) return Promise.reject(webInvalidState('Reader is released'));
        stream._disturbed = true;
        if (!ArrayBuffer.isView(view) || view.byteLength === 0) return Promise.reject(new TypeError('view must be a non-empty ArrayBuffer view'));
        const min = options.min === undefined ? 1 : Number(options.min);
        const capacity = view instanceof DataView ? view.byteLength : view.length;
        if (!Number.isInteger(min) || min <= 0 || min > capacity) return Promise.reject(new RangeError('invalid minimum fill count'));
        return stream._readInto(view, this, min);
      }
      cancel(reason) { if (this._stream) this._stream._disturbed = true; return this._stream ? this._stream._cancel(reason) : Promise.reject(webInvalidState('Reader is released')); }
      releaseLock() { if (!this._stream) return; const stream = this._stream, error = webInvalidState('Reader was released'); stream._byobReads = stream._byobReads.filter(read => { if (read.reader !== this) return true; read.reject(error); return false; }); if (stream._byobRequest && stream._byobRequest._read.reader === this) stream._byobRequest = null; stream._reader = null; this._stream = null; this.closed = Promise.reject(error); this.closed.catch(() => {}); }
    }
    globalThis.ReadableStream = class ReadableStream {
      static from(iterable) {
        if (iterable === null || iterable === undefined) { const error = new TypeError('value is not iterable'); error.code = 'ERR_ARG_NOT_ITERABLE'; throw error; }
        const method = iterable[Symbol.asyncIterator] || iterable[Symbol.iterator];
        if (typeof method !== 'function') { const error = new TypeError('value is not iterable'); error.code = 'ERR_ARG_NOT_ITERABLE'; throw error; }
        const iterator = method.call(iterable);
        if (!iterator || typeof iterator.next !== 'function') throw new TypeError('iterator must provide next()');
        let chain = Promise.resolve(), done = false;
        return new ReadableStream({
          pull(controller) {
            chain = chain.then(async () => {
              if (done) return;
              const result = await iterator.next();
              if (!result || typeof result !== 'object') throw new TypeError('iterator result must be an object');
              if (result.done) { done = true; controller.close(); }
              else controller.enqueue(await result.value);
            }).catch(error => { done = true; controller.error(error); });
            return chain;
          },
          async cancel(reason) {
            if (done) return;
            done = true;
            if (typeof iterator.return === 'function') {
              const result = await iterator.return(reason);
              if (!result || typeof result !== 'object') throw new TypeError('iterator return result must be an object');
            }
          }
        });
      }
      constructor(source = {}, strategy = {}) {
        if (source.type !== undefined && source.type !== 'bytes') { const error = new TypeError('invalid source.type'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        if (source.autoAllocateChunkSize !== undefined && (source.type !== 'bytes' || !Number.isInteger(Number(source.autoAllocateChunkSize)) || Number(source.autoAllocateChunkSize) <= 0)) throw new RangeError('invalid autoAllocateChunkSize');
        if (source.type === 'bytes' && strategy.size !== undefined) { const error = new RangeError('byte streams cannot use a size strategy'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        const defaultHighWaterMark = source.type === 'bytes' ? 0 : 1, highWaterMark = strategy.highWaterMark === undefined ? defaultHighWaterMark : Number(strategy.highWaterMark);
        if (!Number.isFinite(highWaterMark) || highWaterMark < 0) { const error = new RangeError('invalid highWaterMark'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        this._source = source; this._byteStream = source.type === 'bytes'; this._highWaterMark = highWaterMark; this._sizeAlgorithm = typeof strategy.size === 'function' ? strategy.size : () => 1; this._autoAllocateChunkSize = this._byteStream && source.autoAllocateChunkSize !== undefined ? Number(source.autoAllocateChunkSize) : 0; this._queue = []; this._queueSizes = []; this._queueTotalSize = 0; this._reads = []; this._byobReads = []; this._byobRequest = null; this._capacityWaiters = []; this._state = 'readable'; this._closeRequested = false; this._disturbed = false; this._error = undefined; this._reader = null; this._started = false; this._pulling = false; this._pullAgain = false;
        this._closed = new Promise((resolve, reject) => { this._resolveClosed = resolve; this._rejectClosed = reject; });
        this._closed.catch(() => {}); this._controller = this._byteStream ? new ReadableByteStreamController(this) : new ReadableStreamDefaultController(this);
        try { Promise.resolve(typeof source.start === 'function' ? source.start(this._controller) : undefined).then(() => { this._started = true; this._callPullIfNeeded(); }, error => this._controller.error(error)); } catch (error) { this._controller.error(error); }
      }
      get locked() { return this._reader !== null; }
      getReader(options = {}) { if (options.mode === undefined) return new ReadableStreamDefaultReader(this); if (options.mode === 'byob') return new ReadableStreamBYOBReader(this); const error = new TypeError('invalid reader mode'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
      _chunkSize(chunk) { const size = Number(this._sizeAlgorithm(chunk)); if (!Number.isFinite(size) || size < 0) { const error = new RangeError('invalid chunk size'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } return size; }
      _dequeue() { const value = this._queue.shift(), size = this._queueSizes.shift() || 0; this._queueTotalSize = Math.max(0, this._queueTotalSize - size); this._notifyCapacity(); return value; }
      _hasCapacity() { return this._state === 'readable' && !this._closeRequested && (this._reads.length > 0 || this._byobReads.length > 0 || this._highWaterMark - this._queueTotalSize > 0); }
      _waitForCapacity() { if (this._hasCapacity()) return Promise.resolve(); if (this._state === 'errored') return Promise.reject(this._error); if (this._state !== 'readable' || this._closeRequested) return Promise.reject(webInvalidState('ReadableStream is closed')); return new Promise((resolve, reject) => this._capacityWaiters.push({ resolve, reject })); }
      _notifyCapacity() { if (!this._hasCapacity() || !this._capacityWaiters.length) return; const waiters = this._capacityWaiters.splice(0); for (const waiter of waiters) waiter.resolve(); }
      _rejectCapacity(error) { const waiters = this._capacityWaiters.splice(0); for (const waiter of waiters) waiter.reject(error); }
      _finishCloseIfReady() { if (!this._closeRequested || this._queue.length || this._state !== 'readable') return; this._state = 'closed'; this._byobRequest = null; this._rejectCapacity(webInvalidState('ReadableStream is closed')); while (this._reads.length) this._reads.shift().resolve({ value: undefined, done: true }); while (this._byobReads.length) { const read = this._byobReads.shift(); read.resolve({ value: byobResultView(read.view, read.filled), done: read.filled === 0 }); } this._resolveClosed(); }
      _callPullIfNeeded() {
        if (!this._started || this._state !== 'readable' || this._closeRequested || typeof this._source.pull !== 'function') return;
        if (!this._reads.length && !this._byobReads.length && this._highWaterMark - this._queueTotalSize <= 0) return;
        if (this._pulling) { this._pullAgain = true; return; }
        this._pulling = true;
        Promise.resolve().then(() => this._source.pull(this._controller)).then(() => { this._pulling = false; if (this._pullAgain) { this._pullAgain = false; this._callPullIfNeeded(); } }, error => { this._pulling = false; this._controller.error(error); });
      }
      _drainByob() {
        while (this._byobReads.length && this._queue.length) {
          const read = this._byobReads[0], target = new Uint8Array(read.view.buffer, read.view.byteOffset, read.view.byteLength);
          while (read.filled < target.byteLength && this._queue.length) {
            const chunk = this._queue[0], length = Math.min(chunk.byteLength, target.byteLength - read.filled); target.set(chunk.subarray(0, length), read.filled); read.filled += length;
            this._queueTotalSize = Math.max(0, this._queueTotalSize - length);
            if (length === chunk.byteLength) { this._queue.shift(); this._queueSizes.shift(); } else { this._queue[0] = chunk.subarray(length); this._queueSizes[0] -= length; }
          }
          if (read.filled < read.minBytes && this._state === 'readable') break;
          this._byobReads.shift(); if (this._byobRequest && this._byobRequest._read === read) this._byobRequest = null; read.resolve({ value: byobResultView(read.view, read.filled), done: false });
        }
        this._finishCloseIfReady();
        if (this._state === 'closed' && !this._queue.length) {
          this._byobRequest = null; while (this._byobReads.length) { const read = this._byobReads.shift(); read.resolve({ value: byobResultView(read.view, 0), done: true }); }
        }
        this._notifyCapacity(); this._prepareByobRequest(); this._callPullIfNeeded();
      }
      _prepareByobRequest() {
        if (!this._byobReads.length || this._state !== 'readable' || this._byobRequest) return;
        const read = this._byobReads[0], remaining = new Uint8Array(read.view.buffer, read.view.byteOffset + read.filled, read.view.byteLength - read.filled);
        this._byobRequest = new ReadableStreamBYOBRequest(this, { ...read, view: remaining });
        this._byobRequest._read = read;
        this._callPullIfNeeded();
      }
      _readInto(view, reader, min) {
        if (this._state === 'errored') return Promise.reject(this._error);
        const elementSize = view instanceof DataView ? 1 : view.BYTES_PER_ELEMENT, result = new Promise((resolve, reject) => this._byobReads.push({ reader, view, elementSize, minBytes: min * elementSize, filled: 0, resolve, reject })); this._notifyCapacity(); this._drainByob(); this._prepareByobRequest();
        return result;
      }
      _cancel(reason) { if (this._state === 'closed') return Promise.resolve(); if (this._state === 'errored') return Promise.reject(this._error); this._queue.length = 0; this._queueSizes.length = 0; this._queueTotalSize = 0; this._closeRequested = true; this._rejectCapacity(reason); const finish = () => this._finishCloseIfReady(); if (typeof this._source.cancel === 'function') return Promise.resolve(this._source.cancel(reason)).then(finish); finish(); return Promise.resolve(); }
      cancel(reason) { if (this.locked) return Promise.reject(webInvalidState('ReadableStream is locked')); this._disturbed = true; return this._cancel(reason); }
      tee() {
        if (this.locked) throw webInvalidState('ReadableStream is locked');
        const byteStream = this._byteStream, reader = this.getReader(), controllers = [null, null], cancelled = [false, false], reasons = [undefined, undefined], demand = [false, false];
        let reading = false, finished = false, resolveCancellation, rejectCancellation;
        const cancellation = new Promise((resolve, reject) => { resolveCancellation = resolve; rejectCancellation = reject; });
        const finish = result => {
          if (finished) return;
          finished = true;
          for (let index = 0; index < 2; index++) {
            if (cancelled[index]) continue;
            if (result && result.error) controllers[index].error(result.error);
            else controllers[index].close();
          }
          resolveCancellation();
        };
        const pump = () => {
          if (reading || finished || !demand.some((wanted, index) => wanted && !cancelled[index])) return;
          reading = true;
          reader.read().then(result => {
            reading = false;
            if (result.done) { finish(); return; }
            for (let index = 0; index < 2; index++) {
              if (cancelled[index]) continue;
              demand[index] = false;
              controllers[index].enqueue(byteStream ? new Uint8Array(result.value) : result.value);
            }
            pump();
          }, error => { reading = false; finish({ error }); });
        };
        const pull = index => { demand[index] = true; pump(); };
        const cancel = (index, reason) => {
          if (cancelled[index]) return cancellation;
          cancelled[index] = true; reasons[index] = reason; demand[index] = false;
          if (cancelled[0] && cancelled[1] && !finished) {
            finished = true;
            Promise.resolve(reader.cancel(reasons)).then(resolveCancellation, rejectCancellation);
          } else {
            pump();
          }
          return cancellation;
        };
        const branches = [0, 1].map(index => new ReadableStream({
          type: byteStream ? 'bytes' : undefined,
          start(controller) { controllers[index] = controller; },
          pull() { return pull(index); },
          cancel(reason) { return cancel(index, reason); }
        }));
        return branches;
      }
      async pipeTo(destination, options = {}) {
        const signal = options.signal;
        if (signal !== undefined && (!signal || typeof signal.aborted !== 'boolean' || typeof signal.addEventListener !== 'function')) throw new TypeError('signal must be an AbortSignal');
        const reader = this.getReader(); let writer;
        try { writer = destination.getWriter(); } catch (error) { reader.releaseLock(); throw error; }
        let abort, abortPromise;
        if (signal !== undefined) {
          abortPromise = new Promise((resolve, reject) => { abort = () => reject(signal.reason); if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true }); });
        }
        const wait = operation => abortPromise ? Promise.race([operation, abortPromise]) : operation;
        const isAbort = error => signal !== undefined && signal.aborted && error === signal.reason;
        const abortBoth = async error => {
          const actions = [];
          if (!options.preventCancel) actions.push(reader.cancel(error));
          if (!options.preventAbort) actions.push(writer.abort(error));
          await Promise.all(actions);
        };
        try {
          while (true) {
            let result;
            try { result = await wait(reader.read()); }
            catch (error) { if (isAbort(error)) await abortBoth(error); else if (!options.preventAbort) await writer.abort(error); throw error; }
            if (result.done) break;
            try { await wait(writer.write(result.value)); }
            catch (error) { if (isAbort(error)) await abortBoth(error); else if (!options.preventCancel) await reader.cancel(error); throw error; }
          }
          if (!options.preventClose) {
            try { await wait(writer.close()); }
            catch (error) { if (isAbort(error)) await abortBoth(error); else if (!options.preventCancel) await reader.cancel(error); throw error; }
          }
        } finally {
          if (signal !== undefined && abort) signal.removeEventListener('abort', abort);
          reader.releaseLock(); writer.releaseLock();
        }
      }
      pipeThrough(transform, options) { if (this.locked) throw webInvalidState('ReadableStream is locked'); if (!transform || !transform.readable || !transform.writable) throw new TypeError('transform must contain readable and writable streams'); if (transform.writable.locked) throw webInvalidState('WritableStream is locked'); this.pipeTo(transform.writable, options).catch(error => transform.readable._controller.error(error)); return transform.readable; }
      values(options = {}) { const reader = this.getReader(); return { next: () => reader.read(), return: async () => { if (!options.preventCancel) await reader.cancel(); reader.releaseLock(); return { value: undefined, done: true }; }, [Symbol.asyncIterator]() { return this; } }; }
      [Symbol.asyncIterator]() { return this.values(); }
    };
    globalThis.ReadableStreamDefaultController = ReadableStreamDefaultController;
    globalThis.ReadableStreamDefaultReader = ReadableStreamDefaultReader;
    globalThis.ReadableByteStreamController = ReadableByteStreamController;
    globalThis.ReadableStreamBYOBReader = ReadableStreamBYOBReader;
    globalThis.ReadableStreamBYOBRequest = ReadableStreamBYOBRequest;
    class WritableStreamDefaultController {
      constructor(stream) { this._stream = stream; this._abortController = new AbortController(); this.signal = this._abortController.signal; }
      error(error) { if (this._stream) this._stream._errorStream(error); }
    }
    class WritableStreamDefaultWriter {
      constructor(stream) { if (stream.locked) throw webInvalidState('WritableStream is locked'); this._stream = stream; this._releaseError = null; stream._writer = this; }
      get ready() { if (this._stream) return this._stream._ready; const promise = Promise.reject(this._releaseError); promise.catch(() => {}); return promise; }
      get closed() { if (this._stream) return this._stream._closed; const promise = Promise.reject(this._releaseError); promise.catch(() => {}); return promise; }
      write(chunk) { return this._stream ? this._stream._write(chunk) : Promise.reject(webInvalidState('Writer is released')); }
      close() { return this._stream ? this._stream._close() : Promise.reject(webInvalidState('Writer is released')); }
      abort(reason) { return this._stream ? this._stream._abort(reason) : Promise.reject(webInvalidState('Writer is released')); }
      releaseLock() { if (!this._stream) return; this._releaseError = webInvalidState('Writer was released'); this._stream._writer = null; this._stream = null; }
      get desiredSize() { return this._stream ? this._stream._desiredSize() : null; }
    }
    globalThis.WritableStream = class WritableStream {
      constructor(sink = {}, strategy = {}) { const highWaterMark = strategy.highWaterMark === undefined ? 1 : Number(strategy.highWaterMark); if (!Number.isFinite(highWaterMark) || highWaterMark < 0) { const error = new RangeError('invalid highWaterMark'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } this._sink = sink; this._highWaterMark = highWaterMark; this._sizeAlgorithm = typeof strategy.size === 'function' ? strategy.size : () => 1; this._queueTotalSize = 0; this._backpressured = false; this._ready = Promise.resolve(); this._resolveReady = null; this._rejectReady = null; this._state = 'writable'; this._error = undefined; this._writer = null; this._chain = Promise.resolve(); this._closed = new Promise((resolve, reject) => { this._resolveClosed = resolve; this._rejectClosed = reject; }); this._closed.catch(() => {}); this._controller = new WritableStreamDefaultController(this); this._setBackpressure(this._desiredSize() <= 0); if (typeof sink.start === 'function') { try { this._chain = Promise.resolve(sink.start(this._controller)).catch(error => { this._errorStream(error); throw error; }); } catch (error) { this._errorStream(error); this._chain = Promise.reject(error); } } }
      get locked() { return this._writer !== null; }
      getWriter() { return new WritableStreamDefaultWriter(this); }
      _desiredSize() { if (this._state === 'errored') return null; if (this._state === 'closed') return 0; return this._highWaterMark - this._queueTotalSize; }
      _setBackpressure(value) { if (this._backpressured === value) return; this._backpressured = value; if (value) { this._ready = new Promise((resolve, reject) => { this._resolveReady = resolve; this._rejectReady = reject; }); this._ready.catch(() => {}); } else { if (this._resolveReady) this._resolveReady(); this._resolveReady = null; this._rejectReady = null; this._ready = Promise.resolve(); } }
      _errorReady(error) { if (this._rejectReady) this._rejectReady(error); this._resolveReady = null; this._rejectReady = null; this._backpressured = false; this._ready = Promise.reject(error); this._ready.catch(() => {}); }
      _errorStream(error) { if (this._state !== 'writable') return; this._state = 'errored'; this._error = error; this._errorReady(error); this._rejectClosed(error); }
      _write(chunk) { if (this._state === 'errored') return Promise.reject(this._error); if (this._state !== 'writable') return Promise.reject(webInvalidState('WritableStream is closed')); let size; try { size = Number(this._sizeAlgorithm(chunk)); if (!Number.isFinite(size) || size < 0) { const error = new RangeError('invalid chunk size'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } } catch (error) { this._errorStream(error); return Promise.reject(error); } this._queueTotalSize += size; this._setBackpressure(this._desiredSize() <= 0); const operation = this._chain = this._chain.then(() => typeof this._sink.write === 'function' ? this._sink.write(chunk, this._controller) : undefined); return operation.then(value => { this._queueTotalSize = Math.max(0, this._queueTotalSize - size); if (this._state === 'writable') this._setBackpressure(this._desiredSize() <= 0); return Promise.resolve().then(() => value); }, error => { this._queueTotalSize = Math.max(0, this._queueTotalSize - size); this._errorStream(error); return Promise.resolve().then(() => { throw error; }); }); }
      _close() { if (this._state === 'errored') return Promise.reject(this._error); if (this._state !== 'writable') return Promise.reject(webInvalidState('WritableStream is closed')); this._state = 'closed'; this._setBackpressure(false); this._chain = this._chain.then(() => typeof this._sink.close === 'function' ? this._sink.close() : undefined).then(() => this._resolveClosed(), error => { this._error = error; this._state = 'errored'; this._errorReady(error); this._rejectClosed(error); throw error; }); return this._chain; }
      close() { if (this.locked) { const error = new TypeError('WritableStream is locked'); error.code = 'ERR_INVALID_STATE'; return Promise.reject(error); } return this._close(); }
      _abort(reason) { if (this._state !== 'writable') return Promise.resolve(); this._state = 'errored'; this._error = reason; this._errorReady(reason); this._controller._abortController.abort(reason); const operation = this._chain = this._chain.then(() => typeof this._sink.abort === 'function' ? this._sink.abort(reason) : undefined).then(() => this._rejectClosed(reason)); return operation.then(() => Promise.resolve()); }
      abort(reason) { if (this.locked) { const error = new TypeError('WritableStream is locked'); error.code = 'ERR_INVALID_STATE'; return Promise.reject(error); } return this._abort(reason); }
    };
    globalThis.WritableStreamDefaultWriter = WritableStreamDefaultWriter;
    globalThis.WritableStreamDefaultController = WritableStreamDefaultController;
    class TransformStreamDefaultController {
      constructor(readableController) { this._readableController = readableController; this._writable = null; }
      get desiredSize() { return this._readableController.desiredSize; }
      enqueue(chunk) { this._readableController.enqueue(chunk); }
      error(reason) { this._readableController.error(reason); if (this._writable) this._writable._errorStream(reason); }
      terminate() { const error = webInvalidState('TransformStream has been terminated'); this._readableController._stream._rejectCapacity(error); this._readableController.close(); if (this._writable) this._writable._errorStream(error); }
    }
    globalThis.TransformStream = class TransformStream {
      constructor(transformer = {}, writableStrategy = {}, readableStrategy = {}) { let readableController, writable; const readableQueueStrategy = { ...readableStrategy, highWaterMark: readableStrategy.highWaterMark === undefined ? 0 : readableStrategy.highWaterMark }, cancel = reason => typeof transformer.cancel === 'function' ? transformer.cancel(reason) : undefined; this.readable = new ReadableStream({ start(value) { readableController = value; }, cancel(error) { if (writable) writable._errorStream(error); return cancel(error); } }, readableQueueStrategy); const controller = new TransformStreamDefaultController(readableController), fail = error => { controller.error(error); throw error; }; writable = this.writable = new WritableStream({ start() { return typeof transformer.start === 'function' ? transformer.start(controller) : undefined; }, write(chunk) { return readableController._stream._waitForCapacity().then(() => typeof transformer.transform === 'function' ? transformer.transform(chunk, controller) : controller.enqueue(chunk)).catch(fail); }, close() { return Promise.resolve().then(() => typeof transformer.flush === 'function' ? transformer.flush(controller) : undefined).then(() => readableController.close()).catch(fail); }, abort(error) { return Promise.resolve(cancel(error)).then(() => readableController.error(error)); } }, writableStrategy); controller._writable = writable; }
    };
    globalThis.TransformStreamDefaultController = TransformStreamDefaultController;
    const hexFromBytes = value => Array.from(value, byte => byte.toString(16).padStart(2, '0')).join('');
    const bytesFromHex = value => new Uint8Array(String(value).match(/../g)?.map(pair => parseInt(pair, 16)) || []);
    const compressionTransform = (operation, format) => {
      const normalized = String(format);
      if (normalized !== 'gzip' && normalized !== 'deflate' && normalized !== 'deflate-raw' && normalized !== 'br') throw new TypeError('Unsupported compression format');
      const nativeFormat = normalized === 'deflate-raw' ? 'deflateRaw' : normalized === 'br' ? 'brotli' : normalized, handle = __thaw_zlib_stream_create(operation, nativeFormat);
      let active = true;
      const release = () => { if (active) { active = false; __thaw_zlib_stream_drop(handle); } };
      return new TransformStream({
        transform(chunk, controller) {
          try {
            if (!(chunk instanceof ArrayBuffer) && !ArrayBuffer.isView(chunk)) throw new TypeError('chunk must be an ArrayBuffer or view');
            const bytes = chunk instanceof ArrayBuffer ? new Uint8Array(chunk) : new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength);
            const output = bytesFromHex(__thaw_zlib_stream_write(handle, hexFromBytes(bytes), false));
            if (output.byteLength) controller.enqueue(output);
          } catch (error) { release(); throw error; }
        },
        flush(controller) {
          try { const output = bytesFromHex(__thaw_zlib_stream_write(handle, '', true)); active = false; if (output.byteLength) controller.enqueue(output); }
          catch (error) { release(); throw error; }
        },
        cancel: release
      });
    };
    globalThis.CompressionStream = class CompressionStream { constructor(format) { const stream = compressionTransform('compress', format); this.readable = stream.readable; this.writable = stream.writable; } };
    globalThis.DecompressionStream = class DecompressionStream { constructor(format) { const stream = compressionTransform('decompress', format); this.readable = stream.readable; this.writable = stream.writable; } };
    const queuingStrategyMarks = new WeakMap();
    const queuingStrategyInit = options => {
      if (options === null || (typeof options !== 'object' && typeof options !== 'function')) { const error = new TypeError('init must be an object'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; }
      if (options.highWaterMark === undefined) { const error = new TypeError('init.highWaterMark is required'); error.code = 'ERR_MISSING_OPTION'; throw error; }
      return Number(options.highWaterMark);
    };
    const strategyHighWaterMark = function() { if (!queuingStrategyMarks.has(this)) throw new TypeError('invalid queuing strategy receiver'); return queuingStrategyMarks.get(this); };
    class ByteLengthQueuingStrategy { constructor(options) { queuingStrategyMarks.set(this, queuingStrategyInit(options)); } }
    Object.defineProperties(ByteLengthQueuingStrategy.prototype, {
      highWaterMark: { get: strategyHighWaterMark, enumerable: true, configurable: true },
      size: { value(chunk) { return chunk.byteLength; }, writable: true, enumerable: true, configurable: true }
    });
    class CountQueuingStrategy { constructor(options) { queuingStrategyMarks.set(this, queuingStrategyInit(options)); } }
    Object.defineProperties(CountQueuingStrategy.prototype, {
      highWaterMark: { get: strategyHighWaterMark, enumerable: true, configurable: true },
      size: { value() { return 1; }, writable: true, enumerable: true, configurable: true }
    });
    globalThis.ByteLengthQueuingStrategy = ByteLengthQueuingStrategy;
    globalThis.CountQueuingStrategy = CountQueuingStrategy;
    globalThis.TextEncoderStream = class TextEncoderStream { constructor() { const encoder = new TextEncoder(); let pendingHigh = ''; const transform = new TransformStream({ transform(chunk, controller) { let text = pendingHigh + String(chunk); pendingHigh = ''; const last = text.charCodeAt(text.length - 1); if (last >= 0xd800 && last <= 0xdbff) { pendingHigh = text.slice(-1); text = text.slice(0, -1); } if (text) controller.enqueue(encoder.encode(text)); }, flush(controller) { if (pendingHigh) controller.enqueue(encoder.encode(pendingHigh)); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = 'utf-8'; } };
    globalThis.TextDecoderStream = class TextDecoderStream { constructor(label = 'utf-8', options = {}) { const decoder = new TextDecoder(label, options); const transform = new TransformStream({ transform(chunk, controller) { const text = decoder.decode(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength), { stream: true }); if (text) controller.enqueue(text); }, flush(controller) { const text = decoder.decode(); if (text) controller.enqueue(text); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = decoder.encoding || String(label).toLowerCase(); this.fatal = Boolean(options.fatal); this.ignoreBOM = Boolean(options.ignoreBOM); } };
  }
  if (typeof globalThis.FormData !== 'function') {
    const formDataEntries = new WeakMap();
    class FormData {
      constructor() { formDataEntries.set(this, []); }
      append(name, value, filename = undefined) { const entry = [String(name), value instanceof Blob ? value : String(value)]; if (filename !== undefined) entry.push(String(filename)); formDataEntries.get(this).push(entry); }
      delete(name) { const key = String(name); formDataEntries.set(this, formDataEntries.get(this).filter(entry => entry[0] !== key)); }
      get(name) { const key = String(name), entry = formDataEntries.get(this).find(item => item[0] === key); return entry ? entry[1] : null; }
      getAll(name) { const key = String(name); return formDataEntries.get(this).filter(entry => entry[0] === key).map(entry => entry[1]); }
      has(name) { const key = String(name); return formDataEntries.get(this).some(entry => entry[0] === key); }
      set(name, value, filename = undefined) { const key = String(name), entries = formDataEntries.get(this), index = entries.findIndex(entry => entry[0] === key), replacement = [key, value instanceof Blob ? value : String(value)]; if (filename !== undefined) replacement.push(String(filename)); if (index < 0) entries.push(replacement); else { entries[index] = replacement; formDataEntries.set(this, entries.filter((entry, current) => entry[0] !== key || current === index)); } }
      *entries() { for (const entry of formDataEntries.get(this)) yield [entry[0], entry[1]]; }
      *keys() { for (const entry of formDataEntries.get(this)) yield entry[0]; }
      *values() { for (const entry of formDataEntries.get(this)) yield entry[1]; }
      forEach(callback, thisArg = undefined) { for (const [name, value] of this.entries()) callback.call(thisArg, value, name, this); }
      [Symbol.iterator]() { return this.entries(); }
      get [Symbol.toStringTag]() { return 'FormData'; }
    }
    globalThis.FormData = FormData;
    Object.defineProperty(globalThis, '__thaw_form_data_entries', {
      configurable: true,
      value(value) { const entries = formDataEntries.get(value); if (!entries) throw new TypeError('invalid FormData receiver'); return entries.slice(); }
    });
  }
  if (typeof globalThis.Response !== 'function') {
    const responseData = new WeakMap();
    const bytesBodyStream = bytes => new ReadableStream({ start(controller) { if (bytes.byteLength) controller.enqueue(bytes); controller.close(); } });
    let multipartSequence = 0;
    const multipartEscape = value => String(value).replace(/\r/g, '%0D').replace(/\n/g, '%0A').replace(/"/g, '%22');
    const multipartBody = value => {
      const boundary = `----thaw-formdata-${++multipartSequence}`;
      const stream = new ReadableStream({
        async start(controller) {
          try {
            for (const entry of globalThis.__thaw_form_data_entries(value)) {
              const name = multipartEscape(entry[0]), field = entry[1];
              if (field instanceof Blob) {
                const filename = multipartEscape(entry[2] === undefined ? field.name === undefined ? 'blob' : field.name : entry[2]);
                controller.enqueue(Uint8Array.from(encodeUtf8(`--${boundary}\r\nContent-Disposition: form-data; name="${name}"; filename="${filename}"\r\nContent-Type: ${field.type || 'application/octet-stream'}\r\n\r\n`)));
                const bytes = await field.bytes(); if (bytes.byteLength) controller.enqueue(bytes);
                controller.enqueue(Uint8Array.from(encodeUtf8('\r\n')));
              } else controller.enqueue(Uint8Array.from(encodeUtf8(`--${boundary}\r\nContent-Disposition: form-data; name="${name}"\r\n\r\n${field}\r\n`)));
            }
            controller.enqueue(Uint8Array.from(encodeUtf8(`--${boundary}--\r\n`))); controller.close();
          } catch (error) { controller.error(error); }
        }
      });
      return { stream, type: `multipart/form-data; boundary=${boundary}` };
    };
    const responseBody = value => {
      if (value === null || value === undefined) return { stream: null, type: null };
      if (value instanceof ReadableStream) { if (value.locked || value._disturbed) throw new TypeError('Body is unusable'); return { stream: value, type: null }; }
      if (value instanceof Blob) return { stream: new ReadableStream({ async start(controller) { const bytes = await value.bytes(); if (bytes.byteLength) controller.enqueue(bytes); controller.close(); } }), type: value.type || null };
      if (value instanceof FormData) return multipartBody(value);
      if (value instanceof URLSearchParams) { const bytes = Uint8Array.from(encodeUtf8(value.toString())); return { stream: bytesBodyStream(bytes), type: 'application/x-www-form-urlencoded;charset=UTF-8' }; }
      if (value instanceof ArrayBuffer) return { stream: bytesBodyStream(new Uint8Array(value.slice(0))), type: null };
      if (ArrayBuffer.isView(value)) return { stream: bytesBodyStream(new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength))), type: null };
      return { stream: bytesBodyStream(Uint8Array.from(encodeUtf8(String(value)))), type: 'text/plain;charset=UTF-8' };
    };
    const consumeResponseBody = async response => {
      const record = responseData.get(response); if (!record) throw new TypeError('invalid Response receiver');
      if (!record.body) return new Uint8Array();
      if (record.body._disturbed || record.body.locked) throw new TypeError('Body is unusable');
      const reader = record.body.getReader(), chunks = []; let length = 0;
      while (true) { const result = await reader.read(); if (result.done) break; const chunk = result.value instanceof ArrayBuffer ? new Uint8Array(result.value) : ArrayBuffer.isView(result.value) ? new Uint8Array(result.value.buffer, result.value.byteOffset, result.value.byteLength) : Uint8Array.from(encodeUtf8(String(result.value))); chunks.push(chunk); length += chunk.byteLength; }
      const output = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.byteLength; } return output;
    };
    const parseFormDataBody = (type, bytes) => {
      const data = new FormData(), lower = type.toLowerCase();
      if (lower.startsWith('application/x-www-form-urlencoded')) { const params = new URLSearchParams(new TextDecoder().decode(bytes)); for (const [name, value] of params) data.append(name, value); return data; }
      if (!lower.startsWith('multipart/form-data')) throw new TypeError('unsupported form data content type');
      const boundaryMatch = /(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(type);
      if (!boundaryMatch) throw new TypeError('multipart boundary is missing');
      const boundary = boundaryMatch[1] || boundaryMatch[2]; let binary = '';
      for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
      const delimiter = `--${boundary}`, sections = binary.split(delimiter);
      for (let section of sections.slice(1)) {
        if (section.startsWith('--')) break;
        if (section.startsWith('\r\n')) section = section.slice(2);
        if (section.endsWith('\r\n')) section = section.slice(0, -2);
        const marker = section.indexOf('\r\n\r\n'); if (marker < 0) continue;
        const rawHeaders = section.slice(0, marker), content = section.slice(marker + 4), headers = Object.create(null);
        for (const line of rawHeaders.split('\r\n')) { const colon = line.indexOf(':'); if (colon >= 0) headers[line.slice(0, colon).toLowerCase()] = line.slice(colon + 1).trim(); }
        const disposition = headers['content-disposition'] || '', nameMatch = /(?:^|;)\s*name="([^"]*)"/i.exec(disposition); if (!nameMatch) continue;
        const decodeParameter = value => value.replace(/%22/gi, '"').replace(/%0D/gi, '\r').replace(/%0A/gi, '\n');
        const name = decodeParameter(nameMatch[1]), filenameMatch = /(?:^|;)\s*filename="([^"]*)"/i.exec(disposition), partBytes = Uint8Array.from(content, character => character.charCodeAt(0));
        if (filenameMatch) data.append(name, new File([partBytes], decodeParameter(filenameMatch[1]), { type: headers['content-type'] || 'application/octet-stream' }));
        else data.append(name, new TextDecoder().decode(partBytes));
      }
      return data;
    };
    class Response {
      constructor(body = null, init = {}) {
        const status = init.status === undefined ? 200 : Number(init.status);
        if (!Number.isInteger(status) || status < 200 || status > 599) throw new RangeError('status must be between 200 and 599');
        if (body !== null && body !== undefined && (status === 204 || status === 205 || status === 304)) throw new TypeError(`Invalid response status code ${status}`);
        const statusText = init.statusText === undefined ? '' : String(init.statusText);
        if (/[\0\r\n]/.test(statusText)) throw new TypeError('invalid statusText');
        const headers = new Headers(init.headers), normalized = responseBody(body);
        if (normalized.type && !headers.has('content-type')) headers.set('content-type', normalized.type);
        responseData.set(this, { body: normalized.stream, headers, status, statusText, type: 'default', url: '', redirected: false });
      }
      get body() { return responseData.get(this).body; }
      get bodyUsed() { const body = responseData.get(this).body; return Boolean(body && body._disturbed); }
      get headers() { return responseData.get(this).headers; }
      get ok() { const status = responseData.get(this).status; return status >= 200 && status <= 299; }
      get redirected() { return responseData.get(this).redirected; }
      get status() { return responseData.get(this).status; }
      get statusText() { return responseData.get(this).statusText; }
      get type() { return responseData.get(this).type; }
      get url() { return responseData.get(this).url; }
      async arrayBuffer() { const bytes = await consumeResponseBody(this); return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); }
      async blob() { const record = responseData.get(this), bytes = await consumeResponseBody(this); return new Blob([bytes], { type: record.headers.get('content-type') || '' }); }
      async bytes() { return consumeResponseBody(this); }
      async json() { return JSON.parse(await this.text()); }
      async text() { return new TextDecoder().decode(await consumeResponseBody(this)); }
      async formData() { const type = this.headers.get('content-type') || ''; return parseFormDataBody(type, await consumeResponseBody(this)); }
      clone() { const record = responseData.get(this); if (record.body && (record.body._disturbed || record.body.locked)) throw new TypeError('Body has already been consumed'); let body = null; if (record.body) { const branches = record.body.tee(); record.body = branches[0]; body = branches[1]; } const clone = new Response(body, { status: record.status, statusText: record.statusText, headers: record.headers }); const cloneRecord = responseData.get(clone); cloneRecord.type = record.type; cloneRecord.url = record.url; cloneRecord.redirected = record.redirected; return clone; }
      static error() { const response = new Response(); const record = responseData.get(response); record.status = 0; record.type = 'error'; return response; }
      static json(value, init = {}) { const body = JSON.stringify(value); if (body === undefined) throw new TypeError('value is not JSON serializable'); const headers = new Headers(init.headers); if (!headers.has('content-type')) headers.set('content-type', 'application/json'); return new Response(body, { ...init, headers }); }
      static redirect(url, status = 302) { const code = Number(status); if (![301, 302, 303, 307, 308].includes(code)) throw new RangeError('invalid redirect status'); return new Response(null, { status: code, headers: { location: new URL(String(url)).href } }); }
      get [Symbol.toStringTag]() { return 'Response'; }
    }
    Object.defineProperty(globalThis, '__thaw_set_response_metadata', {
      configurable: true,
      value(response, url, redirected) {
        const record = responseData.get(response); if (!record) throw new TypeError('invalid Response receiver');
        record.url = String(url); record.redirected = Boolean(redirected); return response;
      }
    });
    const requestData = new WeakMap();
    const consumeRequestBody = async request => {
      const record = requestData.get(request); if (!record) throw new TypeError('invalid Request receiver');
      if (!record.body) return new Uint8Array();
      if (record.body._disturbed || record.body.locked) throw new TypeError('Body is unusable');
      const reader = record.body.getReader(), chunks = []; let length = 0;
      while (true) { const result = await reader.read(); if (result.done) break; const chunk = result.value instanceof ArrayBuffer ? new Uint8Array(result.value) : ArrayBuffer.isView(result.value) ? new Uint8Array(result.value.buffer, result.value.byteOffset, result.value.byteLength) : Uint8Array.from(encodeUtf8(String(result.value))); chunks.push(chunk); length += chunk.byteLength; }
      const output = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.byteLength; } return output;
    };
    class Request {
      constructor(input, init = {}) {
        const inherited = requestData.get(input), url = new URL(inherited ? inherited.url : String(input));
        if (url.username || url.password) throw new TypeError('Request URL cannot contain credentials');
        let method = init.method === undefined ? inherited ? inherited.method : 'GET' : String(init.method);
        if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(method)) throw new TypeError('invalid HTTP method');
        const upperMethod = method.toUpperCase(); if (['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT'].includes(upperMethod)) method = upperMethod;
        if (upperMethod === 'CONNECT' || upperMethod === 'TRACE' || upperMethod === 'TRACK') throw new TypeError(`unsupported HTTP method ${upperMethod}`);
        const headers = new Headers(init.headers === undefined ? inherited ? inherited.headers : undefined : init.headers), hasBody = Object.prototype.hasOwnProperty.call(init, 'body');
        let normalized = { stream: null, type: null };
        if (hasBody) normalized = responseBody(init.body);
        else if (inherited && inherited.body) {
          if (inherited.body._disturbed || inherited.body.locked) throw new TypeError('Body is unusable');
          const reader = inherited.body.getReader(); inherited.body._disturbed = true;
          normalized.stream = new ReadableStream({ async pull(controller) { const result = await reader.read(); if (result.done) controller.close(); else controller.enqueue(result.value); }, cancel(reason) { return reader.cancel(reason); } });
        }
        if ((method === 'GET' || method === 'HEAD') && normalized.stream) throw new TypeError('Request with GET/HEAD method cannot have body');
        if (hasBody && init.body instanceof ReadableStream && init.duplex !== 'half') throw new TypeError('duplex option is required for a streaming body');
        if (normalized.type && !headers.has('content-type')) headers.set('content-type', normalized.type);
        const sourceSignal = init.signal === undefined ? inherited ? inherited.signal : null : init.signal, signalController = new AbortController();
        if (sourceSignal) { if (typeof sourceSignal.aborted !== 'boolean' || typeof sourceSignal.addEventListener !== 'function') throw new TypeError('signal must be an AbortSignal'); if (sourceSignal.aborted) signalController.abort(sourceSignal.reason); else sourceSignal.addEventListener('abort', () => signalController.abort(sourceSignal.reason), { once: true }); }
        const select = (value, fallback, accepted, name) => { const selected = value === undefined ? fallback : String(value); if (!accepted.includes(selected)) throw new TypeError(`invalid Request ${name}`); return selected; };
        const mode = select(init.mode, inherited ? inherited.mode : 'cors', ['same-origin', 'no-cors', 'cors'], 'mode');
        const cache = select(init.cache, inherited ? inherited.cache : 'default', ['default', 'no-store', 'reload', 'no-cache', 'force-cache', 'only-if-cached'], 'cache');
        if (cache === 'only-if-cached' && mode !== 'same-origin') throw new TypeError('only-if-cached requires same-origin mode');
        const credentials = select(init.credentials, inherited ? inherited.credentials : 'same-origin', ['omit', 'same-origin', 'include'], 'credentials');
        const redirect = select(init.redirect, inherited ? inherited.redirect : 'follow', ['follow', 'manual', 'error'], 'redirect');
        const referrerPolicy = select(init.referrerPolicy, inherited ? inherited.referrerPolicy : '', ['', 'no-referrer', 'no-referrer-when-downgrade', 'same-origin', 'origin', 'strict-origin', 'origin-when-cross-origin', 'strict-origin-when-cross-origin', 'unsafe-url'], 'referrerPolicy');
        let referrer = init.referrer === undefined ? inherited ? inherited.referrer : 'about:client' : String(init.referrer);
        if (referrer && referrer !== 'about:client') referrer = new URL(referrer).href;
        requestData.set(this, {
          body: normalized.stream, headers, method, url: url.href, signal: signalController.signal,
          cache, credentials,
          destination: inherited ? inherited.destination : '',
          integrity: init.integrity === undefined ? inherited ? inherited.integrity : '' : String(init.integrity),
          keepalive: init.keepalive === undefined ? inherited ? inherited.keepalive : false : Boolean(init.keepalive),
          mode, redirect, referrer, referrerPolicy,
          duplex: 'half'
        });
      }
      get body() { return requestData.get(this).body; }
      get bodyUsed() { const body = requestData.get(this).body; return Boolean(body && body._disturbed); }
      get cache() { return requestData.get(this).cache; }
      get credentials() { return requestData.get(this).credentials; }
      get destination() { return requestData.get(this).destination; }
      get duplex() { return requestData.get(this).duplex; }
      get headers() { return requestData.get(this).headers; }
      get integrity() { return requestData.get(this).integrity; }
      get keepalive() { return requestData.get(this).keepalive; }
      get method() { return requestData.get(this).method; }
      get mode() { return requestData.get(this).mode; }
      get redirect() { return requestData.get(this).redirect; }
      get referrer() { return requestData.get(this).referrer; }
      get referrerPolicy() { return requestData.get(this).referrerPolicy; }
      get signal() { return requestData.get(this).signal; }
      get url() { return requestData.get(this).url; }
      async arrayBuffer() { const bytes = await consumeRequestBody(this); return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); }
      async blob() { const record = requestData.get(this), bytes = await consumeRequestBody(this); return new Blob([bytes], { type: record.headers.get('content-type') || '' }); }
      async bytes() { return consumeRequestBody(this); }
      async json() { return JSON.parse(await this.text()); }
      async text() { return new TextDecoder().decode(await consumeRequestBody(this)); }
      async formData() { const type = this.headers.get('content-type') || ''; return parseFormDataBody(type, await consumeRequestBody(this)); }
      clone() { const record = requestData.get(this); if (record.body && (record.body._disturbed || record.body.locked)) throw new TypeError('Body has already been consumed'); let body = null; if (record.body) { const branches = record.body.tee(); record.body = branches[0]; body = branches[1]; } return new Request(record.url, { method: record.method, headers: record.headers, body, signal: record.signal, cache: record.cache, credentials: record.credentials, integrity: record.integrity, keepalive: record.keepalive, mode: record.mode, redirect: record.redirect, referrer: record.referrer, referrerPolicy: record.referrerPolicy, duplex: 'half' }); }
      get [Symbol.toStringTag]() { return 'Request'; }
    }
    globalThis.Response = Response;
    globalThis.Request = Request;
  }
