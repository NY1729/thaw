  // The native bridge is captured before the temporary bootstrap property is
  // removed. Only the public async_hooks exports remain reachable by bundles.
  const thawWeakMap = WeakMap;
  const thawSet = Set;
  const thawAsyncHooksRegistrations = new thawSet();
  globalThis.__thawAsyncLocalStorages = thawAsyncHooksRegistrations;
  const thawStorageStates = new thawWeakMap();
  const thawResourceFrames = new thawWeakMap();
  const thawWeakGet = thawWeakMap.prototype.get;
  const thawWeakSet = thawWeakMap.prototype.set;
  const thawSetHas = thawSet.prototype.has;
  const thawSetAdd = thawSet.prototype.add;
  const thawSetDelete = thawSet.prototype.delete;
  let thawNextStorageId = 1;
  let thawNextAsyncId = 2;
  let thawCurrentAsyncId = 1;
  let thawCurrentTriggerId = 0;
  let thawCurrentResource = {};
  const thawApply = Reflect.apply;
  const thawStateOf = storage => thawApply(thawWeakGet, thawStorageStates, [storage]);
  const thawFrameGet = thawAsyncContext.get;
  const thawFrameSwap = thawAsyncContext.swap;
  // Frames retain an ID and an empty token, never their storage or store.
  // A generation's WeakMap owns values only while frame tokens stay live.
  const thawGeneration = () => ({ active: true, values: new thawWeakMap() });
  const thawFrame = (parent, id, generation, token) => ({ parent, id, generation, token });
  const thawStoreFrame = (parent, state, value) => {
    const token = {};
    thawApply(thawWeakSet, state.generation.values, [token, value]);
    return thawFrame(parent, state.id, state.generation, token);
  };
  const thawWithFrame = (frame, callback, thisArg, args) => {
    const previous = thawFrameSwap(frame);
    try { return thawApply(callback, thisArg, args); }
    finally { thawFrameSwap(previous); }
  };
  // Keep only the newest active cell for each storage ID. Disabled generation
  // maps are emptied even if a saved or engine-restored frame still exists.
  const thawCompactFrame = (frame, replacingId) => {
    const retained = [];
    const seen = new thawSet();
    thawApply(thawSetAdd, seen, [replacingId]);
    for (let current = frame; current; current = current.parent) {
      if (!current.generation.active || thawApply(thawSetHas, seen, [current.id])) continue;
      thawApply(thawSetAdd, seen, [current.id]);
      retained.push(current);
    }
    let parent = undefined;
    for (let index = retained.length - 1; index >= 0; index--) {
      const current = retained[index];
      parent = thawFrame(parent, current.id, current.generation, current.token);
    }
    return parent;
  };
  const thawFindStore = (state, frame) => {
    for (let current = frame; current; current = current.parent) {
      if (current.id === state.id && current.generation === state.generation
        && current.generation.active)
        return thawApply(thawWeakGet, current.generation.values, [current.token]);
    }
    return state.defaultValue;
  };
  function AsyncLocalStorage(options) {
    if (!(this instanceof AsyncLocalStorage)) return new AsyncLocalStorage(options);
    thawApply(thawWeakSet, thawStorageStates,
      [this, { id: thawNextStorageId++, defaultValue: options && options.defaultValue,
        generation: thawGeneration(), enabled: true }]);
    this.enabled = true;
    this.name = options && options.name || '';
    thawApply(thawSetAdd, thawAsyncHooksRegistrations, [this]);
  }
  AsyncLocalStorage.prototype.disable = function() {
    const state = thawStateOf(this);
    state.generation.active = false;
    // Empty before JS finally or the pending-job executor restores an old
    // frame: its token then cannot retain or recover the former store.
    state.generation.values = new thawWeakMap();
    state.generation = thawGeneration();
    state.enabled = false;
    this.enabled = false;
    thawApply(thawSetDelete, thawAsyncHooksRegistrations, [this]);
  };
  AsyncLocalStorage.prototype.getStore = function() {
    const state = thawStateOf(this);
    return state.enabled ? thawFindStore(state, thawFrameGet()) : undefined;
  };
  AsyncLocalStorage.prototype.enterWith = function(store) {
    const state = thawStateOf(this);
    state.enabled = true;
    this.enabled = true;
    thawApply(thawSetAdd, thawAsyncHooksRegistrations, [this]);
    thawFrameSwap(thawStoreFrame(thawCompactFrame(thawFrameGet(), state.id), state, store));
  };
  AsyncLocalStorage.prototype.run = function(store, callback) {
    const state = thawStateOf(this);
    state.enabled = true;
    this.enabled = true;
    thawApply(thawSetAdd, thawAsyncHooksRegistrations, [this]);
    const frame = thawStoreFrame(thawFrameGet(), state, store);
    return thawWithFrame(frame, callback, null, Array.prototype.slice.call(arguments, 2));
  };
  AsyncLocalStorage.prototype.exit = function(callback) {
    const state = thawStateOf(this);
    const frame = thawStoreFrame(thawFrameGet(), state, undefined);
    return thawWithFrame(frame, callback, null, Array.prototype.slice.call(arguments, 1));
  };
  AsyncLocalStorage.bind = function(callback) {
    const frame = thawFrameGet();
    return function() { return thawWithFrame(frame, callback, this, Array.prototype.slice.call(arguments)); };
  };
  AsyncLocalStorage.snapshot = function() {
    const frame = thawFrameGet();
    return function(callback) {
      return thawWithFrame(frame, callback, null, Array.prototype.slice.call(arguments, 1));
    };
  };
  // ---- async_hooks.createHook ----
  // Enabled hooks and the dispatcher the runtime's timers / nextTick queue / promise hook feed. Events are
  // emitted synchronously; an event raised while a hook callback runs is not re-emitted (as in Node, a hook
  // must not depend on its own resources).
  const thawEnabledHooks = new thawSet();
  const thawLiveResources = new Map();
  const thawScopeStack = [];
  const thawPromiseIds = new thawWeakMap();
  let thawEmitting = false;
  const thawHookNames = ['init', 'before', 'after', 'destroy', 'promiseResolve'];
  const thawEmit = (name, a, b, c, d) => {
    if (thawEmitting || thawEnabledHooks.size === 0) return;
    thawEmitting = true;
    try {
      for (const hook of Array.from(thawEnabledHooks)) {
        const callback = hook[name];
        if (typeof callback === 'function') thawApply(callback, hook.callbacks, [a, b, c, d]);
      }
    } catch (error) {
      globalThis.__thaw_report_uncaught(error);
    } finally {
      thawEmitting = false;
    }
  };
  const thawEnterScope = (id, trigger, resource) => {
    thawScopeStack.push([thawCurrentAsyncId, thawCurrentTriggerId, thawCurrentResource]);
    thawCurrentAsyncId = id;
    thawCurrentTriggerId = trigger;
    thawCurrentResource = resource;
  };
  const thawLeaveScope = () => {
    const previous = thawScopeStack.pop();
    thawCurrentAsyncId = previous[0];
    thawCurrentTriggerId = previous[1];
    thawCurrentResource = previous[2];
  };
  // Like Node, `destroy` is delivered after the current callback finished (never from inside it): ids are
  // queued and flushed by one unreferenced zero-delay timer.
  let thawDestroyQueue = [];
  const thawQueueDestroy = id => {
    if (thawEnabledHooks.size === 0) return;
    if (thawDestroyQueue.length === 0) {
      globalThis.__thaw_set_timeout_ref(() => {
        const ids = thawDestroyQueue;
        thawDestroyQueue = [];
        for (const queued of ids) thawEmit('destroy', queued);
      }, 0, false);
    }
    thawDestroyQueue.push(id);
  };
  const thawBus = {
    enabled: false,
    init(type, resource, triggerAsyncId) {
      const id = thawNextAsyncId++;
      const trigger = triggerAsyncId === undefined ? thawCurrentAsyncId : triggerAsyncId;
      thawLiveResources.set(id, { trigger, resource });
      thawEmit('init', id, type, trigger, resource);
      return id;
    },
    before(id) {
      const live = thawLiveResources.get(id);
      thawEnterScope(id, live ? live.trigger : 0, live ? live.resource : {});
      thawEmit('before', id);
    },
    after(id) {
      thawEmit('after', id);
      thawLeaveScope();
    },
    destroy(id) {
      if (!thawLiveResources.delete(id)) return;
      thawQueueDestroy(id);
    },
  };
  Object.defineProperty(globalThis, '__thaw_async_hook_bus', {
    value: thawBus, configurable: false, enumerable: false, writable: false,
  });
  // QuickJS reports promise creation (with the parent of a `then` result) and resolution.
  Object.defineProperty(globalThis, '__thaw_async_hooks_promise', {
    value: function(kind, promise, parent) {
      if (kind === 0) {
        const id = thawNextAsyncId++;
        const known = parent === undefined ? undefined : thawApply(thawWeakGet, thawPromiseIds, [parent]);
        thawApply(thawWeakSet, thawPromiseIds, [promise, { id }]);
        thawEmit('init', id, 'PROMISE', known ? known.id : thawCurrentAsyncId, promise);
      } else if (kind === 3) {
        const known = thawApply(thawWeakGet, thawPromiseIds, [promise]);
        if (known) thawEmit('promiseResolve', known.id);
      }
    },
    configurable: false, enumerable: false, writable: false,
  });
  const thawRefreshBus = () => {
    const enabled = thawEnabledHooks.size > 0;
    if (enabled === thawBus.enabled) return;
    thawBus.enabled = enabled;
    globalThis.__thaw_async_hooks_promise_hook(enabled);
  };
  function AsyncHook(callbacks) {
    if (callbacks === null || typeof callbacks !== 'object') {
      const error = new TypeError('The "hook" argument must be of type object.');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    const state = { callbacks };
    for (const name of thawHookNames) {
      if (callbacks[name] !== undefined && typeof callbacks[name] !== 'function') {
        const error = new TypeError(`hook.${name} must be a function`);
        error.code = 'ERR_ASYNC_CALLBACK';
        throw error;
      }
      state[name] = callbacks[name];
    }
    this._state = state;
  }
  AsyncHook.prototype.enable = function() {
    thawApply(thawSetAdd, thawEnabledHooks, [this._state]);
    thawRefreshBus();
    return this;
  };
  AsyncHook.prototype.disable = function() {
    thawApply(thawSetDelete, thawEnabledHooks, [this._state]);
    thawRefreshBus();
    return this;
  };
  function AsyncResource(type, options) {
    if (!(this instanceof AsyncResource)) return new AsyncResource(type, options);
    if (typeof type !== 'string') {
      const error = new TypeError('The "type" argument must be of type string.');
      error.code = 'ERR_INVALID_ARG_TYPE';
      throw error;
    }
    this.type = String(type);
    this._asyncId = thawNextAsyncId++;
    this._triggerAsyncId = options && options.triggerAsyncId !== undefined
      ? Number(options.triggerAsyncId) : thawCurrentAsyncId;
    thawApply(thawWeakSet, thawResourceFrames, [this, thawFrameGet()]);
    this._destroyed = false;
    thawEmit('init', this._asyncId, this.type, this._triggerAsyncId, this);
  }
  AsyncResource.prototype.asyncId = function() { return this._asyncId; };
  AsyncResource.prototype.triggerAsyncId = function() { return this._triggerAsyncId; };
  AsyncResource.prototype.runInAsyncScope = function(callback, thisArg) {
    const args = Array.prototype.slice.call(arguments, 2);
    const previousId = thawCurrentAsyncId;
    const previousTrigger = thawCurrentTriggerId;
    const previousResource = thawCurrentResource;
    thawCurrentAsyncId = this._asyncId;
    thawCurrentTriggerId = this._triggerAsyncId;
    thawCurrentResource = this;
    thawEmit('before', this._asyncId);
    try { return thawWithFrame(thawApply(thawWeakGet, thawResourceFrames, [this]), callback, thisArg, args); }
    finally {
      thawEmit('after', this._asyncId);
      thawCurrentAsyncId = previousId;
      thawCurrentTriggerId = previousTrigger;
      thawCurrentResource = previousResource;
    }
  };
  AsyncResource.prototype.emitDestroy = function() {
    if (!this._destroyed) {
      this._destroyed = true;
      thawQueueDestroy(this._asyncId);
    }
    return this;
  };
  AsyncResource.prototype.bind = function(callback, thisArg) {
    const resource = this;
    return function() {
      return resource.runInAsyncScope(callback, thisArg === undefined ? this : thisArg,
        ...arguments);
    };
  };
  AsyncResource.bind = function(callback, type, thisArg) {
    return new AsyncResource(type || callback.name || 'bound-anonymous-fn').bind(callback, thisArg);
  };
  const thawAsyncHooksExports = {
    AsyncLocalStorage: AsyncLocalStorage,
    AsyncResource: AsyncResource,
    createHook(callbacks) { return new AsyncHook(callbacks); },
    executionAsyncId() { return thawCurrentAsyncId; },
    triggerAsyncId() { return thawCurrentTriggerId; },
    executionAsyncResource() { return thawCurrentResource; },
  };
  thawAsyncHooksExports.default = thawAsyncHooksExports;
  thawAsyncHooksExports.__esModule = true;
  Object.defineProperty(globalThis, '__thaw_async_hooks_exports', {
    value: thawAsyncHooksExports, configurable: false, enumerable: false, writable: false,
  });
