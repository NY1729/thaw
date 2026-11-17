// node:sqlite (Node 22.23.1) over the thaw-js natives `__thaw_sqlite_*` (cargo feature `sqlite`).
// Handles are integers in the host's tables; this file owns the Node surface: validation, error codes,
// tagged wire decoding ({i} bigint, {b} blob, {f} non-finite real), null-prototype rows and iterators.
// Not implemented yet (PR4): user functions, aggregates, sessions, applyChangeset, extensions, backup.

var STATES = new WeakMap();
var IteratorPrototype = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
var MIN_I64 = BigInt('-9223372036854775808');
var MAX_I64 = BigInt('9223372036854775807');

if (typeof process !== 'undefined' && process && typeof process.emitWarning === 'function') {
  process.emitWarning('SQLite is an experimental feature and might change at any time', 'ExperimentalWarning');
}

function fail(Type, code, message) {
  var error = new Type(message);
  error.code = code;
  return error;
}

function notImplemented(name) {
  return fail(Error, 'ERR_METHOD_NOT_IMPLEMENTED', name + ' is not implemented on thaw yet (planned for a later change)');
}

function nullProto(source) {
  var out = Object.create(null);
  Object.keys(source).forEach(function (key) { out[key] = source[key]; });
  return out;
}

function result(done, value) {
  var out = Object.create(null);
  out.done = done;
  out.value = value;
  return out;
}

function boolOption(options, name, fallback) {
  var value = options[name];
  if (value === undefined) return fallback;
  if (typeof value !== 'boolean') throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options.' + name + '" argument must be a boolean.');
  return value;
}

function boolArg(name, value) {
  if (typeof value !== 'boolean') throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "' + name + '" argument must be a boolean.');
  return value;
}

function sqlArg(sql) {
  if (typeof sql !== 'string') throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "sql" argument must be a string.');
  return sql;
}

function databasePath(path) {
  var text;
  if (typeof path === 'string') text = path;
  else if (path instanceof Uint8Array) text = Buffer.from(path.buffer, path.byteOffset, path.byteLength).toString('utf8');
  else if (path !== null && typeof path === 'object' && path.protocol === 'file:') text = decodeURIComponent(path.pathname);
  if (text === undefined || text.indexOf('\u0000') !== -1) {
    throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "path" argument must be a string, Uint8Array, or URL without null bytes.');
  }
  return text;
}

function stateOf(db) {
  var state = STATES.get(db);
  if (state === undefined) throw fail(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type DatabaseSync');
  return state;
}

function openState(db) {
  var state = stateOf(db);
  if (state.id === null) throw fail(Error, 'ERR_INVALID_STATE', 'database is not open');
  return state;
}

function stmtOf(stmt) {
  var state = STATES.get(stmt);
  if (state === undefined) throw fail(TypeError, 'ERR_INVALID_THIS', 'Value of "this" must be of type StatementSync');
  return state;
}

function isNamedObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value) && !ArrayBuffer.isView(value);
}

function hexOf(view) {
  var bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
  var parts = [];
  for (var i = 0; i < bytes.length; i++) parts.push((bytes[i] < 16 ? '0' : '') + bytes[i].toString(16));
  return parts.join('');
}

// One bound value as a wire cell (see sqlite/wire.rs). NaN goes as null, which SQLite stores as NULL.
function cellOf(value, label) {
  switch (typeof value) {
    case 'number':
      if (value === Infinity) return { f: 'inf' };
      if (value === -Infinity) return { f: '-inf' };
      return value;
    case 'string':
      // SQLite binds text up to its first NUL (node:sqlite does the same)
      var nul = value.indexOf('\u0000');
      return nul === -1 ? value : value.slice(0, nul);
    case 'bigint':
      if (value < MIN_I64 || value > MAX_I64) throw fail(TypeError, 'ERR_INVALID_ARG_VALUE', 'BigInt value is too large to bind.');
      return { i: value.toString() };
    case 'object':
      if (value === null) return null;
      if (ArrayBuffer.isView(value)) return { b: hexOf(value) };
  }
  throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'Provided value cannot be bound to SQLite parameter ' + label + '.');
}

// Bind list for the natives: `[key, cell]` pairs. A leading plain object binds its keys by name; every
// other argument binds by its 1-based position.
function encodeParams(args) {
  var pairs = [];
  var first = 0;
  if (args.length > 0 && isNamedObject(args[0])) {
    first = 1;
    Object.keys(args[0]).forEach(function (key) {
      pairs.push([key, cellOf(args[0][key], key)]);
    });
  }
  for (var i = first; i < args.length; i++) {
    pairs.push([i + 1, cellOf(args[i], i + 1)]);
  }
  return JSON.stringify(pairs);
}

function hexBytes(text) {
  var out = new Uint8Array(text.length / 2);
  for (var i = 0; i < out.length; i++) out[i] = parseInt(text.substr(i * 2, 2), 16);
  return out;
}

function decodeCell(cell, readBigInts) {
  if (cell === null || typeof cell !== 'object') return cell;
  if (cell.i !== undefined) {
    if (!readBigInts) throw fail(RangeError, 'ERR_OUT_OF_RANGE', 'Value is too large to be represented as a JavaScript number: ' + cell.i);
    return BigInt(cell.i);
  }
  if (cell.b !== undefined) return hexBytes(cell.b);
  return cell.f === 'inf' ? Infinity : -Infinity;
}

function columnNames(id) {
  return JSON.parse(__thaw_sqlite_stmt_columns(id)).map(function (column) { return column.name; });
}

// A row from a native step/get/all result: an array of cells, shaped as the statement's flags ask.
function shapeRow(cells, names, state) {
  if (state.returnArrays) return cells.map(function (cell) { return decodeCell(cell, state.readBigInts); });
  var row = Object.create(null);
  for (var i = 0; i < cells.length; i++) row[names[i]] = decodeCell(cells[i], state.readBigInts);
  return row;
}

function numberCell(cell, state) {
  return decodeCell(cell, state.readBigInts);
}

function DatabaseSync(path, options) {
  if (!(this instanceof DatabaseSync)) throw fail(TypeError, 'ERR_CONSTRUCT_CALL_REQUIRED', 'Cannot call constructor without `new`');
  var file = databasePath(path);
  if (options === undefined) options = {};
  else if (options === null || typeof options !== 'object') throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options" argument must be an object.');
  var timeout = options.timeout === undefined ? 0 : options.timeout;
  if (typeof timeout !== 'number' || Math.floor(timeout) !== timeout) throw fail(TypeError, 'ERR_INVALID_ARG_TYPE', 'The "options.timeout" argument must be an integer.');
  var state = {
    id: null,
    path: file,
    open: boolOption(options, 'open', true),
    flags: {
      readOnly: boolOption(options, 'readOnly', false),
      foreignKeys: boolOption(options, 'enableForeignKeyConstraints', true),
      doubleQuotedStrings: boolOption(options, 'enableDoubleQuotedStringLiterals', false),
      allowExtension: boolOption(options, 'allowExtension', false),
      timeout: timeout
    }
  };
  STATES.set(this, state);
  Object.defineProperty(this, 'isOpen', { enumerable: true, configurable: true, get: function () { return stateOf(this).id !== null; } });
  Object.defineProperty(this, 'isTransaction', { enumerable: true, configurable: true, get: function () { return __thaw_sqlite_is_transaction(openState(this).id); } });
  if (state.open) this.open();
}

DatabaseSync.prototype.open = function open() {
  var state = stateOf(this);
  if (state.id !== null) throw fail(Error, 'ERR_INVALID_STATE', 'database is already open');
  state.id = __thaw_sqlite_open(state.path, JSON.stringify(state.flags));
};

DatabaseSync.prototype.close = function close() {
  var state = openState(this);
  __thaw_sqlite_close(state.id);
  state.id = null;
};

DatabaseSync.prototype.exec = function exec(sql) {
  sqlArg(sql);
  __thaw_sqlite_exec(openState(this).id, sql);
};

DatabaseSync.prototype.prepare = function prepare(sql) {
  sqlArg(sql);
  var db = openState(this);
  var stmt = Object.create(StatementSync.prototype);
  var state = { id: __thaw_sqlite_prepare(db.id, sql), readBigInts: false, returnArrays: false };
  STATES.set(stmt, state);
  Object.defineProperty(stmt, 'sourceSQL', { enumerable: true, configurable: true, get: function () { return __thaw_sqlite_stmt_source_sql(state.id); } });
  Object.defineProperty(stmt, 'expandedSQL', { enumerable: true, configurable: true, get: function () { return __thaw_sqlite_stmt_expanded_sql(state.id); } });
  return stmt;
};

DatabaseSync.prototype.location = function location(dbName) {
  if (dbName !== undefined && dbName !== 'main') return null;
  return __thaw_sqlite_location(openState(this).id);
};

DatabaseSync.prototype.function = function () { throw notImplemented('DatabaseSync.prototype.function'); };
DatabaseSync.prototype.aggregate = function () { throw notImplemented('DatabaseSync.prototype.aggregate'); };
DatabaseSync.prototype.createSession = function () { throw notImplemented('DatabaseSync.prototype.createSession'); };
DatabaseSync.prototype.applyChangeset = function () { throw notImplemented('DatabaseSync.prototype.applyChangeset'); };
DatabaseSync.prototype.enableLoadExtension = function () { throw notImplemented('DatabaseSync.prototype.enableLoadExtension'); };
DatabaseSync.prototype.loadExtension = function () { throw notImplemented('DatabaseSync.prototype.loadExtension'); };

if (typeof Symbol.dispose === 'symbol') {
  DatabaseSync.prototype[Symbol.dispose] = function () {
    if (stateOf(this).id !== null) this.close();
  };
}

function StatementSync() {
  throw fail(Error, 'ERR_ILLEGAL_CONSTRUCTOR', 'Illegal constructor');
}

StatementSync.prototype.run = function run() {
  var state = stmtOf(this);
  var counts = JSON.parse(__thaw_sqlite_stmt_run(state.id, encodeParams(arguments)));
  return {
    lastInsertRowid: numberCell(counts.lastInsertRowid, state),
    changes: numberCell(counts.changes, state)
  };
};

StatementSync.prototype.get = function get() {
  var state = stmtOf(this);
  var text = __thaw_sqlite_stmt_get(state.id, encodeParams(arguments));
  if (text === null) return undefined;
  return shapeRow(JSON.parse(text), columnNames(state.id), state);
};

StatementSync.prototype.all = function all() {
  var state = stmtOf(this);
  var rows = JSON.parse(__thaw_sqlite_stmt_all(state.id, encodeParams(arguments)));
  if (rows.length === 0) return [];
  var names = state.returnArrays ? null : columnNames(state.id);
  return rows.map(function (cells) { return shapeRow(cells, names, state); });
};

function StatementIterator(state, names) {
  return { state: state, names: names, done: false };
}

var ITERATORS = new WeakMap();

var StatementIteratorPrototype = Object.create(IteratorPrototype);
StatementIteratorPrototype.next = function next() {
  var it = ITERATORS.get(this);
  if (it.done) return result(true, null);
  var text = __thaw_sqlite_stmt_step(it.state.id);
  if (text === null) {
    it.done = true;
    __thaw_sqlite_stmt_reset(it.state.id);
    return result(true, null);
  }
  return result(false, shapeRow(JSON.parse(text), it.names, it.state));
};
StatementIteratorPrototype.return = function () {
  var it = ITERATORS.get(this);
  if (!it.done) {
    it.done = true;
    try {
      __thaw_sqlite_stmt_reset(it.state.id);
    } catch (error) {
      // the statement was finalized by its database's close: nothing left to reset
    }
  }
  return result(true, null);
};
StatementIteratorPrototype[Symbol.iterator] = function () { return this; };

StatementSync.prototype.iterate = function iterate() {
  var state = stmtOf(this);
  __thaw_sqlite_stmt_start(state.id, encodeParams(arguments));
  var iterator = Object.create(StatementIteratorPrototype);
  ITERATORS.set(iterator, StatementIterator(state, state.returnArrays ? null : columnNames(state.id)));
  return iterator;
};

StatementSync.prototype.columns = function columns() {
  return JSON.parse(__thaw_sqlite_stmt_columns(stmtOf(this).id)).map(nullProto);
};

StatementSync.prototype.setReadBigInts = function setReadBigInts(on) {
  var state = stmtOf(this);
  boolArg('readBigInts', on);
  __thaw_sqlite_stmt_set_flag(state.id, 'readBigInts', on);
  state.readBigInts = on;
};

StatementSync.prototype.setReturnArrays = function setReturnArrays(on) {
  var state = stmtOf(this);
  state.returnArrays = boolArg('returnArrays', on);
};

StatementSync.prototype.setAllowBareNamedParameters = function setAllowBareNamedParameters(on) {
  var state = stmtOf(this);
  __thaw_sqlite_stmt_set_flag(state.id, 'allowBareNamedParameters', boolArg('allowBareNamedParameters', on));
};

StatementSync.prototype.setAllowUnknownNamedParameters = function setAllowUnknownNamedParameters(on) {
  var state = stmtOf(this);
  __thaw_sqlite_stmt_set_flag(state.id, 'allowUnknownNamedParameters', boolArg('enabled', on));
};

function backup() {
  throw notImplemented('backup');
}

module.exports = {
  DatabaseSync: DatabaseSync,
  StatementSync: StatementSync,
  constants: {
    SQLITE_CHANGESET_OMIT: 0,
    SQLITE_CHANGESET_REPLACE: 1,
    SQLITE_CHANGESET_ABORT: 2,
    SQLITE_CHANGESET_DATA: 1,
    SQLITE_CHANGESET_NOTFOUND: 2,
    SQLITE_CHANGESET_CONFLICT: 3,
    SQLITE_CHANGESET_CONSTRAINT: 4,
    SQLITE_CHANGESET_FOREIGN_KEY: 5
  },
  backup: backup
};
