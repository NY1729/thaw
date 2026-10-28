(function () {
  if (typeof globalThis.AbortController === 'function') return;
  // ponytail: no weak dependent-signal graph (thaw-js WeakRefs are strong anyway); AbortSignal.any keeps its sources alive.
  const states = new WeakMap();
  const apply = Reflect.apply;
  const stateOf = s => states.get(s);
  const need = s => { const st = states.get(s); if (!st) throw new TypeError('Illegal invocation'); return st; };
  const dispatch = EventTarget.prototype.dispatchEvent;
  const doAbort = (signal, reason) => {
    const st = need(signal);
    if (st.aborted) return;
    st.aborted = true;
    st.reason = reason === undefined ? new DOMException('This operation was aborted', 'AbortError') : reason;
    const dependents = st.dependents.filter(d => !states.get(d).aborted); st.dependents = [];
    for (const d of dependents) { const ds = states.get(d); ds.aborted = true; ds.reason = st.reason; }
    let thrown = false, first;
    for (const target of [signal, ...dependents]) {
      try {
        const algs = states.get(target).algorithms; states.get(target).algorithms = [];
        for (const a of algs) a();
        const event = new Event('abort');
        apply(dispatch, target, [event]);
      } catch (e) { if (!thrown) { thrown = true; first = e; } }
    }
    if (thrown) throw first;
  };
  const Illegal = () => { throw new TypeError('Illegal constructor'); };
  let creating = false;
  class AbortSignal extends EventTarget {
    constructor() {
      if (!creating) Illegal();
      super();
      states.set(this, { aborted: false, reason: undefined, onabort: null, algorithms: [], dependents: [] });
    }
    get onabort() { return need(this).onabort; }
    set onabort(v) {
      const st = need(this);
      st.onabort = typeof v === 'function' ? v : null;
      if (st.onabort && !st.bound) {
        st.bound = true;
        this.addEventListener('abort', event => { if (st.onabort) apply(st.onabort, this, [event]); });
      }
    }
    get aborted() { return need(this).aborted; }
    get reason() { return need(this).reason; }
    throwIfAborted() { const st = need(this); if (st.aborted) throw st.reason; }
    static abort(reason) { const s = make(); doAbort(s, reason); return s; }
    static timeout(ms) {
      if (arguments.length === 0) throw new TypeError('1 argument required');
      const n = +ms;
      if (!Number.isFinite(n)) throw new TypeError('milliseconds must be a finite integer');
      const delay = Math.trunc(n);
      if (delay < 0 || delay > 9007199254740991) throw new RangeError('The value of "milliseconds" is out of range. It must be a non-negative integer.');
      const s = make();
      const t = setTimeout(() => doAbort(s, new DOMException('The operation was aborted due to timeout', 'TimeoutError')), delay);
      if (t && typeof t.unref === 'function') t.unref();
      return s;
    }
    static any(signals) {
      const inputs = [...signals];
      for (const s of inputs) need(s);
      const combined = make();
      const cs = states.get(combined);
      const hit = inputs.find(s => states.get(s).aborted);
      if (hit) { cs.aborted = true; cs.reason = states.get(hit).reason; return combined; }
      for (const s of inputs) states.get(s).dependents.push(combined);
      return combined;
    }
  }
  const make = () => { creating = true; try { return new AbortSignal(); } finally { creating = false; } };
  Object.defineProperty(AbortSignal.prototype, Symbol.toStringTag, { value: 'AbortSignal', configurable: true });
  class AbortController {
    #signal = make();
    get signal() { return this.#signal; }
    abort(reason) { doAbort(this.#signal, reason); }
  }
  Object.defineProperty(AbortController.prototype, Symbol.toStringTag, { value: 'AbortController', configurable: true });
  globalThis.__thawSetSignalBridge({
    stateOf,
    add(st, algorithm) { st.algorithms.push(algorithm); },
    remove(st, algorithm) { const i = st.algorithms.indexOf(algorithm); if (i !== -1) st.algorithms.splice(i, 1); },
    listenersChanged() {},
  });
  globalThis.AbortSignal = AbortSignal;
  globalThis.AbortController = AbortController;
})();
