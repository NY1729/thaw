(() => {
  const wasmBytes = value => {
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    throw new TypeError('WebAssembly binary must be an ArrayBuffer or ArrayBufferView');
  };
  const wasmHex = value => Array.from(wasmBytes(value), byte => byte.toString(16).padStart(2, '0')).join('');
  const wasmUnhex = value => {
    const output = new Uint8Array(value.length / 2);
    for (let index = 0; index < output.length; index++) output[index] = parseInt(value.slice(index * 2, index * 2 + 2), 16);
    return output;
  };
  const wasmResult = (encoded, ErrorType = Error) => {
    const result = JSON.parse(encoded);
    if (!result.ok) throw new ErrorType(result.error);
    return result;
  };
  const wasmFinalizer = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(resource => {
    if (resource.resources) for (const entry of resource.resources) if (entry.binding) entry.value.__thawUnbind(entry.binding);
    __thaw_wasm_release(resource.kind, resource.handle);
  }) : null;
  const wasmFuncrefs = new Map();
  const wasmCachedFuncref = key => { const entry = wasmFuncrefs.get(key); return entry && typeof entry.deref === 'function' ? entry.deref() : entry; };
  const wasmCacheFuncref = (key, value) => wasmFuncrefs.set(key, typeof WeakRef === 'function' ? new WeakRef(value) : value);
  const wasmExternrefObjects = new WeakMap(), wasmExternrefPrimitives = new Map();
  const wasmRetainExternref = value => {
    if (value === null) return 0;
    const objectLike = (typeof value === 'object' && value !== null) || typeof value === 'function';
    const references = objectLike ? wasmExternrefObjects : wasmExternrefPrimitives;
    if (references.has(value)) return __thaw_wasm_retain_value_at(value, references.get(value));
    const handle = __thaw_wasm_retain_value(value); references.set(value, handle); return handle;
  };
  const wasmPrepareFuncref = value => {
    if (value.__thawWasmBridge === undefined) {
      const bridge = (...args) => {
        const called = wasmResult(__thaw_wasm_call_funcref(value.__thawWasmInstance, value.__thawWasmFuncref, JSON.stringify(args.map(wasmEncodeValue))), WebAssembly.RuntimeError);
        const values = called.values.map(item => wasmDecodeValue(item, value.__thawWasmInstance));
        return values.length === 0 ? undefined : (values.length === 1 ? values[0] : values);
      };
      Object.defineProperty(value, '__thawWasmBridge', { value: __thaw_wasm_retain_funcref(bridge, value.__thawWasmInstance) });
      Object.defineProperty(value, '__thawWasmRestore', { value: __thaw_wasm_retain_funcref(value, value.__thawWasmInstance) });
    }
    return { t: 'funcref', v: value.__thawWasmFuncref, instance: value.__thawWasmInstance, bridge: value.__thawWasmBridge, restore: value.__thawWasmRestore, parameters: value.__thawWasmParameters, results: value.__thawWasmResults };
  };
  const wasmEncodeValue = value => typeof value === 'function' && value.__thawWasmFuncref !== undefined
    ? wasmPrepareFuncref(value)
    : typeof value === 'bigint'
    ? { t: 'bigint', v: String(value) }
    : typeof value === 'number' ? { t: 'number', v: Number.isFinite(value) ? value : null }
    : { t: 'externref', v: wasmRetainExternref(value) };
  const wasmDecodeValue = (value, instance) => {
    if (value.t === 'bigint') return BigInt(value.v);
    if (value.t === 'externref') return __thaw_wasm_restore_value(Number(value.v));
    if (value.t === 'jsfuncref') return __thaw_wasm_restore_import(Number(value.v));
    if (value.t !== 'funcref') return value.v === null ? NaN : value.v;
    if (!value.v) return null;
    const key = instance + ':' + value.v;
    let cached = wasmCachedFuncref(key);
    if (!cached) {
      const callable = (...args) => {
        const called = wasmResult(__thaw_wasm_call_funcref(instance, value.v, JSON.stringify(args.map(wasmEncodeValue))), WebAssembly.RuntimeError);
        const values = called.values.map(item => wasmDecodeValue(item, instance));
        return values.length === 0 ? undefined : (values.length === 1 ? values[0] : values);
      };
      Object.defineProperty(callable, '__thawWasmInstance', { value: instance });
      Object.defineProperty(callable, '__thawWasmFuncref', { value: value.v });
      Object.defineProperty(callable, '__thawWasmParameters', { value: value.parameters || [] });
      Object.defineProperty(callable, '__thawWasmResults', { value: value.results || [] });
      wasmCacheFuncref(key, callable); cached = callable;
    }
    return cached;
  };
  class WasmModule {
    constructor(bytes) {
      const result = wasmResult(__thaw_wasm_compile(wasmHex(bytes)), WebAssembly.CompileError);
      Object.defineProperty(this, '__thawHandle', { value: result.handle });
      Object.defineProperty(this, '__thawExports', { value: result.exports });
      Object.defineProperty(this, '__thawImports', { value: result.imports });
      if (wasmFinalizer) wasmFinalizer.register(this, { kind: 'module', handle: result.handle });
    }
    dispose() { if (this.__thawDisposed) return; if (__thaw_wasm_release('module', this.__thawHandle)) this.__thawDisposed = true; }
    static exports(module) {
      if (!(module instanceof WasmModule)) throw new TypeError('WebAssembly.Module.exports(): argument 0 must be a WebAssembly.Module');
      return module.__thawExports.map(value => ({ ...value }));
    }
    static imports(module) {
      if (!(module instanceof WasmModule)) throw new TypeError('WebAssembly.Module.imports(): argument 0 must be a WebAssembly.Module');
      return module.__thawImports.map(value => ({ ...value }));
    }
    static customSections(module, name) {
      if (!(module instanceof WasmModule)) throw new TypeError('WebAssembly.Module.customSections(): argument 0 must be a WebAssembly.Module');
      const result = wasmResult(__thaw_wasm_custom_sections(module.__thawHandle, String(name)));
      return result.sections.map(section => wasmUnhex(section).buffer);
    }
  }
  class WasmMemory {
    constructor(descriptor, internal) {
      if (internal) {
        this.__thawInstance = descriptor.instance;
        this.__thawName = descriptor.name;
      } else {
        if (!descriptor || descriptor.initial === undefined) throw new TypeError('WebAssembly.Memory(): Property initial is required');
        const initial = Number(descriptor.initial), maximum = descriptor.maximum === undefined ? -1 : Number(descriptor.maximum);
        const result = wasmResult(__thaw_wasm_memory_create(initial, maximum), RangeError);
        this.__thawInstance = 0;
        this.__thawName = String(result.handle);
        this.__thawMaximum = descriptor.maximum === undefined ? null : Number(descriptor.maximum);
      }
      this.__thawBindings = [];
      this.__thawRefresh();
    }
    __thawRefresh(binding) {
      const instance = binding ? binding.instance : this.__thawInstance, name = binding ? binding.name : this.__thawName;
      const result = wasmResult(__thaw_wasm_memory(instance, name, 'read', ''));
      const bytes = wasmUnhex(result.value);
      if (this.__thawBuffer && bytes.byteLength > this.__thawBuffer.byteLength && binding) {
        const delta = (bytes.byteLength - this.__thawBuffer.byteLength) / 65536;
        wasmResult(__thaw_wasm_memory(this.__thawInstance, this.__thawName, 'grow', String(delta)), RangeError);
        for (const other of this.__thawBindings) if (other !== binding) wasmResult(__thaw_wasm_memory(other.instance, other.name, 'grow', String(delta)), RangeError);
      }
      if (this.__thawBuffer && this.__thawBuffer.byteLength === bytes.byteLength) new Uint8Array(this.__thawBuffer).set(bytes);
      else {
        if (this.__thawBuffer) __thaw_detach_array_buffer(this.__thawBuffer);
        this.__thawBuffer = bytes.buffer;
      }
      if (binding) { const data = wasmHex(this.__thawBuffer); wasmResult(__thaw_wasm_memory(this.__thawInstance, this.__thawName, 'write', data)); for (const other of this.__thawBindings) if (other !== binding) wasmResult(__thaw_wasm_memory(other.instance, other.name, 'write', data)); }
    }
    __thawSync() {
      const data = wasmHex(this.__thawBuffer);
      wasmResult(__thaw_wasm_memory(this.__thawInstance, this.__thawName, 'write', data));
      for (const binding of this.__thawBindings) wasmResult(__thaw_wasm_memory(binding.instance, binding.name, 'write', data));
    }
    __thawBind(instance, module, name) { const binding = { instance, name: 'import:' + module + '\x1f' + name }; this.__thawBindings.push(binding); return binding; }
    __thawUnbind(binding) { this.__thawBindings = this.__thawBindings.filter(value => value !== binding); }
    get buffer() { return this.__thawBuffer; }
    grow(delta) {
      this.__thawSync();
      const result = wasmResult(__thaw_wasm_memory(this.__thawInstance, this.__thawName, 'grow', String(Number(delta))), RangeError);
      for (const binding of this.__thawBindings) wasmResult(__thaw_wasm_memory(binding.instance, binding.name, 'grow', String(Number(delta))), RangeError);
      this.__thawRefresh();
      return result.value;
    }
  }
  class WasmGlobal {
    constructor(descriptor, value, internal) {
      if (internal) { this.__thawInstance = descriptor.instance; this.__thawName = descriptor.name; return; }
      if (!descriptor || !['i32','i64','f32','f64','externref'].includes(String(descriptor.value))) throw new TypeError('WebAssembly.Global(): invalid value type');
      this.__thawType = String(descriptor.value); this.__thawMutable = Boolean(descriptor.mutable);
      this.__thawLocalValue = this.__thawConvert(value === undefined ? (this.__thawType === 'i64' ? 0n : 0) : value);
      this.__thawBindings = [];
    }
    __thawConvert(value) {
      if (this.__thawType === 'i64') return BigInt.asIntN(64, BigInt(value));
      if (this.__thawType === 'externref') return value;
      if (this.__thawType === 'i32') return Number(value) | 0;
      if (this.__thawType === 'f32') return Math.fround(Number(value));
      return Number(value);
    }
    get value() { return this.__thawInstance === undefined ? this.__thawLocalValue : wasmDecodeValue(wasmResult(__thaw_wasm_global(this.__thawInstance, this.__thawName, undefined)).value); }
    set value(value) {
      if (this.__thawInstance === undefined) {
        if (!this.__thawMutable) throw new TypeError('set WebAssembly.Global.value: immutable global');
        this.__thawLocalValue = this.__thawConvert(value); this.__thawSync(); return;
      }
      wasmResult(__thaw_wasm_global(this.__thawInstance, this.__thawName, JSON.stringify(wasmEncodeValue(value))));
    }
    __thawBind(instance, module, name) { const binding = { instance, name: 'import:' + module + '\x1f' + name }; this.__thawBindings.push(binding); return binding; }
    __thawUnbind(binding) { this.__thawBindings = this.__thawBindings.filter(value => value !== binding); }
    __thawSync() { if (this.__thawInstance !== undefined) return; const encoded = JSON.stringify(wasmEncodeValue(this.__thawLocalValue)); for (const binding of this.__thawBindings) wasmResult(__thaw_wasm_global(binding.instance, binding.name, encoded)); }
    __thawRefresh(binding) { if (!binding) return; this.__thawLocalValue = wasmDecodeValue(wasmResult(__thaw_wasm_global(binding.instance, binding.name, undefined)).value); const encoded = JSON.stringify(wasmEncodeValue(this.__thawLocalValue)); for (const other of this.__thawBindings) if (other !== binding) wasmResult(__thaw_wasm_global(other.instance, other.name, encoded)); }
    valueOf() { return this.value; }
  }
  class WasmTable {
    constructor(descriptor, value = null, internal = false) {
      if (internal) { this.__thawInstance = descriptor.instance; this.__thawName = descriptor.name; this.__thawBindings = []; return; }
      if (!descriptor || !['anyfunc','funcref','externref'].includes(String(descriptor.element))) throw new TypeError('WebAssembly.Table(): invalid element type');
      const initial = Number(descriptor.initial), maximum = descriptor.maximum === undefined ? Infinity : Number(descriptor.maximum);
      if (!Number.isInteger(initial) || initial < 0 || initial > maximum) throw new RangeError('WebAssembly.Table(): invalid table limits');
      this.__thawElement = descriptor.element === 'externref' ? 'externref' : 'funcref'; this.__thawMaximum = maximum;
      this.__thawValidate(value); this.__thawValues = Array(initial).fill(value); this.__thawBindings = [];
    }
    __thawValidate(value) { if (this.__thawElement === 'funcref' && value !== null && (typeof value !== 'function' || value.__thawWasmFuncref === undefined)) throw new TypeError('WebAssembly.Table(): funcref value must be null or an exported WebAssembly function'); }
    get length() { return this.__thawInstance === undefined ? this.__thawValues.length : wasmResult(__thaw_wasm_table(this.__thawInstance, this.__thawName, 'size', 0, undefined)).value; }
    get(index) { index = Number(index); if (!Number.isInteger(index) || index < 0 || index >= this.length) throw new RangeError('WebAssembly.Table.get(): invalid index'); return this.__thawInstance === undefined ? this.__thawValues[index] : wasmDecodeValue(wasmResult(__thaw_wasm_table(this.__thawInstance, this.__thawName, 'get', index, undefined)).value, this.__thawInstance); }
    set(index, value = null) { index = Number(index); if (!Number.isInteger(index) || index < 0 || index >= this.length) throw new RangeError('WebAssembly.Table.set(): invalid index'); this.__thawValidate(value); if (this.__thawInstance === undefined) { this.__thawValues[index] = value; this.__thawSync(); } else wasmResult(__thaw_wasm_table(this.__thawInstance, this.__thawName, 'set', index, JSON.stringify(wasmEncodeValue(value)))); }
    grow(delta, value = null) { delta = Number(delta); const previous = this.length; this.__thawValidate(value); if (!Number.isInteger(delta) || delta < 0 || (this.__thawMaximum !== undefined && previous + delta > this.__thawMaximum)) throw new RangeError('WebAssembly.Table.grow(): failed to grow table'); if (this.__thawInstance !== undefined) return wasmResult(__thaw_wasm_table(this.__thawInstance, this.__thawName, 'grow', delta, JSON.stringify(wasmEncodeValue(value))), RangeError).value; this.__thawValues.push(...Array(delta).fill(value)); for (const binding of this.__thawBindings) wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'grow', delta, JSON.stringify(wasmEncodeValue(value))), RangeError); return previous; }
    __thawBind(instance, module, name) { const binding = { instance, name: 'import:' + module + '\x1f' + name }; this.__thawBindings.push(binding); return binding; }
    __thawUnbind(binding) { this.__thawBindings = this.__thawBindings.filter(value => value !== binding); }
    __thawSync() { if (this.__thawInstance !== undefined) return; for (const binding of this.__thawBindings) { let size = wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'size', 0, undefined)).value; if (size < this.__thawValues.length) wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'grow', this.__thawValues.length - size, JSON.stringify(wasmEncodeValue(null))), RangeError); for (let index = 0; index < this.__thawValues.length; index++) wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'set', index, JSON.stringify(wasmEncodeValue(this.__thawValues[index])))); } }
    __thawRefresh(binding) { if (!binding || this.__thawInstance !== undefined) return; const size = wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'size', 0, undefined)).value, values = []; for (let index = 0; index < size; index++) values.push(wasmDecodeValue(wasmResult(__thaw_wasm_table(binding.instance, binding.name, 'get', index, undefined)).value, binding.instance)); this.__thawValues = values; this.__thawSync(); }
  }
  class WasmInstance {
    constructor(module, imports = {}) {
      if (!(module instanceof WasmModule)) throw new TypeError('WebAssembly.Instance(): argument 0 must be a WebAssembly.Module');
      if (imports === null || (typeof imports !== 'object' && typeof imports !== 'function')) throw new TypeError('WebAssembly.Instance(): imports must be an object');
      const wasi = imports && imports.wasi_snapshot_preview1 && imports.wasi_snapshot_preview1.__thawWasiOptions;
      const linkage = { wasi: wasi, functions: [], memories: [], globals: [], tables: [] }, pendingResources = [];
      for (const item of module.__thawImports) {
        if (item.module === 'wasi_snapshot_preview1' && wasi) continue;
        const namespace = imports[item.module];
        if (namespace === null || (typeof namespace !== 'object' && typeof namespace !== 'function')) throw new WebAssembly.LinkError(`WebAssembly import namespace '${item.module}' is not provided`);
        const value = namespace[item.name];
        if (item.kind === 'function' && typeof value !== 'function') throw new WebAssembly.LinkError(`WebAssembly import '${item.module}.${item.name}' must be a function`);
        if (item.kind === 'memory' && !(value instanceof WasmMemory)) throw new WebAssembly.LinkError(`WebAssembly import '${item.module}.${item.name}' must be a Memory`);
        if (item.kind === 'global' && !(value instanceof WasmGlobal)) throw new WebAssembly.LinkError(`WebAssembly import '${item.module}.${item.name}' must be a Global`);
        if (item.kind === 'table' && (!(value instanceof WasmTable) || value.__thawInstance !== undefined)) throw new WebAssembly.LinkError(`WebAssembly import '${item.module}.${item.name}' must be a standalone Table`);
      }
      try {
        for (const item of module.__thawImports) {
          if (item.module === 'wasi_snapshot_preview1' && wasi) continue;
          const value = imports[item.module][item.name];
          if (item.kind === 'function') linkage.functions.push({ module: item.module, name: item.name, handle: __thaw_wasm_retain_import(value) });
          else if (item.kind === 'memory') { value.__thawSync(); linkage.memories.push({ module: item.module, name: item.name, value: { data: wasmHex(value.buffer), maximum: value.__thawMaximum } }); pendingResources.push({ value, item }); }
          else if (item.kind === 'global') { linkage.globals.push({ module: item.module, name: item.name, value: wasmEncodeValue(value.value) }); pendingResources.push({ value, item }); }
          else if (item.kind === 'table') { linkage.tables.push({ module: item.module, name: item.name, value: { values: value.__thawValues.map(wasmEncodeValue), maximum: Number.isFinite(value.__thawMaximum) ? value.__thawMaximum : null } }); pendingResources.push({ value, item }); }
          else throw new WebAssembly.LinkError(`WebAssembly import '${item.module}.${item.name}' has an unsupported kind`);
        }
      } catch (error) { __thaw_wasm_release_pending(JSON.stringify(linkage.functions.map(value => value.handle))); throw error; }
      const rawResult = JSON.parse(__thaw_wasm_instantiate(module.__thawHandle, JSON.stringify(linkage)));
      if (!rawResult.ok) { __thaw_wasm_release_pending(JSON.stringify(linkage.functions.map(value => value.handle))); throw new WebAssembly.LinkError(rawResult.error); }
      const result = rawResult;
      Object.defineProperty(this, '__thawHandle', { value: result.handle });
      const exports = {}, resources = pendingResources.map(resource => ({ value: resource.value, binding: resource.value.__thawBind(this.__thawHandle, resource.item.module, resource.item.name) }));
      for (const item of result.exports) {
        if (item.kind === 'function') {
          const callable = (...args) => {
            for (const resource of resources) resource.value.__thawSync();
            const raw = JSON.parse(__thaw_wasm_call(this.__thawHandle, item.name, JSON.stringify(args.map(wasmEncodeValue))));
            for (const resource of resources) resource.value.__thawRefresh(resource.binding);
            if (raw.exit !== undefined) { const exit = new Error('WASI exited with code ' + raw.exit); exit.__thawWasiExit = raw.exit; throw exit; }
            const called = raw.ok ? raw : (() => { throw new WebAssembly.RuntimeError(raw.error); })();
            const values = called.values.map(value => wasmDecodeValue(value, this.__thawHandle));
            return values.length === 0 ? undefined : (values.length === 1 ? values[0] : values);
          };
          Object.defineProperty(callable, 'length', { value: item.parameters || 0 });
          const reference = wasmResult(__thaw_wasm_export_funcref(this.__thawHandle, item.name)).value;
          Object.defineProperty(callable, '__thawWasmInstance', { value: this.__thawHandle });
          Object.defineProperty(callable, '__thawWasmFuncref', { value: reference.v });
          Object.defineProperty(callable, '__thawWasmParameters', { value: item.parameterTypes || [] });
          Object.defineProperty(callable, '__thawWasmResults', { value: item.resultTypes || [] });
          wasmCacheFuncref(this.__thawHandle + ':' + reference.v, callable);
          exports[item.name] = callable;
        } else if (item.kind === 'memory') {
          const memory = new WasmMemory({ instance: this.__thawHandle, name: item.name }, true);
          resources.push({ value: memory }); exports[item.name] = memory;
        } else if (item.kind === 'global') exports[item.name] = new WasmGlobal({ instance: this.__thawHandle, name: item.name }, undefined, true);
        else if (item.kind === 'table') { const table = new WasmTable({ instance: this.__thawHandle, name: item.name }, null, true); resources.push({ value: table }); exports[item.name] = table; }
      }
      Object.defineProperty(this, 'exports', { value: Object.freeze(exports), enumerable: true });
      Object.defineProperty(this, '__thawResources', { value: resources });
      if (wasmFinalizer) wasmFinalizer.register(this, { kind: 'instance', handle: this.__thawHandle, resources });
    }
    dispose() { if (this.__thawDisposed) return; if (!__thaw_wasm_release('instance', this.__thawHandle)) return; this.__thawDisposed = true; for (const resource of this.__thawResources) if (resource.binding) resource.value.__thawUnbind(resource.binding); }
  }
  globalThis.WebAssembly = {
    CompileError: class CompileError extends Error { constructor(message) { super(message); this.name = 'CompileError'; } },
    LinkError: class LinkError extends Error { constructor(message) { super(message); this.name = 'LinkError'; } },
    RuntimeError: class RuntimeError extends Error { constructor(message) { super(message); this.name = 'RuntimeError'; } },
    Module: WasmModule,
    Instance: WasmInstance,
    Memory: WasmMemory,
    Global: WasmGlobal,
    Table: WasmTable,
    validate(bytes) { try { new WasmModule(bytes); return true; } catch (_) { return false; } },
    compile(bytes) { return Promise.resolve().then(() => new WasmModule(bytes)); },
    instantiate(source, imports) {
      return Promise.resolve().then(() => source instanceof WasmModule
        ? new WasmInstance(source, imports)
        : (() => { const module = new WasmModule(source); return { module, instance: new WasmInstance(module, imports) }; })());
    },
    compileStreaming(source) { return Promise.resolve(source).then(response => {
      if (!response || typeof response.arrayBuffer !== 'function') throw new TypeError('WebAssembly streaming source must be a Response');
      if (response.ok === false) throw new TypeError('WebAssembly streaming response was not successful');
      const contentType = response.headers && typeof response.headers.get === 'function' ? response.headers.get('content-type') : null;
      if (!contentType || String(contentType).split(';', 1)[0].trim().toLowerCase() !== 'application/wasm') throw new TypeError('WebAssembly streaming response has an unsupported MIME type');
      return response.arrayBuffer();
    }).then(bytes => new WasmModule(bytes)); },
    instantiateStreaming(source, imports) { return this.compileStreaming(source).then(module => ({ module, instance: new WasmInstance(module, imports) })); }
  };
  let nextTimerId = 1;
