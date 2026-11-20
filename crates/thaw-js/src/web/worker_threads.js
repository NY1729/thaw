// node:worker_threads, main-thread side (W1, W3, W5): isMainThread/threadId, MessageChannel, environment data, and the
// `Worker` class over the `__thaw_worker_*` natives of worker.rs (one OS thread and one Interp per worker). The
// stdio streams (W5) are the private shim of worker_stream.js. `resourceLimits.maxOldGenerationSizeMb` is enforced by
// heap_limit (an estimate, not V8's heap). `env: SHARE_ENV` shares this realm's environment table (process_env.rs).
(function () {
  const SHARE_ENV = Symbol.for('nodejs.worker_threads.SHARE_ENV');
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
  // `dest` is the port token of the realm the wire goes to (a port in it moves there); see worker.rs
  const encode = (value, transfer, dest = 0) => __thaw_worker_encode(value, transfer, dest);
  const { Readable, Writable, hexToBytes } = __thaw_worker_stream;

  // postMessageToThread (W-D). The message runs the destination's 'workerMessage' listeners on its own loop, and the
  // ack that comes back settles the sender's promise. Ack ids are per realm: pendingSends is this realm's table.
  // Only the main thread (0) and its workers are reachable: a worker's send to another worker is FAILED.
  const liveThreads = new Map(); // threadId -> worker state, while the worker runs
  const pendingSends = new Map();
  let nextAck = 1;
  function threadError(code, message, cause, type = Error) {
    const error = new type(message);
    error.code = code;
    if (cause !== undefined) error.cause = cause;
    return error;
  }
  // The sender's side of an ack: 'ok', 'failed' (no thread or no listener), 'error' (the listener threw), 'timeout'.
  function settleSend(ack, status, wire) {
    const entry = pendingSends.get(ack);
    if (entry === undefined) return;
    pendingSends.delete(ack);
    clearTimeout(entry.timer);
    if (status === 'ok') entry.resolve(undefined);
    else if (status === 'timeout') entry.reject(threadError('ERR_WORKER_MESSAGING_TIMEOUT', 'Sending a message to another thread timed out'));
    else if (status === 'error') entry.reject(threadError('ERR_WORKER_MESSAGING_ERRORED', 'The destination thread threw an error while processing the message', wire === '' ? undefined : decode(wire)));
    else entry.reject(threadError('ERR_WORKER_MESSAGING_FAILED', 'Cannot find the destination thread or listener'));
  }
  // The receiver's side: run this realm's 'workerMessage' listeners, then answer through reply(ack, status, wire).
  function deliverThreadMessage(ack, source, wire, reply) {
    if (globalThis.process.listenerCount('workerMessage') === 0) return reply(ack, 'failed', '');
    try {
      globalThis.process.emit('workerMessage', decode(wire), source);
    } catch (error) {
      let cause = '';
      try { cause = encode(error); } catch (_) { /* not cloneable: the error has no cause */ }
      return reply(ack, 'error', cause);
    }
    return reply(ack, 'ok', '');
  }
  // route(target) is the channel to a thread: deliver(ack, wire), or undefined when there is no such thread.
  function postToThread(selfId, route, target, value, transferList, timeout) {
    if (timeout !== undefined && typeof timeout !== 'number') {
      const received = typeof timeout === 'string' ? `type string ('${timeout}')` : `type ${typeof timeout}`;
      return Promise.reject(threadError('ERR_INVALID_ARG_TYPE', `The "timeout" argument must be of type number. Received ${received}`, undefined, TypeError));
    }
    if (timeout !== undefined && !(timeout >= 0)) {
      return Promise.reject(threadError('ERR_OUT_OF_RANGE', `The value of "timeout" is out of range. It must be >= 0. Received ${timeout}`, undefined, RangeError));
    }
    if (target === selfId) return Promise.reject(threadError('ERR_WORKER_MESSAGING_SAME_THREAD', 'Cannot sent a message to the same thread'));
    let transfer;
    try { transfer = __thaw_transfer_of(transferList); } catch (error) { return Promise.reject(error); }
    const deliver = route(target);
    // an unknown thread keeps its buffers (Node moves them only for a thread that receives the message)
    let wire;
    try { wire = encode(value, deliver === undefined ? [] : transfer, deliver === undefined ? 0 : liveThreads.get(target).token); } catch (error) { return Promise.reject(error); }
    return new Promise((resolve, reject) => {
      const ack = nextAck++;
      const entry = { resolve, reject, timer: undefined };
      pendingSends.set(ack, entry);
      if (timeout !== undefined) entry.timer = setTimeout(() => settleSend(ack, 'timeout', ''), timeout);
      // an unknown thread fails on a later turn than a zero timeout, so timeout 0 still times out first (as in Node)
      if (deliver === undefined) setTimeout(() => settleSend(ack, 'failed', ''), 0);
      else deliver(ack, wire);
    });
  }
  const mainRoute = target => {
    const state = liveThreads.get(target);
    return state === undefined ? undefined : (ack, wire) => __thaw_worker_thread_send(state.handle, 0, ack, wire);
  };

  class Worker extends Emitter {
    constructor(source, options = {}) {
      super();
      // a file worker runs its file as a CommonJS main module (the disk loader resolves relative paths)
      let code = String(source);
      // the file a non-eval worker runs (a bundled module's virtual path runs from its bundle, worker.rs)
      let file;
      if (!options || !options.eval) {
        if (!code.startsWith('/') && !code.startsWith('./') && !code.startsWith('../')) {
          const error = new Error('The worker script or module filename must be an absolute path or a relative path starting with ./ or ../. Received "' + code + '"');
          error.code = 'ERR_WORKER_PATH';
          throw error;
        }
        file = code;
        code = 'globalThis.__thaw_run_main_file(' + JSON.stringify(code) + ')';
      }
      const threadId = nextThreadId++;
      // the worker's port token: ports that move to it are routed by this (worker.rs), unlike the Node threadId
      const token = __thaw_port_address();
      // transferList (Node: any iterable, here by Array.from, so a non-iterable value is ignored): its buffers move to the
      // worker whether or not workerData names them. A bad list or a non-cloneable workerData throws here, as in Node.
      const transfer = options.transferList === undefined || options.transferList === null ? [] : __thaw_transfer_check(Array.from(options.transferList));
      const opts = {
        threadId,
        workerData: encode(options.workerData, transfer, token),
        // the worker starts with a snapshot of the parent's environment data
        envData: encode(Array.from(environmentData)),
      };
      // SHARE_ENV: the worker reads and writes this realm's environment table (no copy is sent); otherwise the worker
      // starts with a copy of `env` (default: process.env)
      const shareEnv = options.env === SHARE_ENV;
      if (!shareEnv) {
        opts.env = {};
        const env = options.env === undefined ? process.env : options.env;
        for (const key of Object.keys(env)) opts.env[key] = String(env[key]);
      }
      if (options.argv !== undefined) opts.argv = Array.from(options.argv, String);
      if (options.execArgv !== undefined) opts.execArgv = Array.from(options.execArgv, String);
      // stdio (W5): the worker's console output reaches stdout/stderr only when asked for; stdin exists only when asked
      opts.stdin = options.stdin === true;
      opts.stdout = options.stdout === true;
      opts.stderr = options.stderr === true;
      // resourceLimits as Node 22 reports them right after construction: unset numbers are -1, stackSizeMb is 4
      const limits = options.resourceLimits;
      const limit = key => (limits && typeof limits[key] === 'number' ? limits[key] : -1);
      const resourceLimits = {
        maxYoungGenerationSizeMb: limit('maxYoungGenerationSizeMb'),
        maxOldGenerationSizeMb: limit('maxOldGenerationSizeMb'),
        codeRangeSizeMb: limit('codeRangeSizeMb'),
        stackSizeMb: limits && typeof limits.stackSizeMb === 'number' ? limits.stackSizeMb : 4,
      };
      opts.resourceLimits = resourceLimits;
      // heap limit (MiB, -1 = none) and stack (MiB): the natives' numeric arguments
      const handle = __thaw_worker_spawn(code, JSON.stringify(opts), resourceLimits.maxOldGenerationSizeMb, resourceLimits.stackSizeMb, shareEnv, token, file);
      let resolveExit;
      const state = {
        handle,
        threadId,
        token,
        exited: false,
        exitCode: undefined,
        refd: true,
        resourceLimits,
        exitPromise: new Promise(resolve => { resolveExit = resolve; }),
        resolveExit,
      };
      states.set(this, state);
      liveThreads.set(threadId, state);
      byHandle.set(handle, this);
      this.stdin = options.stdin === true ? new Writable((hex, end) => __thaw_worker_stdin(handle, hex, end)) : null;
      // ponytail: always Readables (as in Node); they get data only when stdout/stderr were asked for
      this.stdout = new Readable();
      this.stderr = new Readable();
    }

    postMessage(value, transferOrOptions) {
      const state = states.get(this);
      // after the worker exited Node neither checks the list nor moves its buffers
      if (state.exited) return;
      __thaw_worker_post(state.handle, encode(value, __thaw_transfer_of(transferOrOptions), state.token));
    }

    terminate() {
      const state = states.get(this);
      if (!state.exited) __thaw_worker_terminate(state.handle);
      return state.exitPromise;
    }

    // Node: a separate method that awaits terminate() (resolves undefined, not the exit code)
    async [Symbol.asyncDispose]() {
      await this.terminate();
    }

    ref() {
      const state = states.get(this);
      state.refd = true;
      if (!state.exited) __thaw_worker_ref(state.handle, true);
    }

    unref() {
      const state = states.get(this);
      state.refd = false;
      if (!state.exited) __thaw_worker_ref(state.handle, false);
    }

    hasRef() { return states.get(this).refd; }

    // Node: a prototype getter, -1 once the worker has exited
    get threadId() {
      const state = states.get(this);
      return state.exited ? -1 : state.threadId;
    }

    get resourceLimits() { return states.get(this).resourceLimits; }
  }

  // Events from the Rust poll (worker.rs `poll`), delivered by the event loop: [{handle, type, ...}].
  globalThis.__thaw_worker_dispatch = function (json) {
    for (const event of JSON.parse(json)) {
      // port events have no handle: a port end of this thread, routed by the main thread (messaging.js)
      if (event.type === 'port') { __thaw_port_deliver(event.addr, event.value); continue; }
      if (event.type === 'portClose') { __thaw_port_hangup(event.addr); continue; }
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
      } else if (event.type === 'thread') {
        deliverThreadMessage(event.ack, state.threadId, event.value, (ack, status, wire) => __thaw_worker_thread_ack(state.handle, ack, status, wire));
      } else if (event.type === 'threadAck') {
        settleSend(event.ack, event.status, event.value);
      } else if (event.type === 'stdout' || event.type === 'stderr') {
        const stream = event.type === 'stdout' ? worker.stdout : worker.stderr;
        stream.__thawPush(hexToBytes(event.value));
        stream.__thawFlush();
      } else if (event.type === 'exit') {
        state.exited = true;
        state.exitCode = event.code;
        byHandle.delete(event.handle);
        liveThreads.delete(state.threadId);
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

  // The clone marks (extras_late structuredClone checks them; it installs after this file, so read them per call).
  // Node ignores a non-object argument.
  const isObjectLike = value => (typeof value === 'object' && value !== null) || typeof value === 'function';

  // moveMessagePortToContext (Node 22.23.1, probes in thaw-tmp/we1-scratch). The result is a new wrapper over the same
  // port (nothing is transferred, so the original stays usable). Delivered messages are re-prototyped into the context's
  // realm. ponytail: only plain objects, arrays, Map, Set, Date and Error are re-prototyped, and the event is a plain
  // object (not a MessageEvent, with `ports` as received).
  // vm_context.js marks each context (VM_CONTEXT) and publishes its prototypes on its first run (VM_PROTOTYPES). String
  // keys: symbols are per realm. ponytail: a context never run before a message arrives keeps the host prototypes.
  const VM_CONTEXT = '__thaw_vm_context';
  const VM_PROTOTYPES = '__thaw_vm_prototypes';
  const movedPorts = new WeakMap(); // wrapper -> { port, context, listening }
  const hostPrototypes = [Object.prototype, Array.prototype, Map.prototype, Set.prototype, Date.prototype, Error.prototype];
  const adoptInto = (value, protos, seen) => {
    if (typeof value !== 'object' || value === null || seen.has(value)) return;
    seen.add(value);
    const kind = hostPrototypes.indexOf(Object.getPrototypeOf(value));
    if (kind < 0) return;
    if (kind === 2) for (const [key, entry] of value) { adoptInto(key, protos, seen); adoptInto(entry, protos, seen); }
    else if (kind === 3) for (const entry of value) adoptInto(entry, protos, seen);
    else if (kind !== 4) for (const entry of Object.values(value)) adoptInto(entry, protos, seen);
    Object.setPrototypeOf(value, protos[kind]);
  };
  class HandleWrap {
    close() { return movedPorts.get(this).port.close(); }
    hasRef() { return movedPorts.get(this).port.hasRef(); }
    ref() { movedPorts.get(this).port.ref(); }
    unref() { movedPorts.get(this).port.unref(); }
  }
  class MessagePort extends HandleWrap {
    postMessage(value, transferList) { return movedPorts.get(this).port.postMessage(value, transferList); }
    start() {
      const state = movedPorts.get(this);
      if (!state.listening) {
        state.listening = true;
        state.port.addEventListener('message', event => {
          adoptInto(event.data, state.context[VM_PROTOTYPES] || hostPrototypes, new Set());
          if (typeof this.onmessage === 'function') this.onmessage.call(this, { data: event.data, target: this, type: 'message', ports: event.ports });
        });
      }
      state.port.start();
    }
  }
  function moveMessagePortToContext(port, context) {
    if (!globalThis.__thaw_is_message_port(port)) {
      const error = new TypeError('The "port" argument must be a MessagePort instance');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    if (!isObjectLike(context) || context[VM_CONTEXT] !== true) {
      const error = new TypeError('Invalid context argument');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    if (port.__thawClosed) {
      const error = new Error('Cannot send data on closed MessagePort');
      error.code = 'ERR_CLOSED_MESSAGE_PORT';
      throw error;
    }
    const moved = new MessagePort();
    movedPorts.set(moved, { port, context, listening: false });
    return moved;
  }

  const workerThreadsExports = {
    isMainThread: true,
    isInternalThread: false,
    threadId: 0,
    threadName: '',
    postMessageToThread(target, value, transferList, timeout) {
      return postToThread(0, mainRoute, target, value, transferList, timeout);
    },
    workerData: null,
    parentPort: null,
    SHARE_ENV,
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
      // a message another thread already posted to this port is still in the native queue: take it now
      if (port.__thawQueue.length === 0 && typeof __thaw_worker_poll_ports === 'function') {
        const pending = __thaw_worker_poll_ports();
        if (pending !== '[]') globalThis.__thaw_worker_dispatch(pending);
      }
      const record = port.__thawQueue.shift();
      return record ? { message: record.data } : undefined;
    },
    moveMessagePortToContext,
    markAsUntransferable(value) { if (isObjectLike(value)) globalThis.__thaw_clone_marks.untransferable.add(value); },
    isMarkedAsUntransferable(value) { return isObjectLike(value) && globalThis.__thaw_clone_marks.untransferable.has(value); },
    markAsUncloneable(value) { if (isObjectLike(value)) globalThis.__thaw_clone_marks.uncloneable.add(value); },
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
