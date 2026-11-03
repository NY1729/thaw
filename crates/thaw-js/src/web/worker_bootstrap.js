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
  if (opts.env !== undefined) {
    for (const key of Object.keys(process.env)) delete process.env[key];
    Object.assign(process.env, opts.env);
  }

  const parentPort = new MessagePort();
  // Messages to the parent are wire strings; transferring ports between threads is not supported yet.
  parentPort.postMessage = function postMessage(value, transferOrOptions = []) {
    const transfer = Array.isArray(transferOrOptions) ? transferOrOptions
      : (transferOrOptions && transferOrOptions.transfer) || [];
    if (Array.from(transfer).length) throw new DOMException('Transferring ports to a worker parent is not supported', 'DataCloneError');
    __thaw_worker_send(__thaw_worker_encode(value));
  };

  // Keep-alive: the worker runs while parentPort has a referenced message listener (close() or unref() releases it).
  const sync = () => {
    const listening = parentPort.__thawRefed && !parentPort.__thawClosed
      && ((parentPort.__thawNodeListeners.get('message') || []).length > 0 || parentPort.onmessage !== null);
    __thaw_worker_set_alive(listening);
  };
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
      if (event.type !== 'message') continue;
      parentPort.__thawQueue.push({ data: decode(event.value), ports: [] });
      parentPort.__thawSchedule();
    }
    // queued after the delivery microtasks, so listeners added or removed by a handler are seen
    queueMicrotask(sync);
  };

  const exports = {
    isMainThread: false,
    threadId: opts.threadId === undefined ? 1 : opts.threadId,
    threadName: '',
    workerData,
    parentPort,
    resourceLimits: {},
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
      if (!globalThis.__thaw_is_message_port(port)) {
        const error = new TypeError('The "port" argument must be a MessagePort instance');
        error.code = 'ERR_INVALID_ARG_TYPE';
        throw error;
      }
      const record = port.__thawQueue.shift();
      return record ? { message: record.data } : undefined;
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
