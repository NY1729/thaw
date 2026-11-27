// Native `node:events` EventEmitter, inlined into a module that subclasses it (see
// `event_emitter_base.rs`). Same storage as Node: `_events[name]` is the listener itself for one
// listener and an array for several; `once` listeners are wrapper functions carrying `.listener`.
// (Keys go through String(): a method parameter typed `string` is not accepted as an assignment key otherwise.)
function __thawEventsCoded(error: any, code: string): any {
  Object.defineProperty(error, "code", { value: code, writable: true, enumerable: true, configurable: true });
  return error;
}
function __thawEventsReceived(value: any): string {
  if (value === undefined || value === null) return " Received " + String(value);
  if (typeof value === "function") return " Received function " + String(value.name);
  if (typeof value === "object") return " Received an instance of " + String(value.constructor !== undefined && value.constructor.name ? value.constructor.name : "Object");
  let text: string = typeof value === "string" ? "'" + value + "'" : String(value);
  if (text.length > 28) text = text.slice(0, 25) + "...";
  return " Received type " + typeof value + " (" + text + ")";
}
class EventEmitter {
  _events: any = Object.create(null);
  _eventsCount: number = 0;
  _maxListeners: any = undefined;
  _add(type: string, listener: any, prepend: boolean): EventEmitter {
    if (typeof listener !== "function") {
      throw __thawEventsCoded(new TypeError('The "listener" argument must be of type function.' + __thawEventsReceived(listener)), "ERR_INVALID_ARG_TYPE");
    }
    let events: any = this._events;
    let existing: any = undefined;
    if (events === undefined) { events = Object.create(null); this._events = events; this._eventsCount = 0; }
    else {
      if (events.newListener !== undefined) { this.emit("newListener", type, listener.listener ? listener.listener : listener); events = this._events; }
      existing = events[String(type)];
    }
    if (existing === undefined) { events[String(type)] = listener; this._eventsCount = this._eventsCount + 1; }
    else {
      if (typeof existing === "function") { existing = prepend ? [listener, existing] : [existing, listener]; events[String(type)] = existing; }
      else if (prepend) existing.unshift(listener); else existing.push(listener);
      const m: number = this._maxListeners === undefined ? 10 : this._maxListeners;
      if (m > 0 && existing.length > m && !existing.warned) {
        existing.warned = true;
        const warning: any = new Error("Possible EventEmitter memory leak detected. " + existing.length + " " + type + " listeners added to [" + (this as any).constructor.name + "]. MaxListeners is " + m + ". Use emitter.setMaxListeners() to increase limit");
        warning.name = "MaxListenersExceededWarning"; warning.emitter = this; warning.type = type; warning.count = existing.length;
        (process as any).emitWarning(warning);
      }
    }
    return this;
  }
  _wrapOnce(type: string, listener: any): any {
    const state: any = { fired: false, wrapFn: undefined, target: this, type: type, listener: listener };
    const wrapped: any = (...args: any[]) => {
      if (!state.fired) {
        state.target.removeListener(state.type, state.wrapFn);
        state.fired = true;
        return state.listener.apply(state.target, args);
      }
    };
    wrapped.listener = listener;
    state.wrapFn = wrapped;
    return wrapped;
  }
  on(event: string, listener: any): EventEmitter { return this._add(event, listener, false); }
  once(event: string, listener: any): EventEmitter { return this._add(event, this._wrapOnce(event, listener), false); }
  prependListener(event: string, listener: any): EventEmitter { return this._add(event, listener, true); }
  prependOnceListener(event: string, listener: any): EventEmitter { return this._add(event, this._wrapOnce(event, listener), true); }
  addListener(event: string, listener: any): EventEmitter { return this._add(event, listener, false); }
  off(event: string, listener: any): EventEmitter { return this.removeListener(event, listener); }
  removeListener(event: string, listener: any): EventEmitter {
    if (typeof listener !== "function") {
      throw __thawEventsCoded(new TypeError('The "listener" argument must be of type function.' + __thawEventsReceived(listener)), "ERR_INVALID_ARG_TYPE");
    }
    const events: any = this._events;
    if (events === undefined) return this;
    const list: any = events[String(event)];
    if (list === undefined) return this;
    if (list === listener || list.listener === listener) {
      this._eventsCount = this._eventsCount - 1;
      if (this._eventsCount === 0) this._events = Object.create(null);
      else {
        delete events[String(event)];
        if (events.removeListener) this.emit("removeListener", event, list.listener || listener);
      }
    } else if (typeof list !== "function") {
      let position = -1;
      for (let i = list.length - 1; i >= 0; i--) {
        if (list[i] === listener || list[i].listener === listener) { position = i; break; }
      }
      if (position < 0) return this;
      list.splice(position, 1);
      if (list.length === 1) events[String(event)] = list[0];
      if (events.removeListener !== undefined) this.emit("removeListener", event, listener);
    }
    return this;
  }
  removeAllListeners(event?: string): EventEmitter {
    const events: any = this._events;
    if (events === undefined) return this;
    if (events.removeListener === undefined) {
      if (event === undefined) { this._events = Object.create(null); this._eventsCount = 0; }
      else if (events[String(event)] !== undefined) {
        this._eventsCount = this._eventsCount - 1;
        if (this._eventsCount === 0) this._events = Object.create(null); else delete events[String(event)];
      }
      return this;
    }
    if (event === undefined) {
      const names: string[] = Object.keys(events);
      for (const key of names) {
        if (key === "removeListener") continue;
        this.removeAllListeners(key);
      }
      this.removeAllListeners("removeListener");
      this._events = Object.create(null);
      this._eventsCount = 0;
      return this;
    }
    const listeners: any = events[String(event)];
    if (typeof listeners === "function") this.removeListener(event, listeners);
    else if (listeners !== undefined) {
      for (let i = listeners.length - 1; i >= 0; i--) this.removeListener(event, listeners[i]);
    }
    return this;
  }
  emit(event: string, ...args: any[]): boolean {
    const events: any = this._events;
    if (events === undefined) {
      if (event !== "error") return false;
    }
    let doError = event === "error";
    if (events !== undefined) doError = doError && events.error === undefined;
    if (doError) {
      const error = args[0];
      if (error instanceof Error) throw error;
      const unhandled: any = new Error("Unhandled error. (" + (typeof error === "string" ? "'" + error + "'" : String(error)) + ")");
      __thawEventsCoded(unhandled, "ERR_UNHANDLED_ERROR");
      unhandled.context = error;
      throw unhandled;
    }
    const handler: any = events[String(event)];
    if (handler === undefined) return false;
    if (typeof handler === "function") handler.apply(this, args);
    else {
      const snapshot: any[] = handler.slice();
      for (let i = 0; i < snapshot.length; i++) snapshot[i].apply(this, args);
    }
    return true;
  }
  listenerCount(event: string, listener?: any): number {
    const events: any = this._events;
    if (events === undefined) return 0;
    const handler: any = events[String(event)];
    if (handler === undefined) return 0;
    if (typeof handler === "function") {
      if (listener != null) return listener === handler || listener === handler.listener ? 1 : 0;
      return 1;
    }
    if (listener != null) {
      let matching = 0;
      for (let i = 0; i < handler.length; i++) if (handler[i] === listener || handler[i].listener === listener) matching++;
      return matching;
    }
    return handler.length;
  }
  listeners(event: string): any[] {
    const events: any = this._events;
    if (events === undefined) return [];
    const handler: any = events[String(event)];
    if (handler === undefined) return [];
    if (typeof handler === "function") return [handler.listener || handler];
    return handler.map((l: any) => l.listener || l);
  }
  rawListeners(event: string): any[] {
    const events: any = this._events;
    if (events === undefined) return [];
    const handler: any = events[String(event)];
    if (handler === undefined) return [];
    if (typeof handler === "function") return [handler];
    return handler.slice();
  }
  eventNames(): string[] { return this._eventsCount > 0 ? Object.keys(this._events) : []; }
  setMaxListeners(n: number): EventEmitter {
    if (typeof n !== "number") throw __thawEventsCoded(new TypeError('The "setMaxListeners" argument must be of type number.' + __thawEventsReceived(n)), "ERR_INVALID_ARG_TYPE");
    if (n < 0 || n !== n) throw __thawEventsCoded(new RangeError('The value of "setMaxListeners" is out of range. It must be >= 0. Received ' + String(n)), "ERR_OUT_OF_RANGE");
    this._maxListeners = n;
    return this;
  }
  getMaxListeners(): number { return this._maxListeners === undefined ? 10 : this._maxListeners; }
}


