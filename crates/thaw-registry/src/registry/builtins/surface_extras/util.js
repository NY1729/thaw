// util.MIMEType / util.MIMEParams (WHATWG MIME sniffing syntax): appended by builtins.rs `with_surface`.
(function () {
  var U = module.exports;
  var TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/, QUOTED = /^[\t\u0020-\u007e\u0080-\u00ff]*$/;
  function invalid(name, value) { var e = new TypeError('The MIME syntax for a ' + name + ' in "' + value + '" is invalid'); e.code = 'ERR_INVALID_MIME_SYNTAX'; return e; }
  function hide(o, k, v) { Object.defineProperty(o, k, { value: v, writable: true, configurable: true, enumerable: false }); }
  function MIMEParams() { hide(this, '_map', new Map()); }
  MIMEParams.prototype.delete = function (name) { this._map.delete(String(name).toLowerCase()); };
  MIMEParams.prototype.get = function (name) { var v = this._map.get(String(name).toLowerCase()); return v === undefined ? null : v; };
  MIMEParams.prototype.has = function (name) { return this._map.has(String(name).toLowerCase()); };
  MIMEParams.prototype.set = function (name, value) {
    name = String(name).toLowerCase(); value = String(value);
    if (!TOKEN.test(name)) throw invalid('parameter name', name);
    if (!QUOTED.test(value)) throw invalid('parameter value', value);
    this._map.set(name, value);
  };
  MIMEParams.prototype.entries = function () { return this._map.entries(); };
  MIMEParams.prototype.keys = function () { return this._map.keys(); };
  MIMEParams.prototype.values = function () { return this._map.values(); };
  MIMEParams.prototype[Symbol.iterator] = MIMEParams.prototype.entries;
  MIMEParams.prototype.toString = function () {
    var out = [];
    this._map.forEach(function (v, k) { out.push(k + '=' + (v === '' || !TOKEN.test(v) ? '"' + v.replace(/(["\\])/g, '\\$1') + '"' : v)); });
    return out.join(';');
  };
  MIMEParams.prototype.toJSON = MIMEParams.prototype.toString;
  function MIMEType(string) {
    if (!(this instanceof MIMEType)) throw new TypeError("Class constructor MIMEType cannot be invoked without 'new'");
    var input = String(string).replace(/^[ \t\n\r\f]+|[ \t\n\r\f]+$/g, ''), semi = input.indexOf(';'), essence = semi < 0 ? input : input.slice(0, semi), slash = essence.indexOf('/');
    if (slash < 0) throw invalid('type', input);
    var type = essence.slice(0, slash), subtype = essence.slice(slash + 1);
    if (!TOKEN.test(type)) throw invalid('type', input);
    if (!TOKEN.test(subtype.replace(/[ \t]+$/, ''))) throw invalid('subtype', input);
    hide(this, '_type', type.toLowerCase()); hide(this, '_subtype', subtype.replace(/[ \t]+$/, '').toLowerCase()); hide(this, '_params', new MIMEParams());
    var rest = semi < 0 ? '' : input.slice(semi + 1), re = /\s*([^=;]*)(?:=("(?:[^"\\]|\\.)*"|[^;]*))?(?:;|$)/g, m;
    while (rest && (m = re.exec(rest)) && m[0] !== '') {
      var name = m[1].toLowerCase(), value = m[2] === undefined ? '' : m[2];
      if (value.charAt(0) === '"') value = value.slice(1, -1).replace(/\\(.)/g, '$1');
      if (TOKEN.test(name) && QUOTED.test(value) && !this._params.has(name)) this._params._map.set(name, value);
    }
  }
  Object.defineProperties(MIMEType.prototype, {
    type: { get: function () { return this._type; }, set: function (v) { v = String(v); if (!TOKEN.test(v)) throw invalid('type', v); this._type = v.toLowerCase(); }, enumerable: true },
    subtype: { get: function () { return this._subtype; }, set: function (v) { v = String(v); if (!TOKEN.test(v)) throw invalid('subtype', v); this._subtype = v.toLowerCase(); }, enumerable: true },
    essence: { get: function () { return this._type + '/' + this._subtype; }, enumerable: true },
    params: { get: function () { return this._params; }, enumerable: true }
  });
  MIMEType.prototype.toString = function () { var p = this._params.toString(); return this.essence + (p ? ';' + p : ''); };
  MIMEType.prototype.toJSON = MIMEType.prototype.toString;
  U.MIMEType = MIMEType; U.MIMEParams = MIMEParams;
  // Deprecated type predicates and helpers Node 22 still exports, plus util.diff / parseEnv / the system error table.
  var ERR_TABLE = {"-7":["E2BIG","argument list too long"],"-13":["EACCES","permission denied"],"-98":["EADDRINUSE","address already in use"],"-99":["EADDRNOTAVAIL","address not available"],"-97":["EAFNOSUPPORT","address family not supported"],"-11":["EAGAIN","resource temporarily unavailable"],"-3000":["EAI_ADDRFAMILY","address family not supported"],"-3001":["EAI_AGAIN","temporary failure"],"-3002":["EAI_BADFLAGS","bad ai_flags value"],"-3013":["EAI_BADHINTS","invalid value for hints"],"-3003":["EAI_CANCELED","request canceled"],"-3004":["EAI_FAIL","permanent failure"],"-3005":["EAI_FAMILY","ai_family not supported"],"-3006":["EAI_MEMORY","out of memory"],"-3007":["EAI_NODATA","no address"],"-3008":["EAI_NONAME","unknown node or service"],"-3009":["EAI_OVERFLOW","argument buffer overflow"],"-3014":["EAI_PROTOCOL","resolved protocol is unknown"],"-3010":["EAI_SERVICE","service not available for socket type"],"-3011":["EAI_SOCKTYPE","socket type not supported"],"-114":["EALREADY","connection already in progress"],"-9":["EBADF","bad file descriptor"],"-16":["EBUSY","resource busy or locked"],"-125":["ECANCELED","operation canceled"],"-4080":["ECHARSET","invalid Unicode character"],"-103":["ECONNABORTED","software caused connection abort"],"-111":["ECONNREFUSED","connection refused"],"-104":["ECONNRESET","connection reset by peer"],"-89":["EDESTADDRREQ","destination address required"],"-17":["EEXIST","file already exists"],"-14":["EFAULT","bad address in system call argument"],"-27":["EFBIG","file too large"],"-113":["EHOSTUNREACH","host is unreachable"],"-4":["EINTR","interrupted system call"],"-22":["EINVAL","invalid argument"],"-5":["EIO","i/o error"],"-106":["EISCONN","socket is already connected"],"-21":["EISDIR","illegal operation on a directory"],"-40":["ELOOP","too many symbolic links encountered"],"-24":["EMFILE","too many open files"],"-90":["EMSGSIZE","message too long"],"-36":["ENAMETOOLONG","name too long"],"-100":["ENETDOWN","network is down"],"-101":["ENETUNREACH","network is unreachable"],"-23":["ENFILE","file table overflow"],"-105":["ENOBUFS","no buffer space available"],"-19":["ENODEV","no such device"],"-2":["ENOENT","no such file or directory"],"-12":["ENOMEM","not enough memory"],"-64":["ENONET","machine is not on the network"],"-92":["ENOPROTOOPT","protocol not available"],"-28":["ENOSPC","no space left on device"],"-38":["ENOSYS","function not implemented"],"-107":["ENOTCONN","socket is not connected"],"-20":["ENOTDIR","not a directory"],"-39":["ENOTEMPTY","directory not empty"],"-88":["ENOTSOCK","socket operation on non-socket"],"-95":["ENOTSUP","operation not supported on socket"],"-75":["EOVERFLOW","value too large for defined data type"],"-1":["EPERM","operation not permitted"],"-32":["EPIPE","broken pipe"],"-71":["EPROTO","protocol error"],"-93":["EPROTONOSUPPORT","protocol not supported"],"-91":["EPROTOTYPE","protocol wrong type for socket"],"-34":["ERANGE","result too large"],"-30":["EROFS","read-only file system"],"-108":["ESHUTDOWN","cannot send after transport endpoint shutdown"],"-29":["ESPIPE","invalid seek"],"-3":["ESRCH","no such process"],"-110":["ETIMEDOUT","connection timed out"],"-26":["ETXTBSY","text file is busy"],"-18":["EXDEV","cross-device link not permitted"],"-4094":["UNKNOWN","unknown error"],"-4095":["EOF","end of file"],"-6":["ENXIO","no such device or address"],"-31":["EMLINK","too many links"],"-112":["EHOSTDOWN","host is down"],"-121":["EREMOTEIO","remote I/O error"],"-25":["ENOTTY","inappropriate ioctl for device"],"-4028":["EFTYPE","inappropriate file type or format"],"-84":["EILSEQ","illegal byte sequence"],"-94":["ESOCKTNOSUPPORT","socket type not supported"],"-61":["ENODATA","no data available"],"-49":["EUNATCH","protocol driver not attached"],"-8":["ENOEXEC","exec format error"]};
  function defineUtil(name, fn) { if (typeof U[name] !== 'function') U[name] = fn; }
  function ownLength(fn, n) { Object.defineProperty(fn, 'length', { value: n, configurable: true }); return fn; }
  var DEPRECATED = [
    ["isBoolean","DEP0045","The `util.isBoolean` API is deprecated.  Please use `typeof arg === \"boolean\"` instead.",1],
    ["isBuffer","DEP0046","The `util.isBuffer` API is deprecated. Please use `Buffer.isBuffer()` instead.",1],
    ["isDate","DEP0047","The `util.isDate` API is deprecated.  Please use `arg instanceof Date` instead.",0],
    ["isError","DEP0048","The `util.isError` API is deprecated. Please use `ObjectPrototypeToString(e) === \"[object Error]\" || e instanceof Error` instead.",1],
    ["isFunction","DEP0049","The `util.isFunction` API is deprecated.  Please use `typeof arg === \"function\"` instead.",1],
    ["isNull","DEP0050","The `util.isNull` API is deprecated. Please use `arg === null` instead.",1],
    ["isNullOrUndefined","DEP0051","The `util.isNullOrUndefined` API is deprecated. Please use `arg === null || arg === undefined` instead.",1],
    ["isNumber","DEP0052","The `util.isNumber` API is deprecated. Please use `typeof arg === \"number\"` instead.",1],
    ["isObject","DEP0053","The `util.isObject` API is deprecated. Please use `arg !== null && typeof arg === \"object\"` instead.",1],
    ["isPrimitive","DEP0054","The `util.isPrimitive` API is deprecated. Please use `arg === null || (typeof arg !== \"object\" && typeof arg !== \"function\")` instead.",1],
    ["isRegExp","DEP0055","The `util.isRegExp` API is deprecated. Please use `arg instanceof RegExp` instead.",0],
    ["isString","DEP0056","The `util.isString` API is deprecated.  Please use `typeof arg === \"string\"` instead.",1],
    ["isSymbol","DEP0057","The `util.isSymbol` API is deprecated.  Please use `arg === \"symbol\"` instead.",1],
    ["isUndefined","DEP0058","The `util.isUndefined` API is deprecated. Please use `arg === undefined` instead.",1],
    ["_extend","DEP0060","The `util._extend` API is deprecated. Please use Object.assign() instead.",2]
  ];
  var predicates = {
    isBoolean: function (a) { return typeof a === 'boolean'; }, isBuffer: function (a) { return Buffer.isBuffer(a); },
    isDate: function (a) { return a instanceof Date; }, isError: function (e) { return Object.prototype.toString.call(e) === '[object Error]' || e instanceof Error; },
    isFunction: function (a) { return typeof a === 'function'; }, isNull: function (a) { return a === null; },
    isNullOrUndefined: function (a) { return a === null || a === undefined; }, isNumber: function (a) { return typeof a === 'number'; },
    isObject: function (a) { return a !== null && typeof a === 'object'; },
    isPrimitive: function (a) { return a === null || (typeof a !== 'object' && typeof a !== 'function'); },
    isRegExp: function (a) { return a instanceof RegExp; }, isString: function (a) { return typeof a === 'string'; },
    isSymbol: function (a) { return typeof a === 'symbol'; }, isUndefined: function (a) { return a === undefined; },
    _extend: function (target, source) { if (source === null || typeof source !== 'object') return target; var keys = Object.keys(source), i = keys.length; while (i--) target[keys[i]] = source[keys[i]]; return target; }
  };
  DEPRECATED.forEach(function (row) {
    var fn = predicates[row[0]];
    defineUtil(row[0], ownLength(U.deprecate ? U.deprecate(fn, row[2], row[1]) : fn, row[3]));
  });
  defineUtil('debug', U.debuglog);
  defineUtil('getSystemErrorMap', function getSystemErrorMap() { return new Map(Object.keys(ERR_TABLE).map(function (k) { return [Number(k), ERR_TABLE[k]]; })); });
  defineUtil('getSystemErrorMessage', function getSystemErrorMessage(err) {
    if (typeof err !== 'number') throw argTypeError('err', 'of type number', err);
    if (err >= 0 || !Number.isSafeInteger(err)) { var e = new RangeError('The value of "err" is out of range. It must be a negative integer. Received ' + (Number.isInteger(err) ? String(err).replace(/\B(?=(\d{3})+(?!\d))/g, '_') : String(err))); e.code = 'ERR_OUT_OF_RANGE'; throw e; }
    var entry = ERR_TABLE[err]; return entry ? entry[1] : 'Unknown system error ' + Math.max(-2147483648, Math.min(2147483647, err));
  });
  defineUtil('setTraceSigInt', function setTraceSigInt(enable) {
    if (typeof enable !== 'boolean') throw argTypeError('enable', 'of type boolean', enable);
  });
  function argTypeError(name, expected, value) { var e = new TypeError('The "' + name + '" argument must be ' + expected + '. Received ' + (value === null || value === undefined ? value : typeof value === 'function' ? 'function ' + value.name : typeof value === 'object' ? 'an instance of ' + (value.constructor && value.constructor.name || 'Object') : 'type ' + typeof value + ' (' + (typeof value === 'string' ? "'" + value + "'" : String(value)) + ')')); e.code = 'ERR_INVALID_ARG_TYPE'; return e; }
  defineUtil('parseEnv', function parseEnv(content) {
    if (typeof content !== 'string') throw argTypeError('content', 'of type string', content);
    var out = {}, text = content.replace(/\r\n?/g, '\n'), pos = 0, n = text.length;
    function eol(from) { var e = text.indexOf('\n', from); return e < 0 ? n : e; }
    while (pos < n) {
      var end = eol(pos), line = text.slice(pos, end), eq = line.indexOf('=');
      if (/^\s*(#|$)/.test(line) || eq < 0) { pos = end + 1; continue; }
      var key = line.slice(0, eq).trim().replace(/^export\s+/, '');
      pos += eq + 1;
      while (text.charAt(pos) === ' ' || text.charAt(pos) === '\t') pos++;
      var q = text.charAt(pos), value;
      if (q === '"' || q === "'" || q === '`') {
        var close = text.indexOf(q, pos + 1);
        if (close >= 0) {
          value = text.slice(pos + 1, close); if (q === '"') value = value.replace(/\\n/g, '\n');
          pos = eol(close);
        } else { var e2 = eol(pos); value = text.slice(pos, e2).replace(/#.*$/, '').trim(); pos = e2; }
      } else { var e3 = eol(pos); value = text.slice(pos, e3).replace(/#.*$/, '').trim(); pos = e3; }
      pos += 1;
      if (key) out[key] = value;
    }
    return out;
  });
  defineUtil('diff', function diff(actual, expected) {
    [['actual', actual], ['expected', expected]].forEach(function (pair) {
      var value = pair[1];
      if (typeof value === 'string') return;
      if (!Array.isArray(value)) throw argTypeError(pair[0], 'of type string', value);
      for (var i = 0; i < value.length; i++) if (typeof value[i] !== 'string') throw argTypeError(pair[0] + '[' + i + ']', 'of type string', value[i]);
    });
    var a = typeof actual === 'string' ? Array.from(actual) : actual, b = typeof expected === 'string' ? Array.from(expected) : expected;
    if (typeof actual === 'string' && actual === expected) return [];
    var n = a.length, m = b.length, max = n + m, offset = max, v = new Array(2 * max + 2).fill(0), trace = [];
    // Myers shortest edit script, forward pass keeping every frontier
    for (var d = 0; d <= max; d++) {
      trace.push(v.slice());
      for (var k = -d; k <= d; k += 2) {
        var x = (k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])) ? v[offset + k + 1] : v[offset + k - 1] + 1, y = x - k;
        while (x < n && y < m && a[x] === b[y]) { x++; y++; }
        v[offset + k] = x;
        if (x >= n && y >= m) { d = max + 1; break; }
      }
    }
    var out = [], x2 = n, y2 = m;
    for (var step = trace.length - 1; step > 0; step--) {
      var vv = trace[step], dd = step, kk = x2 - y2;
      var prevK = (kk === -dd || (kk !== dd && vv[offset + kk - 1] < vv[offset + kk + 1])) ? kk + 1 : kk - 1;
      var px = vv[offset + prevK], py = px - prevK;
      while (x2 > px && y2 > py) { out.push([0, a[x2 - 1]]); x2--; y2--; }
      if (x2 === px) out.push([-1, b[y2 - 1]]); else out.push([1, a[x2 - 1]]);
      x2 = px; y2 = py;
    }
    while (x2 > 0 && y2 > 0) { out.push([0, a[x2 - 1]]); x2--; y2--; }
    return out.reverse();
  });
})();
