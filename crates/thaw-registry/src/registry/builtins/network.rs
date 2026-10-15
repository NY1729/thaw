pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "dgram" => Some(
            r#"function Socket(type, listener) {
  if (!(this instanceof Socket)) return new Socket(type, listener);
  var options = typeof type === 'object' ? type : { type: type };
  this.type = options.type || 'udp4';
  if (this.type !== 'udp4' && this.type !== 'udp6') throw new TypeError('Bad socket type');
  this._events = Object.create(null);
  this._handle = 0;
  this._address = null;
  this._remote = null;
  this._refed = true;
  this._closed = false;
  this._receiveTimer = null;
  if (typeof listener === 'function') this.on('message', listener);
}
Socket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; };
Socket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; };
Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; };
Socket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; };
function portNumber(value, allowZero) {
  var port = Number(value);
  if (!Number.isInteger(port) || port < (allowZero ? 0 : 1) || port > 65535) throw new RangeError('Invalid UDP port');
  return port;
}
Socket.prototype._checkOpen = function() { if (this._closed) throw new Error('Socket is closed'); };
function nativeError(outcome) {
  var separator = outcome.indexOf('|', 4);
  var error = new Error(outcome.substring(separator + 1));
  error.code = outcome.substring(4, separator);
  return error;
}
Socket.prototype._startReceive = function() {
  var socket = this, retryDelay = 1;
  if (this._receiveTimer !== null || !this._handle) return;
  function schedule(delay) {
    if (!socket._handle || socket._closed) return;
    socket._receiveTimer = setTimeout(pump, delay || 1);
    if (!socket._refed) socket._receiveTimer.unref();
  }
  function pump() {
    socket._receiveTimer = null;
    if (!socket._handle || socket._closed) return;
    var incoming = __thaw_udp_receive(socket._handle);
    if (incoming !== 'pending') {
      if (incoming.indexOf('ok|') !== 0) {
        socket.emit('error', nativeError(incoming));
        retryDelay = Math.min(retryDelay * 2, 1000);
        schedule(retryDelay);
        return;
      } else {
        retryDelay = 1;
        var parts = incoming.split('|'), message = Buffer.from(parts[1], 'hex');
        socket.emit('message', message, { address: parts[2], family: parts[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(parts[3]), size: Number(parts[4]) });
      }
    }
    retryDelay = 1;
    schedule();
  }
  schedule();
};
Socket.prototype._bind = function(host, port) {
  this._checkOpen();
  if (this._handle) { var boundError = new Error('Socket is already bound'); boundError.code = 'ERR_SOCKET_ALREADY_BOUND'; throw boundError; }
  var outcome = __thaw_udp_bind(host, port);
  if (outcome.indexOf('ok|') !== 0) return outcome;
  var fields = outcome.split('|');
  this._handle = Number(fields[1]);
  this._address = { address: fields[2], family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: Number(fields[3]) };
  var socket = this;
  queueMicrotask(function() { if (!socket._closed) socket.emit('listening'); });
  this._startReceive();
  return outcome;
};
Socket.prototype.bind = function(port, address, callback) {
  if (typeof port === 'function') { callback = port; port = undefined; address = undefined; } else if (typeof address === 'function') { callback = address; address = undefined; }
  var options = typeof port === 'object' && port !== null ? port : { port: port, address: address };
  var number = portNumber(options.port == null ? 0 : options.port, true);
  var host = String(options.address || (this.type === 'udp6' ? '::' : '0.0.0.0'));
  this._checkOpen();
  if (this._handle) { var alreadyBound = new Error('Socket is already bound'); alreadyBound.code = 'ERR_SOCKET_ALREADY_BOUND'; throw alreadyBound; }
  var outcome = this._bind(host, number);
  if (outcome.indexOf('ok|') === 0 && typeof callback === 'function') this.once('listening', callback);
  if (outcome.indexOf('ok|') !== 0) {
    var error = nativeError(outcome);
    queueMicrotask(() => this.emit('error', error));
  }
  return this;
};
Socket.prototype.send = function(message) {
  this._checkOpen();
  var args = Array.prototype.slice.call(arguments, 1);
  var callback = typeof args[args.length - 1] === 'function' ? args.pop() : null;
  var connected = this._remote !== null;
  var sliced = typeof args[0] === 'number' && typeof args[1] === 'number' && (connected || args.length >= 3);
  if (sliced) {
    var offset = args.shift(), length = args.shift();
    var original = Buffer.from(message);
    if (!Number.isInteger(offset) || !Number.isInteger(length) || offset < 0 || length < 0 || offset + length > original.length) throw new RangeError('Invalid UDP buffer range');
    message = original.subarray(offset, offset + length);
  }
  var port, address;
  if (connected) {
    if (args.length) throw new TypeError('Connected socket does not take a destination');
    port = this._remote.port; address = this._remote.address;
  } else {
    port = portNumber(args.shift(), false);
    address = args.length && args[0] != null ? String(args.shift()) : (this.type === 'udp6' ? '::1' : '127.0.0.1');
  }
  var buffer = Array.isArray(message) ? Buffer.concat(message.map(function(value) { return Buffer.from(value); })) : Buffer.from(message);
  if (!this._handle) {
    var bound = this._bind(this.type === 'udp6' ? '::' : '0.0.0.0', 0);
    if (bound.indexOf('ok|') !== 0) {
      var bindError = nativeError(bound), socket = this;
      queueMicrotask(function() { if (callback) callback(bindError); else socket.emit('error', bindError); });
      return this;
    }
  }
  var outcome = __thaw_udp_send(this._handle, buffer.toString('hex'), address, port);
  if (outcome.indexOf('ok|') !== 0) {
    var error = nativeError(outcome);
    if (callback) queueMicrotask(function() { callback(error); }); else queueMicrotask(() => this.emit('error', error));
  } else if (callback) queueMicrotask(function() { callback(null, Number(outcome.substring(3))); });
  return this;
};
Socket.prototype.connect = function(port, address, callback) {
  this._checkOpen();
  if (typeof address === 'function') { callback = address; address = undefined; }
  var number = portNumber(port, false);
  address = String(address || (this.type === 'udp6' ? '::1' : '127.0.0.1'));
  if (this._remote) throw new Error('Socket is already connected');
  var socket = this;
  function fail(outcome) {
    var error = nativeError(outcome);
    queueMicrotask(function() { if (typeof callback === 'function') callback(error); else socket.emit('error', error); });
    return socket;
  }
  if (!this._handle) {
    var bound = this._bind(this.type === 'udp6' ? '::' : '0.0.0.0', 0);
    if (bound.indexOf('ok|') !== 0) return fail(bound);
  }
  var outcome = __thaw_udp_connect(this._handle, address, number);
  if (outcome !== 'ok') return fail(outcome);
  this._remote = { address: address, family: this.type === 'udp6' ? 'IPv6' : 'IPv4', port: number };
  queueMicrotask(function() { if (!socket._closed) { socket.emit('connect'); if (typeof callback === 'function') callback(); } });
  return this;
};
Socket.prototype.disconnect = function() {
  this._checkOpen();
  if (!this._remote) throw new Error('Socket is not connected');
  var outcome = __thaw_udp_disconnect(this._handle);
  if (outcome !== 'ok') throw nativeError(outcome);
  this._remote = null;
  return this;
};
Socket.prototype.address = function() { if (!this._address || this._closed) throw new Error('Socket is not running'); return this._address; };
Socket.prototype.remoteAddress = function() { if (!this._remote || this._closed) throw new Error('Socket is not connected'); return this._remote; };
Socket.prototype.close = function(callback) {
  if (this._closed) throw new Error('Socket is closed');
  this._closed = true;
  if (typeof callback === 'function') this.once('close', callback);
  if (this._receiveTimer !== null) { clearTimeout(this._receiveTimer); this._receiveTimer = null; }
  if (this._handle) __thaw_udp_close(this._handle);
  this._handle = 0;
  this._address = null;
  this._remote = null;
  queueMicrotask(() => this.emit('close'));
  return this;
};
Socket.prototype.ref = function() { this._refed = true; if (this._receiveTimer !== null) this._receiveTimer.ref(); return this; };
Socket.prototype.unref = function() { this._refed = false; if (this._receiveTimer !== null) this._receiveTimer.unref(); return this; };
Socket.prototype.hasRef = function() { return this._refed; };
Socket.prototype.setBroadcast = function(flag) { var outcome = this._handle ? __thaw_udp_set_broadcast(this._handle, Boolean(flag)) : 'err|EBADF|setBroadcast EBADF'; if (outcome !== 'ok') throw nativeError(outcome); }; function udpOption(socket, syscall, name, a, b, c) {
  if (socket._closed) { var running = new Error('Not running'); running.code = 'ERR_SOCKET_DGRAM_NOT_RUNNING'; throw running; }
  if (!socket._handle) { var unbound = new Error(syscall + ' EBADF'); unbound.code = 'EBADF'; unbound.syscall = syscall; throw unbound; }
  var outcome = __thaw_udp_option(socket._handle, name, a === undefined ? '' : String(a), b === undefined ? '' : String(b), c === undefined ? '' : String(c));
  if (outcome !== 'ok') { var failure = nativeError(outcome); failure.syscall = syscall; failure.message = syscall + ' ' + failure.code; throw failure; }
}
function udpNumber(value, name) { if (typeof value !== 'number') { var error = new TypeError('The "' + name + '" argument must be of type number. Received ' + (value === null ? 'null' : typeof value === 'object' ? 'an instance of ' + (value.constructor && value.constructor.name || 'Object') : 'type ' + typeof value + ' (' + String(value) + ')')); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } return value; }
Socket.prototype.setTTL = function(ttl) { udpNumber(ttl, 'ttl'); udpOption(this, 'setTTL', 'ttl', ttl); return ttl; };
Socket.prototype.setMulticastTTL = function(ttl) { udpNumber(ttl, 'ttl'); udpOption(this, 'setMulticastTTL', 'multicast_ttl', ttl); return ttl; };
Socket.prototype.setMulticastLoopback = function(flag) { udpOption(this, 'setMulticastLoopback', 'multicast_loop', flag ? '1' : '0'); return flag; };
Socket.prototype.setMulticastInterface = function(interfaceAddress) { if (typeof interfaceAddress !== 'string') { var error = new TypeError('The "interfaceAddress" argument must be of type string. Received ' + typeof interfaceAddress); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } udpOption(this, 'setMulticastInterface', 'multicast_if', interfaceAddress); };
Socket.prototype.addMembership = function(group, interfaceAddress) { if (!group) { var missing = new TypeError('The "multicastAddress" argument must be specified'); missing.code = 'ERR_MISSING_ARGS'; throw missing; } udpOption(this, 'addMembership', 'add_membership', group, interfaceAddress); };
Socket.prototype.dropMembership = function(group, interfaceAddress) { if (!group) { var missing = new TypeError('The "multicastAddress" argument must be specified'); missing.code = 'ERR_MISSING_ARGS'; throw missing; } udpOption(this, 'dropMembership', 'drop_membership', group, interfaceAddress); };
Socket.prototype.addSourceSpecificMembership = function(source, group, interfaceAddress) { if (typeof source !== 'string' || typeof group !== 'string') { var error = new TypeError('The "sourceAddress" and "groupAddress" arguments must be of type string'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } udpOption(this, 'addSourceSpecificMembership', 'add_source_membership', source, group, interfaceAddress); };
Socket.prototype.dropSourceSpecificMembership = function(source, group, interfaceAddress) { if (typeof source !== 'string' || typeof group !== 'string') { var error = new TypeError('The "sourceAddress" and "groupAddress" arguments must be of type string'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } udpOption(this, 'dropSourceSpecificMembership', 'drop_source_membership', source, group, interfaceAddress); };
Socket.prototype.getRecvBufferSize = Socket.prototype.getSendBufferSize = function() { return 212992; };
Socket.prototype.setRecvBufferSize = Socket.prototype.setSendBufferSize = function() { return this; };
function createSocket(type, listener) { return new Socket(type, listener); }
module.exports = { Socket: Socket, createSocket: createSocket };
module.exports.default = module.exports;
module.exports.__esModule = true;
"#,
        ),
        "dns" => Some(
            r#"function isIPv4(value) { if (typeof value !== 'string' || !/^(?:\d{1,3}\.){3}\d{1,3}$/.test(value)) return false; return value.split('.').every(function(part) { return String(Number(part)) === part && Number(part) <= 255; }); } function ipv6Words(value) { if (typeof value !== 'string') return null; var address = value, zone = address.indexOf('%'); if (zone >= 0) { if (zone === address.length - 1 || address.indexOf('%', zone + 1) >= 0 || !/^[A-Za-z0-9_.-]+$/.test(address.slice(zone + 1))) return null; address = address.slice(0, zone); } if (address.indexOf(':') < 0 || address.indexOf(':::') >= 0) return null; var marker = address.indexOf('::'), compressed = marker >= 0; if (compressed && address.indexOf('::', marker + 2) >= 0) return null; var leftText = compressed ? address.slice(0, marker) : address, rightText = compressed ? address.slice(marker + 2) : '', left = leftText ? leftText.split(':') : [], right = rightText ? rightText.split(':') : [], groups = left.concat(right), words = [];  for (var i = 0; i < groups.length; i++) { var group = groups[i]; if (group.indexOf('.') >= 0) { if (i !== groups.length - 1 || address.slice(-group.length) !== group || !isIPv4(group)) return null; var octets = group.split('.').map(Number); words.push(octets[0] * 256 + octets[1], octets[2] * 256 + octets[3]); } else { if (!/^[0-9a-fA-F]{1,4}$/.test(group)) return null; words.push(parseInt(group, 16)); } } if (compressed) { var missing = 8 - words.length; if (missing < 1) return null; var leftCount = 0; for (var j = 0; j < left.length; j++) leftCount += left[j].indexOf('.') >= 0 ? 2 : 1; words = words.slice(0, leftCount).concat(Array(missing).fill(0), words.slice(leftCount)); } return words.length === 8 ? words : null; } function isIPv6(value) { return ipv6Words(value) !== null; } function isIP(value) { return isIPv4(value) ? 4 : isIPv6(value) ? 6 : 0; }
var ORDERS = ['verbatim', 'ipv4first', 'ipv6first'];
var defaultOrder = globalThis.__thaw_dns_order || 'verbatim';
function describe(value) { return typeof value === 'string' ? "'" + value + "'" : String(value); }
function invalidValue(name, allowed, value) { var error = new TypeError("The " + (name.indexOf('.') < 0 ? "argument '" : "property '") + name + "' must be one of: " + allowed + ". Received " + describe(value)); error.code = 'ERR_INVALID_ARG_VALUE'; return error; }
function invalidType(name, expected, value) { var error = new TypeError('The "' + name + '" argument must be ' + expected + '. Received ' + describe(value)); error.code = 'ERR_INVALID_ARG_TYPE'; return error; }
function validateOrder(order, name) { if (ORDERS.indexOf(order) < 0) throw invalidValue(name, ORDERS.map(function(value) { return "'" + value + "'"; }).join(', '), order); return order; }
// Normalizes options.family. Callback lookup also accepts the legacy 'IPv4'/'IPv6' spellings, promises lookup does not.
function normalizeFamily(family, legacyNames) {
  if (family === undefined || family === null) return 0;
  if (legacyNames && family === 'IPv4') return 4;
  if (legacyNames && family === 'IPv6') return 6;
  if (family !== 0 && family !== 4 && family !== 6) throw invalidValue('options.family', '0, 4, 6', family);
  return family;
}
function resultOrder(options) {
  if (options.order !== undefined) return validateOrder(options.order, 'options.order');
  if (typeof options.verbatim === 'boolean') return options.verbatim ? 'verbatim' : 'ipv4first';
  return defaultOrder;
}
function applyOrder(found, order) {
  if (order === 'verbatim') return found;
  var first = order === 'ipv4first' ? 4 : 6;
  return found.filter(function(entry) { return entry.family === first; }).concat(found.filter(function(entry) { return entry.family !== first; }));
}
function hostAddresses(hostname, family) {
  if (String(hostname).toLowerCase() === 'localhost') {
    if (family === 4) return [{ address: '127.0.0.1', family: 4 }];
    if (family === 6) return [{ address: '::1', family: 6 }];
    return [{ address: '127.0.0.1', family: 4 }, { address: '::1', family: 6 }];
  }
  if (typeof __thaw_dns_lookup === 'function') {
    var resolved = JSON.parse(__thaw_dns_lookup(String(hostname)));
    if (resolved.ok) return resolved.addresses.filter(function(entry) { return !family || family === entry.family; });
  }
  return [];
}
// An IP literal bypasses resolution and the family filter, exactly like getaddrinfo with AI_NUMERICHOST.
function addresses(hostname, family) {
  var literal = isIP(hostname);
  return literal ? [{ address: String(hostname), family: literal }] : hostAddresses(hostname, family);
}
function failure(hostname, syscall) {
  var error = new Error(syscall + ' ENOTFOUND ' + hostname);
  error.code = 'ENOTFOUND'; error.errno = -3008; error.syscall = syscall; error.hostname = String(hostname);
  return error;
}
// Name resolution runs on a host worker thread (`__thaw_dns_lookup_start`) and is polled from a timer, so a slow resolver never
// blocks the event loop. IP literals and `localhost` complete without a worker, but still asynchronously (a microtask).
function resolveAsync(hostname, family, done) {
  var literal = isIP(hostname);
  if (literal || String(hostname).toLowerCase() === 'localhost' || typeof __thaw_dns_lookup_start !== 'function') {
    queueMicrotask(function() { done(addresses(hostname, family)); });
    return;
  }
  var operation = __thaw_dns_lookup_start(String(hostname)), polls = 0;
  function step() {
    var outcome = __thaw_async_poll(operation);
    if (outcome === 'pending') { setTimeout(step, polls++ < 50 ? 1 : 5); return; }
    var resolved = JSON.parse(outcome);
    done(resolved.ok ? resolved.addresses.filter(function(entry) { return !family || family === entry.family; }) : []);
  }
  setTimeout(step, 1);
}
// Shared by dns.lookup and dns.promises.lookup: validates arguments synchronously (throws on invalid input) and returns a plan.
function lookupPlan(hostname, options, legacyNames) {
  if (typeof options === 'number') options = { family: options };
  options = options || {};
  var family = normalizeFamily(options.family, legacyNames);
  var order = resultOrder(options);
  if (hostname !== undefined && hostname !== null && hostname !== '' && typeof hostname !== 'string') throw invalidType('hostname', 'of type string', hostname);
  if (!hostname) return { empty: true, family: family, all: Boolean(options.all) };
  return { hostname: hostname, family: family, order: order, all: Boolean(options.all) };
}
// Resolves a plan; done(error, result) where result is the empty marker or { found, all }.
function lookupResolve(plan, done) {
  if (plan.empty) { queueMicrotask(function() { done(null, plan); }); return; }
  resolveAsync(plan.hostname, plan.family, function(list) {
    var found = applyOrder(list, plan.order);
    if (!found.length) done(failure(plan.hostname, 'getaddrinfo')); else done(null, { found: found, all: plan.all });
  });
}
function lookup(hostname, options, callback) {
  if (typeof options === 'function') { callback = options; options = {}; }
  if (typeof callback !== 'function') throw invalidType('callback', 'of type function', callback);
  var plan = lookupPlan(hostname, options, true);
  lookupResolve(plan, function(error, result) {
    if (error) { callback(error); return; }
    if (result.empty) { if (result.all) callback(null, []); else callback(null, null, result.family === 6 ? 6 : 4); }
    else if (result.all) callback(null, result.found);
    else callback(null, result.found[0].address, result.found[0].family);
  });
  return {};
}
var QUERY_NAMES = { A: 'queryA', AAAA: 'queryAaaa' };
function queryAsync(hostname, rrtype, done) {
  var type = String(rrtype || 'A').toUpperCase();
  resolveAsync(hostname, type === 'AAAA' ? 6 : 4, function(found) {
    if (!found.length) done(failure(hostname, QUERY_NAMES[type] || 'query' + type));
    else done(null, found.map(function(entry) { return entry.address; }));
  });
}
var DNS_RECORD_TYPES = ['A', 'AAAA', 'ANY', 'CAA', 'CNAME', 'MX', 'NAPTR', 'NS', 'PTR', 'SOA', 'SRV', 'TXT'];
var DNS_SYSCALLS = { A: 'queryA', AAAA: 'queryAaaa', ANY: 'queryAny', CAA: 'queryCaa', CNAME: 'queryCname', MX: 'queryMx', NAPTR: 'queryNaptr', NS: 'queryNs', PTR: 'queryPtr', SOA: 'querySoa', SRV: 'querySrv', TXT: 'queryTxt' };
function systemServers() { return JSON.parse(__thaw_dns_default_servers()); }
function invalidIp(entry) { var error = new TypeError('Invalid IP address: ' + String(entry)); error.code = 'ERR_INVALID_IP_ADDRESS'; return error; }
// Accepts 'ip', 'ip:port', '[v6]' and '[v6]:port'; the canonical form drops the default port and brackets only when a port follows.
function canonicalServer(entry) {
  if (typeof entry !== 'string') throw invalidType('servers[]', 'of type string', entry);
  var host = entry, port;
  var bracket = /^\[(.+)\](?::(\d+))?$/.exec(entry);
  if (bracket) { host = bracket[1]; port = bracket[2]; }
  else { var split = /^([^:]+):(\d+)$/.exec(entry); if (split) { host = split[1]; port = split[2]; } }
  if (!isIP(host)) throw invalidIp(entry);
  if (port !== undefined && (Number(port) < 1 || Number(port) > 65535)) throw invalidIp(entry);
  if (port === undefined || Number(port) === 53) return host;
  return isIP(host) === 6 ? '[' + host + ']:' + port : host + ':' + port;
}
function queryError(code, syscall, hostname) {
  var error = new Error(syscall + ' ' + code + ' ' + hostname);
  error.errno = undefined; error.code = code; error.syscall = syscall; error.hostname = String(hostname);
  return error;
}
function Resolver(options) {
  if (!(this instanceof Resolver)) return new Resolver(options);
  options = options || {};
  this._servers = null;
  this._timeout = typeof options.timeout === 'number' && options.timeout > 0 ? options.timeout : 3000;
  this._tries = typeof options.tries === 'number' && options.tries > 0 ? Math.floor(options.tries) : 2;
  this._pending = [];
}
Resolver.prototype.getServers = function() { return (this._servers || systemServers().map(function(server) { return canonicalServer(server); })).slice(); };
Resolver.prototype.setServers = function(servers) {
  if (!Array.isArray(servers)) throw invalidType('servers', 'an instance of Array', servers);
  var canonical = servers.map(canonicalServer);
  this._servers = canonical;
};
Resolver.prototype.setLocalAddress = function() {};
// Runs one DNS query on a host worker thread and polls it from a timer, so the event loop keeps running while we wait.
function dnsQuery(resolver, hostname, type, done) {
  var servers = resolver._servers || systemServers();
  var operation = __thaw_dns_query_start(String(hostname), type, JSON.stringify(servers), resolver._timeout, resolver._tries);
  var entry = { operation: operation, done: done, timer: null, type: type, hostname: hostname }, polls = 0;
  resolver._pending.push(entry);
  function step() {
    var outcome = __thaw_async_poll(operation);
    if (outcome === 'pending') { entry.timer = setTimeout(step, polls++ < 50 ? 1 : 5); return; }
    var index = resolver._pending.indexOf(entry);
    if (index >= 0) resolver._pending.splice(index, 1);
    var result = JSON.parse(outcome);
    if (result.ok) done(null, result.records); else done(queryError(result.code, DNS_SYSCALLS[type], hostname));
  }
  entry.timer = setTimeout(step, 1);
}
Resolver.prototype.cancel = function() {
  var entries = this._pending.splice(0);
  entries.forEach(function(entry) {
    if (entry.timer) clearTimeout(entry.timer);
    __thaw_async_cancel(entry.operation);
    queueMicrotask(function() { entry.done(queryError('ECANCELLED', DNS_SYSCALLS[entry.type], entry.hostname)); });
  });
};
function dnsRecordShape(record, includeType) {
  var shaped;
  switch (record.type) {
    case 'A': case 'AAAA': shaped = { address: record.address, ttl: record.ttl }; break;
    case 'MX': shaped = { exchange: record.exchange, priority: record.priority }; break;
    case 'SRV': shaped = { name: record.name, port: record.port, priority: record.priority, weight: record.weight }; break;
    case 'SOA': shaped = { nsname: record.nsname, hostmaster: record.hostmaster, serial: record.serial, refresh: record.refresh, retry: record.retry, expire: record.expire, minttl: record.minttl }; break;
    case 'NAPTR': shaped = { flags: record.flags, service: record.service, regexp: record.regexp, replacement: record.replacement, order: record.order, preference: record.preference }; break;
    case 'TXT': shaped = { entries: record.entries }; break;
    case 'CAA': shaped = {}; Object.keys(record).forEach(function(key) { if (key !== 'type') shaped[key] = record[key]; }); break;
    default: shaped = { value: record.value };
  }
  if (includeType) shaped.type = record.type;
  return shaped;
}
function dnsResult(type, records, options) {
  if (type === 'ANY') return records.map(function(record) { return dnsRecordShape(record, true); });
  var own = records.filter(function(record) { return record.type === type; });
  if (type === 'A' || type === 'AAAA') return own.map(function(record) { return options && options.ttl ? { address: record.address, ttl: record.ttl } : record.address; });
  if (type === 'SOA') return own.length ? dnsRecordShape(own[0], false) : null;
  if (type === 'TXT') return own.map(function(record) { return record.entries; });
  if (type === 'NS' || type === 'CNAME' || type === 'PTR') return own.map(function(record) { return record.value; });
  return own.map(function(record) { return dnsRecordShape(record, false); });
}
// A/AAAA through the system resolver (getaddrinfo) unless this resolver was given explicit servers.
function resolveRecords(resolver, hostname, type, options, done) {
  if (typeof hostname !== 'string') throw invalidType('name', 'of type string', hostname);
  if ((type === 'A' || type === 'AAAA') && resolver._servers === null) {
    queryAsync(hostname, type, function(error, addresses) {
      if (error) { done(error); return; }
      done(null, options && options.ttl ? addresses.map(function(address) { return { address: address, ttl: 0 }; }) : addresses);
    });
    return;
  }
  dnsQuery(resolver, hostname, type, function(error, records) {
    if (error) { done(error); return; }
    done(null, dnsResult(type, records, options));
  });
}
function reverseName(ip) {
  var family = isIP(ip);
  if (family === 4) return ip.split('.').reverse().join('.') + '.in-addr.arpa';
  var words = ipv6Words(ip), nibbles = [];
  words.forEach(function(word) { var hex = ('0000' + word.toString(16)).slice(-4); for (var index = 0; index < 4; index++) nibbles.push(hex[index]); });
  return nibbles.reverse().join('.') + '.ip6.arpa';
}
function reverseRecords(resolver, ip, done) {
  if (!isIP(ip)) { var invalid = queryError('EINVAL', 'getHostByAddr', ip); throw invalid; }
  if (resolver._servers === null && (ip === '127.0.0.1' || ip === '::1')) { queueMicrotask(function() { done(null, ['localhost']); }); return; }
  dnsQuery(resolver, reverseName(ip), 'PTR', function(error, records) {
    if (error) { done(error.code === 'ENODATA' || error.code === 'ENOTFOUND' ? queryError('ENOTFOUND', 'getHostByAddr', ip) : queryError(error.code, 'getHostByAddr', ip)); return; }
    done(null, dnsResult('PTR', records));
  });
}
function requireCallback(callback) { if (typeof callback !== 'function') throw invalidType('callback', 'of type function', callback); }
function defineResolverMethods(prototype) {
  [['resolve4', 'A'], ['resolve6', 'AAAA'], ['resolveAny', 'ANY'], ['resolveCaa', 'CAA'], ['resolveCname', 'CNAME'], ['resolveMx', 'MX'], ['resolveNaptr', 'NAPTR'], ['resolveNs', 'NS'], ['resolvePtr', 'PTR'], ['resolveSoa', 'SOA'], ['resolveSrv', 'SRV'], ['resolveTxt', 'TXT']].forEach(function(pair) {
    prototype[pair[0]] = function(hostname, options, callback) {
      if (typeof options === 'function') { callback = options; options = undefined; }
      requireCallback(callback);
      resolveRecords(this, hostname, pair[1], options, callback);
      return {};
    };
  });
  prototype.resolve = function(hostname, rrtype, callback) {
    if (typeof rrtype === 'function') { callback = rrtype; rrtype = 'A'; }
    if (rrtype === undefined) rrtype = 'A';
    requireCallback(callback);
    if (DNS_RECORD_TYPES.indexOf(rrtype) < 0) { var invalidType = new TypeError("The argument 'rrtype' is invalid. Received " + describe(rrtype)); invalidType.code = 'ERR_INVALID_ARG_VALUE'; throw invalidType; }
    resolveRecords(this, hostname, rrtype, undefined, callback);
    return {};
  };
  prototype.reverse = function(ip, callback) { requireCallback(callback); reverseRecords(this, ip, callback); return {}; };
}
defineResolverMethods(Resolver.prototype);
var moduleResolver = new Resolver();
function moduleMethod(name) { return function() { return moduleResolver[name].apply(moduleResolver, arguments); }; }
function setDefaultResultOrder(order) { defaultOrder = validateOrder(order, 'order'); globalThis.__thaw_dns_order = defaultOrder; }
// The promises API is built once here and shared by dns.promises and node:dns/promises.
function PromisesResolver(options) {
  if (!(this instanceof PromisesResolver)) return new PromisesResolver(options);
  this._resolver = new Resolver(options);
}
PromisesResolver.prototype.getServers = function() { return this._resolver.getServers(); };
PromisesResolver.prototype.setServers = function(servers) { return this._resolver.setServers(servers); };
PromisesResolver.prototype.setLocalAddress = function() {};
PromisesResolver.prototype.cancel = function() { return this._resolver.cancel(); };
function promiseMethod(name) {
  return function() {
    var resolver = this, args = Array.prototype.slice.call(arguments);
    return new Promise(function(resolvePromise, rejectPromise) {
      args.push(function(error, value) { if (error) rejectPromise(error); else resolvePromise(value); });
      resolver[name].apply(resolver, args);
    });
  };
}
['resolve', 'resolve4', 'resolve6', 'resolveAny', 'resolveCaa', 'resolveCname', 'resolveMx', 'resolveNaptr', 'resolveNs', 'resolvePtr', 'resolveSoa', 'resolveSrv', 'resolveTxt', 'reverse'].forEach(function(name) {
  var method = promiseMethod(name);
  PromisesResolver.prototype[name] = function() { return method.apply(this._resolver, arguments); };
});
var promises = {
  lookup: function(hostname, options) {
    return new Promise(function(resolvePromise, rejectPromise) {
      var plan = lookupPlan(hostname, options, false);
      lookupResolve(plan, function(error, result) {
        if (error) rejectPromise(error);
        else if (result.empty) resolvePromise(result.all ? [] : { address: null, family: result.family === 6 ? 6 : 4 });
        else resolvePromise(result.all ? result.found : result.found[0]);
      });
    });
  },
  Resolver: PromisesResolver,
  getDefaultResultOrder: function() { return defaultOrder; },
  setDefaultResultOrder: setDefaultResultOrder,
  getServers: function() { return moduleResolver.getServers(); },
  setServers: function(servers) { return moduleResolver.setServers(servers); }
};
['resolve', 'resolve4', 'resolve6', 'resolveAny', 'resolveCaa', 'resolveCname', 'resolveMx', 'resolveNaptr', 'resolveNs', 'resolvePtr', 'resolveSoa', 'resolveSrv', 'resolveTxt', 'reverse'].forEach(function(name) {
  var method = promiseMethod(name);
  promises[name] = function() { return method.apply(moduleResolver, arguments); };
});
Object.defineProperty(promises, 'default', { value: promises, enumerable: false });
Object.defineProperty(promises, '__esModule', { value: true, enumerable: false });
module.exports = { lookup: lookup, Resolver: Resolver, promises: promises, getDefaultResultOrder: function() { return defaultOrder; }, setDefaultResultOrder: setDefaultResultOrder, getServers: moduleMethod('getServers'), setServers: moduleMethod('setServers'), ADDRCONFIG: 32, V4MAPPED: 8, NODATA: 'ENODATA', FORMERR: 'EFORMERR', SERVFAIL: 'ESERVFAIL', NOTFOUND: 'ENOTFOUND', NOTIMP: 'ENOTIMP', REFUSED: 'EREFUSED' };
['resolve', 'resolve4', 'resolve6', 'resolveAny', 'resolveCaa', 'resolveCname', 'resolveMx', 'resolveNaptr', 'resolveNs', 'resolvePtr', 'resolveSoa', 'resolveSrv', 'resolveTxt', 'reverse'].forEach(function(name) { module.exports[name] = moduleMethod(name); });
module.exports.default = module.exports;
module.exports.__esModule = true;
"#,
        ),
        "dns/promises" => Some(
            r#"module.exports = require('node:dns').promises;
"#,
        ),
        "net" => Some(concat!(
            "var socketReadable = require('node:stream').Readable, socketReadablePush = socketReadable.prototype.push, socketReadableRead = socketReadable.prototype.read, socketReadableDrain = socketReadable.prototype._drainReadable, socketReadablePause = socketReadable.prototype.pause, socketReadableResume = socketReadable.prototype.resume, socketReadableSetEncoding = socketReadable.prototype.setEncoding, socketDefineProperty = Object.defineProperty;\nfunction acceptedPeer(value) { var fields = value.split(':'), last = fields.length - 1; return last < 3 ? ['ok', fields[1]] : ['ok', fields[1], fields.slice(2, last).join(':'), fields[last]]; } function netArgs(args) { var list = Array.prototype.slice.call(args), callback = typeof list[list.length - 1] === 'function' ? list.pop() : undefined, options = {}; if (list[0] !== null && typeof list[0] === 'object') options = Object.assign({}, list[0]); else { options.port = list[0]; if (typeof list[1] === 'string') options.host = list[1]; } return { options: options, callback: callback }; } function isIPv4(value) { if (typeof value !== 'string' || !/^(?:\\d{1,3}\\.){3}\\d{1,3}$/.test(value)) return false; return value.split('.').every(function(part) { return String(Number(part)) === part && Number(part) <= 255; }); } function ipv4Number(value) { return value.split('.').reduce(function(result, part) { return (result * 256 + Number(part)) >>> 0; }, 0); } function ipv6Words(value) { if (typeof value !== 'string') return null; var address = value, zone = address.indexOf('%'); if (zone >= 0) { if (zone === address.length - 1 || address.indexOf('%', zone + 1) >= 0 || !/^[A-Za-z0-9_.-]+$/.test(address.slice(zone + 1))) return null; address = address.slice(0, zone); } if (address.indexOf(':') < 0 || address.indexOf(':::') >= 0) return null; var marker = address.indexOf('::'), compressed = marker >= 0; if (compressed && address.indexOf('::', marker + 2) >= 0) return null; var leftText = compressed ? address.slice(0, marker) : address, rightText = compressed ? address.slice(marker + 2) : '', left = leftText ? leftText.split(':') : [], right = rightText ? rightText.split(':') : [], groups = left.concat(right), words = [];  for (var i = 0; i < groups.length; i++) { var group = groups[i]; if (group.indexOf('.') >= 0) { if (i !== groups.length - 1 || address.slice(-group.length) !== group || !isIPv4(group)) return null; var octets = group.split('.').map(Number); words.push(octets[0] * 256 + octets[1], octets[2] * 256 + octets[3]); } else { if (!/^[0-9a-fA-F]{1,4}$/.test(group)) return null; words.push(parseInt(group, 16)); } } if (compressed) { var missing = 8 - words.length; if (missing < 1) return null; var leftCount = 0; for (var j = 0; j < left.length; j++) leftCount += left[j].indexOf('.') >= 0 ? 2 : 1; words = words.slice(0, leftCount).concat(Array(missing).fill(0), words.slice(leftCount)); } return words.length === 8 ? words : null; } function isIPv6(value) { return ipv6Words(value) !== null; } function isIP(value) { return isIPv4(value) ? 4 : isIPv6(value) ? 6 : 0; }\nfunction SocketAddress(options) { if (!(this instanceof SocketAddress)) return new SocketAddress(options); options = options || {}; var family = typeof options.family === 'string' ? options.family.toLowerCase() : options.family; if (family === undefined) family = 'ipv4'; if (family !== 'ipv4' && family !== 'ipv6') { var familyError = new TypeError(\"The property 'options.family' must be one of: 'ipv4', 'ipv6'. Received \" + String(options.family)); familyError.code = 'ERR_INVALID_ARG_VALUE'; throw familyError; } this.address = options.address === undefined ? (family === 'ipv6' ? '::' : '127.0.0.1') : String(options.address); if (isIP(this.address) !== (family === 'ipv6' ? 6 : 4)) { var addressError = new TypeError('Invalid socket address'); addressError.code = 'ERR_INVALID_ADDRESS'; throw addressError; } this.family = family; this.port = options.port === undefined ? 0 : Number(options.port); if (!Number.isInteger(this.port) || this.port < 0 || this.port > 65535) { var portError = new RangeError('Port should be >= 0 and < 65536. Received ' + options.port); portError.code = 'ERR_SOCKET_BAD_PORT'; throw portError; } this.flowlabel = options.flowlabel === undefined ? 0 : Number(options.flowlabel); } SocketAddress.prototype.toJSON = function() { return { address: this.address, port: this.port, family: this.family, flowlabel: this.flowlabel }; };\nfunction BlockList() { if (!(this instanceof BlockList)) return new BlockList(); this.rules = []; } /* shortcut: retain mapped IPv6 family isolation, upgrade when full Node BlockList parity is required. */ function blockFamily(address, type) { var actual = isIP(address), expected = type === undefined ? actual : type === 'ipv4' ? 4 : type === 'ipv6' ? 6 : 0; return actual && actual === expected ? actual : 0; } function compareWords(a, b) { for (var i = 0; i < 8; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1; return 0; } BlockList.prototype.addAddress = function(address, type) { var family = blockFamily(address, type); if (!family) throw new TypeError('Invalid IP address'); this.rules.push({ kind: 'address', address: String(address), family: family }); }; BlockList.prototype.addRange = function(start, end, type) { var family = blockFamily(start, type), endFamily = isIP(end); if (!family || family !== endFamily || (type !== undefined && blockFamily(end, type) !== family)) throw new TypeError('Invalid IP range'); if (family === 4 ? ipv4Number(start) > ipv4Number(end) : compareWords(ipv6Words(start), ipv6Words(end)) > 0) throw new TypeError('Invalid IP range'); this.rules.push({ kind: 'range', start: String(start), end: String(end), family: family }); }; BlockList.prototype.addSubnet = function(network, prefix, type) { var family = blockFamily(network, type); prefix = Number(prefix); if (!family || !Number.isInteger(prefix) || prefix < 0 || prefix > (family === 4 ? 32 : 128)) throw new TypeError('Invalid subnet'); this.rules.push({ kind: 'subnet', network: String(network), prefix: prefix, family: family }); }; BlockList.prototype.check = function(address, type) { var family = blockFamily(address, type); if (!family) return false; var value = family === 4 ? ipv4Number(address) : ipv6Words(address); return this.rules.some(function(rule) { if (rule.family !== family) return false; if (family === 4) { if (rule.kind === 'address') return value === ipv4Number(rule.address); if (rule.kind === 'range') return value >= ipv4Number(rule.start) && value <= ipv4Number(rule.end); var mask = rule.prefix === 0 ? 0 : (0xffffffff << (32 - rule.prefix)) >>> 0; return (value & mask) === (ipv4Number(rule.network) & mask); } var first = rule.kind === 'address' ? ipv6Words(rule.address) : rule.kind === 'range' ? ipv6Words(rule.start) : ipv6Words(rule.network); if (!first) return false; if (rule.kind === 'address') return compareWords(value, first) === 0; if (rule.kind === 'range') { var last = ipv6Words(rule.end); return last !== null && compareWords(value, first) >= 0 && compareWords(value, last) <= 0; } var full = Math.floor(rule.prefix / 16), bits = rule.prefix % 16; for (var i = 0; i < full; i++) if (value[i] !== first[i]) return false; return !bits || (value[full] & (0xffff << (16 - bits))) === (first[full] & (0xffff << (16 - bits))); }); };\nfunction Socket(options) { if (!(this instanceof Socket)) return new Socket(options); socketReadable.call(this, { autoDestroy: false }); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.pending = true; this.bytesRead = 0; this.bytesWritten = 0; this.remoteAddress = undefined; this.remotePort = undefined; this.remoteFamily = undefined; this._refed = true; this._reading = false; this._readEnded = false; this._readGeneration = 0; this._terminalEventsGeneration = -1; var socket = this; this._readableState = { endEmitted: false, errorEmitted: false }; socketDefineProperty(this._readableState, 'length', { enumerable: true, get: function() { return socket.readableLength; } }); } function addSocketListener(socket, name, listener, once) { var key = String(name); (socket._events[key] || (socket._events[key] = [])).push({ listener: listener, once: once }); if (key === 'data' && socket.readableFlowing !== false) { socket.readableFlowing = true; socketReadableDrain.call(socket); if (socket._handle && !socket.destroyed) socket._startRead(); } return socket; } Socket.prototype.on = Socket.prototype.addListener = function(name, listener) { return addSocketListener(this, name, listener, false); }; Socket.prototype.once = function(name, listener) { return addSocketListener(this, name, listener, true); }; Socket.prototype.off = Socket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; Socket.prototype.emit = function(name) { if (name === 'end' && this._readableState) this._readableState.endEmitted = true; if (name === 'error' && this._readableState) this._readableState.errorEmitted = true; var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); if (key === 'error' && !list.length) throw args[0] instanceof Error ? args[0] : new Error('Unhandled error event'); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; Socket.prototype.connect = function() { var generation = this._readGeneration; var normalized = netArgs(arguments), options = normalized.options, listener = normalized.callback; if (typeof listener === 'function') this.once('connect', listener); var hostname = String(options.host || 'localhost'); var numericPort = Number(options.port); var socket = this; if (this._connectOp) { __thaw_async_cancel(this._connectOp); this._connectOp = 0; } if (this._connectTimer) { clearTimeout(this._connectTimer); this._connectTimer = null; } this.connecting = true; this._pendingWrites = []; this._pendingEnd = null; var operation = __thaw_net_connect_start(hostname, numericPort), polls = 0; this._connectOp = operation; function settle(outcome) { socket._connectOp = 0; socket._connectTimer = null; if (generation !== socket._readGeneration || socket.destroyed) return; socket.connecting = false; socket.pending = false; if (outcome.indexOf('ok:') === 0) { var peer = acceptedPeer(outcome); if (peer[2] === undefined) { peer[2] = hostname; peer[3] = numericPort; } socket._handle = Number(peer[1]); socket.readable = true; socket.writable = true; netApplyOptions(socket, 'tcp'); var queued = socket._pendingWrites || [], deferredCallbacks = []; socket._pendingWrites = null; var flushError = null; queued.forEach(function(entry) { if (flushError) return; var written = __thaw_net_write(socket._handle, entry.chunk.toString('hex')); if (written !== 'ok') { flushError = new Error(written.substring(4)); return; } if (typeof entry.callback === 'function') deferredCallbacks.push(entry.callback); }); if (flushError) { queueMicrotask(function() { if (generation === socket._readGeneration) socket.destroy(flushError); }); return; } socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; queueMicrotask(() => { if (generation !== socket._readGeneration || socket.destroyed) return; try { socket.emit('connect'); if (socket._pendingEnd && generation === socket._readGeneration && !socket.destroyed) { var pendingEnd = socket._pendingEnd; socket._pendingEnd = null; socket.end(undefined, undefined, pendingEnd.callback); } } catch (error) { if (generation === socket._readGeneration) { try { socket.destroy(); } catch (_) {} } throw error; } }); deferredCallbacks.forEach(function(callback) { queueMicrotask(callback); }); } else { var error = connectFailureError(outcome, hostname, numericPort, 'ECONNREFUSED'); if (generation === socket._readGeneration) socket.destroyed = true; queueMicrotask(() => { if (generation === socket._readGeneration) plainTerminalEvents(socket, generation, error, false); }); }  } function netConnectStep() { var outcome = __thaw_async_poll(operation); if (outcome === 'pending') { socket._connectTimer = setTimeout(netConnectStep, polls++ < 50 ? 1 : 5); if (socket._refed === false) __thaw_set_timer_ref(socket._connectTimer, false); return; } settle(outcome); } this._connectTimer = setTimeout(netConnectStep, 1); if (this._refed === false) __thaw_set_timer_ref(this._connectTimer, false); return this; }; function connectFailureError(outcome, hostname, port, fallbackCode) { var found = /^err:([A-Z][A-Z0-9_]*)\\|(-?\\d+)\\|([a-z]+)\\|([^|]*)\\|([\\s\\S]*)$/.exec(outcome); if (!found) { var plain = new Error(outcome.substring(4)); plain.code = fallbackCode; if (fallbackCode === 'ECONNREFUSED') { plain.syscall = 'connect'; plain.address = hostname; plain.port = port; } return plain; } var failure; if (found[3] === 'getaddrinfo') { failure = new Error('getaddrinfo ' + found[1] + ' ' + hostname); failure.errno = Number(found[2]); failure.code = found[1]; failure.syscall = found[3]; failure.hostname = hostname; } else { failure = new Error(found[3] + ' ' + found[1] + ' ' + (found[4] || hostname) + ':' + port); failure.errno = Number(found[2]); failure.code = found[1]; failure.syscall = found[3]; failure.address = found[4] || hostname; failure.port = port; } return failure; } function netDrainWatch(socket) { if (socket._drainTimer) return; var generation = socket._readGeneration; socket._drainTimer = setTimeout(function netDrainStep() { socket._drainTimer = null; if (generation !== socket._readGeneration || !socket._handle) return; if (__thaw_net_pending(socket._handle) > 0) { netDrainWatch(socket); return; } socket._needDrain = false; socket.emit('drain'); }, 1); if (socket._refed === false) __thaw_set_timer_ref(socket._drainTimer, false); } function socketWriteRejected(socket, callback) { var destroyed = socket.destroyed === true; var failure = new Error(destroyed ? 'Cannot call write after a stream was destroyed' : 'write after end'); failure.code = destroyed ? 'ERR_STREAM_DESTROYED' : 'ERR_STREAM_WRITE_AFTER_END'; queueMicrotask(function() { if (typeof callback === 'function') { try { callback(failure); } catch (_) {} } if (!destroyed) socket.destroy(failure); }); return false; } function socketWriteFailure(socket, outcome, callback) { var text = String(outcome).substring(4); var code = /Broken pipe/.test(text) ? 'EPIPE' : 'ECONNRESET'; var failure = new Error('write ' + code); failure.code = code; failure.syscall = 'write'; failure.errno = code === 'EPIPE' ? -32 : -104; queueMicrotask(function() { if (typeof callback === 'function') { try { callback(failure); } catch (_) {} } socket.destroy(failure); }); return false; } Socket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.connecting && this._pendingWrites && !this._handle) { var queuedChunk = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); this._pendingWrites.push({ chunk: queuedChunk, callback: callback }); this.bytesWritten += queuedChunk.length; return true; } if (!this._handle || !this.writable) return socketWriteRejected(this, callback); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_net_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') return socketWriteFailure(this, outcome, callback); this.bytesWritten += buffer.length; if (typeof callback === 'function') queueMicrotask(callback); if (__thaw_net_pending(this._handle) >= 16384) { this._needDrain = true; netDrainWatch(this); return false; } return true; }; Socket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (!this._handle) return this; var generation = this._readGeneration, outcome = __thaw_net_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (generation !== this._readGeneration) return; if (outcome.indexOf('data-err:') === 0) { var split = outcome.indexOf(':', 9), partial = Buffer.from(outcome.substring(9, split), 'hex'); outcome = 'err:' + outcome.substring(split + 1); if (partial.length) { this.bytesRead += partial.length; this.emit('data', partial); if (generation !== this._readGeneration) return; } } if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } if (generation !== this._readGeneration) return; this.emit('end'); if (generation !== this._readGeneration) return; if (typeof callback === 'function') callback(); if (generation !== this._readGeneration) return; this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); if (generation === this._readGeneration) this.emit('close', true); } }); return this; }; Socket.prototype.destroy = function(error) { if (this._handle) __thaw_net_destroy(this._handle); this._handle = 0; this.destroyed = true; this.readable = false; this.writable = false; queueMicrotask(() => { if (error) this.emit('error', error); this.emit('close', Boolean(error)); }); return this; }; Socket.prototype.setEncoding = function(encoding) { var original = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return original.call(this, name, value); }; return this; }; Socket.prototype.setTimeout = function(timeout, callback) { if (typeof callback === 'function') this.once('timeout', callback); return this; }; function netApplyOptions(socket, kind) { if (socket._noDelay !== undefined) __thaw_net_set_option(kind, socket._handle, 'nodelay', socket._noDelay, 0); if (socket._keepAlive) __thaw_net_set_option(kind, socket._handle, 'keepalive', socket._keepAlive.enable, socket._keepAlive.delay); }
Socket.prototype.setNoDelay = function(noDelay) { this._noDelay = noDelay === undefined ? true : Boolean(noDelay); if (this._handle) __thaw_net_set_option('tcp', this._handle, 'nodelay', this._noDelay, 0); return this; };
Socket.prototype.setKeepAlive = function(enable, initialDelay) { this._keepAlive = { enable: Boolean(enable), delay: Number(initialDelay) || 0 }; if (this._handle) __thaw_net_set_option('tcp', this._handle, 'keepalive', this._keepAlive.enable, this._keepAlive.delay); return this; }; Socket.prototype.ref = function() { this._refed = true; if (this._pollTimer) __thaw_set_timer_ref(this._pollTimer, true); if (this._connectTimer) __thaw_set_timer_ref(this._connectTimer, true); return this; }; Socket.prototype.unref = function() { this._refed = false; if (this._pollTimer) __thaw_set_timer_ref(this._pollTimer, false); if (this._connectTimer) __thaw_set_timer_ref(this._connectTimer, false); return this; }; Socket.prototype.hasRef = function() { return this._refed; }; function createConnection() { var socket = new Socket(); return socket.connect.apply(socket, arguments); }\nfunction Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); if (typeof options === 'function') { listener = options; options = {}; } this._events = Object.create(null); this._handle = 0; this._address = null; this.listening = false; this.maxConnections = 0; this.connections = 0; this._refed = true; this._allowHalfOpen = Boolean(options && options.allowHalfOpen); if (typeof listener === 'function') this.on('connection', listener); } Server.prototype.on = Socket.prototype.on; Server.prototype.once = Socket.prototype.once; Server.prototype.off = Server.prototype.removeListener = Socket.prototype.off; Server.prototype.emit = Socket.prototype.emit; Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_net_accept(this._handle); if (accepted.indexOf('ok:') !== 0) return; var peer = acceptedPeer(accepted); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; this.connections++; this.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } }); return this; }; Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_net_close_listener(this._handle); this._handle = 0; this.listening = false; this._closePending = true; if (this.connections === 0 || this._closeNow) queueMicrotask(() => { if (this._closePending) { this._closePending = false; this.emit('close'); } }); return this; }; Server.prototype.closeAllConnections = Server.prototype.closeIdleConnections = function() {}; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }\nmodule.exports = { isIP: isIP, isIPv4: isIPv4, isIPv6: isIPv6, BlockList: BlockList, SocketAddress: SocketAddress, Socket: Socket, createConnection: createConnection, connect: createConnection, Server: Server, createServer: createServer }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
            r#"
             Socket.prototype.cork = Socket.prototype.uncork = function() { return this; };
             Socket.prototype._drainReadable = socketReadableDrain; Socket.prototype.push = function(chunk, encoding) { return socketReadablePush.call(this, chunk, encoding); }; Socket.prototype.read = function(size) { var value = socketReadableRead.call(this, size); if (this._handle && !this.destroyed && !this._paused && !this._httpReadPaused && this.readableLength < this.readableHighWaterMark) this._startRead(); return value; }; Socket.prototype._read = function() { if (this._handle && !this.destroyed) this._startRead(); }; Socket.prototype.setEncoding = socketReadableSetEncoding; Socket.prototype._refreshTimeout = function() { if (this._timeoutTimer) clearTimeout(this._timeoutTimer); if (!this.timeout || !this._handle || this.destroyed) return; var socket = this; this._timeoutTimer = setTimeout(function() { socket._timeoutTimer = null; if (socket._handle && !socket.destroyed) socket.emit('timeout'); }, this.timeout); };
             function plainTerminalEvents(socket, generation, error, ended) { var ownsGeneration = function() { return socket._readGeneration === generation; }; if (ownsGeneration()) { if (socket._terminalEventsGeneration === generation) return; socket._terminalEventsGeneration = generation; } var failed = false, first; function deliver(fn) { try { fn(); } catch (caught) { if (!failed) { failed = true; first = caught; } } } if (error) deliver(function() { socket.emit('error', error); }); if (ended) deliver(function() { if (!ownsGeneration()) return; socket._readEnded = true; socket.readable = false; try { socketReadablePush.call(socket, null); } finally { if (ownsGeneration()) socket.destroyed = true; } }); deliver(function() { socket.emit('close', Boolean(error)); }); if (failed) throw first; }
             Socket.prototype._startRead = function() { if (this._reading || this._paused || this._httpReadPaused || this._readEnded || !this._handle || this.destroyed) return; this._reading = true; var socket = this, generation = this._readGeneration; function tick() { socket._pollTimer = __thaw_set_timeout_ref(pump, 0, socket._refed); } function pump() { socket._pollTimer = 0; if (generation !== socket._readGeneration) return; if (!socket._handle || socket.destroyed || socket._readEnded) { socket._reading = false; return; } if (socket._paused || socket._httpReadPaused) { socket._reading = false; return; } var result = socket._isServer ? __thaw_net_poll_read(socket._handle, true) : __thaw_net_poll_read(socket._handle); if (result === 'pending') { tick(); return; } if (result.indexOf('ok:') === 0) { var data = Buffer.from(result.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket._refreshTimeout(); try { var canContinue = socketReadablePush.call(socket, data); if (generation !== socket._readGeneration) return; if (!canContinue) { socket._reading = false; return; } } catch (error) { if (generation === socket._readGeneration) { try { socket.destroy(); } catch (_) {} } throw error; } } if (socket._handle && !socket.destroyed && !socket._paused && !socket._httpReadPaused) tick(); else socket._reading = false; return; } var failed = result.indexOf('err:') === 0; socket._reading = false; if (!failed && socket._isServer) { socket._readEnded = true; socket.readable = false; try { socketReadablePush.call(socket, null); } catch (error) { if (generation === socket._readGeneration) { try { socket.destroy(); } catch (_) {} } throw error; } if (generation !== socket._readGeneration) return; if (socket.destroyed) return; if (!socket.writable) socket.destroy(); else if (!socket._allowHalfOpen) queueMicrotask(function() { if (generation === socket._readGeneration && !socket.destroyed && socket._readEnded && socket.writable && socket._handle) socket.end(); }); return; } socket._handle = 0; socket.readable = false; socket.writable = false; if (socket._timeoutTimer) clearTimeout(socket._timeoutTimer); if (failed) { socket.destroyed = true; var error = new Error(result.substring(4)); error.code = 'ECONNRESET'; plainTerminalEvents(socket, generation, error, false); } else plainTerminalEvents(socket, generation, null, true); } queueMicrotask(pump); };
             Socket.prototype._setHttpReadPaused = function(value) { this._httpReadPaused = Boolean(value); if (!this._httpReadPaused && !this._paused) this._startRead(); };
             Socket.prototype.pause = function() { return socketReadablePause.call(this); };
             Socket.prototype.resume = function() { socketReadableResume.call(this); if (this._handle && !this.destroyed && !this._httpReadPaused) this._startRead(); return this; };
             Socket.prototype.isPaused = function() { return Boolean(this._paused); };
             var connectWithoutRead = Socket.prototype.connect; Socket.prototype.connect = function() { var socket = this, previousGeneration = this._readGeneration, previousHandle = this._handle, previousConnecting = Boolean(this.connecting && this._connectOp), encoding = this.readableEncoding; if (previousHandle) { __thaw_net_destroy(previousHandle); this._handle = 0; } if (this._timeoutTimer) clearTimeout(this._timeoutTimer); this._timeoutTimer = null; this._chunks = []; this.readableLength = 0; this.readableEnded = false; this.readableAborted = false; this.readable = true; this._endEmitted = false; this._readableDidRead = false; this.errored = null; this.closed = false; this._decoder = null; this._readableEof = false; this.readableEncoding = null; if (this._readableState) { this._readableState.endEmitted = false; this._readableState.errorEmitted = false; } if (encoding) socketReadableSetEncoding.call(this, encoding); this._readEnded = false; this._readGeneration++; this._reading = false; this._terminalEventsGeneration = -1; this._isServer = false; this._allowHalfOpen = false; this.destroyed = false; var generation = this._readGeneration; if (previousHandle || previousConnecting) queueMicrotask(function() { plainTerminalEvents(socket, previousGeneration, null, false); }); this.once('connect', function() { if (socket._readGeneration !== generation) return; socket._refreshTimeout(); socket._startRead(); }); return connectWithoutRead.apply(this, arguments); };
             var writeWithoutTimeout = Socket.prototype.write; Socket.prototype.write = function() { var result = writeWithoutTimeout.apply(this, arguments); this._refreshTimeout(); return result; };
             Socket.prototype.setTimeout = function(timeout, callback) { this.timeout = Math.max(0, Number(timeout) || 0); if (typeof callback === 'function') this.once('timeout', callback); this._refreshTimeout(); return this; };
             Socket.prototype.destroy = function(error) { if (this.destroyed || this._terminalEventsGeneration === this._readGeneration) return this; this.destroyed = true; if (this._connectOp) { __thaw_async_cancel(this._connectOp); this._connectOp = 0; } if (this._connectTimer) { clearTimeout(this._connectTimer); this._connectTimer = null; } this.connecting = false; this._pendingWrites = null; this._pendingEnd = null; this._readGeneration++; var generation = this._readGeneration; this._reading = false; var handle = this._handle; this._handle = 0; this.readable = false; this.writable = false; if (this._timeoutTimer) clearTimeout(this._timeoutTimer); this._timeoutTimer = null; var thrown, hasThrown = false; try { if (handle) __thaw_net_destroy(handle); } catch (caught) { thrown = caught; hasThrown = true; } var socket = this; queueMicrotask(function() { plainTerminalEvents(socket, generation, error, false); }); if (hasThrown) throw thrown; return this; };
             Socket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (this.connecting && this._pendingWrites && !this._handle) { this._pendingEnd = { callback: callback }; this.writableEnded = true; return this; } if (!this._handle || !this.writable) return this; var outcome = __thaw_net_shutdown(this._handle), socket = this; this.writable = false; this.writableEnded = true; if (outcome !== 'ok') { var shutdownError = new Error(outcome.substring(4)); shutdownError.code = 'ECONNRESET'; if (typeof callback === 'function') queueMicrotask(function() { callback(shutdownError); }); this.destroy(shutdownError); return this; } queueMicrotask(function() { socket.writableFinished = true; if (typeof callback === 'function') callback(); socket.emit('finish'); }); if (this._isServer && this._readEnded) this.destroy(); else this._startRead(); return this; };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var outcome = __thaw_net_listen(hostname, Number(options.port)); if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_net_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { var error = new Error(accepted.substring(4)); error.code = 'ECONNABORTED'; server.emit('error', error); if (server._handle) setTimeout(pump, 1); return; } var peer = acceptedPeer(accepted); var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); if (server._closePending && server.connections === 0) queueMicrotask(function() { if (server._closePending) { server._closePending = false; server.emit('close'); } }); }); server.emit('connection', socket); var incoming = __thaw_net_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else { var readError = new Error(incoming.substring(4)); readError.code = 'ECONNRESET'; socket.emit('error', readError); } if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
"#,
            r#"
             Server.prototype.listen = function() {
               var normalized = netArgs(arguments), options = normalized.options, callback = normalized.callback;
               if (this.listening) { var listenError = new Error('Listen method has been called more than once without closing.'); listenError.code = 'ERR_SERVER_ALREADY_LISTEN'; throw listenError; }
               if (typeof callback === 'function') this.once('listening', callback);
               var hostname = options.host ? String(options.host) : '::', outcome = __thaw_net_listen(hostname, Number(options.port || 0));
               if (!options.host && outcome.indexOf('ok:') !== 0) { hostname = '0.0.0.0'; outcome = __thaw_net_listen(hostname, Number(options.port || 0)); }
               if (outcome.indexOf('ok:') !== 0) { var error = new Error(outcome.substring(4)); error.code = 'EADDRINUSE'; queueMicrotask(() => this.emit('error', error)); return this; }
               var fields = outcome.split(':'), server = this;
               this._handle = Number(fields[1]); this._address = { address: hostname, family: isIPv6(hostname) ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true;
               function schedulePump(delay) { server._pollTimer = __thaw_set_timeout_ref(pump, delay, server._refed); }
               function pump() {
                 server._pollTimer = 0;
                 if (!server._handle || !server.listening) return;
                 var accepted = __thaw_net_poll_accept(server._handle);
                 if (accepted === 'err:pending') { schedulePump(1); return; }
                 if (accepted.indexOf('ok:') !== 0) { var error = new Error(accepted.substring(4)); error.code = 'ECONNABORTED'; server.emit('error', error); if (server._handle) schedulePump(1); return; }
                 var peer = acceptedPeer(accepted); if (server.maxConnections > 0 && server.connections >= server.maxConnections) { __thaw_net_destroy(Number(peer[1])); server.emit('drop', { localAddress: server._address.address, localPort: server._address.port, localFamily: server._address.family, remoteAddress: peer[2], remotePort: Number(peer[3]), remoteFamily: isIPv6(peer[2]) ? 'IPv6' : 'IPv4' }); schedulePump(0); return; } var socket = new Socket(); socket._handle = Number(peer[1]); socket.pending = false; socket.readable = true; socket.writable = true; socket._isServer = true; socket._allowHalfOpen = server._allowHalfOpen; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = isIPv6(peer[2]) ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); if (server._closePending && server.connections === 0) queueMicrotask(function() { if (server._closePending) { server._closePending = false; server.emit('close'); } }); }); try { server.emit('connection', socket); socket._startRead(); } catch (error) { try { socket.destroy(); } catch (_) {} if (server._handle) schedulePump(0); throw error; }
                 if (server._handle) schedulePump(0);
               }
               queueMicrotask(function() { server.emit('listening'); pump(); }); return this;
             };
             Server.prototype.ref = function() { this._refed = true; if (this._pollTimer) __thaw_set_timer_ref(this._pollTimer, true); return this; };
             Server.prototype.unref = function() { this._refed = false; if (this._pollTimer) __thaw_set_timer_ref(this._pollTimer, false); return this; };
"#,
        )),
        "tls" => Some(concat!(
            r#"function tlsCredentialHex(name, value) { if (value === undefined) return ''; if (typeof value !== 'string' && !ArrayBuffer.isView(value)) { var error = new TypeError('The "options.' + name + '" property must be of type string or an instance of Buffer, TypedArray, or DataView'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } return Buffer.from(value).toString('hex'); }
function serializeCa(value) { if (value === undefined) return undefined; var values = Array.isArray(value) ? value : [value]; if (!Array.isArray(value) && typeof value !== 'string' && !Buffer.isBuffer(value)) throw new TypeError('CA must be a string, Buffer, or array'); var encoded = []; for (var i = 0; i < values.length; i++) { if (!Object.prototype.hasOwnProperty.call(values, i)) throw new TypeError('CA array contains a hole'); var entry = values[i]; if (typeof entry !== 'string' && !Buffer.isBuffer(entry)) throw new TypeError('CA entries must be strings or Buffers'); if (Array.isArray(value) && (entry.length === 0)) throw new TypeError('CA entries must not be empty'); encoded.push(Buffer.from(entry).toString('hex')); } return encoded.join(','); } function TLSSocket(socket, options) { if (!(this instanceof TLSSocket)) return new TLSSocket(socket, options); this._events = Object.create(null); this._handle = 0; this.connecting = false; this.destroyed = false; this.readable = false; this.writable = false; this.encrypted = true; this.authorized = false; this.authorizationError = null; this.alpnProtocol = false; this.servername = null; this.bytesRead = 0; this.bytesWritten = 0; this._refed = true; this._identityGeneration = 0; if (socket && typeof socket === 'object' && Number(socket._handle) > 0 && typeof __thaw_tls_wrap_start === 'function' && !(options && options.isServer)) { var self = this, wrapOptions = Object.assign({}, options || {}, { socket: socket }); queueMicrotask(function() { self.connect(wrapOptions); }); } } TLSSocket.prototype.on = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: false }); return this; }; TLSSocket.prototype.once = function(name, listener) { var key = String(name); (this._events[key] || (this._events[key] = [])).push({ listener: listener, once: true }); return this; }; TLSSocket.prototype.off = TLSSocket.prototype.removeListener = function(name, listener) { var key = String(name); this._events[key] = (this._events[key] || []).filter(function(entry) { return entry.listener !== listener; }); return this; }; TLSSocket.prototype.emit = function(name) { var key = String(name), list = (this._events[key] || []).slice(), args = Array.prototype.slice.call(arguments, 1); if (key === 'error' && !list.length) throw args[0] instanceof Error ? args[0] : new Error('Unhandled error event'); list.forEach(function(entry) { if (entry.once) this.off(key, entry.listener); entry.listener.apply(this, args); }, this); return list.length > 0; }; TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; var caHex = serializeCa(options.ca); if (typeof listener === 'function') this.once('secureConnect', listener); var host = String(options.host || 'localhost'), port = Number(options.port), servername = String(options.servername || host); this.connecting = true; var outcome = __thaw_tls_connect(host, port, servername, caHex); this.connecting = false; this.servername = servername; if (outcome.indexOf('ok:') === 0) { this._handle = Number(outcome.substring(3)); this.readable = true; this.writable = true; this.authorized = true; queueMicrotask(() => { this.emit('connect'); this.emit('secureConnect'); }); } else { var error = connectFailureError(outcome, host, port, 'ERR_TLS_CERT_ALTNAME_INVALID'); this.authorizationError = error.message; this.destroyed = true; queueMicrotask(() => { this.emit('error', error); this.emit('close', true); }); } return this; }; function connectFailureError(outcome, hostname, port, fallbackCode) { var found = /^err:([A-Z][A-Z0-9_]*)\|(-?\d+)\|([a-z]+)\|([^|]*)\|([\s\S]*)$/.exec(outcome); if (!found) { var plain = new Error(outcome.substring(4)); plain.code = fallbackCode; if (fallbackCode === 'ECONNREFUSED') { plain.syscall = 'connect'; plain.address = hostname; plain.port = port; } return plain; } var failure; if (found[3] === 'getaddrinfo') { failure = new Error('getaddrinfo ' + found[1] + ' ' + hostname); failure.errno = Number(found[2]); failure.code = found[1]; failure.syscall = found[3]; failure.hostname = hostname; } else { failure = new Error(found[3] + ' ' + found[1] + ' ' + (found[4] || hostname) + ':' + port); failure.errno = Number(found[2]); failure.code = found[1]; failure.syscall = found[3]; failure.address = found[4] || hostname; failure.port = port; } return failure; } function socketWriteRejected(socket, callback) { var destroyed = socket.destroyed === true; var failure = new Error(destroyed ? 'Cannot call write after a stream was destroyed' : 'write after end'); failure.code = destroyed ? 'ERR_STREAM_DESTROYED' : 'ERR_STREAM_WRITE_AFTER_END'; queueMicrotask(function() { if (typeof callback === 'function') { try { callback(failure); } catch (_) {} } if (!destroyed) socket.destroy(failure); }); return false; } function socketWriteFailure(socket, outcome, callback) { var text = String(outcome).substring(4); var code = /Broken pipe/.test(text) ? 'EPIPE' : 'ECONNRESET'; var failure = new Error('write ' + code); failure.code = code; failure.syscall = 'write'; failure.errno = code === 'EPIPE' ? -32 : -104; queueMicrotask(function() { if (typeof callback === 'function') { try { callback(failure); } catch (_) {} } socket.destroy(failure); }); return false; } TLSSocket.prototype.write = function(value, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.connecting && this._pendingWrites && !this._handle) { var queuedChunk = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); this._pendingWrites.push({ chunk: queuedChunk, callback: callback }); this.bytesWritten += queuedChunk.length; return true; } if (!this._handle || !this.writable) return socketWriteRejected(this, callback); var buffer = Buffer.isBuffer(value) ? value : Buffer.from(value, encoding); var outcome = __thaw_tls_write(this._handle, buffer.toString('hex')); if (outcome !== 'ok') return socketWriteFailure(this, outcome, callback); this.bytesWritten += buffer.length; if (callback) queueMicrotask(callback); return true; }; TLSSocket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); var outcome = __thaw_tls_finish(this._handle); this._handle = 0; this.writable = false; this.readable = false; this.destroyed = true; queueMicrotask(() => { if (outcome.indexOf('ok:') === 0) { var data = Buffer.from(outcome.substring(3), 'hex'); if (data.length) { this.bytesRead += data.length; this.emit('data', data); } this.emit('end'); if (callback) callback(); this.emit('close', false); } else { var error = new Error(outcome.substring(4)); error.code = 'ECONNRESET'; this.emit('error', error); this.emit('close', true); } }); return this; }; TLSSocket.prototype.destroy = function(error) { if (this._handle) __thaw_tls_destroy(this._handle); this._handle = 0; this.destroyed = true; if (error) queueMicrotask(() => this.emit('error', error)); queueMicrotask(() => this.emit('close', Boolean(error))); return this; }; TLSSocket.prototype.setEncoding = function(encoding) { var emit = this.emit; this.emit = function(name, value) { if (name === 'data' && Buffer.isBuffer(value)) value = value.toString(encoding); return emit.call(this, name, value); }; return this; }; TLSSocket.prototype._sessionInfo = function() { if (!this._handle || this.destroyed) return null; if (this._sessionInfoHandle !== this._handle) { var raw = __thaw_tls_session_info(this._handle); this._sessionInfoHandle = this._handle; this._sessionInfoValue = raw ? JSON.parse(raw) : null; } return this._sessionInfoValue; }; TLSSocket.prototype.getProtocol = function() { var info = this._sessionInfo(); return info ? info.protocol : null; }; TLSSocket.prototype.getCipher = function() { var info = this._sessionInfo(); return info ? { name: info.name, standardName: info.standardName, version: info.protocol } : null; }; TLSSocket.prototype.getPeerCertificate = function() { return this.authorized ? { subject: {}, issuer: {}, valid_from: '', valid_to: '' } : {}; }; TLSSocket.prototype.getCertificate = function() { return {}; }; TLSSocket.prototype.getFinished = TLSSocket.prototype.getPeerFinished = function() { return undefined; }; TLSSocket.prototype.isSessionReused = function() { return false; }; TLSSocket.prototype.renegotiate = function(options, callback) { if (callback) queueMicrotask(function() { callback(new Error('TLS renegotiation is not supported')); }); return false; }; TLSSocket.prototype.setMaxSendFragment = function() { return true; }; TLSSocket.prototype.enableTrace = function() {}; TLSSocket.prototype.setNoDelay = function(noDelay) { this._noDelay = noDelay === undefined ? true : Boolean(noDelay); if (this._handle) __thaw_net_set_option('tls', this._handle, 'nodelay', this._noDelay, 0); return this; };
TLSSocket.prototype.setKeepAlive = function(enable, initialDelay) { this._keepAlive = { enable: Boolean(enable), delay: Number(initialDelay) || 0 }; if (this._handle) __thaw_net_set_option('tls', this._handle, 'keepalive', this._keepAlive.enable, this._keepAlive.delay); return this; };
TLSSocket.prototype.ref = function() { this._refed = true; if (this._connectTimer) __thaw_set_timer_ref(this._connectTimer, true); return this; }; TLSSocket.prototype.unref = function() { this._refed = false; if (this._connectTimer) __thaw_set_timer_ref(this._connectTimer, false); return this; }; function connect() { var list = Array.prototype.slice.call(arguments), listener = typeof list[list.length - 1] === 'function' ? list.pop() : undefined, options; if (list[0] !== null && typeof list[0] === 'object') options = list[0]; else { options = { port: list[0] }; var next = 1; if (typeof list[1] === 'string') { options.host = list[1]; next = 2; } if (list[next] !== null && typeof list[next] === 'object') options = Object.assign({}, list[next], options); } return new TLSSocket().connect(options, listener); } function SecureContext(options) { if (!(this instanceof SecureContext)) return new SecureContext(options); this.context = options || {}; this.options = options || {}; } SecureContext.prototype.setKey = function(key) { this.options.key = key; }; SecureContext.prototype.setCert = function(cert) { this.options.cert = cert; }; SecureContext.prototype.addCACert = function(ca) { var list = this.options.ca || (this.options.ca = []); if (!Array.isArray(list)) list = this.options.ca = [list]; list.push(ca); }; function createSecureContext(options) { return new SecureContext(options); }
function identityError(host, cert, reason, code) { var error = new Error(reason); error.code = code || 'ERR_TLS_CERT_ALTNAME_INVALID'; error.reason = reason; error.host = host; error.cert = cert; return error; }
function parseSubjectAltName(value) {
  var tokens = [], current = '', quoted = false, escaped = false;
  for (var i = 0; i < value.length; i++) { var ch = value[i]; if (escaped) { current += ch; escaped = false; continue; } if (quoted && ch === '\\') { current += ch; escaped = true; continue; } if (ch === '"') quoted = !quoted; if (ch === ',' && !quoted) { tokens.push(current.trim()); current = ''; } else current += ch; }
  if (quoted || escaped) return { error: 'Malformed subjectAltName quoting' };
  if (current || value.length) tokens.push(current.trim());
  var dns = [], ips = [];
  for (var j = 0; j < tokens.length; j++) { var token = tokens[j], match = /^DNS:(.*)$/i.exec(token); if (match) { var name = match[1]; if (name[0] === '"') { if (name.length < 2 || name[name.length - 1] !== '"') return { error: 'Malformed quoted DNS subjectAltName' }; try { name = JSON.parse(name); } catch (_) { return { error: 'Malformed quoted DNS subjectAltName' }; } if (typeof name !== 'string') return { error: 'Malformed quoted DNS subjectAltName' }; } else if (name.indexOf('"') >= 0 || name.indexOf('\\') >= 0) return { error: 'Malformed DNS subjectAltName' }; dns.push(name); continue; } match = /^IP Address:(.*)$/i.exec(token); if (match) { ips.push(match[1]); continue; } if (/^(?:DNS|IP Address):/.test(token)) return { error: 'Malformed subjectAltName token' }; }
  return { dns: dns, ips: ips };
}
function dnsNameMatch(host, certName) {
  if (typeof certName !== 'string' || !certName || /[^\x00-\x7f]/.test(certName) || /[\x00-\x20\x7f]/.test(certName)) return false;
  var expected = host.toLowerCase(), pattern = certName.toLowerCase(); if (expected.endsWith('.')) expected = expected.slice(0, -1); if (pattern.endsWith('.')) pattern = pattern.slice(0, -1);
  var hostLabels = expected.split('.'), certLabels = pattern.split('.');
  if (hostLabels.some(function(label) { return !label || !/^[a-z0-9-]+$/.test(label) || label[0] === '-' || label[label.length - 1] === '-'; })) return false;
  if (certLabels.length !== hostLabels.length || certLabels.some(function(label, index) { if (label === '*') return index !== 0 || certLabels.length <= 2; return !label || !/^[a-z0-9-]+$/.test(label) || label[0] === '-' || label[label.length - 1] === '-'; })) return false;
  return certLabels[0] === '*' ? hostLabels.slice(1).join('.') === certLabels.slice(1).join('.') : expected === pattern;
}
function checkServerIdentity(hostname, cert) {
  hostname = String(hostname); cert = cert || {};
  var hostIp = __thaw_tls_canonical_ip(hostname), dns = [], ips = [], hasDns = false;
  if (cert.sanError) return identityError(hostname, cert, 'Malformed subjectAltName extension', 'ERR_TLS_CERT_ALTNAME_FORMAT');
  if (Array.isArray(cert.dnsNames)) { dns = cert.dnsNames; hasDns = cert.hasDnsSan === true || (cert.hasDnsSan !== false && Object.prototype.hasOwnProperty.call(cert, 'dnsNames')); } if (Array.isArray(cert.ipAddresses)) ips = cert.ipAddresses;
  if (typeof cert.subjectaltname === 'string' && !Array.isArray(cert.dnsNames) && !Array.isArray(cert.ipAddresses)) { var parsed = parseSubjectAltName(cert.subjectaltname); if (parsed.error) return identityError(hostname, cert, parsed.error, 'ERR_TLS_CERT_ALTNAME_FORMAT'); dns = parsed.dns; ips = parsed.ips; hasDns = dns.length > 0; }
  if (hostIp) { if (ips.some(function(ip) { return __thaw_tls_canonical_ip(ip) === hostIp; })) return undefined; }
  else {
    if (!hostname || /[^\x00-\x7f]/.test(hostname) || /[\x00-\x20\x7f]/.test(hostname)) return identityError(hostname, cert, 'Invalid hostname');
    if (dns.some(function(name) { return dnsNameMatch(hostname, name); })) return undefined;
    if (!hasDns) { var cn = cert.subject && cert.subject.CN, names = Array.isArray(cn) ? cn : (typeof cn === 'string' ? [cn] : []); if (names.some(function(name) { return dnsNameMatch(hostname, name); })) return undefined; }
  }
  return identityError(hostname, cert, "Hostname/IP does not match certificate's altnames: " + String(cert.subjectaltname || (cert.subject && cert.subject.CN) || 'no names'));
}
function finalizeIdentity(socket, options, handle) {
  var generation = socket._identityGeneration || 0; var native = JSON.parse(__thaw_tls_authorization(handle));
  if (socket.destroyed || socket._handle !== handle || (socket._identityGeneration || 0) !== generation) return false;
  socket.authorized = Boolean(native.authorized); socket.authorizationError = native.authorizationError || null;
  if (!native.chainVerified) { if (options.rejectUnauthorized !== false) { var chainError = new Error(socket.authorizationError || 'Server certificate could not be verified'); chainError.code = 'UNABLE_TO_VERIFY_LEAF_SIGNATURE'; chainError.reason = chainError.message; chainError.host = socket.servername || ''; chainError.cert = socket.getPeerCertificate(); if (socket._handle === handle) socket.destroy(chainError); return false; } return true; }
  var checker = options && options.checkServerIdentity;
  if (checker !== undefined && typeof checker !== 'function') throw new TypeError('checkServerIdentity must be a function');
  if (typeof checker !== 'function') checker = checkServerIdentity;
  var result, thrown = false;
  try { result = checker(socket.servername || socket.remoteAddress || '', socket.getPeerCertificate()); }
  catch (error) { result = error; thrown = true; }
  if (socket.destroyed || socket._handle !== handle || (socket._identityGeneration || 0) !== generation) return false;
  if (!thrown && !result) { if (native.authorized) { socket.authorized = true; socket.authorizationError = null; } return true; }
  var failure = { authorizationError: 'TLS identity check failed', message: 'TLS identity check failed', code: undefined }, normalizationFailed = false;
  try {
    var code = result == null ? undefined : result.code;
    var message = result == null ? undefined : result.message;
    var codeText = code ? String(code) : undefined;
    var messageText = message ? (message === code ? codeText : String(message)) : undefined;
    var selectedText = codeText || messageText || String(result);
    failure.code = codeText; failure.authorizationError = selectedText; failure.message = messageText || selectedText;
  } catch (_) { normalizationFailed = true; }
  if (socket.destroyed || socket._handle !== handle || (socket._identityGeneration || 0) !== generation) return false;
  socket.authorized = false; socket.authorizationError = failure.authorizationError;
  if (thrown || normalizationFailed || options.rejectUnauthorized !== false) {
    var identityError = new Error(failure.message);
    if (failure.code) identityError.code = failure.code;
    if (normalizationFailed) identityError.code = 'ERR_TLS_IDENTITY_NORMALIZATION';
    if (socket._handle === handle && !socket.destroyed && (socket._identityGeneration || 0) === generation) socket.destroy(identityError);
    return false;
  }
  return true;
}
var tlsPreparedConnectKey = Symbol('thawTlsPreparedConnect'); function getCiphers() { return ['tls_aes_128_gcm_sha256', 'tls_aes_256_gcm_sha384', 'tls_chacha20_poly1305_sha256']; } module.exports = { TLSSocket: TLSSocket, SecureContext: SecureContext, connect: connect, createSecureContext: createSecureContext, checkServerIdentity: checkServerIdentity, getCiphers: getCiphers, __thawSerializeCa: serializeCa, __thawFinalizeIdentity: finalizeIdentity, rootCertificates: [], DEFAULT_MIN_VERSION: 'TLSv1.2', DEFAULT_MAX_VERSION: 'TLSv1.3', CLIENT_RENEG_LIMIT: 3, CLIENT_RENEG_WINDOW: 600 }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
            r#"function acceptedTlsPeer(value) { var fields = value.split(':'), last = fields.length - 1; return ['ok', fields[1], fields.slice(2, last - 1).join(':'), fields[last - 1], fields[last]]; } 
             function Server(options, listener) { if (!(this instanceof Server)) return new Server(options, listener); this._events = Object.create(null); this._options = options || {}; this._handle = 0; this._address = null; this.listening = false; this.connections = 0; this.maxConnections = 0; this._refed = true; if (typeof listener === 'function') this.on('secureConnection', listener); }
             Server.prototype.on = TLSSocket.prototype.on; Server.prototype.once = TLSSocket.prototype.once; Server.prototype.off = Server.prototype.removeListener = TLSSocket.prototype.off; Server.prototype.emit = TLSSocket.prototype.emit;
             function terminalEvents(socket, error, ended) { var failed = false, first; function deliver(fn) { try { fn(); } catch (caught) { if (!failed) { failed = true; first = caught; } } } if (error) deliver(function() { socket.emit('error', error); }); if (ended) deliver(function() { socket.emit('end'); }); deliver(function() { socket.emit('close', Boolean(error)); }); if (failed) throw first; }
             TLSSocket.prototype.destroy = function(error) { if (this.destroyed) return this; this.destroyed = true; if (this._connectOp) { __thaw_async_cancel(this._connectOp); this._connectOp = 0; } if (this._connectTimer) { clearTimeout(this._connectTimer); this._connectTimer = null; } this.connecting = false; this._pendingWrites = null; this._pendingEnd = null; this._identityGeneration = (this._identityGeneration || 0) + 1; var generation = this._identityGeneration; this._reading = false; var handle = this._handle; this._handle = 0; this.readable = false; this.writable = false; if (handle) __thaw_tls_destroy(handle); var socket = this; queueMicrotask(function() { if (socket._identityGeneration === generation) terminalEvents(socket, error, false); }); return this; };
             TLSSocket.prototype._finishRead = function() { if (this.destroyed) return; this.destroyed = true; this._reading = false; var handle = this._handle; this._handle = 0; this.readable = false; this.writable = false; if (handle) __thaw_tls_destroy(handle); var socket = this; queueMicrotask(function() { terminalEvents(socket, null, false); }); };
             TLSSocket.prototype._startRead = function() { if (this._reading || this._paused || this._httpReadPaused || this._readEnded || !this._readReady || !this._handle || this.destroyed) return; this._reading = true; var socket = this; function pump() { if (!socket._handle || socket.destroyed || socket._paused || socket._httpReadPaused || socket._readEnded) { socket._reading = false; return; } var result = __thaw_tls_poll_read(socket._handle); if (result === 'pending') { setTimeout(pump, 0); return; } if (result.indexOf('ok:') === 0) { var data = Buffer.from(result.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; try { socket.emit('data', data); } catch (error) { try { socket.destroy(); } catch (_) {} throw error; } } if (socket._handle && !socket.destroyed && !socket._paused && !socket._httpReadPaused) setTimeout(pump, 0); else socket._reading = false; return; } socket._reading = false; if (result.indexOf('err:') === 0) { var error = new Error(result.substring(4)); error.code = 'ECONNRESET'; socket.destroy(error); return; } if (socket._isServer) { socket._readEnded = true; socket.readable = false; try { socket.emit('end'); } catch (error) { try { socket.destroy(); } catch (_) {} throw error; } if (!socket.writable) socket._finishRead(); return; } socket._handle = 0; socket.readable = false; socket.writable = false; socket.destroyed = true; terminalEvents(socket, null, true); } queueMicrotask(pump); };
             TLSSocket.prototype._setHttpReadPaused = function(value) { this._httpReadPaused = Boolean(value); if (!this._httpReadPaused && !this._paused) this._startRead(); };
             TLSSocket.prototype.pause = function() { this._paused = true; return this; };
             TLSSocket.prototype.resume = function() { this._paused = false; this._startRead(); return this; };
             TLSSocket.prototype.isPaused = function() { return Boolean(this._paused); };
             TLSSocket.prototype.end = function(value, encoding, callback) { if (typeof value === 'function') { callback = value; value = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (value !== undefined) this.write(value, encoding); if (this.connecting && this._pendingWrites && !this._handle) { this._pendingEnd = { callback: callback }; return this; } if (!this._handle || !this.writable) return this; var outcome = __thaw_tls_shutdown(this._handle); this.writable = false; if (outcome !== 'ok') { var failure = new Error(outcome.substring(4)); failure.code = 'ECONNRESET'; if (typeof callback === 'function') queueMicrotask(function() { callback(failure); }); this.destroy(failure); return this; } if (typeof callback === 'function') { var socket = this; queueMicrotask(function() { try { callback(); } catch (error) { try { socket.destroy(); } catch (_) {} throw error; } }); } this._readReady = true; if (this._isServer && this._readEnded) this._finishRead(); else this._startRead(); return this; };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var outcome = __thaw_tls_server_listen(hostname, Number(options.port), Buffer.from(cert).toString('hex'), Buffer.from(key).toString('hex')); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; queueMicrotask(() => { this.emit('listening'); var accepted = __thaw_tls_server_accept(this._handle); if (accepted.indexOf('ok:') !== 0) { this.emit('tlsClientError', new Error(accepted.substring(4))); return; } var peer = acceptedTlsPeer(accepted); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; this.connections++; this.emit('secureConnection', socket); var incoming = __thaw_tls_server_read(socket._handle); if (incoming.indexOf('ok:') === 0) { var data = Buffer.from(incoming.substring(3), 'hex'); if (data.length) { socket.bytesRead += data.length; socket.emit('data', data); } socket.emit('end'); } else socket.emit('error', new Error(incoming.substring(4))); }); return this; };
             Server.prototype.address = function() { return this._address; }; Server.prototype.getConnections = function(callback) { queueMicrotask(() => callback(null, this.connections)); }; Server.prototype.close = function(callback) { if (typeof callback === 'function') this.once('close', callback); if (this._handle) __thaw_tls_server_close(this._handle); this._handle = 0; this.listening = false; this._closePending = true; if (this.connections === 0 || this._closeNow) queueMicrotask(() => { if (this._closePending) { this._closePending = false; this.emit('close'); } }); return this; }; Server.prototype.ref = function() { this._refed = true; return this; }; Server.prototype.unref = function() { this._refed = false; return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var connectWithoutIdentity = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function(options, listener) { options = options || {}; if (options.cert === undefined && options.key === undefined) return connectWithoutIdentity.call(this, options, listener); serializeCa(options.ca); var cert = options.cert, key = options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; var nativeConnect = __thaw_tls_connect; __thaw_tls_connect = function(host, port, servername, ca) { return __thaw_tls_connect_with_identity(host, port, servername, ca, cert === undefined ? '' : Buffer.from(cert).toString('hex'), key === undefined ? '' : Buffer.from(key).toString('hex')); }; try { return connectWithoutIdentity.call(this, options, listener); } finally { __thaw_tls_connect = nativeConnect; } };
             var listenWithoutClientAuth = Server.prototype.listen; Server.prototype.listen = function() { if (!this._options.requestCert) return listenWithoutClientAuth.apply(this, arguments); var caHex = serializeCa(this._options.ca); var rejectUnauthorized = this._options.rejectUnauthorized !== false; var nativeListen = __thaw_tls_server_listen; __thaw_tls_server_listen = function(host, port, cert, key) { return __thaw_tls_server_listen_with_ca(host, port, cert, key, caHex, rejectUnauthorized); }; try { return listenWithoutClientAuth.apply(this, arguments); } finally { __thaw_tls_server_listen = nativeListen; } };
             Server.prototype.listen = function(port, host, callback) { var options = typeof port === 'object' ? port : { port: port, host: host }; if (typeof host === 'function') callback = host; var caHex = serializeCa(this._options.ca); if (typeof callback === 'function') this.once('listening', callback); var hostname = String(options.host || '127.0.0.1'); var cert = this._options.cert, key = this._options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; if (cert === undefined || key === undefined) { queueMicrotask(() => this.emit('error', new Error('cert and key are required'))); return this; } var certHex = Buffer.from(cert).toString('hex'), keyHex = Buffer.from(key).toString('hex'); var outcome = this._options.requestCert ? __thaw_tls_server_listen_with_ca(hostname, Number(options.port), certHex, keyHex, caHex, this._options.rejectUnauthorized !== false) : __thaw_tls_server_listen(hostname, Number(options.port), certHex, keyHex); if (outcome.indexOf('ok:') !== 0) { queueMicrotask(() => this.emit('error', new Error(outcome.substring(4)))); return this; } var fields = outcome.split(':'); this._handle = Number(fields[1]); this._address = { address: hostname, family: hostname.indexOf(':') >= 0 ? 'IPv6' : 'IPv4', port: Number(fields[2]) }; this.listening = true; var server = this; function pump() { if (!server._handle || !server.listening) return; if (server.maxConnections > 0 && server.connections >= server.maxConnections) { setTimeout(pump, 1); return; } var accepted = __thaw_tls_server_poll_accept(server._handle); if (accepted === 'err:pending') { setTimeout(pump, 1); return; } if (accepted.indexOf('ok:') !== 0) { server.emit('tlsClientError', new Error(accepted.substring(4))); if (server._handle) setTimeout(pump, 1); return; } var peer = acceptedTlsPeer(accepted); var socket = new TLSSocket(); socket._handle = Number(peer[1]); socket.authorized = true; socket.readable = true; socket.writable = true; socket.remoteAddress = peer[2]; socket.remotePort = Number(peer[3]); socket.remoteFamily = peer[2].indexOf(':') >= 0 ? 'IPv6' : 'IPv4'; server.connections++; socket.once('close', function() { server.connections = Math.max(0, server.connections - 1); if (server._closePending && server.connections === 0) queueMicrotask(function() { if (server._closePending) { server._closePending = false; server.emit('close'); } }); }); socket._isServer = true; socket._readReady = true; try { server.emit('secureConnection', socket); socket._startRead(); } catch (error) { try { socket.destroy(); } catch (_) {} if (server._handle) setTimeout(pump, 0); throw error; } if (server._handle) setTimeout(pump, 0); } queueMicrotask(function() { server.emit('listening'); pump(); }); return this; };
             function tlsVersionField(options) { var spec = tlsVersionSpec(options); return spec === ',' ? '' : '|' + spec; } function tlsVersionSpec(options) { var order = ['TLSv1', 'TLSv1.1', 'TLSv1.2', 'TLSv1.3']; function check(value, name) { if (value === undefined || value === null) return ''; if (order.indexOf(value) < 0) { var error = new TypeError(JSON.stringify(value) + ' is not a valid ' + name + ' TLS protocol version'); error.code = 'ERR_TLS_INVALID_PROTOCOL_VERSION'; throw error; } return value; } var minimum = check(options.minVersion, 'minVersion'), maximum = check(options.maxVersion, 'maxVersion'), method = options.secureProtocol; if (typeof method === 'string') { var pinned = /^TLSv1(?:_([123]))?_(?:client_|server_)?method$/.exec(method); if (pinned) { var version = pinned[1] ? 'TLSv1.' + pinned[1] : 'TLSv1'; minimum = minimum || version; maximum = maximum || version; } } return minimum + ',' + maximum; }
             function alpnSpec(value) { if (value === undefined) return ''; if (Buffer.isBuffer(value)) { var protocols = [], offset = 0; while (offset < value.length) { var length = value[offset++]; if (!length || length > value.length - offset) throw new RangeError('Invalid ALPN protocol list'); protocols.push(value.subarray(offset, offset + length).toString('hex')); offset += length; } return protocols.join(','); } var values = Array.isArray(value) ? value : [value]; return values.map(function(protocol) { return Buffer.from(protocol).toString('hex'); }).join(','); }
             TLSSocket.prototype.connect = function(options, listener) { if (typeof options === 'number') options = { port: options }; options = options || {}; if (options.socket && typeof options.socket === 'object' && Number(options.socket._handle) > 0 && typeof __thaw_tls_wrap_start === 'function') { var wrappedSocket = options.socket; this._wrapHandle = Number(wrappedSocket._handle); wrappedSocket._handle = 0; options = Object.assign({}, options, { host: options.host || wrappedSocket.remoteAddress || 'localhost', port: options.port === undefined ? Number(wrappedSocket.remotePort || 0) : options.port }); } var prepared = options[tlsPreparedConnectKey], host, port, servername, caHex, certHex, keyHex, tlsOptions, rejectUnauthorized; if (prepared) { host = prepared.host; port = prepared.port; servername = prepared.servername; caHex = prepared.caHex; certHex = prepared.certHex; keyHex = prepared.keyHex; tlsOptions = prepared.tlsOptions; rejectUnauthorized = prepared.rejectUnauthorized; } else { host = String(options.host || 'localhost'); port = Number(options.port); servername = String(options.servername || host); caHex = serializeCa(options.ca); var cert = options.cert, key = options.key; if (Array.isArray(cert)) cert = cert[0]; if (Array.isArray(key)) key = key[0]; certHex = tlsCredentialHex('cert', cert); keyHex = tlsCredentialHex('key', key); if (options.checkServerIdentity !== undefined && typeof options.checkServerIdentity !== 'function') throw new TypeError('checkServerIdentity must be a function'); rejectUnauthorized = options.rejectUnauthorized !== false; tlsOptions = (rejectUnauthorized ? '1|' : '0|') + alpnSpec(options.ALPNProtocols) + '|1' + tlsVersionField(options); } if (typeof listener === 'function') this.once('secureConnect', listener); this.connecting = true; var socket = this, operation, polls = 0; try { operation = this._wrapHandle ? __thaw_tls_wrap_start(this._wrapHandle, servername, caHex, certHex, keyHex, tlsOptions) : __thaw_tls_connect_start(host, port, servername, caHex, certHex, keyHex, tlsOptions); } catch (error) { if (!prepared || this._identityGeneration === prepared.generation) this.connecting = false; throw error; } this._connectOp = operation; this._pendingWrites = []; this._pendingEnd = null; function settle(outcome) { socket._connectOp = 0; socket._connectTimer = null; if (socket.destroyed || (prepared && socket._identityGeneration !== prepared.generation)) { if (outcome.indexOf('ok:') === 0) __thaw_tls_destroy(Number(outcome.split(':')[1])); return; } socket.connecting = false; socket.servername = servername; if (outcome.indexOf('ok:') === 0) { var fields = outcome.split(':'); socket._handle = Number(fields[1]); socket.alpnProtocol = fields[2] ? Buffer.from(fields[2], 'hex').toString() : false; socket.readable = true; socket.writable = true; socket.authorized = rejectUnauthorized; socket.authorizationError = socket.authorized ? null : 'UNABLE_TO_VERIFY_LEAF_SIGNATURE'; var connectedHandle = socket._handle, connectedGeneration = socket._identityGeneration || 0; if (socket._noDelay !== undefined) __thaw_net_set_option('tls', socket._handle, 'nodelay', socket._noDelay, 0); if (socket._keepAlive) __thaw_net_set_option('tls', socket._handle, 'keepalive', socket._keepAlive.enable, socket._keepAlive.delay); if (prepared && prepared.afterConnect) prepared.afterConnect.call(socket); var queued = socket._pendingWrites || [], deferredCallbacks = []; socket._pendingWrites = null; if (!socket.destroyed && socket._handle === connectedHandle) { var flushError = null; queued.forEach(function(entry) { if (flushError) return; var written = __thaw_tls_write(connectedHandle, entry.chunk.toString('hex')); if (written !== 'ok') { flushError = new Error(written.substring(4)); return; } if (typeof entry.callback === 'function') deferredCallbacks.push(entry.callback); }); if (flushError) { queueMicrotask(function() { if (!socket.destroyed) socket.destroy(flushError); }); return; } } queueMicrotask(() => { if (socket.destroyed || socket._handle !== connectedHandle || (socket._identityGeneration || 0) !== connectedGeneration) return; try { socket.emit('connect'); if (socket.destroyed || socket._handle !== connectedHandle || (socket._identityGeneration || 0) !== connectedGeneration) return; socket.emit('secureConnect'); if (!socket.destroyed && socket._handle === connectedHandle && (socket._identityGeneration || 0) === connectedGeneration) { socket._readReady = true; socket._startRead(); } if (socket._pendingEnd && !socket.destroyed && socket._handle === connectedHandle) { var pendingEnd = socket._pendingEnd; socket._pendingEnd = null; socket.end(undefined, undefined, pendingEnd.callback); } } catch (error) { try { if (socket._handle === connectedHandle && (socket._identityGeneration || 0) === connectedGeneration) socket.destroy(); } catch (_) {} throw error; } }); deferredCallbacks.forEach(function(callback) { queueMicrotask(callback); }); } else { var error = connectFailureError(outcome, host, port, 'ERR_TLS_CERT_ALTNAME_INVALID'), failedGeneration = socket._identityGeneration || 0; socket.authorizationError = error.message; socket.destroyed = true; queueMicrotask(() => { if (socket._identityGeneration === failedGeneration) terminalEvents(socket, error, false); }); }  } function netConnectStep() { var outcome = __thaw_async_poll(operation); if (outcome === 'pending') { socket._connectTimer = setTimeout(netConnectStep, polls++ < 50 ? 1 : 5); if (socket._refed === false) __thaw_set_timer_ref(socket._connectTimer, false); return; } settle(outcome); } this._connectTimer = setTimeout(netConnectStep, 1); if (this._refed === false) __thaw_set_timer_ref(this._connectTimer, false); return this; };
             var listenWithoutAlpn = Server.prototype.listen; Server.prototype.listen = function() { var serverOptions = this._options, protocols = alpnSpec(serverOptions.ALPNProtocols), caHex = serializeCa(serverOptions.ca), flags = (serverOptions.requestCert ? 1 : 0) | (serverOptions.rejectUnauthorized !== false ? 2 : 0), versions = tlsVersionField(serverOptions); var basic = __thaw_tls_server_listen, withCa = __thaw_tls_server_listen_with_ca; __thaw_tls_server_listen = __thaw_tls_server_listen_with_ca = function(host, port, cert, key) { return __thaw_tls_server_listen_with_options(host, port, cert, key, caHex, flags, protocols + versions); }; try { return listenWithoutAlpn.apply(this, arguments); } finally { __thaw_tls_server_listen = basic; __thaw_tls_server_listen_with_ca = withCa; } };
             var emitWithoutAlpn = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection' && arguments[1]) { var protocol = __thaw_tls_alpn(arguments[1]._handle); arguments[1].alpnProtocol = protocol ? Buffer.from(protocol, 'hex').toString() : false; } return emitWithoutAlpn.apply(this, arguments); };
             function rememberCertificates(socket) { if (!socket || !socket._handle) return; var handle = socket._handle, peer = __thaw_tls_peer_certificate(handle), local = __thaw_tls_local_certificate(handle), authorization = JSON.parse(__thaw_tls_authorization(handle)); if (socket._handle !== handle) return; socket._peerCertificateRaw = peer ? Buffer.from(peer, 'hex') : undefined; socket._localCertificateRaw = local ? Buffer.from(local, 'hex') : undefined; socket._peerCertificateMetadata = JSON.parse(__thaw_tls_peer_certificate_metadata(handle)); socket._localCertificateMetadata = JSON.parse(__thaw_tls_local_certificate_metadata(handle)); socket.authorized = Boolean(authorization.authorized); socket.authorizationError = authorization.authorizationError || null; socket._tlsChainVerified = Boolean(authorization.chainVerified); } module.exports.__thawRememberCertificates = rememberCertificates;
             function certificateInfo(raw, metadata) { if (!raw) return {}; var digest = __thaw_crypto_hash_hex('sha256', raw.toString('hex')).toUpperCase().replace(/(..)(?=.)/g, '$1:'); var info = Object.assign({ raw: Buffer.from(raw), fingerprint256: digest }, structuredClone(metadata || {})); ['subject', 'issuer'].forEach(function(key) { var name = Object.assign({}, info[key]); if (Array.isArray(name.CN)) { if (name.CN.length === 1) name.CN = name.CN[0]; else if (!name.CN.length) delete name.CN; } info[key] = name; }); return info; }
             var connectWithoutCertificates = TLSSocket.prototype.connect; TLSSocket.prototype.connect = function() { var socket = this, ownerGeneration = this._identityGeneration || 0, ownerHandle = this._handle; function stillOwned() { return socket._identityGeneration === ownerGeneration && socket._handle === ownerHandle; } var supplied = typeof arguments[0] === 'number' ? { port: arguments[0] } : (arguments[0] || {}), options = Object.assign({}, supplied); if (options.socket && typeof options.socket === 'object' && Number(options.socket._handle) > 0 && typeof __thaw_tls_wrap_start === 'function') { var wrappedSocket = options.socket; this._wrapHandle = Number(wrappedSocket._handle); wrappedSocket._handle = 0; options.host = options.host || wrappedSocket.remoteAddress || 'localhost'; if (options.port === undefined) options.port = Number(wrappedSocket.remotePort || 0); } if (options.secureContext) { var contextOptions = options.secureContext.options || {}; ['ca', 'cert', 'key', 'minVersion', 'maxVersion', 'secureProtocol'].forEach(function(name) { if (contextOptions[name] !== undefined) options[name] = contextOptions[name]; }); } if (!stillOwned()) return this; var checker = options.checkServerIdentity; if (checker !== undefined && typeof checker !== 'function') throw new TypeError('checkServerIdentity must be a function'); var rejectUnauthorized = options.rejectUnauthorized !== false, caHex = serializeCa(options.ca); if (!stillOwned()) return this; var host = String(options.host || 'localhost'); if (!stillOwned()) return this; var port = Number(options.port); if (!stillOwned()) return this; var servername = String(options.servername || host); if (!stillOwned()) return this; var cert = options.cert; if (Array.isArray(cert)) cert = cert[0]; if (!stillOwned()) return this; var certHex = tlsCredentialHex('cert', cert); if (!stillOwned()) return this; var key = options.key; if (Array.isArray(key)) key = key[0]; if (!stillOwned()) return this; var keyHex = tlsCredentialHex('key', key); if (!stillOwned()) return this; var alpnProtocols = alpnSpec(options.ALPNProtocols); if (!stillOwned()) return this; var prepared = { host: host, port: port, servername: servername, caHex: caHex, certHex: certHex, keyHex: keyHex, tlsOptions: (rejectUnauthorized ? '1|' : '0|') + alpnProtocols + '|1' + tlsVersionField(options), rejectUnauthorized: rejectUnauthorized, checkServerIdentity: checker, generation: ownerGeneration + 1 }; var listener = arguments[1], preparedOptions = {}; preparedOptions[tlsPreparedConnectKey] = prepared; if (!stillOwned()) return this; this._identityGeneration = prepared.generation; this._handle = 0; this.destroyed = false; this.connecting = false; this.readable = false; this.writable = false; this._readReady = false; this.authorized = false; this.authorizationError = null; this._peerCertificateRaw = this._localCertificateRaw = this._peerCertificateMetadata = this._localCertificateMetadata = undefined; this._tlsChainVerified = false; if (ownerHandle) __thaw_tls_destroy(ownerHandle); if (this._identityGeneration !== prepared.generation || this._handle !== 0) return this; prepared.afterConnect = function() { if (this._identityGeneration !== prepared.generation) return; rememberCertificates(this); if (this._identityGeneration !== prepared.generation) return; if (this._handle) finalizeIdentity(this, prepared, this._handle); }; return connectWithoutCertificates.call(this, preparedOptions, listener); }; TLSSocket.prototype.getPeerCertificate = function(detailed) { if (this.destroyed) return null; var leaf = certificateInfo(this._peerCertificateRaw, this._peerCertificateMetadata); if (!detailed || !leaf.raw || !this._handle) return leaf; var chain = JSON.parse(__thaw_tls_peer_chain(this._handle)), infos = chain.map(function(hex, index) { return index === 0 ? leaf : certificateInfo(Buffer.from(hex, 'hex'), JSON.parse(__thaw_tls_certificate_metadata_hex(hex))); }); if (!infos.length) infos.push(leaf); for (var index = 0; index < infos.length; index++) infos[index].issuerCertificate = infos[index + 1] || infos[index]; return leaf; }; TLSSocket.prototype.getCertificate = function() { return this.destroyed ? null : certificateInfo(this._localCertificateRaw, this._localCertificateMetadata); };
             var emitWithoutCertificates = Server.prototype.emit; Server.prototype.emit = function(name) { if (name === 'secureConnection') rememberCertificates(arguments[1]); return emitWithoutCertificates.apply(this, arguments); };
             module.exports.Server = Server; module.exports.createServer = createServer;
"#,
        )),
        _ => None,
    }
}
