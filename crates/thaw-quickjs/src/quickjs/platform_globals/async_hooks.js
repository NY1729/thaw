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
  function AsyncResource(type, options) {
    if (!(this instanceof AsyncResource)) return new AsyncResource(type, options);
    this.type = String(type);
    this._asyncId = thawNextAsyncId++;
    this._triggerAsyncId = options && options.triggerAsyncId !== undefined
      ? Number(options.triggerAsyncId) : thawCurrentAsyncId;
    thawApply(thawWeakSet, thawResourceFrames, [this, thawFrameGet()]);
    this._destroyed = false;
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
    try { return thawWithFrame(thawApply(thawWeakGet, thawResourceFrames, [this]), callback, thisArg, args); }
    finally {
      thawCurrentAsyncId = previousId;
      thawCurrentTriggerId = previousTrigger;
      thawCurrentResource = previousResource;
    }
  };
  AsyncResource.prototype.emitDestroy = function() { this._destroyed = true; return this; };
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
    createHook(callbacks) {
      return { enable() { return this; }, disable() { return this; }, callbacks: callbacks || {} };
    },
    executionAsyncId() { return thawCurrentAsyncId; },
    triggerAsyncId() { return thawCurrentTriggerId; },
    executionAsyncResource() { return thawCurrentResource; },
  };
  thawAsyncHooksExports.default = thawAsyncHooksExports;
  thawAsyncHooksExports.__esModule = true;
  Object.defineProperty(globalThis, '__thaw_async_hooks_exports', {
    value: thawAsyncHooksExports, configurable: false, enumerable: false, writable: false,
  });
