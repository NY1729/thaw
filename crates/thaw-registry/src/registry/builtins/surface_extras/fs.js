// node:fs members that are pure logic: appended by builtins.rs `with_surface`.
(function () {
  var F = module.exports;
  function received(value) {
    if (value === null || value === undefined) return String(value);
    if (typeof value === 'function') return 'function ' + value.name;
    if (typeof value === 'object') return 'an instance of ' + (value.constructor && value.constructor.name || 'Object');
    return 'type ' + typeof value + ' (' + (typeof value === 'string' ? "'" + value + "'" : String(value)) + ')';
  }
  F._toUnixTimestamp = function _toUnixTimestamp(time, name = 'time') {
    if (typeof time === 'string' && +time == time) return +time;
    if (Number.isFinite(time)) return time < 0 ? Date.now() / 1000 : time;
    if (time instanceof Date) return time.getTime() / 1000;
    var e = new TypeError('The "' + name + '" argument must be an instance of Date or an Time in seconds. Received ' + received(time));
    e.code = 'ERR_INVALID_ARG_TYPE';
    throw e;
  };
  // legacy aliases of the stream classes (settable accessors in Node)
  [['FileReadStream', 'ReadStream'], ['FileWriteStream', 'WriteStream']].forEach(function (pair) {
    var value;
    Object.defineProperty(F, pair[0], { get: function () { return value === undefined ? F[pair[1]] : value; }, set: function (v) { value = v; }, enumerable: true, configurable: true });
  });
})();
