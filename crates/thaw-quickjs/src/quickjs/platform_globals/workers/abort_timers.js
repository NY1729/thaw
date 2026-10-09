  if (typeof globalThis.AbortController !== 'function') {
    const abortError = message => {
      return new DOMException(message || 'This operation was aborted', 'AbortError');
    };
    const timeoutError = () => {
      return new DOMException('The operation timed out', 'TimeoutError');
    };
    class AbortSignal extends EventTarget {
      constructor() {
        super();
        this.aborted = false;
        this.reason = undefined;
        this.onabort = null;
      }
      throwIfAborted() {
        if (this.aborted) throw this.reason;
      }
      __thawAbort(reason) {
        if (this.aborted) return;
        this.aborted = true;
        this.reason = reason === undefined ? abortError() : reason;
        const event = new Event('abort');
        if (typeof this.onabort === 'function') this.onabort.call(this, event);
        this.dispatchEvent(event);
      }
      static abort(reason) {
        const signal = new AbortSignal();
        signal.__thawAbort(reason);
        return signal;
      }
      static timeout(milliseconds) {
        const signal = new AbortSignal();
        setTimeout(() => signal.__thawAbort(timeoutError()), milliseconds);
        return signal;
      }
      static any(signals) {
        const combined = new AbortSignal();
        const subscriptions = [];
        const sources = [...signals];
        for (const signal of sources) {
          if (!signal || typeof signal.addEventListener !== 'function') {
            throw new TypeError('AbortSignal.any expects AbortSignal values');
          }
        }
        const finish = signal => {
          if (combined.aborted) return;
          combined.__thawAbort(signal.reason);
          for (const [source, listener] of subscriptions) {
            source.removeEventListener('abort', listener);
          }
        };
        for (const signal of sources) {
          if (signal.aborted) {
            finish(signal);
            break;
          }
          const listener = () => finish(signal);
          subscriptions.push([signal, listener]);
          signal.addEventListener('abort', listener, { once: true });
        }
        return combined;
      }
    }
    class AbortController {
      constructor() { this.signal = new AbortSignal(); }
      abort(reason) { this.signal.__thawAbort(reason); }
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
      if (!timers.has(id)) continue;
      if (timer.repeat) timer.due = nativeDateNow() + timer.milliseconds;
      else timers.delete(id);
      invokeTimer(timer);
    }
    return due.length;
  };
