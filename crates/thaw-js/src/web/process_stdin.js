// process.stdin for the main realm (port of thaw-quickjs runtime.js). Rust reads the terminal on a reader
// thread (sysio.rs) and the event loop hands each batch to globalThis.__thaw_stdin_dispatch.
(function () {
  const stdinListeners = new Map();
  let stdinEncoding;
  let stdinStarted = false;
  let stdinPaused = false;
  let stdinPending = [];
  // User code may replace `JSON.parse`; the event-loop dispatch must not depend on it.
  const intrinsicJsonParse = JSON.parse;
  const drainStdin = () => {
    if (stdinPaused) return;
    while (stdinPending.length && !stdinPaused) {
      const event = stdinPending.shift();
      if (event.type === 'data') {
        const value = Buffer.from(event.value, 'hex');
        const chunk = stdinEncoding ? value.toString(stdinEncoding) : value;
        for (const listener of (stdinListeners.get('data') || []).slice()) listener(chunk);
      } else {
        for (const listener of (stdinListeners.get('end') || []).slice()) listener();
      }
    }
  };
  const startStdin = () => {
    if (stdinStarted) return;
    stdinStarted = true;
    globalThis.__thaw_process_start_stdin();
  };
  const stdin = {
    fd: 0,
    isTTY: globalThis.__thaw_process_is_tty(0) ? true : undefined,
    setEncoding(encoding) { stdinEncoding = String(encoding); return this; },
    on(name, listener) {
      const key = String(name), listeners = stdinListeners.get(key) || [];
      listeners.push(listener);
      stdinListeners.set(key, listeners);
      if (key === 'data' || key === 'end') startStdin();
      return this;
    },
    once(name, listener) {
      const wrapped = value => { this.off(name, wrapped); listener(value); };
      wrapped.listener = listener;
      return this.on(name, wrapped);
    },
    off(name, listener) {
      const key = String(name), listeners = stdinListeners.get(key) || [];
      stdinListeners.set(key, listeners.filter(entry => entry !== listener && entry.listener !== listener));
      return this;
    },
    removeListener(name, listener) { return this.off(name, listener); },
    resume() { stdinPaused = false; startStdin(); process.nextTick(drainStdin); return this; },
    pause() { stdinPaused = true; return this; },
    [Symbol.asyncIterator]() {
      const queue = [];
      let finished = false, wake = null;
      const poke = () => { const w = wake; wake = null; if (w) w(); };
      const onData = chunk => { queue.push(chunk); poke(); };
      const onEnd = () => { finished = true; poke(); };
      const stop = () => { finished = true; this.off('data', onData); this.off('end', onEnd); };
      this.on('data', onData);
      this.on('end', onEnd);
      return {
        async next() {
          while (!queue.length && !finished) await new Promise(resolve => { wake = resolve; });
          if (queue.length) return { value: queue.shift(), done: false };
          stop();
          return { value: undefined, done: true };
        },
        async return() { stop(); queue.length = 0; return { value: undefined, done: true }; },
        [Symbol.asyncIterator]() { return this; }
      };
    },
    ref() { globalThis.__thaw_process_set_stdin_ref(true); return this; },
    unref() { globalThis.__thaw_process_set_stdin_ref(false); return this; }
  };
  if (stdin.isTTY) {
    stdin.isRaw = false;
    stdin.setRawMode = enabled => {
      const raw = Boolean(enabled);
      if (!globalThis.__thaw_process_set_raw_mode(raw)) throw new Error('failed to set stdin raw mode');
      stdin.isRaw = raw;
      return stdin;
    };
  }
  globalThis.__thaw_stdin_dispatch = events => {
    stdinPending.push(...intrinsicJsonParse(events));
    drainStdin();
  };
  process.stdin = stdin;
})();
