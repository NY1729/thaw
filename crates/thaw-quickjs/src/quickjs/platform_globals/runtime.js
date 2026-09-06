  globalThis.global = globalThis;
  const timers = new Map();
  const normalizeDelay = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) return 0;
    return Math.min(Math.trunc(number), 2147483647);
  };
  const schedule = (callback, delay, repeat, args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('timer callback must be a function');
    }
    const id = nextTimerId++;
    const milliseconds = normalizeDelay(delay);
    timers.set(id, { callback, args, repeat, milliseconds,
                     due: Date.now() + milliseconds });
    return id;
  };
  globalThis.setTimeout = (callback, delay = 0, ...args) =>
    schedule(callback, delay, false, args);
  globalThis.clearTimeout = id => { timers.delete(Number(id)); };
  globalThis.setInterval = (callback, delay = 0, ...args) =>
    schedule(callback, delay, true, args);
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = (callback, ...args) =>
    schedule(callback, 0, false, args);
  globalThis.clearImmediate = globalThis.clearTimeout;
  globalThis.queueMicrotask = callback => {
    if (typeof callback !== 'function') {
      throw new TypeError('microtask callback must be a function');
    }
    Promise.resolve().then(callback);
  };
  const consoleInspect = value => {
    if (typeof value === 'string') return value;
    if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);
    try { const encoded = JSON.stringify(value); return encoded === undefined ? String(value) : encoded; }
    catch (_) { return '[Circular]'; }
  };
  const consoleFormat = (...args) => {
    if (args.length === 0) return '';
    if (typeof args[0] !== 'string') return args.map(consoleInspect).join(' ');
    let index = 1;
    let output = args[0].replace(/%[sdifjoOc%]/g, token => {
      if (token === '%%') return '%';
      if (index >= args.length) return token;
      const value = args[index++];
      if (token === '%s') return String(value);
      if (token === '%d' || token === '%f') return String(Number(value));
      if (token === '%i') return String(parseInt(value, 10));
      if (token === '%c') return '';
      return consoleInspect(value);
    });
    while (index < args.length) output += ' ' + consoleInspect(args[index++]);
    return output;
  };
  globalThis.Console = class Console {
    constructor(stdout, stderr) {
      const options = stdout && stdout.stdout ? stdout : { stdout, stderr };
      this._stdout = options.stdout;
      this._stderr = options.stderr || options.stdout;
      this._counts = new Map();
      this._timers = new Map();
      this._indent = '';
      for (const name of ['log', 'info', 'debug', 'warn', 'error']) {
        this[name] = this[name].bind(this);
      }
    }
    _write(stream, args) {
      const text = this._indent + consoleFormat(...args) + '\n';
      if (typeof stream === 'function') stream(text);
      else if (stream && typeof stream.write === 'function') stream.write(text);
    }
    log(...args) { this._write(this._stdout, args); }
    info(...args) { this.log(...args); }
    debug(...args) { this.log(...args); }
    warn(...args) { this._write(this._stderr, args); }
    error(...args) { this.warn(...args); }
    dir(value, options) { this.log(consoleInspect(value)); }
    dirxml(...args) { this.log(...args); }
    table(value) { this.log(consoleInspect(value)); }
    assert(condition, ...args) {
      if (!condition) this.error('Assertion failed' + (args.length ? ': ' + consoleFormat(...args) : ''));
    }
    count(label = 'default') {
      const name = String(label); const value = (this._counts.get(name) || 0) + 1;
      this._counts.set(name, value); this.log(`${name}: ${value}`);
    }
    countReset(label = 'default') { this._counts.delete(String(label)); }
    time(label = 'default') { this._timers.set(String(label), Date.now()); }
    timeLog(label = 'default', ...args) {
      const name = String(label); const started = this._timers.get(name);
      if (started !== undefined) this.log(`${name}: ${Date.now() - started}ms`, ...args);
    }
    timeEnd(label = 'default') { const name = String(label); this.timeLog(name); this._timers.delete(name); }
    group(...args) { if (args.length) this.log(...args); this._indent += '  '; }
    groupCollapsed(...args) { this.group(...args); }
    groupEnd() { this._indent = this._indent.substring(0, Math.max(0, this._indent.length - 2)); }
    trace(...args) {
      const error = new Error(consoleFormat(...args));
      this.error(`Trace: ${error.message}` + (error.stack ? `\n${error.stack}` : ''));
    }
    clear() {}
    profile() {}
    profileEnd() {}
    timeStamp() {}
  };
  if (typeof globalThis.console === 'undefined') {
    globalThis.console = new Console(globalThis.__thaw_console_stdout,
                                     globalThis.__thaw_console_stderr);
  }
  const nextTick = (callback, ...args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('process.nextTick callback must be a function');
    }
    queueMicrotask(() => callback(...args));
  };
  if (typeof globalThis.process === 'undefined') globalThis.process = {};
  const hostInfo = typeof globalThis.__thaw_os_info === 'function'
    ? JSON.parse(globalThis.__thaw_os_info()) : {};
  const processStart = Date.now();
  let processCwd = typeof globalThis.process.cwd === 'function'
    ? globalThis.process.cwd() : '/';
  const processListeners = new Map();
  const processOn = (event, listener, once = false) => {
    if (typeof listener !== 'function') throw new TypeError('listener must be a function');
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    listeners.push({ listener, once });
    processListeners.set(name, listeners);
    return globalThis.process;
  };
  const processOff = (event, listener) => {
    const name = String(event);
    const listeners = processListeners.get(name) || [];
    processListeners.set(name, listeners.filter(entry => entry.listener !== listener));
    return globalThis.process;
  };
  const processEmit = (event, ...args) => {
    const name = String(event);
    const listeners = (processListeners.get(name) || []).slice();
    for (const entry of listeners) {
      if (entry.once) processOff(name, entry.listener);
      entry.listener(...args);
    }
    return listeners.length !== 0;
  };
  const hrtime = previous => {
    const nanoseconds = BigInt(Date.now() - processStart) * 1000000n;
    let seconds = Number(nanoseconds / 1000000000n);
    let remainder = Number(nanoseconds % 1000000000n);
    if (previous !== undefined) {
      seconds -= Number(previous[0]);
      remainder -= Number(previous[1]);
      if (remainder < 0) { seconds--; remainder += 1000000000; }
    }
    return [seconds, remainder];
  };
  hrtime.bigint = () => BigInt(Date.now() - processStart) * 1000000n;
  Object.assign(globalThis.process, {
    argv: globalThis.process.argv || [],
    env: globalThis.process.env || {},
    platform: globalThis.process.platform || hostInfo.platform || 'linux',
    arch: globalThis.process.arch || hostInfo.arch || '',
    version: globalThis.process.version || '',
    execPath: globalThis.process.execPath || '/usr/bin/node',
    config: globalThis.process.config || { variables: {} },
    versions: Object.assign({ node: '', modules: '', uv: '' },
                            globalThis.process.versions || {}),
    cwd: () => processCwd,
    chdir: directory => {
      const value = String(directory);
      processCwd = value.charAt(0) === '/' ? value : processCwd.replace(/\/$/, '') + '/' + value;
    },
    nextTick,
    uptime: () => (Date.now() - processStart) / 1000,
    hrtime,
    memoryUsage: () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 }),
    cpuUsage: () => ({ user: 0, system: 0 }),
    on: (event, listener) => processOn(event, listener),
    once: (event, listener) => processOn(event, listener, true),
    off: processOff,
    removeListener: processOff,
    emit: processEmit,
    listenerCount: event => (processListeners.get(String(event)) || []).length,
    emitWarning: (warning, options) => {
      const value = warning instanceof Error ? warning : new Error(String(warning));
      value.name = options && options.type ? String(options.type) : 'Warning';
      if (options && options.code) value.code = String(options.code);
      if (!processEmit('warning', value) && globalThis.console
          && typeof globalThis.console.warn === 'function') globalThis.console.warn(value);
    },
    exitCode: globalThis.process.exitCode,
    title: globalThis.process.title || 'thaw'
  });

  if (typeof globalThis.DOMException !== 'function') {
    const legacyCodes = {
      IndexSizeError: 1, HierarchyRequestError: 3, WrongDocumentError: 4,
      InvalidCharacterError: 5, NoModificationAllowedError: 7,
      NotFoundError: 8, NotSupportedError: 9, InUseAttributeError: 10,
      InvalidStateError: 11, SyntaxError: 12, InvalidModificationError: 13,
      NamespaceError: 14, InvalidAccessError: 15, TypeMismatchError: 17,
      SecurityError: 18, NetworkError: 19, AbortError: 20,
      URLMismatchError: 21, QuotaExceededError: 22, TimeoutError: 23,
      InvalidNodeTypeError: 24, DataCloneError: 25
    };
    class DOMException extends Error {
      constructor(message = '', name = 'Error') {
        super(String(message));
        this.name = String(name);
        this.code = legacyCodes[this.name] || 0;
      }
    }
    for (const [name, code] of Object.entries(legacyCodes)) {
      const constant = name.replace(/Error$/, '')
        .replace(/([a-z])([A-Z])/g, '$1_$2').toUpperCase() + '_ERR';
      Object.defineProperty(DOMException, constant, { value: code });
      Object.defineProperty(DOMException.prototype, constant, { value: code });
    }
    globalThis.DOMException = DOMException;
  }
