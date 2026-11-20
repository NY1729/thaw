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
    const codedError = (code, message) => {
      const error = new Error(message);
      error.code = code;
      return error;
    };
    const notFound = request => codedError('MODULE_NOT_FOUND', "Cannot find module '" + request + "'");
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
    // package.json "exports" / "imports" (Node's require conditions, `*` patterns, null targets).
    const matchMap = (map, key) => {
      if (Object.prototype.hasOwnProperty.call(map, key)) return { target: map[key], match: undefined };
      let best;
      for (const pattern of Object.keys(map)) {
        const star = pattern.indexOf('*');
        if (star < 0 || pattern.length <= (best ? best.pattern.length : 0)) continue;
        const prefix = pattern.slice(0, star), suffix = pattern.slice(star + 1);
        if (key.length >= pattern.length && key.startsWith(prefix) && key.endsWith(suffix)) {
          best = { pattern, target: map[pattern], match: key.slice(prefix.length, key.length - suffix.length) };
        }
      }
      return best;
    };
    const conditions = ['require', 'node', 'module-sync', 'default'];
    const resolveTarget = (target, match, pkgDir, file) => {
      if (typeof target === 'string') {
        if (!target.startsWith('./') || target.split('/').some(part => part === '..' || part === 'node_modules')) {
          throw codedError('ERR_INVALID_PACKAGE_TARGET', 'Invalid "exports" target "' + target + '" defined in ' + file);
        }
        return normalize(pkgDir + '/' + (match === undefined ? target : target.split('*').join(match)));
      }
      if (Array.isArray(target)) {
        let failure;
        for (const item of target) {
          try { const resolved = resolveTarget(item, match, pkgDir, file); if (resolved !== undefined) return resolved; } catch (error) { failure = error; }
        }
        if (failure) throw failure;
        return undefined;
      }
      if (target === null) return null;
      if (typeof target === 'object') {
        for (const condition of Object.keys(target)) {
          if (!conditions.includes(condition)) continue;
          const resolved = resolveTarget(target[condition], match, pkgDir, file);
          if (resolved !== undefined) return resolved;
        }
      }
      return undefined;
    };
    const resolveExports = (pkgDir, exportsField, key) => {
      const file = pkgDir + '/package.json';
      const map = typeof exportsField === 'string' || Array.isArray(exportsField) || !Object.keys(exportsField).some(k => k.startsWith('.'))
        ? { '.': exportsField } : exportsField;
      const hit = matchMap(map, key);
      const resolved = hit && resolveTarget(hit.target, hit.match, pkgDir, file);
      if (!resolved) throw codedError('ERR_PACKAGE_PATH_NOT_EXPORTED', 'Package subpath \'' + key + '\' is not defined by "exports" in ' + file);
      const found = readFile(resolved);
      if (!found) throw notFound(key);
      return found;
    };
    const findInNodeModules = (base, request) => {
      const start = request.startsWith('@') ? request.indexOf('/') + 1 : 0;
      const slash = request.indexOf('/', start);
      const name = slash < 0 ? request : request.slice(0, slash);
      const key = slash < 0 ? '.' : './' + request.slice(slash + 1);
      for (let dir = base; ; dir = dirOf(dir)) {
        if (dir.split('/').pop() !== 'node_modules') {
          const pkgDir = normalize(dir + '/node_modules/' + name);
          const manifest = readFile(pkgDir + '/package.json');
          const exportsField = manifest && JSON.parse(manifest.source).exports;
          if (exportsField !== undefined && exportsField !== null) return resolveExports(pkgDir, exportsField, key);
          const found = find(pkgDir, key);
          if (found) return found;
        }
        if (dir === '/') return undefined;
      }
    };
    const resolveImports = (base, request) => {
      let scope;
      for (let dir = base; !scope; dir = dirOf(dir)) {
        const manifest = readFile(normalize(dir + '/package.json'));
        if (manifest) scope = { dir, file: normalize(dir + '/package.json'), imports: JSON.parse(manifest.source).imports };
        if (dir === '/') break;
      }
      const hit = scope && scope.imports && matchMap(scope.imports, request);
      const resolved = hit && resolveTarget(hit.target, hit.match, scope.dir, scope.file);
      if (!resolved) throw codedError('ERR_PACKAGE_IMPORT_NOT_DEFINED', 'Package import specifier "' + request + '" is not defined in package ' + (scope ? scope.file : '(none)'));
      const found = readFile(resolved);
      if (!found) throw notFound(request);
      return found;
    };
    const resolveRequest = (base, request) => {
      if (isPath(request)) return find(base, request);
      if (request.startsWith('#')) return resolveImports(base, request);
      return findInNodeModules(base, request);
    };
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
