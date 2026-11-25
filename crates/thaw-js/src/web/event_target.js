  let eventListenerSignalBridge = null;
  // AbortSignal (abort.js) registers the listener `signal` option bridge through this one-shot hook.
  globalThis.__thawSetSignalBridge = bridge => { eventListenerSignalBridge = bridge; delete globalThis.__thawSetSignalBridge; };
  const NativeWeakMap = WeakMap;
  const eventState = new NativeWeakMap();
  const entrySignals = new NativeWeakMap();
  const weakMapGet = NativeWeakMap.prototype.get;
  const weakMapSet = NativeWeakMap.prototype.set;
  const weakMapDelete = NativeWeakMap.prototype.delete;
  const apply = Reflect.apply;
  const eventStateOf = event => apply(weakMapGet, eventState, [event]);
  const listenerSignalOf = entry => apply(weakMapGet, entrySignals, [entry]);
  // ponytail: thaw-js has no DOMException yet; a plain Error with the same name/message stands in.
  const NativeDOMException = typeof DOMException === 'function' ? DOMException : class DOMException extends Error {
    constructor(message = '', name = 'Error') { super(message); this.name = name; }
  };
  // Listener exceptions are uncaught, not dispatch failures: report them asynchronously.
  const reportUncaught = error => {
    if (typeof globalThis.__thaw_report_uncaught === 'function') globalThis.__thaw_report_uncaught(error);
    else setTimeout(() => { throw error; }, 0);
  };
  // Event state is one record per event (never own properties); the public fields are prototype accessors.
  const invalidThis = message => { const error = new TypeError(message); error.code = 'ERR_INVALID_THIS'; return error; };
  const eventRecord = event => {
    const record = eventStateOf(event);
    if (!record) throw invalidThis('Value of "this" must be of type Event');
    return record;
  };
  if (typeof globalThis.Event !== 'function') {
    globalThis.Event = class Event {
      constructor(type, options = {}) {
        apply(weakMapSet, eventState, [this, {
          type: String(type), bubbles: Boolean(options.bubbles), cancelable: Boolean(options.cancelable),
          composed: Boolean(options.composed), timeStamp: typeof performance === 'object' && performance !== null ? performance.now() : Date.now(),
          target: null, dispatching: false, inPassiveListener: false, defaultPrevented: false,
          propagationStopped: false, immediateStopped: false,
        }]);
      }
      preventDefault() {
        const record = eventRecord(this);
        if (record.cancelable && !record.inPassiveListener) record.defaultPrevented = true;
      }
      stopPropagation() { eventRecord(this).propagationStopped = true; }
      stopImmediatePropagation() {
        const record = eventRecord(this);
        record.propagationStopped = true;
        record.immediateStopped = true;
      }
      // Legacy (Node): does nothing while the event is being dispatched.
      initEvent(type, bubbles = false, cancelable = false) {
        const record = eventRecord(this);
        if (arguments.length === 0) {
          const error = new TypeError('The "type" argument must be specified');
          error.code = 'ERR_MISSING_ARGS';
          throw error;
        }
        if (record.dispatching) return;
        record.type = String(type);
        record.bubbles = Boolean(bubbles);
        record.cancelable = Boolean(cancelable);
      }
      composedPath() {
        const record = eventRecord(this);
        return record.dispatching ? [record.target] : [];
      }
    };
    Object.defineProperty(globalThis.Event.prototype, Symbol.toStringTag, { value: 'Event', configurable: true });
    // Node defines the phase constants on the constructor only, not on Event.prototype.
    Object.defineProperties(globalThis.Event, {
      NONE: { value: 0, enumerable: true },
      CAPTURING_PHASE: { value: 1, enumerable: true },
      AT_TARGET: { value: 2, enumerable: true },
      BUBBLING_PHASE: { value: 3, enumerable: true },
    });
    const defineField = (key, read, write, configurable = true) => Object.defineProperty(globalThis.Event.prototype, key, {
      get() { return read(eventRecord(this)); },
      set: write && function (value) { write(eventRecord(this), value); },
      enumerable: true, configurable,
    });
    defineField('type', record => record.type);
    defineField('bubbles', record => record.bubbles);
    defineField('cancelable', record => record.cancelable);
    defineField('composed', record => record.composed);
    defineField('target', record => record.target);
    defineField('srcElement', record => record.target);
    defineField('currentTarget', record => (record.dispatching ? record.target : null));
    defineField('eventPhase', record => (record.dispatching ? 2 : 0));
    defineField('defaultPrevented', record => record.defaultPrevented);
    defineField('timeStamp', record => record.timeStamp);
    defineField('isTrusted', () => false, undefined, false);
    defineField('returnValue', record => !record.cancelable || !record.defaultPrevented,
      (record, value) => { if (!value && record.cancelable && !record.inPassiveListener) record.defaultPrevented = true; });
    defineField('cancelBubble', record => record.propagationStopped,
      (record, value) => { if (value) record.propagationStopped = true; });
  }
  if (typeof globalThis.CustomEvent !== 'function') {
    const customDetails = new NativeWeakMap();
    globalThis.CustomEvent = class CustomEvent extends globalThis.Event {
      constructor(type, options = {}) {
        super(type, options);
        apply(weakMapSet, customDetails, [this, { detail: options.detail === undefined ? null : options.detail }]);
      }
    };
    Object.defineProperty(globalThis.CustomEvent.prototype, 'detail', {
      get() {
        const holder = apply(weakMapGet, customDetails, [this]);
        if (!holder) throw invalidThis('Value of "this" must be of type CustomEvent');
        return holder.detail;
      },
      enumerable: true, configurable: true,
    });
  }
  if (typeof globalThis.EventTarget !== 'function') {
    const mapGet = Map.prototype.get;
    const mapSet = Map.prototype.set;
    const arraySlice = Array.prototype.slice;
    const arrayIncludes = Array.prototype.includes;
    const arrayIndexOf = Array.prototype.indexOf;
    const arraySplice = Array.prototype.splice;
    const arrayPush = Array.prototype.push;
    const arraySome = Array.prototype.some;
    const arrayFind = Array.prototype.find;
    const mapRead = (map, key) => apply(mapGet, map, [key]);
    const mapWrite = (map, key, value) => apply(mapSet, map, [key, value]);
    const liveRecords = (target, type) => mapRead(target.__thawListeners, type) || [];
    const removeEventListenerRecord = (target, type, entry) => {
      const list = liveRecords(target, type);
      const index = apply(arrayIndexOf, list, [entry]);
      if (index === -1) return;
      apply(arraySplice, list, [index, 1]);
      const binding = listenerSignalOf(entry);
      if (binding) {
        apply(weakMapDelete, entrySignals, [entry]);
        eventListenerSignalBridge.remove(binding.state, binding.algorithm);
      }
      if (eventListenerSignalBridge) eventListenerSignalBridge.listenersChanged(target);
    };
    globalThis.EventTarget = class EventTarget {
      constructor() { this.__thawListeners = new Map(); }
      addEventListener(type, callback, options = {}) {
        const callable = callback == null || typeof callback === 'function'
          || typeof callback.handleEvent === 'function';
        if (!callable) {
          throw new TypeError('event listener must be callable');
        }
        const name = String(type);
        let capture = false, once = false, passive = false, signal;
        if (typeof options === 'boolean') capture = options;
        else if (options != null) {
          capture = Boolean(options.capture);
          once = Boolean(options.once);
          passive = Boolean(options.passive);
          signal = options.signal;
        }
        let signalState;
        if (signal != null) {
          signalState = eventListenerSignalBridge && eventListenerSignalBridge.stateOf(signal);
          if (!signalState) throw new TypeError('signal must be an AbortSignal');
          if (signalState.aborted) return;
        }
        if (callback == null) return;
        const listeners = liveRecords(this, name);
        if (apply(arraySome, listeners,
          [entry => entry.callback === callback && entry.capture === capture])) {
          // Node still arms the signal: aborting it removes the equal (callback, capture) listener already registered
          // ponytail: the abort algorithm stays on the signal until it fires (no bookkeeping to cancel it)
          if (signalState) {
            eventListenerSignalBridge.add(signalState, () => {
              const existing = apply(arrayFind, liveRecords(this, name),
                [item => item.callback === callback && item.capture === capture]);
              if (existing) removeEventListenerRecord(this, name, existing);
            });
          }
          return;
        }
        const entry = { callback, capture, once, passive };
        if (!mapRead(this.__thawListeners, name)) mapWrite(this.__thawListeners, name, listeners);
        apply(arrayPush, listeners, [entry]);
        if (signalState) {
          const algorithm = () => removeEventListenerRecord(this, name, entry);
          apply(weakMapSet, entrySignals, [entry, { state: signalState, algorithm }]);
          eventListenerSignalBridge.add(signalState, algorithm);
        }
        if (eventListenerSignalBridge) eventListenerSignalBridge.listenersChanged(this);
      }
      removeEventListener(type, callback, options = false) {
        const name = String(type);
        const capture = typeof options === 'boolean' ? options
          : options == null ? false : Boolean(options.capture);
        const listeners = liveRecords(this, name);
        const entry = apply(arrayFind, listeners,
          [item => item.callback === callback && item.capture === capture]);
        if (entry) removeEventListenerRecord(this, name, entry);
      }
      dispatchEvent(event) {
        if (!(event instanceof Event)) throw new TypeError('expected an Event');
        const state = eventStateOf(event);
        if (state.dispatching) {
          // Node: a plain Error (code ERR_EVENT_RECURSION), not a DOMException
          const recursion = new Error(`The event "${event.type}" is already being dispatched`);
          recursion.code = 'ERR_EVENT_RECURSION';
          throw recursion;
        }
        state.dispatching = true;
        let result;
        let thrown = false;
        let firstError;
        try {
          const type = event.type;
          state.target = this;
          const listeners = apply(arraySlice, liveRecords(this, type), []);
          for (const entry of listeners) {
            // ponytail: scan the live list; use removal flags if dispatch lists grow large.
            // Node keeps the stop-immediately flag across dispatches of one event object, so a stopped event
            // runs no listener when it is dispatched again
            if (state.immediateStopped) break;
            if (!apply(arrayIncludes, liveRecords(this, type), [entry])) continue;
            if (entry.once) removeEventListenerRecord(this, type, entry);
            state.inPassiveListener = entry.passive;
            try {
              if (typeof entry.callback === 'function') apply(entry.callback, this, [event]);
              else apply(entry.callback.handleEvent, entry.callback, [event]);
            } catch (error) { reportUncaught(error); }
            finally { state.inPassiveListener = false; }
            if (state.immediateStopped) break;
          }
          result = !event.defaultPrevented;
        } catch (error) {
          thrown = true;
          firstError = error;
        } finally {
          state.inPassiveListener = false;
          state.dispatching = false;
        }
        if (thrown) throw firstError;
        return result;
      }
    };
    globalThis.EventTarget[Symbol.for('nodejs.event_target')] = true;
  }
