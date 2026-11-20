// CommonJS loader for a file on disk: `__thaw_run_main_file(filename)` runs it as the main module with
// relative `require`, JSON, index files, cycles and a module cache. Adapted from thaw-quickjs runtime.js.
// Reads go through the host primitive `__thaw_worker_read_source(path)` (throws when missing).
Object.defineProperty(globalThis, '__thaw_run_main_file', {
  writable: true,
  configurable: true,
  value: filename => {
    const cache = Object.create(null);
    let mainModule;
    const normalize = path => {
      const absolute = path.startsWith('/'), parts = [];
      path.split('/').forEach(part => {
        if (!part || part === '.') return;
        if (part === '..') {
          if (parts.length && parts[parts.length - 1] !== '..') parts.pop();
          else if (!absolute) parts.push(part);
        } else parts.push(part);
      });
      return (absolute ? '/' : '') + parts.join('/');
    };
    const dirOf = path => {
      const slash = path.lastIndexOf('/');
      return slash < 0 ? '.' : slash === 0 ? '/' : path.slice(0, slash);
    };
    const notFound = request => {
      const error = new Error("Cannot find module '" + request + "'");
      error.code = 'MODULE_NOT_FOUND';
      return error;
    };
    const readFile = path => {
      try { return { path, source: globalThis.__thaw_worker_read_source(path) }; } catch (_) { return undefined; }
    };
    const loadFile = path => readFile(path) || readFile(path + '.js') || readFile(path + '.json');
    const loadIndex = path => readFile(path + '/index.js') || readFile(path + '/index.json');
    const loadDirectory = path => {
      const manifest = readFile(path + '/package.json');
      const main = manifest && JSON.parse(manifest.source).main;
      if (typeof main === 'string') {
        const target = normalize(path + '/' + main);
        return loadFile(target) || loadIndex(target) || loadIndex(path);
      }
      return loadIndex(path);
    };
    const find = (base, request) => {
      const target = normalize(request.startsWith('/') ? request : base + '/' + request);
      return loadFile(target) || loadDirectory(target);
    };
    const isPath = request => request.startsWith('./') || request.startsWith('../') || request.startsWith('/') || request === '.' || request === '..';
    const findInNodeModules = (base, request) => {
      const start = request.startsWith('@') ? request.indexOf('/') + 1 : 0;
      const slash = request.indexOf('/', start);
      const name = slash < 0 ? request : request.slice(0, slash);
      const subpath = slash < 0 ? '.' : request.slice(slash + 1);
      for (let dir = base; ; dir = dirOf(dir)) {
        if (dir.split('/').pop() !== 'node_modules') {
          const found = find(normalize(dir + '/node_modules/' + name), subpath);
          if (found) return found;
        }
        if (dir === '/') return undefined;
      }
    };
    const resolveRequest = (base, request) => isPath(request) ? find(base, request) : findInNodeModules(base, request);
    const load = (base, parent, request) => {
      request = String(request);
      if (!isPath(request)) {
        // Builtins first, as Node does; a missing builtin falls through to node_modules.
        const builtinRequire = globalThis.require;
        if (typeof builtinRequire === 'function') { try { return builtinRequire(request); } catch (_) {} }
      }
      const found = resolveRequest(base, request);
      if (!found) throw notFound(request);
      if (cache[found.path]) return cache[found.path].exports;
      const module = { id: found.path, filename: found.path, path: dirOf(found.path), exports: {}, loaded: false, children: [], parent };
      cache[found.path] = module;
      if (parent) parent.children.push(module);
      if (!parent) {
        mainModule = module;
        globalThis.process.mainModule = module;
      }
      try {
        if (found.path.endsWith('.json')) module.exports = JSON.parse(found.source);
        else {
          const require = name => load(module.path, module, name);
          require.resolve = name => {
            const resolved = resolveRequest(module.path, String(name));
            if (!resolved) throw notFound(name);
            return resolved.path;
          };
          require.cache = cache;
          require.main = mainModule;
          Function('module', 'exports', 'require', '__filename', '__dirname', found.source.replace(/^#![^\r\n  ]*/, ''))
            .call(module.exports, module, module.exports, require, found.path, module.path);
        }
        module.loaded = true;
      } catch (error) {
        delete cache[found.path];
        throw error;
      }
      return module.exports;
    };
    const absolute = String(filename).startsWith('/') ? String(filename) : globalThis.process.cwd() + '/' + String(filename);
    return load('/', null, absolute);
  },
});
