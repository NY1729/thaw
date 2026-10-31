const encodeUtf8 = s => new TextEncoder().encode(s);
  const thawImmutableResponseHeaders = new WeakSet();
  if (typeof globalThis.Headers !== 'function') {
    const headerLists = new WeakMap();
    const headerIteratorStates = new WeakMap();
    const headerApply = Reflect.apply;
    const headerName = value => {
      const name = String(value).toLowerCase();
      if (!/^[!#$%&'*+.^_`|~0-9a-z-]+$/.test(name)) throw new TypeError(`invalid header name: ${value}`);
      return name;
    };
    const headerValue = value => {
      const text = String(value);
      if (/[\0\r\n]/.test(text) || /[^\u0000-\u00ff]/.test(text)) throw new TypeError(`invalid header value: ${text}`);
      return text.replace(/^[\t ]+|[\t ]+$/g, '');
    };
    const headersList = value => { const list = headerLists.get(value); if (!list) throw new TypeError('invalid Headers receiver'); return list; };
    const writableHeaders = value => { if (thawImmutableResponseHeaders.has(value)) throw new TypeError('Headers are immutable'); };
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
    const headerIteratorPrototype = {
      next() {
        const state = headerIteratorStates.get(this);
        if (!state) throw new TypeError('invalid Headers iterator receiver');
        const pairs = normalizedHeaderEntries(state.target);
        if (state.index >= pairs.length) return { value: undefined, done: true };
        const [name, value] = pairs[state.index++];
        return { value: state.kind === 'key' ? name : state.kind === 'value' ? value : [name, value], done: false };
      },
      [Symbol.iterator]() { return this; },
      get [Symbol.toStringTag]() { return 'Headers Iterator'; }
    };
    const headerIterator = (target, kind) => {
      headersList(target);
      const iterator = Object.create(headerIteratorPrototype);
      headerIteratorStates.set(iterator, { target, kind, index: 0 });
      return iterator;
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
      append(name, value) { const list = headersList(this), normalized = headerName(name), text = headerValue(value); writableHeaders(this); list.push([normalized, text]); }
      delete(name) { const normalized = headerName(name), list = headersList(this); writableHeaders(this); headerLists.set(this, list.filter(entry => entry[0] !== normalized)); }
      get(name) { const normalized = headerName(name), values = headersList(this).filter(entry => entry[0] === normalized).map(entry => entry[1]); return values.length ? values.join(', ') : null; }
      has(name) { const normalized = headerName(name); return headersList(this).some(entry => entry[0] === normalized); }
      set(name, value) { const normalized = headerName(name), text = headerValue(value), list = headersList(this).filter(entry => entry[0] !== normalized); writableHeaders(this); list.push([normalized, text]); headerLists.set(this, list); }
      getSetCookie() { return headersList(this).filter(entry => entry[0] === 'set-cookie').map(entry => entry[1]); }
      keys() { return headerIterator(this, 'key'); }
      values() { return headerIterator(this, 'value'); }
      entries() { return headerIterator(this, 'entry'); }
      forEach(callback, thisArg = undefined) {
        headersList(this);
        if (typeof callback !== 'function') throw new TypeError('callback must be a function');
        let index = 0, pairs = normalizedHeaderEntries(this);
        while (index < pairs.length) {
          const [name, value] = pairs[index++];
          headerApply(callback, thisArg, [value, name, this]);
          pairs = normalizedHeaderEntries(this);
        }
      }
      [Symbol.iterator]() { return headerIterator(this, 'entry'); }
      get [Symbol.toStringTag]() { return 'Headers'; }
    }
    for (const name of ['append', 'delete', 'get', 'has', 'set', 'getSetCookie', 'keys', 'values', 'entries', 'forEach']) {
      const descriptor = Object.getOwnPropertyDescriptor(Headers.prototype, name); descriptor.enumerable = true; Object.defineProperty(Headers.prototype, name, descriptor);
    }
    globalThis.Headers = Headers;
  }
  if (typeof globalThis.ReadableStream !== 'function') {
    const webInvalidState = message => { const error = new TypeError(message); error.code = 'ERR_INVALID_STATE'; return error; };
    const webSizeAlgorithm = size => { if (size === undefined) return () => 1; if (typeof size !== 'function') throw new TypeError('size must be a function'); return chunk => webApply(size, undefined, [chunk]); };
    const webApply = Reflect.apply;
    const webDeferred = () => {
      let resolve, reject;
      const promise = new Promise((accept, fail) => { resolve = accept; reject = fail; });
      promise.catch(() => {});
      return {
        promise, pending: true,
        resolve() { if (this.pending) { this.pending = false; resolve(); } },
        reject(error) { if (this.pending) { this.pending = false; reject(error); } }
      };
    };
    const webResolved = () => { const record = webDeferred(); record.resolve(); return record; };
    const webRejected = error => { const record = webDeferred(); record.reject(error); return record; };
    const webEnsureRejected = (record, error) => { if (record.pending) { record.reject(error); return record; } return webRejected(error); };
    const webCallback = (dictionary, name) => {
      const callback = dictionary[name];
      if (callback !== undefined && typeof callback !== 'function') throw new TypeError(`${name} must be a function`);
      return callback;
    };
    const webStrategy = strategy => {
      const value = strategy.highWaterMark;
      const highWaterMark = value === undefined ? undefined : Number(value);
      return { highWaterMark, size: webCallback(strategy, 'size') };
    };
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
        stream._state = 'errored'; stream._error = error; stream._queue.length = 0; stream._queueSizes.length = 0; stream._queueTotalSize = 0; stream._rejectCapacity(error); while (stream._reads.length) stream._reads.shift().reject(error); if (stream._reader) stream._reader._closedRecord.reject(error);
      }
      get desiredSize() { return this._stream._state === 'readable' ? this._stream._highWaterMark - this._stream._queueTotalSize : null; }
    }
    // WHATWG Streams, "ReadableByteStreamController" algorithms. The controller owns the byte queue
    // ({ buffer, byteOffset, byteLength } entries) and the pull-into descriptors; the stream only keeps the
    // state, the reader and the read / read-into request lists (`_reads`, `_byobReads`).
    const webIllegalConstructor = () => new TypeError('Illegal constructor');
    const webRangeError = message => { const error = new RangeError(message); error.code = 'ERR_INVALID_ARG_VALUE'; return error; };
    const webTypeError = message => { const error = new TypeError(message); error.code = 'ERR_INVALID_ARG_VALUE'; return error; };
    const webDetached = buffer => buffer.detached === true;
    const webTransfer = buffer => { if (webDetached(buffer)) throw webTypeError('The ArrayBuffer is detached'); return buffer.transfer(); };
    const webEnforceRange = (value, name) => {
      const number = Number(value);
      if (!Number.isFinite(number)) throw webTypeError(`${name} must be a finite number`);
      const integer = Math.trunc(number);
      if (integer < 0 || integer > Number.MAX_SAFE_INTEGER) throw webTypeError(`${name} is out of range`);
      return integer;
    };
    const webTypedArrayTag = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get;
    const webTypedArrayConstructors = { Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, Float32Array, Float64Array, };
    if (typeof BigInt64Array === "function") { webTypedArrayConstructors.BigInt64Array = BigInt64Array; webTypedArrayConstructors.BigUint64Array = BigUint64Array; }
    if (typeof Float16Array === 'function') webTypedArrayConstructors.Float16Array = Float16Array;
    const webViewKind = view => {
      if (view instanceof DataView) return { constructor: DataView, elementSize: 1 };
      const constructor = webTypedArrayConstructors[webTypedArrayTag.call(view)];
      return { constructor, elementSize: constructor.BYTES_PER_ELEMENT };
    };
    const webByobRequestToken = {};
    const webStreamClose = stream => {
      stream._state = 'closed';
      const reader = stream._reader;
      if (!reader) return;
      reader._closedRecord.resolve();
      if (reader instanceof ReadableStreamDefaultReader) while (stream._reads.length) stream._reads.shift().resolve({ value: undefined, done: true });
    };
    const webStreamError = (stream, error) => {
      stream._state = 'errored'; stream._error = error; stream._rejectCapacity(error);
      const reader = stream._reader;
      if (!reader) return;
      reader._closedRecord.reject(error);
      if (reader instanceof ReadableStreamDefaultReader) while (stream._reads.length) stream._reads.shift().reject(error);
      else while (stream._byobReads.length) stream._byobReads.shift().reject(error);
    };
    class ReadableStreamBYOBRequest {
      constructor(token) { if (token !== webByobRequestToken) throw webIllegalConstructor(); this._controller = undefined; this._view = null; }
      get view() { return this._view; }
      respond(bytesWritten) {
        bytesWritten = webEnforceRange(bytesWritten, 'bytesWritten');
        if (this._controller === undefined) throw webInvalidState('This BYOB request has been invalidated');
        if (webDetached(this._view.buffer)) throw webTypeError('The BYOB request view is detached');
        this._controller._respond(bytesWritten);
      }
      respondWithNewView(view) {
        if (!ArrayBuffer.isView(view)) throw webTypeError('view must be an ArrayBuffer view');
        if (this._controller === undefined) throw webInvalidState('This BYOB request has been invalidated');
        if (webDetached(view.buffer)) throw webTypeError('The view is detached');
        this._controller._respondWithNewView(view);
      }
    }
    class ReadableByteStreamController {
      constructor(stream) {
        this._stream = stream; this._queue = []; this._queueTotalSize = 0; this._closeRequested = false; this._pulling = false; this._pullAgain = false;
        this._pendingPullIntos = []; this._byobRequest = null;
      }
      get byobRequest() { return this._getByobRequest(); }
      get desiredSize() { return this._desiredSize(); }
      close() {
        if (this._closeRequested) throw webInvalidState('Controller is already closed');
        if (this._stream._state !== 'readable') throw webInvalidState('ReadableStream is not readable');
        this._close();
      }
      enqueue(chunk) {
        if (!ArrayBuffer.isView(chunk)) throw webTypeError('chunk must be an ArrayBuffer view');
        if (chunk.byteLength === 0) throw webTypeError('chunk must not be empty');
        if (chunk.buffer.byteLength === 0) throw webTypeError('chunk buffer must not be empty');
        if (this._closeRequested) throw webInvalidState('Controller is already closed');
        if (this._stream._state !== 'readable') throw webInvalidState('ReadableStream is not readable');
        this._enqueue(chunk);
      }
      error(error) { this._error(error); }
      _desiredSize() { const state = this._stream._state; return state === 'errored' ? null : state === 'closed' ? 0 : this._stream._highWaterMark - this._queueTotalSize; }
      _shouldCallPull() {
        const stream = this._stream;
        if (stream._state !== 'readable' || this._closeRequested || !stream._started) return false;
        if (stream._reader instanceof ReadableStreamDefaultReader && stream._reads.length > 0) return true;
        if (stream._reader instanceof ReadableStreamBYOBReader && stream._byobReads.length > 0) return true;
        return this._desiredSize() > 0;
      }
      _callPullIfNeeded() {
        if (!this._shouldCallPull()) return;
        if (this._pulling) { this._pullAgain = true; return; }
        const stream = this._stream;
        if (stream._sourcePull === undefined) return;
        this._pulling = true;
        let result;
        try { result = webApply(stream._sourcePull, stream._source, [this]); } catch (error) { result = Promise.reject(error); }
        Promise.resolve(result).then(() => { this._pulling = false; if (this._pullAgain) { this._pullAgain = false; this._callPullIfNeeded(); } }, error => this._error(error));
      }
      _clearPendingPullIntos() { this._invalidateByobRequest(); this._pendingPullIntos = []; }
      _invalidateByobRequest() {
        if (this._byobRequest === null) return;
        this._byobRequest._controller = undefined; this._byobRequest._view = null; this._byobRequest = null;
      }
      _resetQueue() { this._queue = []; this._queueTotalSize = 0; }
      _clearAlgorithms() { this._stream._sourcePull = undefined; this._stream._sourceCancel = undefined; }
      _error(error) {
        const stream = this._stream;
        if (stream._state !== 'readable') return;
        this._clearPendingPullIntos(); this._resetQueue(); this._clearAlgorithms();
        webStreamError(stream, error);
      }
      _close() {
        const stream = this._stream;
        if (this._closeRequested || stream._state !== 'readable') return;
        if (this._queueTotalSize > 0) { this._closeRequested = true; return; }
        if (this._pendingPullIntos.length > 0) {
          const first = this._pendingPullIntos[0];
          if (first.bytesFilled % first.elementSize !== 0) { const error = webInvalidState('Partial read'); this._error(error); throw error; }
        }
        this._clearAlgorithms(); webStreamClose(stream);
      }
      _enqueue(chunk) {
        const stream = this._stream;
        if (this._closeRequested || stream._state !== 'readable') return;
        const { byteOffset, byteLength } = chunk;
        if (webDetached(chunk.buffer)) throw webTypeError('The chunk buffer is detached');
        const buffer = webTransfer(chunk.buffer);
        if (this._pendingPullIntos.length > 0) {
          const first = this._pendingPullIntos[0];
          if (webDetached(first.buffer)) throw webTypeError('The pending pull-into buffer is detached');
          this._invalidateByobRequest();
          first.buffer = webTransfer(first.buffer);
          if (first.readerType === 'none') this._enqueueDetachedPullIntoToQueue(first);
        }
        if (stream._reader instanceof ReadableStreamDefaultReader) {
          this._processReadRequestsUsingQueue();
          if (stream._reads.length === 0) this._enqueueChunkToQueue(buffer, byteOffset, byteLength);
          else {
            if (this._pendingPullIntos.length > 0) this._pendingPullIntos.shift();
            stream._reads.shift().resolve({ value: new Uint8Array(buffer, byteOffset, byteLength), done: false });
          }
        } else if (stream._reader instanceof ReadableStreamBYOBReader) {
          this._enqueueChunkToQueue(buffer, byteOffset, byteLength);
          this._processPullIntoDescriptorsUsingQueue();
        } else this._enqueueChunkToQueue(buffer, byteOffset, byteLength);
        this._callPullIfNeeded();
      }
      _enqueueChunkToQueue(buffer, byteOffset, byteLength) { this._queue.push({ buffer, byteOffset, byteLength }); this._queueTotalSize += byteLength; }
      _enqueueClonedChunkToQueue(buffer, byteOffset, byteLength) {
        let clone;
        try { clone = buffer.slice(byteOffset, byteOffset + byteLength); } catch (error) { this._error(error); throw error; }
        this._enqueueChunkToQueue(clone, 0, byteLength);
      }
      _enqueueDetachedPullIntoToQueue(descriptor) {
        if (descriptor.bytesFilled > 0) this._enqueueClonedChunkToQueue(descriptor.buffer, descriptor.byteOffset, descriptor.bytesFilled);
        this._pendingPullIntos.shift();
      }
      _fillPullIntoDescriptorFromQueue(descriptor) {
        const maxBytesToCopy = Math.min(this._queueTotalSize, descriptor.byteLength - descriptor.bytesFilled), maxBytesFilled = descriptor.bytesFilled + maxBytesToCopy;
        let totalBytesToCopyRemaining = maxBytesToCopy, ready = false;
        const maxAlignedBytes = maxBytesFilled - (maxBytesFilled % descriptor.elementSize);
        if (maxAlignedBytes >= descriptor.minimumFill) { totalBytesToCopyRemaining = maxAlignedBytes - descriptor.bytesFilled; ready = true; }
        while (totalBytesToCopyRemaining > 0) {
          const head = this._queue[0], bytesToCopy = Math.min(totalBytesToCopyRemaining, head.byteLength);
          new Uint8Array(descriptor.buffer, descriptor.byteOffset + descriptor.bytesFilled, bytesToCopy).set(new Uint8Array(head.buffer, head.byteOffset, bytesToCopy));
          if (head.byteLength === bytesToCopy) this._queue.shift(); else { head.byteOffset += bytesToCopy; head.byteLength -= bytesToCopy; }
          this._queueTotalSize -= bytesToCopy;
          descriptor.bytesFilled += bytesToCopy;
          totalBytesToCopyRemaining -= bytesToCopy;
        }
        return ready;
      }
      _convertPullIntoDescriptor(descriptor) {
        const buffer = webTransfer(descriptor.buffer);
        return new descriptor.viewConstructor(buffer, descriptor.byteOffset, descriptor.bytesFilled / descriptor.elementSize);
      }
      _commitPullIntoDescriptor(descriptor) {
        const stream = this._stream, done = stream._state === 'closed', view = this._convertPullIntoDescriptor(descriptor);
        if (descriptor.readerType === 'default') stream._reads.shift().resolve({ value: view, done });
        else stream._byobReads.shift().resolve({ value: view, done });
      }
      _processPullIntoDescriptorsUsingQueue() {
        while (this._pendingPullIntos.length > 0) {
          if (this._queueTotalSize === 0) return;
          const descriptor = this._pendingPullIntos[0];
          if (this._fillPullIntoDescriptorFromQueue(descriptor)) { this._pendingPullIntos.shift(); this._commitPullIntoDescriptor(descriptor); }
        }
      }
      _processReadRequestsUsingQueue() {
        const stream = this._stream;
        while (stream._reads.length > 0) {
          if (this._queueTotalSize === 0) return;
          this._fillReadRequestFromQueue(stream._reads.shift());
        }
      }
      _fillReadRequestFromQueue(readRequest) {
        const entry = this._queue.shift();
        this._queueTotalSize -= entry.byteLength;
        this._handleQueueDrain();
        readRequest.resolve({ value: new Uint8Array(entry.buffer, entry.byteOffset, entry.byteLength), done: false });
      }
      _handleQueueDrain() {
        if (this._queueTotalSize === 0 && this._closeRequested) { this._clearAlgorithms(); webStreamClose(this._stream); } else this._callPullIfNeeded();
      }
      _getByobRequest() {
        if (this._byobRequest === null && this._pendingPullIntos.length > 0) {
          const first = this._pendingPullIntos[0], request = new ReadableStreamBYOBRequest(webByobRequestToken);
          request._controller = this; request._view = new Uint8Array(first.buffer, first.byteOffset + first.bytesFilled, first.byteLength - first.bytesFilled);
          this._byobRequest = request;
        }
        return this._byobRequest;
      }
      _respond(bytesWritten) {
        const first = this._pendingPullIntos[0];
        if (first === undefined) throw webInvalidState('There is no pending read');
        if (this._stream._state === 'closed') { if (bytesWritten !== 0) throw webTypeError('bytesWritten must be 0 when the stream is closed'); }
        else {
          if (bytesWritten === 0) throw webTypeError('bytesWritten must be greater than 0 when the stream is not closed');
          if (first.bytesFilled + bytesWritten > first.byteLength) throw webRangeError('bytesWritten out of range');
        }
        first.buffer = webTransfer(first.buffer);
        this._respondInternal(bytesWritten);
      }
      _respondInternal(bytesWritten) {
        const first = this._pendingPullIntos[0];
        if (webDetached(first.buffer)) throw webTypeError('The pending pull-into buffer is detached');
        this._invalidateByobRequest();
        if (this._stream._state === 'closed') this._respondInClosedState(first); else this._respondInReadableState(bytesWritten, first);
        this._callPullIfNeeded();
      }
      _respondInClosedState(first) {
        if (first.readerType === 'none') this._pendingPullIntos.shift();
        const stream = this._stream;
        if (stream._reader instanceof ReadableStreamBYOBReader) while (stream._byobReads.length > 0) this._commitPullIntoDescriptor(this._pendingPullIntos.shift());
      }
      _respondInReadableState(bytesWritten, descriptor) {
        descriptor.bytesFilled += bytesWritten;
        if (descriptor.readerType === 'none') { this._enqueueDetachedPullIntoToQueue(descriptor); this._processPullIntoDescriptorsUsingQueue(); return; }
        if (descriptor.bytesFilled < descriptor.minimumFill) return;
        this._pendingPullIntos.shift();
        const remainderSize = descriptor.bytesFilled % descriptor.elementSize;
        if (remainderSize > 0) {
          const end = descriptor.byteOffset + descriptor.bytesFilled;
          this._enqueueClonedChunkToQueue(descriptor.buffer, end - remainderSize, remainderSize);
        }
        descriptor.bytesFilled -= remainderSize;
        this._commitPullIntoDescriptor(descriptor);
        this._processPullIntoDescriptorsUsingQueue();
      }
      _respondWithNewView(view) {
        const first = this._pendingPullIntos[0];
        if (first === undefined) throw webInvalidState('There is no pending read');
        if (this._stream._state === 'closed') { if (view.byteLength !== 0) throw webTypeError('view must be empty when the stream is closed'); }
        else if (view.byteLength === 0) throw webTypeError('view must not be empty when the stream is not closed');
        if (first.byteOffset + first.bytesFilled !== view.byteOffset) throw webRangeError('view byteOffset does not match the pending read');
        if (first.bufferByteLength !== view.buffer.byteLength) throw webRangeError('view buffer length does not match the pending read');
        if (first.bytesFilled + view.byteLength > first.byteLength) throw webRangeError('view is larger than the pending read');
        const viewByteLength = view.byteLength;
        first.buffer = webTransfer(view.buffer);
        this._respondInternal(viewByteLength);
      }
      _pullSteps(readRequest) {
        const stream = this._stream;
        if (this._queueTotalSize > 0) { this._fillReadRequestFromQueue(readRequest); return; }
        const autoAllocateChunkSize = stream._autoAllocateChunkSize;
        if (autoAllocateChunkSize) {
          let buffer;
          try { buffer = new ArrayBuffer(autoAllocateChunkSize); } catch (error) { readRequest.reject(error); return; }
          this._pendingPullIntos.push({ buffer, bufferByteLength: autoAllocateChunkSize, byteOffset: 0, byteLength: autoAllocateChunkSize, bytesFilled: 0, minimumFill: 1, elementSize: 1, viewConstructor: Uint8Array, readerType: 'default' });
        }
        stream._reads.push(readRequest);
        this._callPullIfNeeded();
      }
      _cancelSteps(reason) {
        this._clearPendingPullIntos(); this._resetQueue();
        const stream = this._stream;
        let result;
        try { result = stream._sourceCancel === undefined ? undefined : webApply(stream._sourceCancel, stream._source, [reason]); } catch (error) { this._clearAlgorithms(); throw error; }
        this._clearAlgorithms();
        return result;
      }
      _releaseSteps() {
        if (this._pendingPullIntos.length > 0) { const first = this._pendingPullIntos[0]; first.readerType = 'none'; this._pendingPullIntos = [first]; }
      }
      _pullInto(view, min, request) {
        const stream = this._stream, { constructor, elementSize } = webViewKind(view), byteOffset = view.byteOffset, byteLength = view.byteLength;
        let buffer;
        try { buffer = webTransfer(view.buffer); } catch (error) { request.reject(error); return; }
        const descriptor = { buffer, bufferByteLength: buffer.byteLength, byteOffset, byteLength, bytesFilled: 0, minimumFill: min * elementSize, elementSize, viewConstructor: constructor, readerType: 'byob' };
        if (this._pendingPullIntos.length > 0) { this._pendingPullIntos.push(descriptor); stream._byobReads.push(request); return; }
        if (stream._state === 'closed') { request.resolve({ value: new constructor(descriptor.buffer, descriptor.byteOffset, 0), done: true }); return; }
        if (this._queueTotalSize > 0) {
          if (this._fillPullIntoDescriptorFromQueue(descriptor)) {
            const filled = this._convertPullIntoDescriptor(descriptor);
            this._handleQueueDrain();
            request.resolve({ value: filled, done: false });
            return;
          }
          if (this._closeRequested) { const error = webInvalidState('Partial read'); this._error(error); request.reject(error); return; }
        }
        this._pendingPullIntos.push(descriptor); stream._byobReads.push(request);
        this._callPullIfNeeded();
      }
    }
    class ReadableStreamDefaultReader {
      constructor(stream) { if (stream.locked) throw webInvalidState('ReadableStream is locked'); this._stream = stream; stream._reader = this; this._closedRecord = stream._state === 'readable' ? webDeferred() : stream._state === 'closed' ? webResolved() : webRejected(stream._error); }
      get closed() { return this._closedRecord.promise; }
      read() {
        const stream = this._stream; if (!stream) return Promise.reject(webInvalidState('Reader is released'));
        stream._disturbed = true;
        if (stream._byteStream) {
          if (stream._state === 'closed') return Promise.resolve({ value: undefined, done: true });
          if (stream._state === 'errored') return Promise.reject(stream._error);
          return new Promise((resolve, reject) => stream._controller._pullSteps({ reader: this, resolve, reject }));
        }
        if (stream._queue.length) { const value = stream._dequeue(); stream._finishCloseIfReady(); stream._callPullIfNeeded(); return Promise.resolve({ value, done: false }); }
        if (stream._state === 'closed') return Promise.resolve({ value: undefined, done: true });
        if (stream._state === 'errored') return Promise.reject(stream._error);
        const result = new Promise((resolve, reject) => stream._reads.push({ reader: this, resolve, reject }));
        stream._notifyCapacity(); stream._callPullIfNeeded();
        return result;
      }
      cancel(reason) { if (this._stream) this._stream._disturbed = true; return this._stream ? this._stream._cancel(reason) : Promise.reject(webInvalidState('Reader is released')); }
      releaseLock() {
        if (!this._stream) return;
        const stream = this._stream, error = webInvalidState('Reader was released');
        this._closedRecord = webEnsureRejected(this._closedRecord, error);
        stream._reads = stream._reads.filter(read => { if (read.reader !== this) return true; read.reject(error); return false; });
        if (stream._byteStream) stream._controller._releaseSteps();
        stream._reader = null; this._stream = null;
      }
    }
    class ReadableStreamBYOBReader {
      constructor(stream) {
        if (!stream || typeof stream !== 'object' || !('_byteStream' in stream)) throw webTypeError('stream must be a ReadableStream');
        if (stream.locked) throw webInvalidState('ReadableStream is locked');
        if (!stream._byteStream) throw webTypeError('stream must be a byte stream');
        this._stream = stream; stream._reader = this; this._closedRecord = stream._state === 'readable' ? webDeferred() : stream._state === 'closed' ? webResolved() : webRejected(stream._error);
      }
      get closed() { return this._closedRecord.promise; }
      read(view, options = {}) {
        try {
          if (!ArrayBuffer.isView(view)) throw webTypeError('view must be an ArrayBuffer view');
          if (view.byteLength === 0) throw webTypeError('view must not be empty');
          if (view.buffer.byteLength === 0) throw webTypeError('view buffer must not be empty');
          if (webDetached(view.buffer)) throw webTypeError('view buffer is detached');
          if (options === null || (typeof options !== 'object' && typeof options !== 'function')) { if (options !== undefined) throw webTypeError('options must be an object'); options = {}; }
          const min = options.min === undefined ? 1 : webEnforceRange(options.min, 'min');
          if (min === 0) throw webTypeError('min must be greater than 0');
          if (view instanceof DataView) { if (min > view.byteLength) throw webRangeError('min is larger than the view'); }
          else if (min > view.length) throw webRangeError('min is larger than the view');
          const stream = this._stream;
          if (!stream) throw webInvalidState('Reader is released');
          stream._disturbed = true;
          return new Promise((resolve, reject) => {
            const request = { reader: this, resolve, reject };
            if (stream._state === 'errored') reject(stream._error);
            else stream._controller._pullInto(view, min, request);
          });
        } catch (error) { return Promise.reject(error); }
      }
      cancel(reason) { if (this._stream) this._stream._disturbed = true; return this._stream ? this._stream._cancel(reason) : Promise.reject(webInvalidState('Reader is released')); }
      releaseLock() {
        if (!this._stream) return;
        const stream = this._stream, error = webInvalidState('Reader was released');
        this._closedRecord = webEnsureRejected(this._closedRecord, error);
        while (stream._byobReads.length) stream._byobReads.shift().reject(error);
        stream._controller._releaseSteps();
        stream._reader = null; this._stream = null;
      }
    }
    globalThis.ReadableStream = class ReadableStream {
      static from(iterable) {
        if (iterable === null || iterable === undefined) { const error = new TypeError('value is not iterable'); error.code = 'ERR_ARG_NOT_ITERABLE'; throw error; }
        const asyncMethod = iterable[Symbol.asyncIterator];
        const asynchronous = asyncMethod !== null && asyncMethod !== undefined;
        const method = asynchronous ? asyncMethod : iterable[Symbol.iterator];
        if (typeof method !== 'function') { const error = new TypeError('value is not iterable'); error.code = 'ERR_ARG_NOT_ITERABLE'; throw error; }
        const iterator = method.call(iterable);
        if (!iterator) throw new TypeError('iterator must provide next()');
        const next = iterator.next;
        if (typeof next !== 'function') throw new TypeError('iterator must provide next()');
        let chain = Promise.resolve(), done = false;
        return new ReadableStream({
          pull(controller) {
            chain = chain.then(async () => {
              if (done) return;
              const step = next.call(iterator);
              const result = asynchronous ? await step : step;
              if (!result || (typeof result !== 'object' && typeof result !== 'function')) throw new TypeError('iterator result must be an object');
              const finished = Boolean(result.done);
              const value = asynchronous ? (finished ? undefined : result.value) : await result.value;
              if (finished) { done = true; controller.close(); }
              else controller.enqueue(value);
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
        }, { highWaterMark: 0 });
      }
      constructor(source = {}, strategy = {}) {
        const { highWaterMark: strategyHighWaterMark, size } = webStrategy(strategy);
        // Convert the source dictionary once, in Web IDL member order, before start can mutate it.
        const autoAllocateValue = source.autoAllocateChunkSize;
        let autoAllocateChunkSize;
        const cancel = webCallback(source, 'cancel');
        const pull = webCallback(source, 'pull');
        const start = webCallback(source, 'start');
        const type = source.type;
        if (type !== undefined && type !== 'bytes') { const error = new TypeError('invalid source.type'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        if (type === 'bytes' && autoAllocateValue !== undefined) {
          autoAllocateChunkSize = webEnforceRange(autoAllocateValue, 'autoAllocateChunkSize');
          if (autoAllocateChunkSize === 0) throw webTypeError('autoAllocateChunkSize must be greater than 0');
        }
        if (type === 'bytes' && size !== undefined) { const error = new RangeError('byte streams cannot use a size strategy'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        const defaultHighWaterMark = type === 'bytes' ? 0 : 1, highWaterMark = strategyHighWaterMark === undefined ? defaultHighWaterMark : strategyHighWaterMark;
        if (Number.isNaN(highWaterMark) || highWaterMark < 0) { const error = new RangeError('invalid highWaterMark'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        this._source = source; this._sourcePull = pull; this._sourceCancel = cancel; this._byteStream = type === 'bytes'; this._highWaterMark = highWaterMark; this._sizeAlgorithm = webSizeAlgorithm(size); this._autoAllocateChunkSize = this._byteStream && autoAllocateChunkSize !== undefined ? autoAllocateChunkSize : 0; this._queue = []; this._queueSizes = []; this._queueTotalSize = 0; this._reads = []; this._byobReads = []; this._byobRequest = null; this._capacityWaiters = []; this._state = 'readable'; this._closeRequested = false; this._disturbed = false; this._error = undefined; this._reader = null; this._started = false; this._pulling = false; this._pullAgain = false;
        this._controller = this._byteStream ? new ReadableByteStreamController(this) : new ReadableStreamDefaultController(this);
        Promise.resolve(start === undefined ? undefined : webApply(start, source, [this._controller])).then(() => { this._started = true; this._callPullIfNeeded(); }, error => this._controller.error(error));
      }
      get locked() { return this._reader !== null; }
      getReader(options = {}) { if (options.mode === undefined) return new ReadableStreamDefaultReader(this); if (options.mode === 'byob') return new ReadableStreamBYOBReader(this); const error = new TypeError('invalid reader mode'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
      _chunkSize(chunk) { const size = Number(this._sizeAlgorithm(chunk)); if (!Number.isFinite(size) || size < 0) { const error = new RangeError('invalid chunk size'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } return size; }
      _dequeue() { const value = this._queue.shift(), size = this._queueSizes.shift() || 0; this._queueTotalSize = Math.max(0, this._queueTotalSize - size); this._notifyCapacity(); return value; }
      _hasCapacity() { return this._state === 'readable' && !this._closeRequested && (this._reads.length > 0 || this._highWaterMark - this._queueTotalSize > 0); }
      _waitForCapacity() { if (this._hasCapacity()) return Promise.resolve(); if (this._state === 'errored') return Promise.reject(this._error); if (this._state !== 'readable' || this._closeRequested) return Promise.reject(webInvalidState('ReadableStream is closed')); return new Promise((resolve, reject) => this._capacityWaiters.push({ resolve, reject })); }
      _notifyCapacity() { if (!this._hasCapacity() || !this._capacityWaiters.length) return; const waiters = this._capacityWaiters.splice(0); for (const waiter of waiters) waiter.resolve(); }
      _rejectCapacity(error) { const waiters = this._capacityWaiters.splice(0); for (const waiter of waiters) waiter.reject(error); }
      _finishCloseIfReady() { if (!this._closeRequested || this._queue.length || this._state !== 'readable') return; this._state = 'closed'; this._rejectCapacity(webInvalidState('ReadableStream is closed')); while (this._reads.length) this._reads.shift().resolve({ value: undefined, done: true }); if (this._reader) this._reader._closedRecord.resolve(); }
      _callPullIfNeeded() {
        if (this._byteStream) { this._controller._callPullIfNeeded(); return; }
        if (!this._started || this._state !== 'readable' || this._closeRequested || this._sourcePull === undefined) return;
        if (!this._reads.length && !this._byobReads.length && this._highWaterMark - this._queueTotalSize <= 0) return;
        if (this._pulling) { this._pullAgain = true; return; }
        this._pulling = true;
        Promise.resolve().then(() => webApply(this._sourcePull, this._source, [this._controller])).then(() => { this._pulling = false; if (this._pullAgain) { this._pullAgain = false; this._callPullIfNeeded(); } }, error => { this._pulling = false; this._controller.error(error); });
      }
      _cancel(reason) {
        if (this._byteStream) {
          if (this._state === 'closed') return Promise.resolve();
          if (this._state === 'errored') return Promise.reject(this._error);
          webStreamClose(this);
          if (this._reader instanceof ReadableStreamBYOBReader) while (this._byobReads.length) this._byobReads.shift().resolve({ value: undefined, done: true });
          try { return Promise.resolve(this._controller._cancelSteps(reason)).then(() => undefined); } catch (error) { return Promise.reject(error); }
        }
        return this._cancelDefault(reason);
      }
      _cancelDefault(reason) { if (this._state === 'closed') return Promise.resolve(); if (this._state === 'errored') return Promise.reject(this._error); this._queue.length = 0; this._queueSizes.length = 0; this._queueTotalSize = 0; this._closeRequested = true; this._rejectCapacity(reason); this._finishCloseIfReady(); try { return Promise.resolve(this._sourceCancel === undefined ? undefined : webApply(this._sourceCancel, this._source, [reason])).then(() => undefined); } catch (error) { return Promise.reject(error); } }
      cancel(reason) { if (this.locked) return Promise.reject(webInvalidState('ReadableStream is locked')); this._disturbed = true; return this._cancel(reason); }
      _byteTee() {
        const stream = this;
        let reader = new ReadableStreamDefaultReader(stream), reading = false, readAgainForBranch1 = false, readAgainForBranch2 = false, canceled1 = false, canceled2 = false, reason1, reason2, controller1, controller2, resolveCancel;
        const cancelPromise = new Promise(resolve => { resolveCancel = resolve; });
        const cloneAsUint8Array = chunk => new Uint8Array(chunk.buffer.slice(chunk.byteOffset, chunk.byteOffset + chunk.byteLength));
        const failBoth = error => { controller1.error(error); controller2.error(error); resolveCancel(stream._cancel(error)); };
        const forwardReaderError = thisReader => { thisReader.closed.catch(error => { if (thisReader !== reader) return; controller1.error(error); controller2.error(error); if (!canceled1 || !canceled2) resolveCancel(undefined); }); };
        const again = () => { if (readAgainForBranch1) pull1(); else if (readAgainForBranch2) pull2(); };
        const pullWithDefaultReader = () => {
          if (reader instanceof ReadableStreamBYOBReader) { reader.releaseLock(); reader = new ReadableStreamDefaultReader(stream); forwardReaderError(reader); }
          reader.read().then(result => {
            if (result.done) {
              reading = false;
              if (!canceled1) controller1._close();
              if (!canceled2) controller2._close();
              if (controller1._pendingPullIntos.length > 0) controller1._respond(0);
              if (controller2._pendingPullIntos.length > 0) controller2._respond(0);
              if (!canceled1 || !canceled2) resolveCancel(undefined);
              return;
            }
            Promise.resolve().then(() => {
              readAgainForBranch1 = false; readAgainForBranch2 = false;
              const chunk1 = result.value;
              let chunk2 = result.value;
              if (!canceled1 && !canceled2) {
                try { chunk2 = cloneAsUint8Array(chunk1); } catch (error) { failBoth(error); return; }
              }
              if (!canceled1) controller1._enqueue(chunk1);
              if (!canceled2) controller2._enqueue(chunk2);
              reading = false;
              again();
            });
          }, () => { reading = false; });
        };
        const pullWithBYOBReader = (view, forBranch2) => {
          if (reader instanceof ReadableStreamDefaultReader) { reader.releaseLock(); reader = new ReadableStreamBYOBReader(stream); forwardReaderError(reader); }
          const byobController = forBranch2 ? controller2 : controller1, otherController = forBranch2 ? controller1 : controller2;
          const request = {
            reader,
            resolve: ({ value: chunk, done }) => {
              if (done) {
                reading = false;
                const byobCanceled = forBranch2 ? canceled2 : canceled1, otherCanceled = forBranch2 ? canceled1 : canceled2;
                if (!byobCanceled) byobController._close();
                if (!otherCanceled) otherController._close();
                if (chunk !== undefined) {
                  if (!byobCanceled) byobController._respondWithNewView(chunk);
                  if (!otherCanceled && otherController._pendingPullIntos.length > 0) otherController._respond(0);
                }
                if (!byobCanceled || !otherCanceled) resolveCancel(undefined);
                return;
              }
              Promise.resolve().then(() => {
                readAgainForBranch1 = false; readAgainForBranch2 = false;
                const byobCanceled = forBranch2 ? canceled2 : canceled1, otherCanceled = forBranch2 ? canceled1 : canceled2;
                if (!otherCanceled) {
                  let cloned;
                  try { cloned = cloneAsUint8Array(chunk); } catch (error) { failBoth(error); return; }
                  if (!byobCanceled) byobController._respondWithNewView(chunk);
                  otherController._enqueue(cloned);
                } else if (!byobCanceled) byobController._respondWithNewView(chunk);
                reading = false;
                again();
              });
            },
            reject: () => { reading = false; }
          };
          stream._controller._pullInto(view, 1, request);
        };
        const pull1 = () => {
          if (reading) { readAgainForBranch1 = true; return Promise.resolve(); }
          reading = true;
          const byobRequest = controller1._getByobRequest();
          if (byobRequest === null) pullWithDefaultReader(); else pullWithBYOBReader(byobRequest.view, false);
          return Promise.resolve();
        };
        const pull2 = () => {
          if (reading) { readAgainForBranch2 = true; return Promise.resolve(); }
          reading = true;
          const byobRequest = controller2._getByobRequest();
          if (byobRequest === null) pullWithDefaultReader(); else pullWithBYOBReader(byobRequest.view, true);
          return Promise.resolve();
        };
        const cancel1 = reason => { canceled1 = true; reason1 = reason; if (canceled2) resolveCancel(stream._cancel([reason1, reason2])); return cancelPromise; };
        const cancel2 = reason => { canceled2 = true; reason2 = reason; if (canceled1) resolveCancel(stream._cancel([reason1, reason2])); return cancelPromise; };
        const branch1 = new ReadableStream({ type: 'bytes', start(controller) { controller1 = controller; }, pull: pull1, cancel: cancel1 });
        const branch2 = new ReadableStream({ type: 'bytes', start(controller) { controller2 = controller; }, pull: pull2, cancel: cancel2 });
        forwardReaderError(reader);
        return [branch1, branch2];
      }
      tee() {
        if (this.locked) throw webInvalidState('ReadableStream is locked');
        if (this._byteStream) return this._byteTee();
        const byteStream = this._byteStream, reader = this.getReader(), controllers = [null, null], cancelled = [false, false], reasons = [undefined, undefined], demand = [false, false];
        let reading = false, finished = false, resolveCancellation, rejectCancellation;
        const cancellation = new Promise((resolve, reject) => { resolveCancellation = resolve; rejectCancellation = reject; });
        const finish = (errored, error) => {
          if (finished) return;
          finished = true;
          for (let index = 0; index < 2; index++) {
            if (cancelled[index]) continue;
            if (errored) controllers[index].error(error);
            else controllers[index].close();
          }
          resolveCancellation();
        };
        const pump = () => {
          if (reading || finished || !demand.some((wanted, index) => wanted && !cancelled[index])) return;
          reading = true;
          reader.read().then(result => {
            reading = false;
            if (result.done) { finish(false); return; }
            for (let index = 0; index < 2; index++) {
              if (cancelled[index]) continue;
              demand[index] = false;
              controllers[index].enqueue(byteStream ? new Uint8Array(result.value) : result.value);
            }
            pump();
          }, error => { reading = false; finish(true, error); });
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
        let abort, abortEvent;
        try {
          if (signal !== undefined) {
            const abortPromise = new Promise((resolve, reject) => { abort = () => reject(signal.reason); if (signal.aborted) abort(); else signal.addEventListener('abort', abort, { once: true }); });
            abortEvent = abortPromise.then(undefined, error => ({ kind: 'abort', error }));
          }
          const sourceEnded = reader.closed.then(() => ({ kind: 'sourceClosed' }), error => ({ kind: 'sourceError', error }));
          const destinationEnded = writer.closed.then(() => ({ kind: 'destinationClosed' }), error => ({ kind: 'destinationError', error }));
          const wait = (operation, watchSource = false, watchDestination = true) => {
            const pending = [Promise.resolve(operation).then(value => ({ kind: 'value', value }), error => ({ kind: 'operationError', error }))];
            if (abortEvent) pending.push(abortEvent);
            if (watchSource) pending.push(sourceEnded);
            if (watchDestination) pending.push(destinationEnded);
            return Promise.race(pending);
          };
          const abortBoth = async error => {
            const actions = [];
            if (!options.preventAbort && destination._state === 'writable') actions.push(writer.abort(error));
            if (!options.preventCancel && this._state === 'readable') actions.push(reader.cancel(error));
            await Promise.all(actions);
          };
          const fail = async (event, phase) => {
            const error = event.kind === 'destinationClosed' ? webInvalidState('WritableStream closed during pipe') : event.error;
            if (event.kind === 'abort') await abortBoth(error);
            else if (event.kind === 'destinationClosed' || event.kind === 'destinationError' || phase !== 'read') {
              if (!options.preventCancel) await reader.cancel(error);
            } else if (!options.preventAbort) await writer.abort(error);
            throw error;
          };
          const currentState = () => {
            if (this._state === 'errored') return { kind: 'sourceError', error: this._error };
            if (destination._state === 'errored') return { kind: 'destinationError', error: destination._error };
            if (this._state === 'closed') return { kind: 'sourceClosed' };
            if (destination._state === 'closed' || destination._closeQueued) return { kind: 'destinationClosed' };
            return null;
          };
          if (signal !== undefined && signal.aborted) await fail({ kind: 'abort', error: signal.reason }, 'ready');
          while (true) {
            const beforeReady = currentState();
            if (beforeReady && beforeReady.kind === 'sourceClosed') break;
            if (beforeReady) await fail(beforeReady, beforeReady.kind === 'sourceError' ? 'read' : 'ready');
            const ready = await wait(writer.ready, true);
            if (ready.kind === 'abort') await fail(ready, 'ready');
            const afterReady = currentState() || ready;
            if (afterReady.kind === 'sourceClosed') break;
            if (afterReady.kind !== 'value') await fail(afterReady, afterReady.kind === 'sourceError' ? 'read' : 'ready');
            const read = await wait(reader.read());
            if (read.kind !== 'value') await fail(read, 'read');
            const result = read.value;
            if (result.done) break;
            const written = await wait(writer.write(result.value), false, false);
            if (written.kind !== 'value') await fail(written, 'write');
          }
          if (!options.preventClose && destination._state !== 'closed' && !destination._closeQueued) {
            const closed = await wait(writer.close(), false, false);
            if (closed.kind !== 'value') await fail(closed, 'close');
          }
        } finally {
          if (signal !== undefined && abort) signal.removeEventListener('abort', abort);
          reader.releaseLock(); writer.releaseLock();
        }
      }
      pipeThrough(transform, options) { if (this.locked) throw webInvalidState('ReadableStream is locked'); if (!transform || !transform.readable || !transform.writable) throw new TypeError('transform must contain readable and writable streams'); if (transform.writable.locked) throw webInvalidState('WritableStream is locked'); this.pipeTo(transform.writable, options).catch(() => {}); return transform.readable; }
      values(options = {}) {
        const preventCancel = Boolean(options.preventCancel), reader = this.getReader();
        let done = false, chain = Promise.resolve();
        const run = operation => { const result = chain.then(operation); chain = result.catch(() => {}); return result; };
        return {
          next: () => run(async () => {
            if (done) return { value: undefined, done: true };
            try { const result = await reader.read(); if (result.done) { done = true; reader.releaseLock(); } return result; }
            catch (error) { done = true; reader.releaseLock(); throw error; }
          }),
          return: value => run(async () => {
            if (!done) { done = true; try { if (!preventCancel) await reader.cancel(value); } finally { reader.releaseLock(); } }
            return { value, done: true };
          }),
          [Symbol.asyncIterator]() { return this; }
        };
      }
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
      constructor(stream) {
        if (stream.locked) throw webInvalidState('WritableStream is locked');
        this._stream = stream;
        this._readyRecord = stream._state === 'errored' ? webRejected(stream._error) : stream._state === 'writable' && stream._backpressured ? webDeferred() : webResolved();
        this._closedRecord = stream._closedStatus === 'rejected' ? webRejected(stream._closedReason) : stream._closedStatus === 'resolved' ? webResolved() : webDeferred();
        stream._writer = this;
      }
      get ready() { return this._readyRecord.promise; }
      get closed() { return this._closedRecord.promise; }
      write(chunk) { return this._stream ? this._stream._write(chunk) : Promise.reject(webInvalidState('Writer is released')); }
      close() { return this._stream ? this._stream._close() : Promise.reject(webInvalidState('Writer is released')); }
      abort(reason) { return this._stream ? this._stream._abort(reason) : Promise.reject(webInvalidState('Writer is released')); }
      releaseLock() { if (!this._stream) return; const error = webInvalidState('Writer was released'); this._readyRecord = webEnsureRejected(this._readyRecord, error); this._closedRecord = webEnsureRejected(this._closedRecord, error); this._stream._writer = null; this._stream = null; }
      get desiredSize() { return this._stream ? this._stream._desiredSize() : null; }
    }
    globalThis.WritableStream = class WritableStream {
      constructor(sink = {}, strategy = {}) {
        const { highWaterMark: strategyHighWaterMark, size } = webStrategy(strategy);
        const abort = webCallback(sink, 'abort');
        const close = webCallback(sink, 'close');
        const start = webCallback(sink, 'start');
        if (sink.type !== undefined) throw new RangeError('invalid sink.type');
        const write = webCallback(sink, 'write');
        const highWaterMark = strategyHighWaterMark === undefined ? 1 : strategyHighWaterMark;
        if (Number.isNaN(highWaterMark) || highWaterMark < 0) { const error = new RangeError('invalid highWaterMark'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; }
        this._sink = sink; this._sinkWrite = write; this._sinkClose = close; this._sinkAbort = abort; this._highWaterMark = highWaterMark; this._sizeAlgorithm = webSizeAlgorithm(size); this._queueTotalSize = 0; this._backpressured = false; this._state = 'writable'; this._closeQueued = false; this._closeInFlight = false; this._closeResult = null; this._abortPromise = null; this._error = undefined; this._writer = null; this._closedStatus = 'pending'; this._closedReason = undefined; this._chain = Promise.resolve();
        this._controller = new WritableStreamDefaultController(this); this._setBackpressure(this._desiredSize() <= 0);
        if (start !== undefined) this._chain = Promise.resolve(webApply(start, sink, [this._controller])).catch(error => { this._errorStream(error); throw error; });
        this._started = false; Promise.resolve(this._chain).then(() => { this._started = true; }, () => { this._started = true; });
      }
      get locked() { return this._writer !== null; }
      getWriter() { return new WritableStreamDefaultWriter(this); }
      _desiredSize() { if (this._state === 'errored') return null; if (this._state === 'closed') return 0; return this._highWaterMark - this._queueTotalSize; }
      _setBackpressure(value) { if (this._backpressured === value) return; this._backpressured = value; if (this._writer) { if (value) this._writer._readyRecord = webDeferred(); else this._writer._readyRecord.resolve(); } }
      _errorReady(error) { if (this._writer) this._writer._readyRecord = webEnsureRejected(this._writer._readyRecord, error); this._backpressured = false; }
      _resolveWriterClosed() { if (this._closedStatus !== 'pending') return; this._closedStatus = 'resolved'; if (this._writer) this._writer._closedRecord.resolve(); }
      _rejectWriterClosed(error) { if (this._closedStatus !== 'pending') return; this._closedStatus = 'rejected'; this._closedReason = error; if (this._writer) this._writer._closedRecord.reject(error); }
      _errorStream(error) { if (this._state !== 'writable') return; this._state = 'errored'; this._error = error; this._errorReady(error); if (!this._closeInFlight) this._rejectWriterClosed(error); }
      _write(chunk) {
        if (this._state === 'errored') return Promise.reject(this._error);
        if (this._state !== 'writable' || this._closeQueued) return Promise.reject(webInvalidState('WritableStream is closed'));
        let size;
        try { size = Number(this._sizeAlgorithm(chunk)); if (!Number.isFinite(size) || size < 0) { const error = new RangeError('invalid chunk size'); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } }
        catch (error) { this._errorStream(error); return Promise.reject(error); }
        this._queueTotalSize += size;
        this._setBackpressure(this._desiredSize() <= 0);
        // A write submitted while the sink is idle is already in flight (the spec calls the sink
        // synchronously); a later abort() must wait for it instead of rejecting it.
        const startsNow = this._started && !this._pendingWrites; this._pendingWrites = (this._pendingWrites || 0) + 1;
        const operation = this._chain.then(() => {
          if (this._state === 'errored' && !startsNow) throw this._error;
          return this._sinkWrite === undefined ? undefined : webApply(this._sinkWrite, this._sink, [chunk, this._controller]);
        });
        const result = operation.then(() => {
          this._pendingWrites--;
          this._queueTotalSize = Math.max(0, this._queueTotalSize - size);
          if (this._state === 'writable' && !this._closeQueued) this._setBackpressure(this._desiredSize() <= 0);
          this._runDeferredAbort();
        }, error => {
          this._pendingWrites--;
          this._queueTotalSize = Math.max(0, this._queueTotalSize - size);
          this._errorStream(error);
          this._runDeferredAbort();
          throw error;
        });
        this._chain = result.catch(() => {});
        return result;
      }
      _close() {
        if (this._state === 'errored') return Promise.reject(this._error);
        if (this._state !== 'writable' || this._closeQueued) return Promise.reject(webInvalidState('WritableStream is closed'));
        this._closeQueued = true;
        this._setBackpressure(false);
        const operation = this._chain.then(() => {
          if (this._state === 'errored') throw this._error;
          this._closeInFlight = true;
          return this._sinkClose === undefined ? undefined : webApply(this._sinkClose, this._sink, []);
        });
        const result = operation.then(() => {
          this._closeInFlight = false;
          this._error = undefined;
          this._state = 'closed';
          this._resolveWriterClosed();
        }, error => {
          this._closeInFlight = false;
          if (this._state === 'errored') this._rejectWriterClosed(this._error);
          else this._errorStream(error);
          throw error;
        });
        this._closeResult = result;
        this._chain = result.catch(() => {});
        return result;
      }
      close() { if (this.locked) { const error = new TypeError('WritableStream is locked'); error.code = 'ERR_INVALID_STATE'; return Promise.reject(error); } return this._close(); }
      _runDeferredAbort() { if (this._pendingWrites || !this._deferredAbort) return; const run = this._deferredAbort; this._deferredAbort = null; run(); }
      _abort(reason) {
        if (this._abortPromise) return this._abortPromise;
        if (this._state !== 'writable') return Promise.resolve();
        let resolveAbort, rejectAbort;
        const result = new Promise((resolve, reject) => { resolveAbort = resolve; rejectAbort = reject; });
        this._abortPromise = result;
        this._state = 'errored'; this._error = reason; this._errorReady(reason);
        this._controller._abortController.abort(reason);
        if (this._closeInFlight) {
          this._closeResult.then(() => resolveAbort(), error => rejectAbort(error));
        } else if (this._pendingWrites) {
          // Spec: the sink abort runs synchronously when the in-flight write finishes, before the
          // write's own promise reactions.
          this._deferredAbort = () => {
            let operation;
            try { operation = Promise.resolve(this._sinkAbort === undefined ? undefined : webApply(this._sinkAbort, this._sink, [reason])); }
            catch (error) { operation = Promise.reject(error); }
            operation.then(() => { this._rejectWriterClosed(reason); resolveAbort(); }, error => { this._rejectWriterClosed(reason); rejectAbort(error); });
          };
          this._chain = result.catch(() => {});
        } else {
          const operation = this._chain.then(() => this._sinkAbort === undefined ? undefined : webApply(this._sinkAbort, this._sink, [reason]));
          operation.then(() => { this._rejectWriterClosed(reason); resolveAbort(); }, error => { this._rejectWriterClosed(reason); rejectAbort(error); });
          this._chain = result.catch(() => {});
        }
        result.then(() => { this._abortPromise = null; }, () => { this._abortPromise = null; });
        return result;
      }
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
      constructor(transformer = {}, writableStrategy = {}, readableStrategy = {}) {
        const writableQueueStrategy = webStrategy(writableStrategy);
        const readableQueueStrategy = webStrategy(readableStrategy);
        const cancel = webCallback(transformer, 'cancel');
        const flush = webCallback(transformer, 'flush');
        const readableType = transformer.readableType;
        const start = webCallback(transformer, 'start');
        const transform = webCallback(transformer, 'transform');
        const writableType = transformer.writableType;
        if (readableType !== undefined || writableType !== undefined) throw new RangeError('transformer types are not supported');
        let readableController, writable;
        if (readableQueueStrategy.highWaterMark === undefined) readableQueueStrategy.highWaterMark = 0;
        const callCancel = reason => cancel === undefined ? undefined : webApply(cancel, transformer, [reason]);
        this.readable = new ReadableStream({ start(value) { readableController = value; }, cancel(reason) { let outcome; try { outcome = callCancel(reason); } catch (error) { outcome = Promise.reject(error); } return Promise.resolve(outcome).then(() => { if (writable) writable._errorStream(reason); }, error => { if (writable) writable._errorStream(error); throw error; }); } }, readableQueueStrategy);
        const controller = new TransformStreamDefaultController(readableController), fail = error => { controller.error(error); throw error; };
        writable = this.writable = new WritableStream({
          start(writableController) { controller._writable = writable = writableController._stream; const result = start === undefined ? undefined : webApply(start, transformer, [controller]); return result !== null && (typeof result === 'object' || typeof result === 'function') && typeof result.then === 'function' ? Promise.resolve(result).catch(error => { controller.error(error); throw error; }) : result; },
          write(chunk) { return readableController._stream._waitForCapacity().then(() => transform === undefined ? controller.enqueue(chunk) : webApply(transform, transformer, [chunk, controller])).catch(fail); },
          close() { return Promise.resolve().then(() => flush === undefined ? undefined : webApply(flush, transformer, [controller])).then(() => readableController.close()).catch(fail); },
          abort(error) { return Promise.resolve(callCancel(error)).then(() => readableController.error(error)); }
        }, writableQueueStrategy);
        controller._writable = writable;
      }
    };
    globalThis.TransformStreamDefaultController = TransformStreamDefaultController;
    const streamBytes = (value, allowShared = false) => {
      const shared = buffer => typeof SharedArrayBuffer === 'function' && buffer instanceof SharedArrayBuffer;
      if (value instanceof ArrayBuffer || (allowShared && shared(value))) return new Uint8Array(value);
      if (ArrayBuffer.isView(value) && (allowShared || !shared(value.buffer))) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      return null;
    };
    const hexFromBytes = value => Array.from(value, byte => byte.toString(16).padStart(2, '0')).join('');
    const bytesFromHex = value => new Uint8Array(String(value).match(/../g)?.map(pair => parseInt(pair, 16)) || []);
    const compressionTransform = (operation, format) => {
      const normalized = String(format);
      if (normalized !== 'gzip' && normalized !== 'deflate' && normalized !== 'deflate-raw' && normalized !== 'brotli') throw new TypeError('Unsupported compression format');
      const nativeFormat = normalized === 'deflate-raw' ? 'deflateRaw' : normalized, handle = __thaw_zlib_stream_create(operation, nativeFormat);
      let active = true;
      const release = () => { if (active) { active = false; __thaw_zlib_stream_drop(handle); } };
      return new TransformStream({
        transform(chunk, controller) {
          try {
            const bytes = streamBytes(chunk);
            if (!bytes) throw new TypeError('chunk must be an ArrayBuffer or view');
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
    globalThis.TextEncoderStream = class TextEncoderStream { constructor() { const encoder = new TextEncoder(); let pendingHigh = ''; const transform = new TransformStream({ transform(chunk, controller) { let text = pendingHigh + `${chunk}`; pendingHigh = ''; const last = text.charCodeAt(text.length - 1); if (last >= 0xd800 && last <= 0xdbff) { pendingHigh = text.slice(-1); text = text.slice(0, -1); } if (text) controller.enqueue(encoder.encode(text)); }, flush(controller) { if (pendingHigh) controller.enqueue(encoder.encode(pendingHigh)); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = 'utf-8'; } };
    globalThis.TextDecoderStream = class TextDecoderStream { constructor(label = 'utf-8', options = {}) { const decoder = new TextDecoder(label, options); const transform = new TransformStream({ transform(chunk, controller) { const bytes = streamBytes(chunk, true); if (!bytes) throw new TypeError('chunk must be an ArrayBuffer or view'); const text = decoder.decode(bytes, { stream: true }); if (text) controller.enqueue(text); }, flush(controller) { const text = decoder.decode(); if (text) controller.enqueue(text); } }); this.readable = transform.readable; this.writable = transform.writable; this.encoding = decoder.encoding || String(label).toLowerCase(); this.fatal = decoder.fatal; this.ignoreBOM = decoder.ignoreBOM; } };
  }
  if (typeof globalThis.FormData !== 'function') {
    const formDataEntries = new WeakMap();
    const formDataIteratorStates = new WeakMap();
    const formDataApply = Reflect.apply;
    const formDataList = value => { const entries = formDataEntries.get(value); if (!entries) throw new TypeError('invalid FormData receiver'); return entries; };
    const formDataIteratorPrototype = {
      next() {
        const state = formDataIteratorStates.get(this);
        if (!state) throw new TypeError('invalid FormData iterator receiver');
        const entries = formDataList(state.target);
        if (state.index >= entries.length) return { value: undefined, done: true };
        const [name, value] = entries[state.index++];
        return { value: state.kind === 'key' ? name : state.kind === 'value' ? value : [name, value], done: false };
      },
      [Symbol.iterator]() { return this; },
      get [Symbol.toStringTag]() { return 'FormData Iterator'; }
    };
    const formDataIterator = (target, kind) => {
      formDataList(target);
      const iterator = Object.create(formDataIteratorPrototype);
      formDataIteratorStates.set(iterator, { target, kind, index: 0 });
      return iterator;
    };
    const formDataEntry = (name, value, filename) => {
      const key = String(name);
      if (!(value instanceof Blob)) {
        if (filename !== undefined) throw new TypeError('filename requires a Blob value');
        return [key, String(value)];
      }
      const file = filename === undefined && value instanceof File ? value
        : new File([value], filename === undefined ? 'blob' : String(filename), { type: value.type, lastModified: value instanceof File ? value.lastModified : undefined });
      return [key, file];
    };
    class FormData {
      constructor() { formDataEntries.set(this, []); }
      append(name, value, filename = undefined) { formDataEntries.get(this).push(formDataEntry(name, value, filename)); }
      delete(name) { const key = String(name); formDataEntries.set(this, formDataEntries.get(this).filter(entry => entry[0] !== key)); }
      get(name) { const key = String(name), entry = formDataEntries.get(this).find(item => item[0] === key); return entry ? entry[1] : null; }
      getAll(name) { const key = String(name); return formDataEntries.get(this).filter(entry => entry[0] === key).map(entry => entry[1]); }
      has(name) { const key = String(name); return formDataEntries.get(this).some(entry => entry[0] === key); }
      set(name, value, filename = undefined) { const replacement = formDataEntry(name, value, filename), key = replacement[0], entries = formDataEntries.get(this), index = entries.findIndex(entry => entry[0] === key); if (index < 0) entries.push(replacement); else { entries[index] = replacement; formDataEntries.set(this, entries.filter((entry, current) => entry[0] !== key || current === index)); } }
      entries() { return formDataIterator(this, 'entry'); }
      keys() { return formDataIterator(this, 'key'); }
      values() { return formDataIterator(this, 'value'); }
      forEach(callback, thisArg = undefined) {
        formDataList(this);
        if (typeof callback !== 'function') throw new TypeError('callback must be a function');
        let index = 0, entries = formDataList(this);
        while (index < entries.length) {
          const [name, value] = entries[index++];
          formDataApply(callback, thisArg, [value, name, this]);
          entries = formDataList(this);
        }
      }
      [Symbol.iterator]() { return formDataIterator(this, 'entry'); }
      get [Symbol.toStringTag]() { return 'FormData'; }
    }
    globalThis.FormData = FormData;
    Object.defineProperty(globalThis, '__thaw_form_data_entries', {
      configurable: true,
      value(value) { return formDataList(value).slice(); }
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
    // A body chunk must be a Uint8Array; its bytes are copied on receipt because
    // the producer may reuse the buffer for its next chunk.
    const bodyChunkBytes = value => {
      if (!(value instanceof Uint8Array)) throw new TypeError('Received non-Uint8Array chunk');
      return value.slice();
    };
    const consumeResponseBody = async response => {
      const record = responseData.get(response); if (!record) throw new TypeError('invalid Response receiver');
      if (!record.body) return new Uint8Array();
      if (record.body._disturbed || record.body.locked) throw new TypeError('Body is unusable');
      const reader = record.body.getReader(), chunks = []; let length = 0;
      while (true) { const result = await reader.read(); if (result.done) break; const chunk = bodyChunkBytes(result.value); chunks.push(chunk); length += chunk.byteLength; }
      const output = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.byteLength; } return output;
    };
    const parseFormDataBody = (type, bytes) => {
      const data = new FormData(), lower = type.split(';')[0].trim().toLowerCase();
      if (lower === 'application/x-www-form-urlencoded') { const params = new URLSearchParams(new TextDecoder().decode(bytes)); for (const [name, value] of params) data.append(name, value); return data; }
      if (lower !== 'multipart/form-data') throw new TypeError('unsupported form data content type');
      const boundaryMatch = /(?:^|;)\s*boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(type);
      if (!boundaryMatch) throw new TypeError('multipart boundary is missing');
      const boundary = boundaryMatch[1] || boundaryMatch[2]; let binary = '';
      for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
      // A delimiter is `--boundary` at the start of a line (and followed by `--` or the line end),
      // so the boundary text inside a value or file does not split it.
      const delimiter = `--${boundary}`, positions = [];
      for (let at = binary.indexOf(delimiter); at >= 0; at = binary.indexOf(delimiter, at + delimiter.length)) {
        const after = binary.slice(at + delimiter.length, at + delimiter.length + 2);
        if ((at === 0 || binary.slice(at - 2, at) === '\r\n') && (after === '--' || after === '\r\n' || after === '' || /^[ \t]/.test(after))) positions.push(at);
      }
      const sections = positions.map((at, index) => binary.slice(at + delimiter.length, index + 1 < positions.length ? positions[index + 1] : binary.length));
      for (let section of sections) {
        if (section.startsWith('--')) break;
        if (section.startsWith('\r\n')) section = section.slice(2);
        if (section.endsWith('\r\n')) section = section.slice(0, -2);
        const marker = section.indexOf('\r\n\r\n'); if (marker < 0) continue;
        const rawHeaders = section.slice(0, marker), content = section.slice(marker + 4), headers = Object.create(null);
        for (const line of rawHeaders.split('\r\n')) { const colon = line.indexOf(':'); if (colon >= 0) headers[line.slice(0, colon).toLowerCase()] = line.slice(colon + 1).trim(); }
        const disposition = headers['content-disposition'] || '', nameMatch = /(?:^|;)\s*name="([^"]*)"/i.exec(disposition); if (!nameMatch) continue;
        const decodeParameter = value => new TextDecoder().decode(Uint8Array.from(value, character => character.charCodeAt(0))).replace(/%22/gi, '"').replace(/%0D/gi, '\r').replace(/%0A/gi, '\n');
        const name = decodeParameter(nameMatch[1]), filenameMatch = /(?:^|;)\s*filename="([^"]*)"/i.exec(disposition), partBytes = Uint8Array.from(content, character => character.charCodeAt(0));
        if (filenameMatch) data.append(name, new File([partBytes], decodeParameter(filenameMatch[1]), { type: headers['content-type'] || 'text/plain' }));
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
      clone() { const record = responseData.get(this); if (record.body && (record.body._disturbed || record.body.locked)) throw new TypeError('Body has already been consumed'); let body = null; if (record.body) { const branches = record.body.tee(); record.body = branches[0]; body = branches[1]; } const clone = new Response(body, { status: 200, statusText: record.statusText, headers: record.headers }); const cloneRecord = responseData.get(clone); cloneRecord.status = record.status; cloneRecord.type = record.type; cloneRecord.url = record.url; cloneRecord.redirected = record.redirected; if (thawImmutableResponseHeaders.has(record.headers)) thawImmutableResponseHeaders.add(cloneRecord.headers); return clone; }
      static error() { const response = new Response(); const record = responseData.get(response); record.status = 0; record.type = 'error'; thawImmutableResponseHeaders.add(record.headers); return response; }
      static json(value, init = {}) { const body = JSON.stringify(value); if (body === undefined) throw new TypeError('value is not JSON serializable'); const headers = new Headers(init.headers); if (!headers.has('content-type')) headers.set('content-type', 'application/json'); return new Response(body, { ...init, headers }); }
      static redirect(url, status = 302) { const code = Number(status); if (![301, 302, 303, 307, 308].includes(code)) throw new RangeError('invalid redirect status'); const response = new Response(null, { status: code, headers: { location: new URL(String(url)).href } }); thawImmutableResponseHeaders.add(responseData.get(response).headers); return response; }
      get [Symbol.toStringTag]() { return 'Response'; }
    }
    Object.defineProperty(globalThis, '__thaw_set_response_metadata', {
      configurable: true,
      value(response, url, redirected) {
        const record = responseData.get(response); if (!record) throw new TypeError('invalid Response receiver');
        record.url = String(url); record.redirected = Boolean(redirected); thawImmutableResponseHeaders.add(record.headers); return response;
      }
    });
    const requestData = new WeakMap();
    const consumeRequestBody = async (request, signal) => {
      const record = requestData.get(request); if (!record) throw new TypeError('invalid Request receiver');
      if (signal && signal.aborted) throw signal.reason;
      if (!record.body) return new Uint8Array();
      if (record.body._disturbed || record.body.locked) throw new TypeError('Body is unusable');
      const reader = record.body.getReader(), chunks = []; let length = 0, onAbort;
      const interrupted = signal ? new Promise((resolve, reject) => {
        onAbort = () => {
          reject(signal.reason);
          Promise.resolve(reader.cancel(signal.reason)).catch(() => {});
        };
        signal.addEventListener('abort', onAbort, { once: true });
      }) : null;
      const reading = (async () => {
      while (true) { const result = await reader.read(); if (result.done) break; const chunk = bodyChunkBytes(result.value); chunks.push(chunk); length += chunk.byteLength; }
      const output = new Uint8Array(length); let offset = 0; for (const chunk of chunks) { output.set(chunk, offset); offset += chunk.byteLength; } return output;
      })();
      try {
        return await (interrupted ? Promise.race([reading, interrupted]) : reading);
      } finally {
        if (signal) signal.removeEventListener('abort', onAbort);
        reader.releaseLock();
      }
    };
    Object.defineProperty(globalThis, '__thaw_request_bytes_for_fetch', {
      value: request => consumeRequestBody(request, requestData.get(request).signal),
      configurable: false, writable: false,
    });
    Object.defineProperty(globalThis, '__thaw_request_body_replayable', {
      value: request => { const record = requestData.get(request); if (!record) throw new TypeError('invalid Request receiver'); return !record.body || record.bodyHasSource; },
      configurable: false, writable: false,
    });
    class Request {
      constructor(input, init = {}) {
        const inherited = requestData.get(input), url = new URL(inherited ? inherited.url : String(input));
        if (url.username || url.password) throw new TypeError('Request URL cannot contain credentials');
        let method = init.method === undefined ? inherited ? inherited.method : 'GET' : String(init.method);
        if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(method)) throw new TypeError('invalid HTTP method');
        const upperMethod = method.toUpperCase(); if (['DELETE', 'GET', 'HEAD', 'OPTIONS', 'POST', 'PUT'].includes(upperMethod)) method = upperMethod;
        if (upperMethod === 'CONNECT' || upperMethod === 'TRACE' || upperMethod === 'TRACK') throw new TypeError(`unsupported HTTP method ${upperMethod}`);
        const headers = new Headers(init.headers === undefined ? inherited ? inherited.headers : undefined : init.headers), hasBody = init.body !== undefined && init.body !== null;
        let normalized = { stream: null, type: null };
        const inheriting = !hasBody && Boolean(inherited && inherited.body);
        const bodyHasSource = hasBody ? !(init.body instanceof ReadableStream) : inherited ? inherited.bodyHasSource : true;
        if (hasBody) normalized = responseBody(init.body);
        if ((method === 'GET' || method === 'HEAD') && (normalized.stream || inheriting)) throw new TypeError('Request with GET/HEAD method cannot have body');
        if (hasBody && init.body instanceof ReadableStream && init.duplex !== 'half') throw new TypeError('duplex option is required for a streaming body');
        if (normalized.type && !headers.has('content-type')) headers.set('content-type', normalized.type);
        const sourceSignal = init.signal === undefined ? inherited ? inherited.signal : null : init.signal, signalController = new AbortController();
        if (sourceSignal) { if (typeof sourceSignal.aborted !== 'boolean' || typeof sourceSignal.addEventListener !== 'function') throw new TypeError('signal must be an AbortSignal'); if (sourceSignal.aborted) signalController.abort(sourceSignal.reason); else sourceSignal.addEventListener('abort', () => signalController.abort(sourceSignal.reason), { once: true }); }
        const select = (value, fallback, accepted, name) => { const selected = value === undefined ? fallback : String(value); if (!accepted.includes(selected)) throw new TypeError(`invalid Request ${name}`); return selected; };
        const mode = select(init.mode, inherited ? inherited.mode : 'cors', ['same-origin', 'no-cors', 'cors'], 'mode');
        const cache = select(init.cache, inherited ? inherited.cache : 'default', ['default', 'no-store', 'reload', 'no-cache', 'force-cache', 'only-if-cached'], 'cache');
        if (cache === 'only-if-cached' && mode !== 'same-origin') throw new TypeError('only-if-cached requires same-origin mode');
        const keepalive = init.keepalive === undefined ? inherited ? inherited.keepalive : false : Boolean(init.keepalive);
        if ((normalized.stream || inheriting) && !bodyHasSource) {
          if (keepalive) throw new TypeError('keepalive cannot be used with a streaming body');
          if (mode === 'no-cors') throw new TypeError('a streaming body requires same-origin or cors mode');
        }
        const credentials = select(init.credentials, inherited ? inherited.credentials : 'same-origin', ['omit', 'same-origin', 'include'], 'credentials');
        const redirect = select(init.redirect, inherited ? inherited.redirect : 'follow', ['follow', 'manual', 'error'], 'redirect');
        const referrerPolicy = select(init.referrerPolicy, inherited ? inherited.referrerPolicy : '', ['', 'no-referrer', 'no-referrer-when-downgrade', 'same-origin', 'origin', 'strict-origin', 'origin-when-cross-origin', 'strict-origin-when-cross-origin', 'unsafe-url'], 'referrerPolicy');
        let referrer = init.referrer === undefined ? inherited ? inherited.referrer : 'about:client' : String(init.referrer);
        if (referrer && referrer !== 'about:client') referrer = new URL(referrer).href;
        // The original request's body is taken over only after every check above has passed.
        if (inheriting) {
          if (inherited.body._disturbed || inherited.body.locked) throw new TypeError('Body is unusable');
          const reader = inherited.body.getReader(); inherited.body._disturbed = true;
          normalized.stream = new ReadableStream({ async pull(controller) { const result = await reader.read(); if (result.done) controller.close(); else controller.enqueue(result.value); }, cancel(reason) { return reader.cancel(reason); } });
        }
        requestData.set(this, {
          body: normalized.stream, bodyHasSource, headers, method, url: url.href, signal: signalController.signal,
          cache, credentials,
          destination: inherited ? inherited.destination : '',
          integrity: init.integrity === undefined ? inherited ? inherited.integrity : '' : String(init.integrity),
          keepalive,
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
      clone() { const record = requestData.get(this); if (record.body && (record.body._disturbed || record.body.locked)) throw new TypeError('Body has already been consumed'); let body = null; if (record.body) { const branches = record.body.tee(); record.body = branches[0]; body = branches[1]; } const clone = new Request(record.url, { method: record.method, headers: record.headers, body, signal: record.signal, cache: record.cache, credentials: record.credentials, integrity: record.integrity, keepalive: record.keepalive, mode: record.mode, redirect: record.redirect, referrer: record.referrer, referrerPolicy: record.referrerPolicy, duplex: 'half' }); requestData.get(clone).bodyHasSource = record.bodyHasSource; return clone; }
      get [Symbol.toStringTag]() { return 'Request'; }
    }
    globalThis.Response = Response;
    globalThis.Request = Request;
  }
