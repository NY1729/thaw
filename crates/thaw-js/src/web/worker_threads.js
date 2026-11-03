// node:worker_threads, main-thread side (W1, W3, W5): isMainThread/threadId, MessageChannel, environment data, and the
// `Worker` class over the `__thaw_worker_*` natives of worker.rs (one OS thread and one Interp per worker). The
// stdio streams (W5) are the private shim of worker_stream.js. `resourceLimits`, SharedArrayBuffer and SHARE_ENV are
// not implemented yet; thaw-cli keeps programs that use them on QuickJS (thaw_js_worker_threads_blocker).
(function () {
  const environmentData = new Map();
  let nextThreadId = 1;
  // handle (worker.rs table id) -> Worker, for events; Worker -> state, for everything else
  const byHandle = new Map();
  const states = new WeakMap();

  // Small listener set (no `node:events` dependency): on/once/off/emit and friends, as EventEmitter behaves for these events.
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
    prependListener(name, listener) { this.__thawList(name).unshift({ listener, once: false }); return this; }
    once(name, listener) { this.__thawList(name).push({ listener, once: true }); return this; }
    off(name, listener) {
      const list = this.__thawList(name);
      const index = list.findIndex(entry => entry.listener === listener);
      if (index >= 0) list.splice(index, 1);
      return this;
    }
    removeListener(name, listener) { return this.off(name, listener); }
    removeAllListeners(name) {
      if (name === undefined) this.__thawListeners.clear();
      else this.__thawListeners.delete(name);
      return this;
    }
    listenerCount(name) { return this.__thawList(name).length; }
    emit(name, ...args) {
      const list = this.__thawList(name);
      if (list.length === 0) {
        // an unhandled 'error' is the uncaught exception (EventEmitter throws the error it was given)
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

  const decode = wire => __thaw_worker_decode(wire);
  const encode = value => __thaw_worker_encode(value);
  const { Readable, Writable, hexToBytes } = __thaw_worker_stream;

  class Worker extends Emitter {
    constructor(source, options = {}) {
      super();
      if (!options || !options.eval) {
        const error = new Error('Thaw Worker currently requires { eval: true }');
        error.code = 'ERR_WORKER_PATH';
        throw error;
      }
      const threadId = nextThreadId++;
      const opts = {
        threadId,
        // a non-cloneable workerData throws DataCloneError here, synchronously, as in Node
        workerData: encode(options.workerData),
        // the worker starts with a snapshot of the parent's environment data
        envData: encode(Array.from(environmentData)),
        env: {},
      };
      const env = options.env === undefined ? process.env : options.env;
      for (const key of Object.keys(env)) opts.env[key] = String(env[key]);
      if (options.argv !== undefined) opts.argv = Array.from(options.argv, String);
      if (options.execArgv !== undefined) opts.execArgv = Array.from(options.execArgv, String);
      // stdio (W5): the worker's console output reaches stdout/stderr only when asked for; stdin exists only when asked
      opts.stdin = options.stdin === true;
      opts.stdout = options.stdout === true;
      opts.stderr = options.stderr === true;
      const handle = __thaw_worker_spawn(String(source), JSON.stringify(opts));
      let resolveExit;
      const state = {
        handle,
        exited: false,
        exitCode: undefined,
        refd: true,
        exitPromise: new Promise(resolve => { resolveExit = resolve; }),
        resolveExit,
      };
      states.set(this, state);
      byHandle.set(handle, this);
      Object.defineProperty(this, 'threadId', { value: threadId, enumerable: true });
      this.stdin = options.stdin === true ? new Writable((hex, end) => __thaw_worker_stdin(handle, hex, end)) : null;
      // ponytail: always Readables (as in Node); they get data only when stdout/stderr were asked for
      this.stdout = new Readable();
      this.stderr = new Readable();
    }

    postMessage(value, transferList) {
      if (transferList !== undefined && Array.from(transferList).length) {
        throw new DOMException('transfer lists are not supported on thaw-js yet', 'DataCloneError');
      }
      const state = states.get(this);
      if (state.exited) return;
      __thaw_worker_post(state.handle, encode(value));
    }

    terminate() {
      const state = states.get(this);
      if (!state.exited) __thaw_worker_terminate(state.handle);
      return state.exitPromise;
    }

    ref() {
      const state = states.get(this);
      state.refd = true;
      if (!state.exited) __thaw_worker_ref(state.handle, true);
      return this;
    }

    unref() {
      const state = states.get(this);
      state.refd = false;
      if (!state.exited) __thaw_worker_ref(state.handle, false);
      return this;
    }

    hasRef() { return states.get(this).refd; }
  }

  // Events from the Rust poll (worker.rs `poll`), delivered by the event loop: [{handle, type, ...}].
  globalThis.__thaw_worker_dispatch = function (json) {
    for (const event of JSON.parse(json)) {
      const worker = byHandle.get(event.handle);
      if (worker === undefined) continue;
      const state = states.get(worker);
      if (event.type === 'online') {
        worker.emit('online');
      } else if (event.type === 'message') {
        let value;
        try {
          value = decode(event.value);
        } catch (error) {
          worker.emit('messageerror', error);
          continue;
        }
        worker.emit('message', value);
      } else if (event.type === 'error') {
        worker.emit('error', decode(event.value));
      } else if (event.type === 'stdout' || event.type === 'stderr') {
        const stream = event.type === 'stdout' ? worker.stdout : worker.stderr;
        stream.__thawPush(hexToBytes(event.value));
        stream.__thawFlush();
      } else if (event.type === 'exit') {
        state.exited = true;
        state.exitCode = event.code;
        byHandle.delete(event.handle);
        // stdio ends before 'exit', as in Node (the events share one channel, so the data came first)
        for (const stream of [worker.stdout, worker.stderr]) {
          stream.__thawEnd();
          stream.__thawFlush();
        }
        worker.emit('exit', event.code);
        state.resolveExit(event.code);
      }
    }
  };

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
