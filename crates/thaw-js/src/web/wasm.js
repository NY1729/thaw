(function () {
  if (typeof globalThis.WebAssembly === 'object') return;
  // ponytail: honest stub, no wasm engine here. The scan keeps any real use of WebAssembly on retired-engine; only typeof/in probes pass.
  const unsupported = () => new Error('WebAssembly is not supported on this runtime');
  const WebAssembly = {
    validate() { throw unsupported(); },
    compile() { return Promise.reject(unsupported()); },
    instantiate() { return Promise.reject(unsupported()); },
  };
  for (const name of ['CompileError', 'LinkError', 'RuntimeError']) {
    const C = ({ [name]: class extends Error {} })[name];
    Object.defineProperty(C.prototype, 'name', { value: name, writable: true, configurable: true });
    WebAssembly[name] = C;
  }
  for (const name of ['Module', 'Instance', 'Memory', 'Table', 'Global']) {
    WebAssembly[name] = ({ [name]: class { constructor() { throw unsupported(); } } })[name];
  }
  // Member shapes as the real namespace defines them (names, lengths, attributes); every one throws.
  const shape = (target, source) => {
    for (const key of Reflect.ownKeys(source)) {
      Object.defineProperty(target, key, { ...Object.getOwnPropertyDescriptor(source, key), enumerable: true, configurable: true });
    }
  };
  shape(WebAssembly.Module, {
    customSections(module, name) { throw unsupported(); },
    exports(module) { throw unsupported(); },
    imports(module) { throw unsupported(); },
  });
  shape(WebAssembly.Instance.prototype, { get exports() { throw unsupported(); } });
  shape(WebAssembly.Memory.prototype, {
    grow(delta) { throw unsupported(); },
    get buffer() { throw unsupported(); },
  });
  shape(WebAssembly.Table.prototype, {
    get length() { throw unsupported(); },
    get(index) { throw unsupported(); },
    grow(delta) { throw unsupported(); },
    set(index) { throw unsupported(); },
  });
  shape(WebAssembly.Global.prototype, {
    valueOf() { throw unsupported(); },
    get value() { throw unsupported(); },
    set value(value) { throw unsupported(); },
  });
  for (const name of ['CompileError', 'LinkError', 'RuntimeError']) {
    Object.defineProperty(WebAssembly[name].prototype, 'message', { value: '', writable: true, configurable: true });
  }
  Object.defineProperty(globalThis, 'WebAssembly', { value: WebAssembly, writable: true, configurable: true });
})();
