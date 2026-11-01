(function () {
  if (typeof globalThis.WebAssembly === 'object') return;
  // ponytail: honest stub, no wasm engine here. The scan keeps any real use of WebAssembly on QuickJS; only typeof/in probes pass.
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
  Object.defineProperty(globalThis, 'WebAssembly', { value: WebAssembly, writable: true, configurable: true });
})();
