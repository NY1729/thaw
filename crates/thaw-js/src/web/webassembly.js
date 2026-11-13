// The real `WebAssembly` namespace over the `__thaw_wasm_*` natives (web/wasm_natives.rs, cargo
// feature `wasm`). Shapes, attributes and messages follow Node 22 (recorded). Methods and accessors
// are object-literal shorthand (no own `prototype`), and every function gets its own `name`,
// `length` and native text. Handles are the engine's ids; wrappers are cached per handle so the same
// funcref/memory/table/global is the same JS object. Known gaps: no `arguments`/`caller` own props
// on constructors; funcref wrappers first seen through a table are named by their store handle; no
// exception-handling engine (Tag/Exception are shape-only).
(() => {
  'use strict';
  const isObject = (v) => (typeof v === 'object' && v !== null) || typeof v === 'function';
  const hidden = (target, name, value) => Object.defineProperty(target, name, { value, writable: true, enumerable: false, configurable: true });
  const constant = (target, name, value) => Object.defineProperty(target, name, { value, writable: false, enumerable: false, configurable: true });
  // Methods and accessors come from object-literal shorthand, so they have no own `prototype`.
  const members = (target, source) => {
    for (const key of Reflect.ownKeys(source)) {
      const d = Object.getOwnPropertyDescriptor(source, key);
      Object.defineProperty(target, key, Object.assign(d, { enumerable: true, configurable: true }));
    }
  };
  // Own `name`/`length` for each shim function (the table's rows are checked against them), and
  // Node's native text for it.
  const heads = (fn) => {
    const { name, length } = fn;
    Object.defineProperty(fn, 'name', { value: name, configurable: true });
    Object.defineProperty(fn, 'length', { value: length, configurable: true });
    __thaw_wasm_mark_native(fn);
  };
  const walk = (obj) => {
    for (const key of Reflect.ownKeys(obj)) {
      const d = Object.getOwnPropertyDescriptor(obj, key);
      for (const f of [d.value, d.get, d.set]) if (typeof f === 'function') heads(f);
    }
  };
  const newCheck = (newTarget, name) => {
    if (newTarget === undefined) throw new TypeError(`WebAssembly.${name}(): WebAssembly.${name} must be invoked with 'new'`);
  };
  const FUNC_ARG = 'function-typed object must be null (if nullable) or a Wasm function object';
  const TYPE_ARG = 'type incompatibility when transforming from/to JS';

  // ---- natives: an engine error is a plain {__wasmErrorKind, message} object --------------------
  const errorClass = (name) => {
    const C = function (message) { return Reflect.construct(Error, [message, arguments[1]], new.target || C); };
    Object.setPrototypeOf(C, Error);
    const proto = Object.create(Error.prototype);
    hidden(proto, 'constructor', C);
    hidden(proto, 'name', name);
    hidden(proto, 'message', '');
    Object.defineProperty(C, 'prototype', { value: proto, writable: false, enumerable: false, configurable: false });
    return C;
  };
  const CompileError = errorClass('CompileError');
  const LinkError = errorClass('LinkError');
  const RuntimeError = errorClass('RuntimeError');
  const wrapError = (error) => {
    if (!isObject(error) || !('__wasmErrorKind' in error)) return error;
    if (error.__wasmErrorKind === 'Exit') {
      const exit = new Error(`WASI exited with code ${error.code}`);
      exit.__thawWasiExit = error.code;
      return exit;
    }
    const Class = { CompileError, LinkError }[error.__wasmErrorKind] || RuntimeError;
    return new Class(error.message);
  };
  const guard = (fn) => (...args) => {
    try { return fn(...args); } catch (error) { throw wrapError(error); }
  };
  const nat = {
    validate: guard(__thaw_wasm_validate),
    compile: guard(__thaw_wasm_compile),
    moduleImports: guard(__thaw_wasm_module_imports),
    moduleExports: guard(__thaw_wasm_module_exports),
    customSections: guard(__thaw_wasm_module_custom_sections),
    instantiate: guard(__thaw_wasm_instantiate),
    instanceExports: guard(__thaw_wasm_instance_exports),
    callFunc: guard(__thaw_wasm_call_func),
    funcType: guard(__thaw_wasm_func_type),
    memoryCreate: guard(__thaw_wasm_memory_create),
    memoryBuffer: guard(__thaw_wasm_memory_buffer),
    memoryGrow: guard(__thaw_wasm_memory_grow),
    tableCreate: guard(__thaw_wasm_table_create),
    tableLength: guard(__thaw_wasm_table_length),
    tableGet: guard(__thaw_wasm_table_get),
    tableSet: guard(__thaw_wasm_table_set),
    tableGrow: guard(__thaw_wasm_table_grow),
    globalCreate: guard(__thaw_wasm_global_create),
    globalGet: guard(__thaw_wasm_global_get),
    globalSet: guard(__thaw_wasm_global_set),
  };

  // ---- argument conversion -----------------------------------------------------------------------
  const toCount = (value, label) => {
    const n = Number(value);
    if (!Number.isFinite(n)) throw new TypeError(`${label} must be convertible to a valid number`);
    if (n < 0) throw new TypeError(`${label} must be non-negative`);
    return Math.trunc(n);
  };
  const toU32 = (value, name, prefix) => {
    const n = Number(value);
    if (!Number.isFinite(n) || n < 0 || n > 4294967295) throw new TypeError(`${prefix}: Property '${name}' must be in the unsigned long range`);
    return Math.trunc(n);
  };
  // Node's BufferSource is an ArrayBuffer or a TypedArray (not a DataView).
  const isBufferSource = (v) => v instanceof ArrayBuffer || (ArrayBuffer.isView(v) && !(v instanceof DataView));
  const bufferArg = (bytes, prefix) => {
    if (!isBufferSource(bytes)) throw new TypeError(`${prefix}: Argument 0 must be a buffer source`);
    return bytes;
  };
  const copyBytes = (source) => (source instanceof ArrayBuffer
    ? new Uint8Array(source).slice()
    : new Uint8Array(source.buffer, source.byteOffset, source.byteLength).slice());

  // Export function indexes (V8 names an exported function by its function index). Read from the
  // export section; the natives do not carry them.
  const readU32 = (bytes, at) => {
    let result = 0, scale = 1, byte;
    do { byte = bytes[at.at++]; result += (byte & 0x7f) * scale; scale *= 128; } while (byte & 0x80);
    return result;
  };
  const exportIndexes = (bytes) => {
    const found = new Map();
    const at = { at: 8 };
    while (at.at < bytes.length) {
      const id = bytes[at.at++];
      const end = at.at + readU32(bytes, at);
      if (id === 7) {
        const count = readU32(bytes, at);
        for (let i = 0; i < count; i++) {
          const length = readU32(bytes, at);
          const name = new TextDecoder().decode(bytes.subarray(at.at, at.at + length));
          at.at += length;
          const kind = bytes[at.at++];
          const index = readU32(bytes, at);
          if (kind === 0) found.set(name, index);
        }
      }
      at.at = end;
    }
    return found;
  };

  // ---- funcrefs ------------------------------------------------------------------------------------
  const funcHandles = new WeakMap(); // exported function -> funcref handle
  const funcCache = new Map(); // funcref handle -> exported function (identity)
  const funcArg = (value, message) => {
    if (value === null) return null;
    if (typeof value === 'function' && funcHandles.has(value)) return funcHandles.get(value);
    throw new TypeError(message);
  };
  const fromWasm = (value, type) => (value === null || type !== 'anyfunc' ? value : funcFor(value, String(value)));
  const fromResults = (value, types) => {
    if (types.length === 0) return undefined;
    if (types.length === 1) return fromWasm(value, types[0]);
    return value.map((item, i) => fromWasm(item, types[i]));
  };
  const funcFor = (handle, name) => {
    const cached = funcCache.get(handle);
    if (cached !== undefined) return cached;
    const { params, results } = nat.funcType(handle);
    const fn = (...args) => {
      const inputs = params.map((type, i) => (type === 'anyfunc' ? funcArg(args[i], TYPE_ARG) : args[i]));
      return fromResults(nat.callFunc(handle, inputs), results);
    };
    Object.defineProperty(fn, 'name', { value: name, configurable: true });
    Object.defineProperty(fn, 'length', { value: params.length, configurable: true });
    __thaw_wasm_mark_native(fn);
    funcHandles.set(fn, handle);
    funcCache.set(handle, fn);
    return fn;
  };

  // ---- Module ----------------------------------------------------------------------------------------
  const modules = new WeakMap(); // Module -> {handle, bytes}
  const initModule = (self, source, prefix) => {
    bufferArg(source, prefix);
    const handle = nat.compile(source, prefix);
    modules.set(self, { handle, bytes: copyBytes(source) });
    return self;
  };
  const compileModule = (source, prefix) => initModule(Object.create(Module.prototype), source, prefix);
  const moduleOf = (value, prefix) => {
    const data = modules.get(value);
    if (data === undefined) throw new TypeError(`${prefix}: Argument 0 must be a WebAssembly.Module`);
    return data;
  };
  function Module(bytes) {
    newCheck(new.target, 'Module');
    return initModule(this, bytes, 'WebAssembly.Module()');
  }
  Object.defineProperty(Module, 'prototype', { writable: false });
  hidden(Module.prototype, 'constructor', Module);
  members(Module, {
    imports(module) {
      return nat.moduleImports(moduleOf(module, 'WebAssembly.Module.imports()').handle);
    },
    exports(module) {
      return nat.moduleExports(moduleOf(module, 'WebAssembly.Module.exports()').handle);
    },
    customSections(module, name) {
      const data = moduleOf(module, 'WebAssembly.Module.customSections()');
      if (name === undefined) throw new TypeError('WebAssembly.Module.customSections(): Argument 1 is required');
      return nat.customSections(data.handle, String(name));
    },
  });
  constant(Module.prototype, Symbol.toStringTag, 'WebAssembly.Module');

  // ---- Instance --------------------------------------------------------------------------------------
  const instances = new WeakMap(); // Instance -> exports object
  const linkImport = (item, index, importObject, prefix) => {
    const where = `${prefix}: Import #${index} module="${item.module}" function="${item.name}"`;
    const ns = importObject[item.module];
    if (!isObject(ns)) throw new TypeError(`${prefix}: Import #${index} module="${item.module}": module is not an object or function`);
    const value = ns[item.name];
    switch (item.kind) {
      case 'function':
        if (funcHandles.has(value)) return [item.module, item.name, 'wasmFunction', funcHandles.get(value)];
        return [item.module, item.name, typeof value === 'function' ? 'function' : 'missing', value];
      case 'memory':
        if (memories.has(value)) return [item.module, item.name, 'memory', memories.get(value).handle];
        throw new LinkError(`${where}: memory import must be a WebAssembly.Memory object`);
      case 'table':
        if (tables.has(value)) return [item.module, item.name, 'table', tables.get(value).handle];
        throw new LinkError(`${where}: table import requires a WebAssembly.Table`);
      case 'global':
        if (globals.has(value)) return [item.module, item.name, 'global', globals.get(value).handle];
        if (typeof value === 'number' || typeof value === 'bigint') return [item.module, item.name, 'globalValue', value];
        throw new LinkError(`${where}: global import must be a number, valid Wasm reference, or WebAssembly.Global object`);
      default:
        throw new LinkError(`${where}: unsupported import kind ${item.kind}`);
    }
  };
  const exportedValue = (item, indexes) => {
    switch (item.kind) {
      case 'function': return funcFor(item.handle, String(indexes.has(item.name) ? indexes.get(item.name) : item.handle));
      case 'memory': return wrapMemory(item.handle);
      case 'table': return wrapTable(item.handle, item.element);
      case 'global': return wrapGlobal(item.handle, item.type);
      default: return undefined;
    }
  };
  const setupInstance = (self, module, importObject, prefix) => {
    const data = moduleOf(module, prefix);
    if (importObject !== undefined && !isObject(importObject)) throw new TypeError(`${prefix}: Argument 1 must be an object`);
    const imports = nat.moduleImports(data.handle);
    if (imports.length > 0 && importObject === undefined) throw new TypeError(`${prefix}: Imports argument must be present and must be an object`);
    const linkage = imports.map((item, index) => linkImport(item, index, importObject, prefix));
    const handle = nat.instantiate(data.handle, linkage, prefix);
    const indexes = exportIndexes(data.bytes);
    const exportsObject = Object.create(null);
    for (const item of nat.instanceExports(handle)) {
      Object.defineProperty(exportsObject, item.name, { value: exportedValue(item, indexes), writable: false, enumerable: true, configurable: false });
    }
    Object.freeze(exportsObject);
    instances.set(self, exportsObject);
    return self;
  };
  const instantiateModule = (module, importObject, prefix) => setupInstance(Object.create(Instance.prototype), module, importObject, prefix);
  function Instance(module) {
    newCheck(new.target, 'Instance');
    return setupInstance(this, module, arguments[1], 'WebAssembly.Instance()');
  }
  Object.defineProperty(Instance, 'prototype', { writable: false });
  hidden(Instance.prototype, 'constructor', Instance);
  members(Instance.prototype, {
    get exports() {
      const exportsObject = instances.get(this);
      if (exportsObject === undefined) throw new TypeError('WebAssembly.Instance.exports(): Receiver is not a WebAssembly.Instance');
      return exportsObject;
    },
  });
  constant(Instance.prototype, Symbol.toStringTag, 'WebAssembly.Instance');

  // ---- Memory -----------------------------------------------------------------------------------------
  const memories = new WeakMap(); // Memory -> {handle}
  const memoryCache = new Map(); // handle -> Memory
  const wrapMemory = (handle) => {
    let memory = memoryCache.get(handle);
    if (memory === undefined) {
      memory = Object.create(Memory.prototype);
      memories.set(memory, { handle });
      memoryCache.set(handle, memory);
    }
    return memory;
  };
  const memoryHandle = (self, label) => {
    const data = memories.get(self);
    if (data === undefined) throw new TypeError(`${label}: Receiver is not a WebAssembly.Memory`);
    return data.handle;
  };
  function Memory(descriptor) {
    newCheck(new.target, 'Memory');
    const prefix = 'WebAssembly.Memory()';
    if (!isObject(descriptor)) throw new TypeError(`${prefix}: Argument 0 must be a memory descriptor`);
    if (descriptor.initial === undefined) throw new TypeError(`${prefix}: Property 'initial' is required`);
    const initial = toU32(descriptor.initial, 'initial', prefix);
    if (initial > 65536) throw new RangeError(`${prefix}: Property 'initial': value ${initial} is above the upper bound 65536`);
    const maximum = descriptor.maximum === undefined ? undefined : toU32(descriptor.maximum, 'maximum', prefix);
    if (descriptor.shared === true && maximum === undefined) throw new TypeError(`${prefix}: If shared is true, maximum property should be defined.`);
    if (maximum !== undefined && maximum < initial) throw new RangeError(`${prefix}: Property 'maximum': value ${maximum} is below the lower bound ${initial}`);
    const handle = nat.memoryCreate(initial, maximum);
    memories.set(this, { handle });
    memoryCache.set(handle, this);
  }
  Object.defineProperty(Memory, 'prototype', { writable: false });
  hidden(Memory.prototype, 'constructor', Memory);
  members(Memory.prototype, {
    grow(delta) {
      const handle = memoryHandle(this, 'WebAssembly.Memory.grow()');
      return nat.memoryGrow(handle, toCount(delta, 'WebAssembly.Memory.grow(): Argument 0'));
    },
    get buffer() {
      return nat.memoryBuffer(memoryHandle(this, 'WebAssembly.Memory.buffer'));
    },
  });
  constant(Memory.prototype, Symbol.toStringTag, 'WebAssembly.Memory');

  // ---- Table ------------------------------------------------------------------------------------------
  const tables = new WeakMap(); // Table -> {handle, element}
  const tableCache = new Map();
  const elementType = (name) => (name === 'funcref' ? 'anyfunc' : name);
  const wrapTable = (handle, element) => {
    let table = tableCache.get(handle);
    if (table === undefined) {
      table = Object.create(Table.prototype);
      tables.set(table, { handle, element });
      tableCache.set(handle, table);
    }
    return table;
  };
  const tableOf = (self, label) => {
    const data = tables.get(self);
    if (data === undefined) throw new TypeError(`${label}: Receiver is not a WebAssembly.Table`);
    return data;
  };
  function Table(descriptor) {
    newCheck(new.target, 'Table');
    const prefix = 'WebAssembly.Table()';
    if (!isObject(descriptor)) throw new TypeError(`${prefix}: Argument 0 must be a table descriptor`);
    const element = elementType(String(descriptor.element));
    if (descriptor.initial === undefined) throw new TypeError(`${prefix}: Property 'initial' is required`);
    const initial = toU32(descriptor.initial, 'initial', prefix);
    const maximum = descriptor.maximum === undefined ? undefined : toU32(descriptor.maximum, 'maximum', prefix);
    let init = arguments[1];
    if (element === 'anyfunc') init = funcArg(init === undefined ? null : init, `${prefix}: Argument 2 is invalid: ${FUNC_ARG}`);
    const handle = nat.tableCreate(element, initial, maximum, init);
    tables.set(this, { handle, element: element === 'externref' ? 'externref' : 'anyfunc' });
    tableCache.set(handle, this);
  }
  Object.defineProperty(Table, 'prototype', { writable: false });
  hidden(Table.prototype, 'constructor', Table);
  members(Table.prototype, {
    get length() {
      return nat.tableLength(tableOf(this, 'WebAssembly.Table.length()').handle);
    },
    grow(delta) {
      const { handle, element } = tableOf(this, 'WebAssembly.Table.grow()');
      const count = toCount(delta, 'WebAssembly.Table.grow(): Argument 0');
      const init = element === 'anyfunc' ? funcArg(arguments[1] === undefined ? null : arguments[1], `WebAssembly.Table.grow(): Argument 1 is invalid: ${FUNC_ARG}`) : arguments[1];
      return nat.tableGrow(handle, count, init);
    },
    set(index) {
      const { handle, element } = tableOf(this, 'WebAssembly.Table.set()');
      const at = toCount(index, 'WebAssembly.Table.set(): Argument 0');
      const value = element === 'anyfunc' ? funcArg(arguments[1] === undefined ? null : arguments[1], `WebAssembly.Table.set(): Argument 1 is invalid for table: ${FUNC_ARG}`) : arguments[1];
      nat.tableSet(handle, at, value);
      return undefined;
    },
    get(index) {
      const { handle, element } = tableOf(this, 'WebAssembly.Table.get()');
      const at = toCount(index, 'WebAssembly.Table.get(): Argument 0');
      return fromWasm(nat.tableGet(handle, at), element);
    },
  });
  constant(Table.prototype, Symbol.toStringTag, 'WebAssembly.Table');

  // ---- Global -----------------------------------------------------------------------------------------
  const globals = new WeakMap(); // Global -> {handle, type}
  const globalCache = new Map();
  const wrapGlobal = (handle, type) => {
    let global = globalCache.get(handle);
    if (global === undefined) {
      global = Object.create(Global.prototype);
      globals.set(global, { handle, type });
      globalCache.set(handle, global);
    }
    return global;
  };
  const globalOf = (self, label) => {
    const data = globals.get(self);
    if (data === undefined) throw new TypeError(`${label}: Receiver is not a WebAssembly.Global`);
    return data;
  };
  function Global(descriptor) {
    newCheck(new.target, 'Global');
    const prefix = 'WebAssembly.Global()';
    if (!isObject(descriptor)) throw new TypeError(`${prefix}: Argument 0 must be a global descriptor`);
    const type = elementType(String(descriptor.value));
    const value = type === 'anyfunc' ? funcArg(arguments[1] === undefined ? null : arguments[1], `${prefix}: ${FUNC_ARG}`) : arguments[1];
    const handle = nat.globalCreate(type, Boolean(descriptor.mutable), value);
    globals.set(this, { handle, type });
    globalCache.set(handle, this);
  }
  Object.defineProperty(Global, 'prototype', { writable: false });
  hidden(Global.prototype, 'constructor', Global);
  members(Global.prototype, {
    valueOf() {
      const data = globalOf(this, 'WebAssembly.Global.valueOf()');
      return fromWasm(nat.globalGet(data.handle), data.type);
    },
    get value() {
      const data = globalOf(this, 'get WebAssembly.Global.value');
      return fromWasm(nat.globalGet(data.handle), data.type);
    },
    set value(value) {
      const data = globalOf(this, 'set WebAssembly.Global.value');
      const next = data.type === 'anyfunc' ? funcArg(value, `set WebAssembly.Global.value: ${FUNC_ARG}`) : value;
      nat.globalSet(data.handle, next);
    },
  });
  constant(Global.prototype, Symbol.toStringTag, 'WebAssembly.Global');

  // ---- Tag, JSTag, Exception (shape only: wasm code never throws them, plan D3) ----------------------
  const tags = new WeakMap(); // Tag -> {parameters}
  function Tag(type) {
    newCheck(new.target, 'Tag');
    if (!isObject(type) || type.parameters === undefined) throw new TypeError("WebAssembly.Tag(): Argument 0 must be a tag type with 'parameters'");
    tags.set(this, { parameters: Array.from(type.parameters) });
  }
  Object.defineProperty(Tag, 'prototype', { writable: false });
  hidden(Tag.prototype, 'constructor', Tag);
  constant(Tag.prototype, Symbol.toStringTag, 'WebAssembly.Tag');
  const JSTag = Object.create(Tag.prototype);
  tags.set(JSTag, { parameters: ['externref'] });

  const exceptions = new WeakMap(); // Exception -> {tag, values}
  function Exception(tag) {
    newCheck(new.target, 'Exception');
    if (!tags.has(tag)) throw new TypeError('WebAssembly.Exception(): Argument 0 must be a WebAssembly tag');
    exceptions.set(this, { tag, values: Array.from(arguments[1] === undefined ? [] : arguments[1]) });
  }
  Object.defineProperty(Exception, 'prototype', { writable: false });
  hidden(Exception.prototype, 'constructor', Exception);
  const exceptionOf = (self, label) => {
    const data = exceptions.get(self);
    if (data === undefined) throw new TypeError(`${label}: Receiver is not a WebAssembly.Exception`);
    return data;
  };
  members(Exception.prototype, {
    getArg(tag, index) {
      const data = exceptionOf(this, 'WebAssembly.Exception.getArg()');
      if (data.tag !== tag) throw new TypeError('WebAssembly.Exception.getArg(): First argument does not match the exception tag');
      const at = toCount(index, 'WebAssembly.Exception.getArg(): Argument 1');
      if (at >= data.values.length) throw new RangeError('WebAssembly.Exception.getArg(): Index out of range');
      return data.values[at];
    },
    is(tag) {
      return exceptionOf(this, 'WebAssembly.Exception.is()').tag === tag;
    },
  });
  constant(Exception.prototype, Symbol.toStringTag, 'WebAssembly.Exception');

  // ---- compile / validate / instantiate ---------------------------------------------------------------
  // Async entry points settle on a microtask, as Node's do.
  const later = (fn) => Promise.resolve().then(fn);
  const received = (value) => `type ${typeof value} (${String(value)})`;
  const streamedBytes = (source, prefix) => Promise.resolve(source).then((response) => {
    if (typeof Response !== 'function' || !(response instanceof Response)) {
      throw new TypeError(`The "source" argument must be an instance of Response or an Promise resolving to Response. Received ${received(response)}`);
    }
    const type = response.headers.get('Content-Type');
    if (type !== 'application/wasm') throw new TypeError(`${prefix}: Response has unsupported MIME type '${type}'`);
    return response.arrayBuffer();
  });

  const WebAssembly = {};
  members(WebAssembly, {
    compile(bytes) {
      return later(() => compileModule(bytes, 'WebAssembly.compile()'));
    },
    validate(bytes) {
      return nat.validate(bufferArg(bytes, 'WebAssembly.validate()'));
    },
    instantiate(source) {
      const importObject = arguments[1];
      return later(() => {
        if (modules.has(source)) return instantiateModule(source, importObject, 'WebAssembly.instantiate()');
        if (!isBufferSource(source)) throw new TypeError('WebAssembly.instantiate(): Argument 0 must be a buffer source or a WebAssembly.Module object');
        const module = compileModule(source, 'WebAssembly.instantiate()');
        return { module, instance: instantiateModule(module, importObject, 'WebAssembly.instantiate()') };
      });
    },
  });
  hidden(WebAssembly, 'Module', Module);
  hidden(WebAssembly, 'Instance', Instance);
  hidden(WebAssembly, 'Table', Table);
  hidden(WebAssembly, 'Memory', Memory);
  hidden(WebAssembly, 'Global', Global);
  hidden(WebAssembly, 'Tag', Tag);
  Object.defineProperty(WebAssembly, 'JSTag', { value: JSTag, writable: false, enumerable: false, configurable: true });
  hidden(WebAssembly, 'Exception', Exception);
  hidden(WebAssembly, 'CompileError', CompileError);
  hidden(WebAssembly, 'LinkError', LinkError);
  hidden(WebAssembly, 'RuntimeError', RuntimeError);
  members(WebAssembly, {
    compileStreaming(source) {
      return streamedBytes(source, 'WebAssembly.compileStreaming()').then((bytes) => compileModule(bytes, 'WebAssembly.compileStreaming()'));
    },
    instantiateStreaming(source) {
      const importObject = arguments[1];
      return streamedBytes(source, 'WebAssembly.instantiateStreaming()').then((bytes) => {
        const module = compileModule(bytes, 'WebAssembly.instantiateStreaming()');
        return { module, instance: instantiateModule(module, importObject, 'WebAssembly.instantiateStreaming()') };
      });
    },
  });
  constant(WebAssembly, Symbol.toStringTag, 'WebAssembly');

  // Heads and native text for every function the namespace exposes (constructors, statics,
  // prototype methods and accessors, error classes).
  walk(WebAssembly);
  for (const C of [Module, Instance, Memory, Table, Global, Tag, Exception, CompileError, LinkError, RuntimeError]) {
    heads(C);
    walk(C);
    walk(C.prototype);
  }
  Object.defineProperty(globalThis, 'WebAssembly', { value: WebAssembly, writable: true, enumerable: false, configurable: true });
})();
