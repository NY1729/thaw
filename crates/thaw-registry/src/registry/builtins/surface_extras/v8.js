// node:v8 members that need no engine hooks: appended by builtins.rs `with_surface`.
(function () {
  var V = module.exports;
  function notBuilding() { var e = new Error('Not building snapshot'); e.code = 'ERR_NOT_BUILDING_SNAPSHOT'; return e; }
  V.isStringOneByteRepresentation = function isStringOneByteRepresentation(content) {
    if (typeof content !== 'string') { var e = new TypeError('The "content" argument must be of type string. Received ' + typeof content); e.code = 'ERR_INVALID_ARG_TYPE'; throw e; }
    for (var i = 0; i < content.length; i++) if (content.charCodeAt(i) > 0xff) return false;
    return true;
  };
  V.getCppHeapStatistics = function getCppHeapStatistics(detailLevel) {
    return { committed_size_bytes: 0, resident_size_bytes: 0, used_size_bytes: 0, space_statistics: [], type_names: [], detail_level: detailLevel === 'brief' ? 'brief' : 'detailed' };
  };
  V.setHeapSnapshotNearHeapLimit = function setHeapSnapshotNearHeapLimit(limit) {
    if (typeof limit !== 'number' || !Number.isInteger(limit) || limit < 1) { var e = new RangeError('The value of "limit" is out of range. It must be >= 1. Received ' + String(limit)); e.code = 'ERR_OUT_OF_RANGE'; throw e; }
  };
  // No NODE_V8_COVERAGE collection in thaw: like Node without it, these are no-ops.
  V.takeCoverage = function takeCoverage() {};
  V.stopCoverage = function stopCoverage() {};
  // Node outside a snapshot build: isBuildingSnapshot() is false and the callbacks throw ERR_NOT_BUILDING_SNAPSHOT.
  V.startupSnapshot = {
    addSerializeCallback: function addSerializeCallback() { throw notBuilding(); },
    addDeserializeCallback: function addDeserializeCallback() { throw notBuilding(); },
    setDeserializeMainFunction: function setDeserializeMainFunction() { throw notBuilding(); },
    isBuildingSnapshot: function isBuildingSnapshot() { return false; }
  };
  function GCProfiler() { if (!(this instanceof GCProfiler)) throw new TypeError("Class constructor GCProfiler cannot be invoked without 'new'"); this._start = undefined; }
  GCProfiler.prototype.start = function start() { if (this._start === undefined) this._start = Date.now(); };
  GCProfiler.prototype.stop = function stop() {
    if (this._start === undefined) return undefined;
    var data = { version: 1, startTime: this._start, statistics: [], endTime: Date.now() };
    this._start = undefined;
    return data;
  };
  V.GCProfiler = GCProfiler;
})();
