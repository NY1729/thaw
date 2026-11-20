// Worker-side bootstrap (W2b), run by worker.rs in the worker's own realm before the user's source. Reads the spawn
// options from `__thaw_worker_opts` (JSON, set by Rust), builds parentPort on the MessagePort class of messaging.js,
// and keeps the worker alive while parentPort has a referenced 'message' listener.
(function () {
  const opts = JSON.parse(__thaw_worker_opts);
  const decode = wire => __thaw_worker_decode(wire);

  const environmentData = new Map(opts.envData === undefined ? [] : decode(opts.envData));
  const workerData = opts.workerData === undefined ? null : decode(opts.workerData);
  if (opts.argv !== undefined) process.argv = opts.argv;
  if (opts.execArgv !== undefined) process.execArgv = opts.execArgv;
  // a copy-mode worker's own table gets the copy; a SHARE_ENV worker sends no env and keeps the parent's table
  if (opts.env !== undefined) {
    for (const key of Object.keys(process.env)) delete process.env[key];
    Object.assign(process.env, opts.env);
  }

  // Node: the cwd is shared by every thread, so a worker cannot change it
  process.chdir = function chdir() {
    const error = new TypeError('process.chdir() is not supported in workers');
    error.code = 'ERR_WORKER_UNSUPPORTED_OPERATION';
    throw error;
  };

  // stdio (W5): console output is sent to the parent, which captures it or writes it to its own stdout/stderr (the
  // parent decides). Without `stdin: true` the stream is ended from the start (no data).
  const stdin = new __thaw_worker_stream.Readable();
  if (opts.stdin !== true) stdin.__thawEnd();
  Object.defineProperty(process, 'stdin', { get: () => stdin, enumerable: true, configurable: true });

  const parentPort = new MessagePort();
  // Messages to the parent are wire strings; a port in the list moves to the main thread (its routes name it).
  parentPort.postMessage = function postMessage(value, transferOrOptions) {
    __thaw_worker_send(__thaw_worker_encode(value, __thaw_transfer_of(transferOrOptions), __thaw_port_hub()));
    return true;
  };

  // postMessageToThread (W-D), this realm's copy of worker_threads.js's: the parent is the only thread a worker reaches.
  // The worker stays alive while a send is pending, so the ack can arrive (as the sender's promise needs it).
  const pendingSends = new Map();
  let nextAck = 1;
  function threadError(code, message, cause, type = Error) {
    const error = new type(message);
    error.code = code;
    if (cause !== undefined) error.cause = cause;
    return error;
  }
  function settleSend(ack, status, wire) {
    const entry = pendingSends.get(ack);
    if (entry === undefined) return;
    pendingSends.delete(ack);
    clearTimeout(entry.timer);
    if (status === 'ok') entry.resolve(undefined);
    else if (status === 'timeout') entry.reject(threadError('ERR_WORKER_MESSAGING_TIMEOUT', 'Sending a message to another thread timed out'));
    else if (status === 'error') entry.reject(threadError('ERR_WORKER_MESSAGING_ERRORED', 'The destination thread threw an error while processing the message', wire === '' ? undefined : decode(wire)));
    else entry.reject(threadError('ERR_WORKER_MESSAGING_FAILED', 'Cannot find the destination thread or listener'));
    sync();
  }
  function deliverThreadMessage(ack, source, wire, reply) {
    if (process.listenerCount('workerMessage') === 0) return reply(ack, 'failed', '');
    try {
      process.emit('workerMessage', decode(wire), source);
    } catch (error) {
      let cause = '';
      try { cause = __thaw_worker_encode(error); } catch (_) { /* not cloneable: the error has no cause */ }
      return reply(ack, 'error', cause);
    }
    return reply(ack, 'ok', '');
  }
  function postToThread(target, value, transferList, timeout) {
    if (timeout !== undefined && typeof timeout !== 'number') {
      const received = typeof timeout === 'string' ? `type string ('${timeout}')` : `type ${typeof timeout}`;
      return Promise.reject(threadError('ERR_INVALID_ARG_TYPE', `The "timeout" argument must be of type number. Received ${received}`, undefined, TypeError));
    }
    if (timeout !== undefined && !(timeout >= 0)) {
      return Promise.reject(threadError('ERR_OUT_OF_RANGE', `The value of "timeout" is out of range. It must be >= 0. Received ${timeout}`, undefined, RangeError));
    }
    if (target === opts.threadId) return Promise.reject(threadError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot sent a message to the same thread'));
    let transfer;
    try { transfer = __thaw_transfer_of(transferList); } catch (error) { return Promise.reject(error); }
    // only the parent (0) is reachable from a worker. Node detaches the buffers even for another id (FAILED on a later
    // turn), so the wire is encoded with them; that wire is released here, since nothing will decode it.
    const reachable = target === 0;
    let wire;
    try { wire = __thaw_worker_encode(value, transfer, __thaw_port_hub()); } catch (error) { return Promise.reject(error); }
    return new Promise((resolve, reject) => {
      const ack = nextAck++;
      const entry = { resolve, reject, timer: undefined };
      pendingSends.set(ack, entry);
      sync();
      if (timeout !== undefined) entry.timer = setTimeout(() => settleSend(ack, 'timeout', ''), timeout);
      // any other id is FAILED on a later turn than a zero timeout
      if (reachable) __thaw_worker_thread_to_parent(ack, wire);
      else { __thaw_wire_release(wire); setTimeout(() => settleSend(ack, 'failed', ''), 0); }
    });
  }

  // Keep-alive: the worker runs while parentPort has a referenced message listener (close() or unref() releases it)
  // or process.stdin is flowing and not ended, or while a postMessageToThread it sent awaits its ack.
  const sync = () => {
    const listening = parentPort.__thawRefed && !parentPort.__thawClosed
      && ((parentPort.__thawNodeListeners.get('message') || []).length > 0 || parentPort.onmessage !== null);
    __thaw_worker_set_alive(listening || (stdin.flowing && !stdin.endEmitted) || pendingSends.size > 0);
  };
  stdin.onChange = sync;
  for (const name of ['on', 'once', 'off', 'addListener', 'removeListener', 'removeAllListeners', 'ref', 'unref', 'close', 'start']) {
    const original = parentPort[name];
    parentPort[name] = function (...args) {
      const result = original.apply(this, args);
      sync();
      return result;
    };
  }
  const onmessage = Object.getOwnPropertyDescriptor(MessagePort.prototype, 'onmessage');
  Object.defineProperty(parentPort, 'onmessage', {
    get() { return onmessage.get.call(this); },
    set(handler) { onmessage.set.call(this, handler); sync(); },
    enumerable: true, configurable: true,
  });

  // Parent messages arrive as {type, value} records from the Rust poll: queue them on the port like messaging.js does.
  globalThis.__thaw_worker_dispatch = function (json) {
    for (const event of JSON.parse(json)) {
      if (event.type === 'message') {
        const { value, ports } = __thaw_worker_decode_full(event.value);
        parentPort.__thawQueue.push({ data: value, ports });
        parentPort.__thawSchedule();
      } else if (event.type === 'port') {
        __thaw_port_deliver(event.addr, event.value);
      } else if (event.type === 'portClose') {
        __thaw_port_hangup(event.addr);
      } else if (event.type === 'stdin') {
        if (event.value !== '') stdin.__thawPush(__thaw_worker_stream.hexToBytes(event.value));
        if (event.end) stdin.__thawEnd();
        stdin.__thawFlush();
      } else if (event.type === 'thread') {
        deliverThreadMessage(event.ack, event.source, event.value, (ack, status, wire) => __thaw_worker_thread_reply(ack, status, wire));
      } else if (event.type === 'threadAck') {
        settleSend(event.ack, event.status, event.value);
      }
    }
    // queued after the delivery microtasks, so listeners added or removed by a handler are seen
    queueMicrotask(sync);
  };

  const exports = {
    isMainThread: false,
    isInternalThread: false,
    threadId: opts.threadId === undefined ? 1 : opts.threadId,
    threadName: '',
    SHARE_ENV: Symbol.for('nodejs.worker_threads.SHARE_ENV'),
    workerData,
    parentPort,
    postMessageToThread(target, value, transferList, timeout) {
      return postToThread(target, value, transferList, timeout);
    },
    resourceLimits: opts.resourceLimits === undefined ? {} : opts.resourceLimits,
    MessageChannel: globalThis.MessageChannel,
    MessagePort: globalThis.MessagePort,
    BroadcastChannel: globalThis.BroadcastChannel,
    Worker: class Worker {
      constructor() {
        const error = new Error('worker_threads Worker is not supported inside a worker yet');
        error.code = 'ERR_THAW_UNSUPPORTED';
        throw error;
      }
    },
    receiveMessageOnPort(port) {
      // a MessagePort, or a BroadcastChannel (Node reads both through their handle)
      if (!globalThis.__thaw_is_message_port(port) && !(port instanceof globalThis.BroadcastChannel)) {
        const error = new TypeError('The "port" argument must be a MessagePort instance');
        error.code = 'ERR_INVALID_ARG_TYPE';
        throw error;
      }
      // a message another thread already posted to this port is still in the native queue: take it now
      if (port.__thawQueue.length === 0 && typeof __thaw_worker_poll_ports === 'function') {
        const pending = __thaw_worker_poll_ports();
        if (pending !== '[]') globalThis.__thaw_worker_dispatch(pending);
      }
      const record = port.__thawQueue.shift();
      return record ? { message: record.data } : undefined;
    },
    // the same function as the main thread's (worker_threads.js installs it in every realm)
    moveMessagePortToContext(port, context) {
      return globalThis.__thaw_worker_threads_exports.moveMessagePortToContext(port, context);
    },
    getEnvironmentData(key) {
      const value = environmentData.get(key);
      return value === undefined ? undefined : structuredClone(value);
    },
    setEnvironmentData(key, value) {
      if (value === undefined) environmentData.delete(key);
      else environmentData.set(key, structuredClone(value));
    },
  };
  exports.default = exports;
  exports.__esModule = true;
  // The worker realm's own `__thaw_worker_threads_exports` (worker_threads.js) is the main-thread form and is frozen.
  Object.defineProperty(globalThis, '__thaw_worker_side_exports', {
    value: exports, configurable: false, enumerable: false, writable: false,
  });
})();
