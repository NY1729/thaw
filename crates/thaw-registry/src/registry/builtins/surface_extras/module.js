// node:module members that are state or pure path logic (no loader hooks): appended by builtins.rs `with_surface`.
(function () {
  var M = module.exports;
  // no node:path/fs/url imports here: they would pull those builtins into every bundle that uses node:module
  function dirname(p) { var i = p.lastIndexOf('/'); return i <= 0 ? '/' : p.slice(0, i); }
  function absolute(p) {
    var parts = [];
    String(p.charAt(0) === '/' ? p : process.cwd() + '/' + p).split('/').forEach(function (s) { if (s === '..') parts.pop(); else if (s && s !== '.') parts.push(s); });
    return '/' + parts.join('/');
  }
  function argError(name, expected, value) {
    var e = new TypeError('The "' + name + '" argument must be ' + expected + '. Received ' + (value === null ? 'null' : typeof value));
    e.code = 'ERR_INVALID_ARG_TYPE';
    return e;
  }
  var support = { enabled: false, nodeModules: false, generatedCode: false };
  M.setSourceMapsSupport = function setSourceMapsSupport(enabled, options) {
    if (typeof enabled !== 'boolean') throw argError('enabled', 'of type boolean', enabled);
    options = options || {};
    support = { enabled: enabled, nodeModules: Boolean(options.nodeModules), generatedCode: Boolean(options.generatedCode) };
  };
  M.getSourceMapsSupport = function getSourceMapsSupport() { return { enabled: support.enabled, nodeModules: support.nodeModules, generatedCode: support.generatedCode }; };
  // thaw keeps no on-disk compile cache: report Node's DISABLED status instead of pretending to enable one.
  M.enableCompileCache = function enableCompileCache(options) { return { status: M.constants.compileCacheStatus.DISABLED, message: 'thaw does not use a compile cache' }; };
  M.getCompileCacheDir = function getCompileCacheDir() { return undefined; };
  M.flushCompileCache = function flushCompileCache() {};
  var home = (typeof process !== 'undefined' && process.env && process.env.HOME) || '';
  M.globalPaths = (home ? [home + '/.node_modules', home + '/.node_libraries'] : []).concat([dirname(dirname(process.execPath || '/usr/bin/node')) + '/lib/node']);
  M._pathCache = Object.create(null);
  M._debug = function () {};
  M._initPaths = function _initPaths() {};
  M._preloadModules = function _preloadModules(requests) {};
  M._nodeModulePaths = function _nodeModulePaths(from) {
    from = absolute(String(from));
    if (from === '/') return ['/node_modules'];
    var out = [], parts = from.split('/');
    for (var i = parts.length; i > 0; i--) {
      if (parts[i - 1] === 'node_modules') continue;
      out.push(parts.slice(0, i).join('/') + '/node_modules');
    }
    out.push('/node_modules');
    return out;
  };
  M._load = function _load(request, parent, isMain) {
    var base = parent && parent.filename ? parent.filename : process.cwd() + '/';
    return M.createRequire(base)(request);
  };
  M._resolveLookupPaths = function _resolveLookupPaths(request, parent) {
    if (M.isBuiltin(request)) return null;
    var dir = parent && parent.filename ? dirname(parent.filename) : process.cwd();
    if (request.charAt(0) === '.') return [dir];
    return (parent && parent.paths ? parent.paths : M._nodeModulePaths(dir)).concat(M.globalPaths);
  };
  M._resolveFilename = function _resolveFilename(request, parent, isMain, options) {
    if (M.isBuiltin(request)) return request;
    var base = parent && parent.filename ? parent.filename : process.cwd() + '/';
    return M.createRequire(base).resolve(request, options);
  };
  M._findPath = function _findPath(request, paths, isMain) {
    var abs = request.charAt(0) === '/', list = abs ? [''] : paths;
    for (var i = 0; i < list.length; i++) {
      try { return M.createRequire((list[i] || '') + '/noop.js').resolve(abs ? request : './' + request); } catch (e) {}
    }
    return false;
  };
  // Module.wrap / wrapper, and the prototype methods Node defines on every Module.
  var wrapper = ['(function (exports, require, module, __filename, __dirname) { ', '\n});'];
  // accessors in Node: reachable as Module.wrap, but not named exports of the ESM namespace
  Object.defineProperty(M, 'wrapper', { value: wrapper, writable: true, configurable: true, enumerable: false });
  Object.defineProperty(M, 'wrap', { value: function wrap(script) { return wrapper[0] + script + wrapper[1]; }, writable: true, configurable: true, enumerable: false });
  var MP = M.prototype;
  Object.defineProperty(MP, 'isPreloading', { get: function isPreloading() { return false; }, configurable: true });
  MP.require = function require(id) {
    if (typeof id !== 'string') throw argError('id', 'of type string', id);
    if (id === '') { var e = new TypeError("The argument 'id' must be a non-empty string. Received ''"); e.code = 'ERR_INVALID_ARG_VALUE'; throw e; }
    return M.createRequire(this.filename || process.cwd() + '/')(id);
  };
  MP.load = function load(filename) {
    if (typeof filename !== 'string') throw argError('filename', 'of type string', filename);
    this.filename = filename; this.paths = M._nodeModulePaths(dirname(filename));
    this.exports = M.createRequire(filename)(filename); this.loaded = true;
  };
  MP._compile = function _compile(content, filename) {
    var fn = (0, eval)(M.wrap(content)), req = M.createRequire(filename || process.cwd() + '/');
    return fn.call(this.exports, this.exports, req, this, filename, dirname(filename || ''));
  };
  // nearest package.json of what `specifier` names from `base`; existence checks go through the host's fs call, because
  // this bundle must not import node:fs
  function fileExists(p) {
    try { return typeof __thaw_fs === 'function' && JSON.parse(__thaw_fs('exists', p, '', false)).exists === true; } catch (e) { return false; }
  }
  M.findPackageJSON = function findPackageJSON(specifier, base = 'data:') {
    if (arguments.length === 0) { var m = new TypeError('The "specifier" argument must be specified'); m.code = 'ERR_MISSING_ARGS'; throw m; }
    specifier = String(specifier);
    if (typeof base === 'object' && base !== null && typeof base.href === 'string') base = base.href;
    if (typeof base !== 'string') throw argError('base', 'of type string', base);
    if (base.indexOf('file://') === 0) base = decodeURIComponent(base.slice(7));
    if (specifier.indexOf('file://') === 0) specifier = decodeURIComponent(specifier.slice(7));
    var baseDir = base.charAt(0) === '/' ? dirname(base) : process.cwd();
    function missing() { var e = new Error("Cannot find module '" + specifier + "' imported from " + base); e.code = 'ERR_MODULE_NOT_FOUND'; return e; }
    if (specifier.charAt(0) !== '.' && specifier.charAt(0) !== '/') {
      var parts = specifier.split('/'), name = specifier.charAt(0) === '@' ? parts.slice(0, 2).join('/') : parts[0];
      for (var dir = baseDir; ; dir = dirname(dir)) {
        var candidate = (dir === '/' ? '' : dir) + '/node_modules/' + name + '/package.json';
        if (fileExists(candidate)) return candidate;
        if (dir === '/') throw missing();
      }
    }
    var resolved = absolute(specifier.charAt(0) === '/' ? specifier : baseDir + '/' + specifier);
    if (!fileExists(resolved)) throw missing();
    for (var up = dirname(resolved); ; up = dirname(up)) {
      var file = (up === '/' ? '' : up) + '/package.json';
      if (fileExists(file)) return file;
      if (up === '/') return undefined;
    }
  };
  // 0 = file, 1 = directory, -2 = missing (libuv ENOENT), as Module._stat
  Object.defineProperty(M, '_stat', { value: function _stat(filename) {
    try { var r = JSON.parse(__thaw_fs('stat', String(filename), '', false)); return r.ok ? (r.directory ? 1 : 0) : -2; } catch (e) { return -2; }
  }, writable: true, configurable: true, enumerable: false });
  M.runMain = function runMain(main) { return M._load(main || process.argv[1], null, true); };
})();
