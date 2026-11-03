// node:worker_threads, main-thread subset (W1): isMainThread/threadId, MessageChannel and environment data.
// `Worker` is not implemented yet; thaw-cli keeps programs that use it on QuickJS (thaw_js_worker_threads_blocker).
(function () {
  const environmentData = new Map();
  class Worker {
    constructor() {
      const error = new Error('worker_threads Worker is not supported on thaw-js yet');
      error.code = 'ERR_THAW_UNSUPPORTED';
      throw error;
    }
  }
  const workerThreadsExports = {
    isMainThread: true,
    threadId: 0,
    threadName: '',
    workerData: null,
    parentPort: null,
    resourceLimits: {},
    MessageChannel: globalThis.MessageChannel,
    MessagePort: globalThis.MessagePort,
    BroadcastChannel: globalThis.BroadcastChannel,
    Worker,
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
  workerThreadsExports.default = workerThreadsExports;
  workerThreadsExports.__esModule = true;
  Object.defineProperty(globalThis, '__thaw_worker_threads_exports', {
    value: workerThreadsExports, configurable: false, enumerable: false, writable: false,
  });
})();
