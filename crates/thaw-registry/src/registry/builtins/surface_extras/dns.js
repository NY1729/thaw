// Node's resolve*/reverse take (hostname, options) as promises (length 2), and dns.resolveTlsa(hostname, callback) has length 2;
// the shared wrappers use `arguments`, so set `length` here for dns.promises as well as for dns/promises.
(function () {
  var f = module.exports.resolveTlsa;
  if (typeof f === 'function' && f.length !== 2) Object.defineProperty(f, 'length', { value: 2, configurable: true });
  var P = module.exports.promises;
  if (!P) return;
  ['resolve', 'resolve4', 'resolve6', 'resolveAny', 'resolveCaa', 'resolveCname', 'resolveMx', 'resolveNaptr', 'resolveNs', 'resolvePtr', 'resolveSoa', 'resolveSrv', 'resolveTlsa', 'resolveTxt', 'reverse'].forEach(function (name) {
    if (typeof P[name] === 'function' && P[name].length !== 2) Object.defineProperty(P[name], 'length', { value: 2, configurable: true });
  });
})();
