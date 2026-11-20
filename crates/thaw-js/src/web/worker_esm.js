// ES-module entry for a worker: `__thaw_run_main_esm(href)` runs a `data:` JavaScript module (PR-3a) and returns its
// evaluation promise. The module's converted body (`__thaw_esm_convert`, esm.rs convert_entry) becomes an async factory
// compiled by `__thaw_esm_compile` with the module goal, so `import.meta` reads `__thaw_esm_meta`. Static imports are
// evaluated first, in source order, and the factory's `require` reads those results synchronously.
Object.defineProperty(globalThis, '__thaw_run_main_esm', {
  writable: true,
  configurable: true,
  value: href => {
    const failure = (Ctor, code, message) => {
      const error = new Ctor(message);
      error.code = code;
      return error;
    };
    // the builtins a worker module can import: only worker_threads (the worker's require serves no other module)
    const builtinNames = { 'node:worker_threads': 'node:worker_threads', worker_threads: 'node:worker_threads' };
    const resolve = (spec, parent) => {
      const name = String(spec);
      if (builtinNames[name] !== undefined) return builtinNames[name];
      if (name.startsWith('data:')) return name;
      if (name.startsWith('./') || name.startsWith('../') || name.startsWith('/') || name.startsWith('file:')) {
        throw failure(TypeError, 'ERR_UNSUPPORTED_RESOLVE_REQUEST', 'Failed to resolve module specifier "' + name + '" from "' + parent + '": relative and file: modules are not supported in a data: module worker yet');
      }
      if (name.startsWith('node:')) throw failure(Error, 'ERR_UNKNOWN_BUILTIN_MODULE', 'No such built-in module: ' + name);
      throw failure(Error, 'ERR_MODULE_NOT_FOUND', "Cannot find package '" + name + "' imported from " + parent);
    };
    const records = new Map();
    // a record: { url, specs, exports, meta, factory, state (0 new, 1 evaluating, 2 done), promise }; a builtin has no factory
    const load = url => {
      const known = records.get(url);
      if (known !== undefined) return known;
      if (url === 'node:worker_threads') {
        const record = { url, specs: [], exports: globalThis.__thaw_worker_side_exports, state: 2, promise: undefined };
        record.promise = Promise.resolve(record.exports);
        records.set(url, record);
        return record;
      }
      // data: payload, percent-decoded or base64
      const comma = url.indexOf(',');
      const head = url.slice('data:'.length, comma).split(';');
      const mime = head[0].trim().toLowerCase();
      if (mime === 'application/json') {
        throw failure(TypeError, 'ERR_IMPORT_ATTRIBUTE_MISSING', 'Module "' + url + '" needs an import attribute of "type: json"');
      }
      if (mime !== 'text/javascript' && mime !== 'application/javascript') {
        throw failure(RangeError, 'ERR_UNKNOWN_MODULE_FORMAT', 'Unknown module format: ' + mime + ' for URL ' + url);
      }
      const payload = decodeURIComponent(url.slice(comma + 1));
      const source = head.slice(1).some(p => p.trim().toLowerCase() === 'base64') ? Buffer.from(payload, 'base64').toString('utf8') : payload;
      const converted = JSON.parse(__thaw_esm_convert(source));
      const factory = __thaw_esm_compile('(async function (__thaw_esm_require, __thaw_esm_exports, __thaw_esm_meta, __thaw_esm_import) {\n' + converted.code + '\n})');
      const meta = Object.create(null);
      meta.url = url;
      const record = { url, specs: converted.imports, exports: {}, meta, factory, state: 0, promise: undefined };
      records.set(url, record);
      return record;
    };
    const evaluate = record => {
      if (record.promise === undefined) record.promise = run(record);
      return record.promise;
    };
    const run = async record => {
      record.state = 1;
      const deps = new Map();
      for (const spec of record.specs) {
        const dep = load(resolve(spec, record.url));
        deps.set(spec, dep);
        // a dependency still evaluating is a cycle: its exports are read as they are now
        if (dep.state !== 1) await evaluate(dep);
      }
      try {
        // `this` is undefined: the body's "use strict" makes the factory strict
        await record.factory.call(undefined, spec => deps.get(spec).exports, record.exports, record.meta, undefined);
      } finally {
        record.state = 2;
      }
    };
    return evaluate(load(String(href)));
  },
});
