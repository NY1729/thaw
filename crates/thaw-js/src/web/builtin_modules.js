// The worker realm's Node builtin modules: the CommonJS sources of thaw-node-builtins, read through the host primitive
// `__thaw_builtin_source(name)`. `__thaw_builtin_exports(name)` (the `node:` prefix is optional) evaluates a source once
// per realm and returns its module.exports; a source's own `require` resolves through this same table. Unknown names
// give undefined.
(() => {
  const cache = new Map();
  const load = name => {
    const known = cache.get(name);
    if (known !== undefined) return known.exports;
    const source = globalThis.__thaw_builtin_source(name);
    if (source === undefined) return undefined;
    const module = { exports: {} };
    cache.set(name, module);
    const require = spec => load(String(spec).replace(/^node:/, ''));
    __thaw_esm_compile('(function (module, exports, require) {\n' + source + '\n})')(module, module.exports, require);
    return module.exports;
  };
  Object.defineProperty(globalThis, '__thaw_builtin_exports', {
    writable: true,
    configurable: true,
    value: name => load(String(name).replace(/^node:/, '')),
  });
})();
