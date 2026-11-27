// util.MIMEType / util.MIMEParams (WHATWG MIME sniffing syntax): appended by builtins.rs `with_surface`.
(function () {
  var U = module.exports;
  var TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/, QUOTED = /^[\t\u0020-\u007e\u0080-\u00ff]*$/;
  function invalid(name, value) { var e = new TypeError('The MIME syntax for a ' + name + ' in "' + value + '" is invalid'); e.code = 'ERR_INVALID_MIME_SYNTAX'; return e; }
  function MIMEParams() { Object.defineProperty(this, '_map', { value: new Map() }); }
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
    Object.defineProperties(this, { _type: { value: type.toLowerCase(), writable: true }, _subtype: { value: subtype.replace(/[ \t]+$/, '').toLowerCase(), writable: true }, _params: { value: new MIMEParams() } });
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
})();
