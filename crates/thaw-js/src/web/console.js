// `Console` (Node 22 shape): `new Console(stdout[, stderr[, ignoreErrors]])` or
// `new Console({ stdout, stderr, ignoreErrors, groupIndentation })`. Text is formatted by the native
// `__thaw_console_format` (the global console.log formatter), so output matches Node's rules.
// Known gaps: `console.table` on Map/Set, `dir` options, `inspectOptions`, the stderr warnings Node
// emits for bad countReset/timeEnd labels (they go to process, not the stream), and calling without `new`.
// ponytail: table cells use the default inspect depth; Node shrinks objects with more than 2 keys.
const kStateOf = new WeakMap();
const fmt = (...args) => __thaw_console_format(...args);
const TABLE = {
  left: '│ ', middle: ' │ ', right: ' │', tl: '┌', tm: '┬', tr: '┐',
  lm: '├', mm: '┼', rm: '┤', bl: '└', bm: '┴', br: '┘', h: '─',
};
const hasOwn = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
function writableError(name) {
  const e = new TypeError(`Console expects a writable stream instance for ${name}`);
  e.code = 'ERR_CONSOLE_WRITABLE_STREAM';
  return e;
}
function formatTime(ms) {
  if (ms >= 1000) return `${(ms / 1000).toFixed(3)}s`;
  return `${Number(ms.toFixed(3))}ms`;
}
const now = () => (globalThis.performance ? performance.now() : Date.now());
function renderTable(head, columns) {
  const rows = [];
  const widths = head.map(h => h.length);
  const longest = Math.max(0, ...columns.map(c => c.length));
  for (let i = 0; i < head.length; i++) {
    const column = columns[i];
    for (let j = 0; j < longest; j++) {
      if (rows[j] === undefined) rows[j] = [];
      const value = (rows[j][i] = hasOwn(column, j) ? column[j] : '');
      widths[i] = Math.max(widths[i] || 0, String(value).length);
    }
  }
  const divider = widths.map(w => TABLE.h.repeat(w + 2));
  const renderRow = row => TABLE.left +
    row.map((cell, i) => cell + ' '.repeat(widths[i] - cell.length)).join(TABLE.middle) + TABLE.right;
  let out = TABLE.tl + divider.join(TABLE.tm) + TABLE.tr + '\n' + renderRow(head) + '\n' +
    TABLE.lm + divider.join(TABLE.mm) + TABLE.rm + '\n';
  for (const row of rows) out += renderRow(row) + '\n';
  return out + TABLE.bl + divider.join(TABLE.bm) + TABLE.br;
}
function emit(self, stream, text) {
  const st = kStateOf.get(self);
  if (st.indent) text = st.indent + text.replace(/\n/g, '\n' + st.indent);
  text += '\n';
  if (!st.ignoreErrors) {
    stream.write(text);
    return;
  }
  try {
    stream.write(text);
  } catch (_) {
    // ignoreErrors: a failing sink must not break the program
  }
}

// A function with its own `name` and `length` (Node's shape for these members).
function fn(name, length, impl) {
  Object.defineProperties(impl, { name: { value: name }, length: { value: length } });
  return impl;
}
// Node's `_stdoutErrorHandler`: an anonymous arrow (name '', no prototype), kept on each instance.
function errorHandler(self) {
  return fn('', 1, (err) => {
    if (err && !kStateOf.get(self).ignoreErrors) throw err;
  });
}

function Console(options) {
  let stdout = options;
  let stderr = arguments[1];
  let ignoreErrors = arguments[2];
  let groupIndentation = 2;
  if (!(options && typeof options.write === 'function')) {
    options = options || {};
    stdout = options.stdout;
    stderr = options.stderr;
    ignoreErrors = options.ignoreErrors;
    if (options.groupIndentation !== undefined) groupIndentation = options.groupIndentation;
  }
  if (!stdout || typeof stdout.write !== 'function') throw writableError('stdout');
  if (stderr === undefined || stderr === null) stderr = stdout;
  else if (typeof stderr.write !== 'function') throw writableError('stderr');
  const state = {
    stdout, stderr, groupIndentation, ignoreErrors: ignoreErrors !== false,
    indent: '', counts: new Map(), timers: new Map(),
  };
  kStateOf.set(this, state);
  const self = this;
  for (const name of METHODS) {
    const bound = this[name].bind(this);
    Object.defineProperty(bound, 'name', { value: name });
    this[name] = bound;
  }
  Object.defineProperties(this, {
    _stdout: {
      get: fn('get', 0, () => kStateOf.get(self).stdout),
      set: fn('set', 1, (v) => { kStateOf.get(self).stdout = v; }),
      configurable: true,
    },
    _stderr: {
      get: fn('get', 0, () => kStateOf.get(self).stderr),
      set: fn('set', 1, (v) => { kStateOf.get(self).stderr = v; }),
      configurable: true,
    },
    _ignoreErrors: { value: state.ignoreErrors, writable: true, configurable: true },
    _times: { value: state.timers, writable: true, configurable: true },
    _stdoutErrorHandler: { value: errorHandler(this), writable: true, configurable: true },
    _stderrErrorHandler: { value: errorHandler(this), writable: true, configurable: true },
  });
}
const METHODS = ['log', 'info', 'debug', 'warn', 'error', 'dir', 'time', 'timeEnd', 'timeLog', 'trace',
  'assert', 'clear', 'count', 'countReset', 'group', 'groupEnd', 'table', 'dirxml', 'groupCollapsed'];
Console.prototype = Object.create(Object.prototype, {
  constructor: { value: Console, writable: true, configurable: true },
});
// Shorthand methods (no own `prototype`, enumerable as in Node). Node's dirxml IS log and
// groupCollapsed IS group (one object at two paths): the builtin table applier would make a second
// path reuse the first path's object, so the aliases are set here, not as delegating wrappers.
Object.assign(Console.prototype, {
  log(...args) { emit(this, kStateOf.get(this).stdout, fmt(...args)); },
  info(...args) { this.log(...args); },
  debug(...args) { this.log(...args); },
  warn(...args) { emit(this, kStateOf.get(this).stderr, fmt(...args)); },
  error(...args) { this.warn(...args); },
  dir(object) { this.log(fmt('%O', object)); },
  time(label = 'default') {
    label = `${label}`;
    const timers = kStateOf.get(this).timers;
    if (!timers.has(label)) timers.set(label, now());
  },
  timeEnd(label = 'default') {
    this.timeLog(label);
    kStateOf.get(this).timers.delete(`${label}`);
  },
  timeLog(label = 'default', ...data) {
    label = `${label}`;
    const start = kStateOf.get(this).timers.get(label);
    if (start !== undefined) this.log(`${label}: ${formatTime(now() - start)}`, ...data);
  },
  trace(...args) {
    const msg = fmt(...args);
    const stack = String((new Error(msg)).stack || '').split('\n').slice(1);
    this.error(['Trace' + (msg ? ': ' + msg : ''), ...stack].join('\n'));
  },
  assert(expression, ...args) {
    if (expression) return;
    if (typeof args[0] === 'string') args[0] = 'Assertion failed: ' + args[0];
    else args.unshift('Assertion failed');
    this.warn(...args);
  },
  clear() {},
  count(label = 'default') {
    label = `${label}`;
    const counts = kStateOf.get(this).counts;
    const n = (counts.get(label) || 0) + 1;
    counts.set(label, n);
    this.log(`${label}: ${n}`);
  },
  countReset(label = 'default') {
    kStateOf.get(this).counts.delete(`${label}`);
  },
  group(...data) {
    if (data.length) this.log(...data);
    const st = kStateOf.get(this);
    st.indent += ' '.repeat(st.groupIndentation);
  },
  groupEnd() {
    const st = kStateOf.get(this);
    st.indent = st.indent.slice(0, Math.max(0, st.indent.length - st.groupIndentation));
  },
  table(tabularData, properties) {
    if (tabularData === null || typeof tabularData !== 'object') return this.log(tabularData);
    const map = Object.create(null);
    const valuesColumn = [];
    let hasPrimitives = false;
    const indexColumn = Object.keys(tabularData);
    for (let i = 0; i < indexColumn.length; i++) {
      const item = tabularData[indexColumn[i]];
      const primitive = item === null || (typeof item !== 'function' && typeof item !== 'object');
      if (properties === undefined && primitive) {
        hasPrimitives = true;
        valuesColumn[i] = fmt('%O', item);
        continue;
      }
      for (const key of properties || Object.keys(item)) {
        map[key] = map[key] || [];
        map[key][i] = (primitive && properties) || !hasOwn(item, key) ? '' : fmt('%O', item[key]);
      }
    }
    const head = Object.keys(map);
    const columns = head.map(k => map[k]);
    if (hasPrimitives) {
      head.push('Values');
      columns.push(valuesColumn);
    }
    head.unshift('(index)');
    columns.unshift(indexColumn);
    this.log(renderTable(head, columns));
  },
});
Console.prototype.dirxml = Console.prototype.log;
Console.prototype.groupCollapsed = Console.prototype.group;

// The global `console` is one `Console` instance on the process streams (Node: `console instanceof
// console.Console`). Node's global methods report length 0 (context: 1); the extras below exist only on it.
const globalConsole = new Console(process.stdout, process.stderr);
for (const name of METHODS) Object.defineProperty(globalConsole[name], 'length', { value: 0 });
Object.defineProperty(globalThis, 'Console', { value: Console, writable: true, enumerable: false, configurable: true });
Object.defineProperty(globalConsole, 'Console', { value: Console, writable: true, enumerable: true, configurable: true });
Object.assign(globalConsole, {
  profile: fn('profile', 0, () => {}),
  profileEnd: fn('profileEnd', 0, () => {}),
  timeStamp: fn('timeStamp', 0, () => {}),
  context: fn('context', 1, () => Object.assign({}, globalConsole)),
  createTask: fn('createTask', 0, () => ({ run: (f) => f() })),
});
