(function () {
  if (typeof globalThis.ReadableStream === 'function') return;
  function def(o, name, v) {
    Object.defineProperty(o, name, { value: v, writable: true, enumerable: false, configurable: true });
  }
  function lockedError(what) { return new TypeError('This ' + what + ' is locked to a reader or writer'); }
  function deferred() {
    var d = {};
    d.promise = new Promise(function (res, rej) { d.resolve = res; d.reject = rej; });
    return d;
  }
  function strategy(s, dflt) {
    return {
      hwm: s && s.highWaterMark !== undefined ? Number(s.highWaterMark) : dflt,
      size: s && typeof s.size === 'function' ? s.size : function () { return 1; }
    };
  }

  // ----- ReadableStream -----
  function ReadableStream(source, strat) {
    if (!(this instanceof ReadableStream)) throw new TypeError("Class constructor ReadableStream cannot be invoked without 'new'");
    source = source || {};
    var sg = strategy(strat, 1);
    var st = { queue: [], reads: [], state: 'readable', err: undefined, closeReq: false, pulling: false, again: false,
      started: false, locked: false, qsize: 0, closed: deferred() };
    st.closed.promise.catch(function () {});
    def(this, '_s', st);
    function desired() { return st.state === 'errored' ? null : st.state === 'closed' ? 0 : sg.hwm - st.qsize; }
    function finishClose() {
      st.state = 'closed';
      st.reads.splice(0).forEach(function (r) { r.resolve({ value: undefined, done: true }); });
      st.closed.resolve(undefined);
    }
    function fail(e) {
      if (st.state !== 'readable') return;
      st.state = 'errored'; st.err = e; st.queue = []; st.qsize = 0;
      st.reads.splice(0).forEach(function (r) { r.reject(e); });
      st.closed.reject(e);
    }
    function pull() {
      if (!st.started || st.state !== 'readable' || st.closeReq) return;
      if (!(st.reads.length > 0 || desired() > 0)) return;
      if (st.pulling) { st.again = true; return; }
      st.pulling = true;
      var r;
      try { r = source.pull ? source.pull(controller) : undefined; } catch (e) { st.pulling = false; fail(e); return; }
      Promise.resolve(r).then(function () {
        st.pulling = false;
        if (st.again) { st.again = false; pull(); }
      }, fail);
    }
    var controller = {};
    Object.defineProperty(controller, 'desiredSize', { get: desired, enumerable: true, configurable: true });
    def(controller, 'enqueue', function (chunk) {
      if (st.closeReq || st.state !== 'readable') throw new TypeError('Cannot enqueue a chunk into a closed or errored stream');
      if (st.reads.length > 0) st.reads.shift().resolve({ value: chunk, done: false });
      else {
        st.queue.push({ chunk: chunk, size: Number(sg.size(chunk)) });
        st.qsize += st.queue[st.queue.length - 1].size;
      }
      pull();
    });
    def(controller, 'close', function () {
      if (st.closeReq || st.state !== 'readable') throw new TypeError('Cannot close a closed or errored stream');
      st.closeReq = true;
      if (st.queue.length === 0) finishClose();
    });
    def(controller, 'error', fail);
    def(this, '_ops', {
      read: function () {
        if (st.state === 'errored') return Promise.reject(st.err);
        if (st.queue.length > 0) {
          var item = st.queue.shift();
          st.qsize -= item.size;
          if (st.closeReq && st.queue.length === 0) finishClose(); else pull();
          return Promise.resolve({ value: item.chunk, done: false });
        }
        if (st.state === 'closed') return Promise.resolve({ value: undefined, done: true });
        var d = deferred();
        st.reads.push(d);
        pull();
        return d.promise;
      },
      cancel: function (reason) {
        if (st.state === 'errored') return Promise.reject(st.err);
        if (st.state === 'readable') { st.queue = []; st.qsize = 0; finishClose(); }
        try { return Promise.resolve(source.cancel ? source.cancel(reason) : undefined).then(function () {}); }
        catch (e) { return Promise.reject(e); }
      }
    });
    try {
      var started = source.start ? source.start(controller) : undefined;
      Promise.resolve(started).then(function () { st.started = true; pull(); }, fail);
    } catch (e) { fail(e); }
  }
  Object.defineProperty(ReadableStream.prototype, 'locked', { get: function () { return this._s.locked; }, configurable: true });
  def(ReadableStream.prototype, 'getReader', function () {
    if (this._s.locked) throw lockedError('ReadableStream');
    var stream = this, st = this._s, ops = this._ops;
    st.locked = true;
    var reader = Object.create(ReadableStreamDefaultReader.prototype), released = false;
    def(reader, 'read', function () {
      if (released) return Promise.reject(new TypeError('This reader has been released'));
      return ops.read();
    });
    def(reader, 'cancel', function (reason) { return released ? Promise.reject(new TypeError('This reader has been released')) : ops.cancel(reason); });
    def(reader, 'releaseLock', function () {
      if (released) return;
      released = true; st.locked = false;
      st.reads.splice(0).forEach(function (r) { r.reject(new TypeError('Reader released')); });
    });
    Object.defineProperty(reader, 'closed', { get: function () { return st.closed.promise; }, configurable: true });
    return reader;
  });
  def(ReadableStream.prototype, 'cancel', function (reason) {
    if (this._s.locked) return Promise.reject(lockedError('ReadableStream'));
    return this._ops.cancel(reason);
  });
  def(ReadableStream.prototype, 'values', function () {
    var reader = this.getReader();
    return {
      next: function () { return reader.read().then(function (r) { if (r.done) reader.releaseLock(); return r; }); },
      return: function (v) { var p = reader.cancel(v); reader.releaseLock(); return p.then(function () { return { value: v, done: true }; }); },
      [Symbol.asyncIterator]: function () { return this; }
    };
  });
  def(ReadableStream.prototype, Symbol.asyncIterator, ReadableStream.prototype.values);
  def(ReadableStream.prototype, 'pipeTo', function (dest) {
    var reader = this.getReader(), writer = dest.getWriter();
    function step() {
      return reader.read().then(function (r) {
        if (r.done) { reader.releaseLock(); return writer.close().then(function () { writer.releaseLock(); }); }
        return writer.write(r.value).then(step);
      });
    }
    return step().catch(function (e) { reader.releaseLock(); writer.releaseLock(); throw e; });
  });
  // ponytail: tee() and pipeThrough() need TransformStream; add when a package uses them
  Object.defineProperty(ReadableStream.prototype, Symbol.toStringTag, { value: 'ReadableStream', configurable: true });
  function ReadableStreamDefaultReader() { throw new TypeError('Illegal constructor'); }
  Object.defineProperty(ReadableStreamDefaultReader.prototype, Symbol.toStringTag, { value: 'ReadableStreamDefaultReader', configurable: true });

  // ----- WritableStream -----
  function WritableStream(sink, strat) {
    if (!(this instanceof WritableStream)) throw new TypeError("Class constructor WritableStream cannot be invoked without 'new'");
    sink = sink || {};
    var sg = strategy(strat, 1);
    var st = { state: 'writable', err: undefined, locked: false, inflight: 0, closeReq: false, closed: deferred() };
    st.closed.promise.catch(function () {});
    def(this, '_s', st);
    var controller = {};
    function fail(e) {
      if (st.state !== 'writable') return;
      st.state = 'errored'; st.err = e; st.closed.reject(e);
    }
    def(controller, 'error', fail);
    var started;
    try { started = Promise.resolve(sink.start ? sink.start(controller) : undefined); } catch (e) { fail(e); started = Promise.resolve(); }
    var tail = started.catch(fail);
    def(this, '_ops', {
      desired: function () { return st.state === 'errored' ? null : st.state === 'closed' ? 0 : sg.hwm - st.inflight; },
      write: function (chunk) {
        if (st.state === 'errored') return Promise.reject(st.err);
        if (st.state !== 'writable' || st.closeReq) return Promise.reject(new TypeError('Cannot write to a closing or closed stream'));
        var size = Number(sg.size(chunk));
        st.inflight += size;
        var p = tail.then(function () {
          if (st.state === 'errored') throw st.err;
          return sink.write ? sink.write(chunk, controller) : undefined;
        }).then(function () { st.inflight -= size; }, function (e) { st.inflight -= size; fail(e); throw e; });
        tail = p.catch(function () {});
        return p;
      },
      close: function () {
        if (st.state === 'errored') return Promise.reject(st.err);
        if (st.state !== 'writable' || st.closeReq) return Promise.reject(new TypeError('Cannot close a closing or closed stream'));
        st.closeReq = true;
        return tail.then(function () {
          if (st.state === 'errored') throw st.err;
          return sink.close ? sink.close() : undefined;
        }).then(function () { st.state = 'closed'; st.closed.resolve(undefined); }, function (e) { fail(e); throw e; });
      },
      abort: function (reason) {
        if (st.state !== 'writable') return st.state === 'errored' ? Promise.reject(st.err) : Promise.resolve();
        fail(reason);
        try { return Promise.resolve(sink.abort ? sink.abort(reason) : undefined).then(function () {}); }
        catch (e) { return Promise.reject(e); }
      }
    });
  }
  Object.defineProperty(WritableStream.prototype, 'locked', { get: function () { return this._s.locked; }, configurable: true });
  def(WritableStream.prototype, 'getWriter', function () {
    if (this._s.locked) throw lockedError('WritableStream');
    var st = this._s, ops = this._ops, writer = Object.create(WritableStreamDefaultWriter.prototype), released = false;
    st.locked = true;
    function live(fn) { return function (a) { return released ? Promise.reject(new TypeError('This writer has been released')) : fn(a); }; }
    def(writer, 'write', live(ops.write));
    def(writer, 'close', live(ops.close));
    def(writer, 'abort', live(ops.abort));
    def(writer, 'releaseLock', function () { released = true; st.locked = false; });
    Object.defineProperty(writer, 'closed', { get: function () { return st.closed.promise; }, configurable: true });
    Object.defineProperty(writer, 'ready', { get: function () { return Promise.resolve(undefined); }, configurable: true });
    Object.defineProperty(writer, 'desiredSize', { get: ops.desired, configurable: true });
    return writer;
  });
  def(WritableStream.prototype, 'abort', function (reason) {
    return this._s.locked ? Promise.reject(lockedError('WritableStream')) : this._ops.abort(reason);
  });
  def(WritableStream.prototype, 'close', function () {
    return this._s.locked ? Promise.reject(lockedError('WritableStream')) : this._ops.close();
  });
  Object.defineProperty(WritableStream.prototype, Symbol.toStringTag, { value: 'WritableStream', configurable: true });
  function WritableStreamDefaultWriter() { throw new TypeError('Illegal constructor'); }
  Object.defineProperty(WritableStreamDefaultWriter.prototype, Symbol.toStringTag, { value: 'WritableStreamDefaultWriter', configurable: true });

  def(globalThis, 'ReadableStream', ReadableStream);
  def(globalThis, 'WritableStream', WritableStream);
})();
