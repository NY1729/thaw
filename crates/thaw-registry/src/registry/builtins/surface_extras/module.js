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
  M.globalPaths = home ? [home + '/.node_modules', home + '/.node_libraries'] : [];
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
  M.runMain = function runMain(main) { return M._load(main || process.argv[1], null, true); };
})();
