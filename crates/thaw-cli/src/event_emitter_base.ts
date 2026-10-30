// Native `node:events` EventEmitter, inlined into a module that subclasses it (see
// `event_emitter_base.rs`). Event names are strings; listeners are stored by identity.
class EventEmitterEntry {
  fn: (...args: any[]) => void;
  once: boolean;
  constructor(fn: (...args: any[]) => void, once: boolean) { this.fn = fn; this.once = once; }
}
class EventEmitter {
  _events: Map<string, EventEmitterEntry[]> = new Map();
  _maxListeners: number = 10;
  _add(event: string, listener: (...args: any[]) => void, prepend: boolean, once: boolean): EventEmitter {
    if (typeof listener !== "function") throw new TypeError('The "listener" argument must be of type function. Received ' + (listener === undefined ? "undefined" : typeof listener));
    if (this._events.has("newListener")) this.emit("newListener", event, listener);
    if (!this._events.has(event)) { const fresh: EventEmitterEntry[] = []; this._events.set(event, fresh); }
    const list = this._events.get(event)!;
    if (prepend) list.unshift(new EventEmitterEntry(listener, once)); else list.push(new EventEmitterEntry(listener, once));
    return this;
  }
  on(event: string, listener: (...args: any[]) => void): EventEmitter { return this._add(event, listener, false, false); }
  addListener(event: string, listener: (...args: any[]) => void): EventEmitter { return this._add(event, listener, false, false); }
  once(event: string, listener: (...args: any[]) => void): EventEmitter { return this._add(event, listener, false, true); }
  prependListener(event: string, listener: (...args: any[]) => void): EventEmitter { return this._add(event, listener, true, false); }
  prependOnceListener(event: string, listener: (...args: any[]) => void): EventEmitter { return this._add(event, listener, true, true); }
  off(event: string, listener: (...args: any[]) => void): EventEmitter { return this.removeListener(event, listener); }
  removeListener(event: string, listener: (...args: any[]) => void): EventEmitter {
    const list = this._events.get(event);
    if (list === undefined) return this;
    for (let i = list.length - 1; i >= 0; i--) {
      if (list[i].fn === listener) {
        list.splice(i, 1);
        if (list.length === 0) this._events.delete(event);
        if (this._events.has("removeListener")) this.emit("removeListener", event, listener);
        break;
      }
    }
    return this;
  }
  _drop(event: string, entry: EventEmitterEntry): void {
    const list = this._events.get(event);
    if (list === undefined) return;
    const at = list.indexOf(entry);
    if (at < 0) return;
    list.splice(at, 1);
    if (list.length === 0) this._events.delete(event);
    if (this._events.has("removeListener")) this.emit("removeListener", event, entry.fn);
  }
  removeAllListeners(event?: string): EventEmitter {
    if (event === undefined) this._events.clear(); else this._events.delete(event);
    return this;
  }
  emit(event: string, ...args: any[]): boolean {
    const list = this._events.get(event);
    if (list === undefined || list.length === 0) {
      if (event === "error") {
        const error = args[0];
        if (error instanceof Error) throw error;
        throw new Error("Unhandled error. (" + (typeof error === "string" ? "'" + error + "'" : String(error)) + ")");
      }
      return false;
    }
    const snapshot = list.slice();
    for (const entry of snapshot) {
      if (entry.once) this._drop(event, entry);
      const f = entry.fn;
      const n = args.length;
      if (n === 0) f(); else if (n === 1) f(args[0]); else if (n === 2) f(args[0], args[1]); else if (n === 3) f(args[0], args[1], args[2]); else if (n === 4) f(args[0], args[1], args[2], args[3]); else f(args[0], args[1], args[2], args[3], args[4]);
    }
    return true;
  }
  listenerCount(event: string): number { const list = this._events.get(event); return list === undefined ? 0 : list.length; }
  listeners(event: string): ((...args: any[]) => void)[] {
    const out: ((...args: any[]) => void)[] = [];
    if (this._events.has(event)) for (const entry of this._events.get(event)!) out.push(entry.fn);
    return out;
  }
  rawListeners(event: string): ((...args: any[]) => void)[] { return this.listeners(event); }
  eventNames(): string[] { const names: string[] = []; for (const name of this._events.keys()) names.push(name); return names; }
  setMaxListeners(n: number): EventEmitter { this._maxListeners = n; return this; }
  getMaxListeners(): number { return this._maxListeners; }
}
