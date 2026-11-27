// dns.promises.lookupService: reverse lookup of the address plus the service name of the port.
(function () {
  var P = module.exports;
  // getnameinfo's service name for the common well-known TCP ports (thaw has no /etc/services reader); else the number.
  var SERVICES = { 21: 'ftp', 22: 'ssh', 23: 'telnet', 25: 'smtp', 53: 'domain', 80: 'http', 110: 'pop3', 143: 'imap', 443: 'https', 465: 'submissions', 587: 'submission', 993: 'imaps', 995: 'pop3s' };
  function serviceName(port) { return SERVICES[port] || String(port); }
  P.lookupService = function lookupService(address, port) {
    if (arguments.length < 2) { var e = new TypeError('The "address" and "port" arguments must be specified'); e.code = 'ERR_MISSING_ARGS'; throw e; }
    var n = Number(port);
    if (!Number.isInteger(n) || n < 0 || n > 65535) { var r = new RangeError('Port should be >= 0 and < 65536. Received ' + String(port)); r.code = 'ERR_SOCKET_BAD_PORT'; throw r; }
    return P.reverse(String(address)).then(function (names) { return names[0]; }, function () { return String(address); })
      .then(function (hostname) { return { hostname: hostname, service: serviceName(n) }; });
  };
})();
// Node's resolve*/reverse take (hostname, options): length 2 (the shared implementation uses `arguments`).
(function () {
  var P = module.exports;
  ['resolve', 'resolve4', 'resolve6', 'resolveAny', 'resolveCaa', 'resolveCname', 'resolveMx', 'resolveNaptr', 'resolveNs', 'resolvePtr', 'resolveSoa', 'resolveSrv', 'resolveTxt', 'reverse'].forEach(function (name) {
    if (typeof P[name] === 'function' && P[name].length !== 2) Object.defineProperty(P[name], 'length', { value: 2, configurable: true });
  });
})();
