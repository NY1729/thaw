// ES-module loader for a worker entry and its file graph. `__thaw_run_main_esm(spec)` takes a `data:` or `file:` href,
// or an absolute path, and returns the evaluation promise of the entry. Each module is converted (`__thaw_esm_convert`,
// esm.rs convert_entry) into a factory compiled by `__thaw_esm_compile` with the module goal, so `import.meta` reads
// `__thaw_esm_meta` and `import(spec)` calls `__thaw_esm_import`, which resolves against the importing module and
// settles with its namespace. A CommonJS file runs through the disk loader (main_file.js) at its evaluation.
// `__thaw_path_is_esm(path)` tells the spawner which loader an entry path takes.
(() => {
  const read = path => globalThis.__thaw_worker_read_source(path);
  const dirOf = path => {
    const slash = path.lastIndexOf('/');
    return slash <= 0 ? '/' : path.slice(0, slash);
  };
  const pathOf = href => decodeURIComponent(new URL(href).pathname);
  const hrefOf = path => new URL(path, 'file:///').href;
  const failure = (Ctor, code, message) => {
    const error = new Ctor(message);
    error.code = code;
    return error;
  };
  // the factory of a converted module: async only when it awaits at its top level (`tla`), so a module without
  // awaits runs its body synchronously when it is evaluated
  const compileFactory = converted => __thaw_esm_compile((converted.tla ? '(async function' : '(function') +
    ' (__thaw_esm_require, __thaw_esm_exports, __thaw_esm_meta, __thaw_esm_import) {\n' + converted.code + '\n})');
  // the `type` of the nearest package.json (undefined when there is none)
  const packageType = path => {
    for (let dir = dirOf(path); ; dir = dirOf(dir)) {
      let text;
      try { text = read((dir === '/' ? '' : dir) + '/package.json'); } catch (_) {}
      if (text !== undefined) {
        try { return JSON.parse(text).type; } catch (_) { return undefined; }
      }
      if (dir === '/') return undefined;
    }
  };
  // .mjs is ESM, .cjs is CommonJS, .js follows its package's `type`
  const formatOf = path => {
    if (path.endsWith('.mjs')) return 'esm';
    if (path.endsWith('.js')) return packageType(path) === 'module' ? 'esm' : 'cjs';
    return 'cjs';
  };
  Object.defineProperty(globalThis, '__thaw_path_is_esm', {
    writable: true,
    configurable: true,
    value: path => formatOf(String(path)) === 'esm',
  });
  // the builtins a worker module can import: only worker_threads (the worker's require serves no other module)
  const builtinNames = { 'node:worker_threads': 'node:worker_threads', worker_threads: 'node:worker_threads' };
  const shown = url => (url.startsWith('file:') ? pathOf(url) : url);
  const resolve = (spec, parent) => {
    const name = String(spec);
    if (builtinNames[name] !== undefined) return builtinNames[name];
    if (name.startsWith('data:')) return name;
    if (name.startsWith('./') || name.startsWith('../') || name.startsWith('/') || name.startsWith('file:')) {
      if (!parent.startsWith('file:')) {
        throw failure(TypeError, 'ERR_UNSUPPORTED_RESOLVE_REQUEST', 'Failed to resolve module specifier "' + name + '" from "' + parent + '": relative and file: modules are not supported in a data: module worker yet');
      }
      return new URL(name, parent).href;
    }
    if (name.startsWith('node:')) throw failure(Error, 'ERR_UNKNOWN_BUILTIN_MODULE', 'No such built-in module: ' + name);
    throw failure(Error, 'ERR_MODULE_NOT_FOUND', "Cannot find package '" + name + "' imported from " + shown(parent));
  };
  // the CommonJS module at `path`, run by the disk loader; process.mainModule keeps the entry's value
  const runCjs = path => {
    const saved = globalThis.process.mainModule;
    try {
      return globalThis.__thaw_run_main_file(path);
    } finally {
      globalThis.process.mainModule = saved;
    }
  };
  const records = new Map();
  // a record: { url, path, kind ('esm' | 'cjs' | 'builtin'), specs, deps, linked, exports, meta, factory, state (0 new, 1
  // evaluating, 2 done), promise, namespace }; `deps` maps each specifier to its record
  const load = (url, parent) => {
    const known = records.get(url);
    if (known !== undefined) return known;
    let record;
    if (url === 'node:worker_threads') {
      record = { url, kind: 'builtin', specs: [], deps: new Map(), linked: true, exports: globalThis.__thaw_worker_side_exports, state: 2, promise: undefined };
      record.promise = Promise.resolve(record.exports);
    } else if (url.startsWith('file:')) {
      const path = pathOf(url);
      let source;
      try {
        source = read(path);
      } catch (_) {
        throw failure(Error, 'ERR_MODULE_NOT_FOUND', "Cannot find module '" + path + "'" + (parent ? " imported from " + shown(parent) : ''));
      }
      if (formatOf(path) === 'cjs') {
        record = { url, path, kind: 'cjs', specs: [], deps: new Map(), linked: false, exports: undefined, state: 0, promise: undefined };
      } else {
        const converted = JSON.parse(__thaw_esm_convert(source));
        const factory = compileFactory(converted);
        const meta = Object.create(null);
        meta.url = url;
        meta.filename = path;
        meta.dirname = dirOf(path);
        record = { url, path, kind: 'esm', specs: converted.imports, deps: new Map(), linked: false, exports: {}, meta, factory, state: 0, promise: undefined };
      }
    } else {
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
      const factory = compileFactory(converted);
      const meta = Object.create(null);
      meta.url = url;
      record = { url, kind: 'esm', specs: converted.imports, deps: new Map(), linked: false, exports: {}, meta, factory, state: 0, promise: undefined };
    }
    records.set(url, record);
    return record;
  };
  // Links the whole graph before anything runs (as Node does), so a missing or unparsable module fails first
  const link = (url, parent) => {
    const record = load(url, parent);
    if (!record.linked) {
      record.linked = true;
      for (const spec of record.specs) record.deps.set(spec, link(resolve(spec, url), url));
    }
    return record;
  };
  // Evaluates `record` after its dependencies, as Node's async module graph does: sibling dependencies start in source
  // order and may overlap while they await, and the record's body runs once all of them are done. `chain` holds the
  // records being evaluated above this one: a dependency found there is a cycle and is not waited for. Returns undefined
  // when the record finished synchronously (nothing awaited below it), else a promise (pending, or rejected).
  const evaluate = (record, chain) => {
    if (record.state !== 0) return record.promise;
    record.state = 1;
    const inner = chain.concat([record]);
    const waits = [];
    for (const dep of record.deps.values()) {
      if (inner.includes(dep)) continue;
      const pending = evaluate(dep, inner);
      if (pending !== undefined) waits.push(pending);
    }
    const done = () => {
      record.state = 2;
    };
    if (waits.length === 0) {
      try {
        const pending = body(record);
        if (pending === undefined) return done();
        record.promise = pending.then(done, error => {
          done();
          throw error;
        });
      } catch (error) {
        done();
        record.promise = Promise.reject(error);
      }
      return record.promise;
    }
    record.promise = Promise.all(waits).then(() => body(record)).then(done, error => {
      done();
      throw error;
    });
    return record.promise;
  };
  // runs one record's own code: a CommonJS module's body, or an ES-module factory (its promise when it awaits)
  const body = record => {
    if (record.kind === 'cjs') {
      record.exports = runCjs(record.path);
      return undefined;
    }
    if (record.kind === 'esm') {
      // `this` is undefined: the body's "use strict" makes the factory strict
      const deps = record.deps;
      return record.factory.call(undefined, spec => deps.get(spec).exports, record.exports, record.meta, spec => importFrom(record.url, spec));
    }
    return undefined;
  };
  // the namespace object `import()` settles with: an ES module's exports, or for CommonJS and builtins
  // `{ default: module.exports, ...names }` (null prototype, built once so every import sees one object)
  const namespaceOf = record => {
    if (record.kind === 'esm') return record.exports;
    if (record.namespace === undefined) {
      const ns = Object.create(null);
      const exports = record.exports;
      ns.default = exports;
      if (exports !== null && (typeof exports === 'object' || typeof exports === 'function')) {
        for (const key of Object.keys(exports)) if (key !== 'default') ns[key] = exports[key];
      }
      record.namespace = ns;
    }
    return record.namespace;
  };
  // `import(spec)` from the module at `parentUrl`: resolves and links, evaluates the target, and settles with its
  // namespace. It starts after a microtask, so the target never evaluates re-entrantly inside the importer's body.
  const importFrom = (parentUrl, spec) => (async () => {
    await undefined;
    const record = link(resolve(String(spec), parentUrl), parentUrl);
    const pending = evaluate(record, []);
    if (pending !== undefined) await pending;
    return namespaceOf(record);
  })();
  Object.defineProperty(globalThis, '__thaw_run_main_esm', {
    writable: true,
    configurable: true,
    value: spec => {
      const text = String(spec);
      const url = text.startsWith('/') ? hrefOf(text) : text;
      return (async () => {
        const pending = evaluate(link(url, undefined), []);
        if (pending !== undefined) await pending;
      })();
    },
  });
})();
