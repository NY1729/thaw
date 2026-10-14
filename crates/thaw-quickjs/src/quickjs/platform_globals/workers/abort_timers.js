  if (typeof globalThis.AbortController !== 'function') {
    const SignalState = new WeakMap();
    const dispatchApply = Reflect.apply;
    const NativeWeakRef = WeakRef;
    const weakRefDeref = NativeWeakRef.prototype.deref;
    const weakRef = value => new NativeWeakRef(value);
    const weakDeref = reference => dispatchApply(weakRefDeref, reference, []);
    const NativeFinalizationRegistry = FinalizationRegistry;
    const finalizerRegister = NativeFinalizationRegistry.prototype.register;
    const finalizerUnregister = NativeFinalizationRegistry.prototype.unregister;
    const dependentFinalizer = new NativeFinalizationRegistry(held => {
      for (const [reference, token] of held) {
        const root = weakDeref(reference);
        const rootState = root && stateOf(root);
        if (rootState) rootState.dependents = rootState.dependents.filter(entry => entry.token !== token);
      }
    });
    const weakGet = WeakMap.prototype.get;
    const stateOf = signal => dispatchApply(weakGet, SignalState, [signal]);
    const dispatchAbortEvent = EventTarget.prototype.dispatchEvent;
    const addEventListener = EventTarget.prototype.addEventListener;
    const removeEventListener = EventTarget.prototype.removeEventListener;
    const weakSet = WeakMap.prototype.set;
    const mapGet = Map.prototype.get;
    const arrayIndexOf = Array.prototype.indexOf;
    const arraySplice = Array.prototype.splice;
    const arraySlice = Array.prototype.slice;
    const arrayPush = Array.prototype.push;
    const rootsOf = state => {
      if (!state.dependent) return [state.signal];
      const roots = [];
      const seen = new Set();
      for (const reference of state.sources) {
        const root = weakDeref(reference);
        if (root && !seen.has(root)) { seen.add(root); roots.push(root); }
      }
      return roots;
    };
    const pruneDependents = state => {
      const live = [];
      for (const entry of state.dependents) {
        if (entry.strong || weakDeref(entry.reference)) live.push(entry);
      }
      state.dependents = live;
      return live;
    };
    const updateRetention = (signal, state) => {
      const retained = state.dependent && !state.aborted && state.observers > 0;
      for (const reference of state.sources) {
        const root = weakDeref(reference);
        const rootState = root && stateOf(root);
        if (!rootState) continue;
        for (const entry of rootState.dependents) {
          if (weakDeref(entry.reference) === signal) entry.strong = retained ? signal : null;
        }
      }
    };
    const reconcileObservers = (signal, state) => {
      try {
        const listeners = dispatchApply(mapGet, signal.__thawListeners, ['abort']);
        state.observers = (listeners ? listeners.length : 0)
          + (typeof state.onabortValue === 'function' ? 1 : 0)
          + state.abortAlgorithms.length;
        updateRetention(signal, state);
      } catch (_) {
        // Observer bookkeeping must not replace an EventTarget operation's result or throw.
      }
    };
    const addAbortAlgorithm = (state, algorithm) => {
      dispatchApply(arrayPush, state.abortAlgorithms, [algorithm]);
      reconcileObservers(state.signal, state);
    };
    const removeAbortAlgorithm = (state, algorithm) => {
      const index = dispatchApply(arrayIndexOf, state.abortAlgorithms, [algorithm]);
      if (index === -1) return;
      dispatchApply(arraySplice, state.abortAlgorithms, [index, 1]);
      reconcileObservers(state.signal, state);
    };
    const runAbortAlgorithms = state => {
      const algorithms = dispatchApply(arraySlice, state.abortAlgorithms, []);
      state.abortAlgorithms = [];
      reconcileObservers(state.signal, state);
      for (const algorithm of algorithms) dispatchApply(algorithm, undefined, []);
    };
    eventListenerSignalBridge = {
      stateOf,
      add: addAbortAlgorithm,
      remove: removeAbortAlgorithm,
      listenersChanged(target) {
        const state = stateOf(target);
        if (state) reconcileObservers(target, state);
      },
    };
    const detachSources = (signal, state) => {
      if (!state.dependent) return;
      for (const reference of state.sources) {
        const root = weakDeref(reference);
        const rootState = root && stateOf(root);
        if (!rootState) continue;
        rootState.dependents = rootState.dependents.filter(entry =>
          weakDeref(entry.reference) !== signal);
      }
      state.sources = [];
    };
    const fireAbortEvent = signal => {
      let thrown = false;
      let firstError;
      const event = new Event('abort');
      try {
        const onabort = signal.onabort;
        if (typeof onabort === 'function') dispatchApply(onabort, signal, [event]);
      } catch (error) {
        // An event-handler exception is an uncaught exception, not an abort() failure.
        globalThis.__thaw_report_uncaught(error);
      }
      try {
        dispatchApply(dispatchAbortEvent, signal, [event]);
      } catch (error) {
        if (!thrown) { thrown = true; firstError = error; }
      }
      if (thrown) throw firstError;
    };
    const abortSignal = (signal, reason) => {
      const state = stateOf(signal);
      if (!state) throw new TypeError('Illegal invocation');
      if (state.aborted) return;
      const actualReason = reason === undefined
        ? new DOMException('This operation was aborted', 'AbortError') : reason;
      const dependentSignals = [];
      for (const entry of pruneDependents(state)) {
        const dependent = weakDeref(entry.reference);
        const dependentState = dependent && stateOf(dependent);
        if (dependentState && !dependentState.aborted) dependentSignals.push(dependent);
      }

      // Publish every final state and detach the graph before invoking user code.
      state.aborted = true;
      state.reason = actualReason;
      for (const dependent of dependentSignals) {
        const dependentState = stateOf(dependent);
        if (!dependentState.aborted) {
          dependentState.aborted = true;
          dependentState.reason = actualReason;
        }
      }
      for (const dependent of dependentSignals) detachSources(dependent, stateOf(dependent));
      detachSources(signal, state);
      state.dependents = [];
      if (state.finalizerToken) {
        dispatchApply(finalizerUnregister, dependentFinalizer, [state.finalizerToken]);
        state.finalizerToken = null;
      }
      for (const dependent of dependentSignals) {
        const dependentState = stateOf(dependent);
        if (dependentState.finalizerToken) {
          dispatchApply(finalizerUnregister, dependentFinalizer, [dependentState.finalizerToken]);
          dependentState.finalizerToken = null;
        }
      }

      let thrown = false;
      let firstError;
      for (const target of [signal, ...dependentSignals]) {
        try {
          runAbortAlgorithms(stateOf(target));
          fireAbortEvent(target);
        }
        catch (error) { if (!thrown) { thrown = true; firstError = error; } }
      }
      if (thrown) throw firstError;
    };
    const timeoutError = () => new DOMException('The operation timed out', 'TimeoutError');

    class AbortSignal extends EventTarget {
      constructor() {
        super();
        weakSet.call(SignalState, this, {
          signal: this, aborted: false, reason: undefined,
          dependent: false, sources: [], dependents: [], finalizerToken: null,
          observers: 0, onabortValue: null, abortAlgorithms: [],
        });
      }
      get onabort() {
        const state = stateOf(this);
        if (!state) throw new TypeError('Illegal invocation');
        return state.onabortValue;
      }
      set onabort(value) {
        const state = stateOf(this);
        if (!state) throw new TypeError('Illegal invocation');
        state.onabortValue = value;
        reconcileObservers(this, state);
      }
      addEventListener(type, callback, options) {
        const state = stateOf(this);
        if (!state) return dispatchApply(addEventListener, this, arguments);
        try { return dispatchApply(addEventListener, this, arguments); }
        finally { reconcileObservers(this, state); }
      }
      removeEventListener(type, callback, options) {
        const state = stateOf(this);
        if (!state) return dispatchApply(removeEventListener, this, arguments);
        try { return dispatchApply(removeEventListener, this, arguments); }
        finally { reconcileObservers(this, state); }
      }
      get aborted() {
        const state = stateOf(this);
        if (!state) throw new TypeError('Illegal invocation');
        return state.aborted;
      }
      get reason() {
        const state = stateOf(this);
        if (!state) throw new TypeError('Illegal invocation');
        return state.reason;
      }
      throwIfAborted() {
        const state = stateOf(this);
        if (!state) throw new TypeError('Illegal invocation');
        if (state.aborted) throw state.reason;
      }
      __thawAbort(reason) { return abortSignal(this, reason); }
      static abort(reason) {
        const signal = new AbortSignal();
        abortSignal(signal, reason);
        return signal;
      }
      static timeout(milliseconds) {
        if (arguments.length === 0) throw new TypeError('1 argument required');
        const number = +milliseconds;
        if (!Number.isFinite(number)) throw new TypeError('milliseconds must be a finite integer');
        const integer = Math.trunc(number);
        const delay = integer === 0 ? 0 : integer;
        if (delay < 0 || delay > 9007199254740991) {
          throw new TypeError('milliseconds is outside the supported range');
        }
        const signal = new AbortSignal();
        schedule(() => abortSignal(signal, timeoutError()), delay, false, [], true,
          'setTimeout', false, true);
        return signal;
      }
      static any(signals) {
        const inputs = [...signals];
        const inputStates = [];
        for (const signal of inputs) {
          const state = stateOf(signal);
          if (!state) throw new TypeError('AbortSignal.any expects AbortSignal values');
          inputStates.push(state);
        }
        let alreadyAborted = -1;
        for (let index = 0; index < inputStates.length; index++) {
          if (inputStates[index].aborted) { alreadyAborted = index; break; }
        }
        const combined = new AbortSignal();
        const combinedState = stateOf(combined);
        combinedState.dependent = true;
        if (alreadyAborted !== -1) {
          combinedState.aborted = true;
          combinedState.reason = inputStates[alreadyAborted].reason;
          return combined;
        }
        const roots = [];
        const seen = new Set();
        for (const state of inputStates) {
          for (const root of rootsOf(state)) {
            if (!seen.has(root)) { seen.add(root); roots.push(root); }
          }
        }
        const token = roots.length ? {} : null;
        const cleanup = [];
        combinedState.finalizerToken = token;
        for (const root of roots) {
          const rootState = stateOf(root);
          const entryToken = {};
          const entry = { reference: weakRef(combined), token: entryToken, strong: null };
          rootState.dependents.push(entry);
          const rootReference = weakRef(root);
          combinedState.sources.push(rootReference);
          cleanup.push([rootReference, entryToken]);
        }
        if (token) dispatchApply(finalizerRegister, dependentFinalizer,
          [combined, cleanup, token]);
        return combined;
      }
    }
    class AbortController {
      constructor() { this.signal = new AbortSignal(); }
      abort(reason) { abortSignal(this.signal, reason); }
    }
    globalThis.AbortSignal = AbortSignal;
    globalThis.AbortController = AbortController;
  }
  globalThis.__thaw_next_timer_delay = () => {
    if (![...timers.values()].some(timer => timer.refed && !timer.clock)) return -1;
    let due = Infinity;
    for (const timer of timers.values()) if (!timer.clock) due = Math.min(due, timer.due);
    return due === Infinity ? -1 : Math.max(0, due - nativeDateNow());
  };
  // Delay to the next timer of any kind, ref'd or not (-1 when none): a blocking wait on a pending promise
  // may still be resolved by an unref'd timer (e.g. fs.watch/watchFile with `persistent: false`).
  globalThis.__thaw_next_any_timer_delay = () => {
    let due = Infinity;
    for (const timer of timers.values()) if (!timer.clock) due = Math.min(due, timer.due);
    return due === Infinity ? -1 : Math.max(0, due - nativeDateNow());
  };
  globalThis.__thaw_run_due_timers = () => {
    const now = nativeDateNow();
    const due = [...timers.entries()]
      .filter(([, timer]) => !timer.clock && timer.due <= now)
      .sort((a, b) => a[1].due - b[1].due || a[0] - b[0]);
    for (const [id, timer] of due) {
      if (!timers.has(id) || timer.due > nativeDateNow()) continue;
      if (timer.repeat) timer.due = nativeDateNow() + timer.milliseconds;
      else timers.delete(id);
      invokeTimer(timer);
    }
    return due.length;
  };
