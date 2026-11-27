// node:fs/promises members that fail by design on Linux: appended by builtins.rs `with_surface`.
(function () {
  var P = module.exports;
  if (typeof P.lchmod !== 'function') {
    P.lchmod = async function lchmod(path, mode) {
      var e = new Error('The lchmod() method is not implemented');
      e.code = 'ERR_METHOD_NOT_IMPLEMENTED';
      throw e;
    };
  }
})();
