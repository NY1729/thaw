pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "http" => Some(
            r#"var net = require('node:net'), EventEmitter = require('node:events'), StringDecoder = require('node:string_decoder').StringDecoder;
             function IncomingMessage(socket) { EventEmitter.call(this); this.socket = this.connection = socket; this.statusCode = null; this.statusMessage = null; this.headers = {}; this.headersDistinct = {}; this.rawHeaders = []; this.trailers = {}; this.rawTrailers = []; this.complete = false; this.aborted = false; this.readable = true; this.method = null; this.url = ''; this.httpVersion = '1.1'; this.httpVersionMajor = 1; this.httpVersionMinor = 1; this._bodyQueue = []; this._bodyBytes = 0; this._bodyComplete = false; this._flowing = false; this._paused = false; this._drainQueued = false; this._draining = false; this._resumeQueued = false; this._decoder = null; this._decoderEnded = false; this._decoderTail = ''; this._pipeRecords = []; }
             IncomingMessage.prototype = Object.create(EventEmitter.prototype); IncomingMessage.prototype.constructor = IncomingMessage;
             IncomingMessage.prototype.setEncoding = function(encoding) { var next = new StringDecoder(encoding); if (this._decoder && !this._decoderEnded) this._decoderTail += this._decoder.end(); this._decoder = next; this._decoderEnded = false; this._encoding = encoding; if (this._decoderTail) this._scheduleBodyDrain(); return this; };
             IncomingMessage.prototype._blockedPipes = function() { return this._pipeRecords.some(function(record) { return record.active && record.blocked; }); };
             IncomingMessage.prototype._syncReadGate = function() { if (this._bodyReadGate) this._bodyReadGate(); };
             IncomingMessage.prototype._resumeBodyParser = function() { if (this._resumeQueued || !this._bodyResume) return; this._resumeQueued = true; var message = this; queueMicrotask(function() { message._resumeQueued = false; if (message._bodyResume && !message.aborted) message._bodyResume(); }); };
             IncomingMessage.prototype._abortBody = function() { this.aborted = true; this.readable = false; this._bodyQueue.length = 0; this._bodyBytes = 0; this._decoderTail = ''; var records = this._pipeRecords.slice(), thrown, hasThrown = false, message = this; records.forEach(function(record) { try { message._removePipe(record); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } } }); this._bodyResume = null; try { this._syncReadGate(); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } } if (hasThrown) throw thrown; };
             IncomingMessage.prototype.destroy = function(error) { var thrown, hasThrown = false; try { this._abortBody(); } catch (reason) { thrown = reason; hasThrown = true; } try { if (this.socket) this.socket.destroy(error); } catch (reason) { if (!hasThrown) { thrown = reason; hasThrown = true; } } if (hasThrown) throw thrown; return this; };
             IncomingMessage.prototype._scheduleBodyDrain = function() { if (this._drainQueued || this.readableEnded || this.aborted) return; this._drainQueued = true; var message = this; queueMicrotask(function() { message._drainQueued = false; message._drainBody(); }); };
             IncomingMessage.prototype._drainBody = function() { if (this._draining || this.aborted) return; this._draining = true; try { while (this._flowing && !this._paused && !this._blockedPipes() && (this._decoderTail || this._bodyQueue.length)) { if (this._decoderTail) { var oldTail = this._decoderTail; this._decoderTail = ''; this.emit('data', oldTail); continue; } var body = this._bodyQueue.shift(); this._bodyBytes -= body.length; var value = this._decoder ? this._decoder.write(body) : body; if (!this._decoder || value.length) this.emit('data', value); } if (this._bodyComplete && !this.aborted && !this._bodyQueue.length && !this._decoderTail && !this._paused && !this._blockedPipes() && !this.readableEnded && (this._flowing || this.listenerCount('end'))) { if (this._decoder && !this._decoderEnded) { var decoder = this._decoder, finalText = decoder.end(); this._decoderEnded = true; if (finalText) this.emit('data', finalText); if (this._decoder !== decoder) { this._scheduleBodyDrain(); return; } } if (this.aborted || this._paused || this._blockedPipes() || this._bodyQueue.length || this._decoderTail) return; this.readable = false; this.readableEnded = true; this.emit('end'); } } finally { this._draining = false; this._resumeBodyParser(); } };
             IncomingMessage.prototype._queueBody = function(body) { if (!body.length || this.aborted) return 0; var count = Math.min(body.length, 16384 - this._bodyBytes); if (!count) { this._syncReadGate(); return 0; } this._bodyQueue.push(Buffer.from(body.subarray(0, count))); this._bodyBytes += count; this._scheduleBodyDrain(); this._syncReadGate(); return count; };
             IncomingMessage.prototype._finishBody = function() { if (this._bodyComplete || this.aborted) return; this._bodyComplete = true; this._scheduleBodyDrain(); this._syncReadGate(); };
             IncomingMessage.prototype.pause = function() { this._paused = true; this._flowing = false; return this; }; IncomingMessage.prototype.isPaused = function() { return this._paused; };
             IncomingMessage.prototype._afterBodyListener = function(event) { if (event === 'data') this._flowing = true; if (event === 'data' || event === 'end') this._scheduleBodyDrain(); return this; };
             IncomingMessage.prototype.on = IncomingMessage.prototype.addListener = function(event, listener) { EventEmitter.prototype.on.call(this, event, listener); return this._afterBodyListener(event); }; IncomingMessage.prototype.once = function(event, listener) { EventEmitter.prototype.once.call(this, event, listener); return this._afterBodyListener(event); }; IncomingMessage.prototype.prependListener = function(event, listener) { EventEmitter.prototype.prependListener.call(this, event, listener); return this._afterBodyListener(event); }; IncomingMessage.prototype.prependOnceListener = function(event, listener) { EventEmitter.prototype.prependOnceListener.call(this, event, listener); return this._afterBodyListener(event); };
             IncomingMessage.prototype.resume = function() { this._paused = false; this._flowing = true; this._scheduleBodyDrain(); return this; };
             IncomingMessage.prototype._removePipe = function(record) { if (!record.active) return; record.active = false; var index = this._pipeRecords.indexOf(record); if (index >= 0) this._pipeRecords.splice(index, 1); var thrown, hasThrown = false; function remove(run) { try { run(); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } } } remove(function() { record.detachDrain(); }); var message = this; remove(function() { message.removeListener('data', record.onData); }); remove(function() { message.removeListener('end', record.onEnd); }); remove(function() { message._syncReadGate(); }); this._scheduleBodyDrain(); if (hasThrown) throw thrown; };
             IncomingMessage.prototype.pipe = function(destination, options) {
               var source = this, shouldEnd = !(options && options.end === false);
               var record = { destination: destination, active: true, blocked: false, attached: false, writing: false, drainedDuringWrite: false };
               record.detachDrain = function() {
                 if (!record.attached) return;
                 record.attached = false;
                 if (typeof destination.off === 'function') destination.off('drain', record.onDrain);
                 else if (typeof destination.removeListener === 'function') destination.removeListener('drain', record.onDrain);
               };
               record.onDrain = function() {
                 if (!record.active) return;
                 if (record.writing) { record.drainedDuringWrite = true; return; }
                 if (!record.blocked) return;
                 record.blocked = false;
                 var thrown, hasThrown = false;
                 try { record.detachDrain(); } catch (error) { thrown = error; hasThrown = true; }
                 try { source._syncReadGate(); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } }
                 source._scheduleBodyDrain();
                 if (hasThrown) throw thrown;
               };
               record.onData = function(chunk) {
                 if (!record.active) return;
                 record.drainedDuringWrite = false;
                 if (typeof destination.on === 'function') {
                   record.attached = true;
                   try { destination.on('drain', record.onDrain); }
                   catch (error) { try { record.detachDrain(); } catch (_) {} throw error; }
                   if (!record.active) { record.attached = true; record.detachDrain(); return; }
                 }
                 var result;
                 record.writing = true;
                 try { result = destination.write(chunk); }
                 catch (error) { record.writing = false; try { record.detachDrain(); } catch (_) {} throw error; }
                 record.writing = false;
                 if (record.active && result === false && !record.drainedDuringWrite) {
                   if (!record.attached) throw new TypeError('pipe destination must support drain');
                   record.blocked = true;
                 } else record.detachDrain();
                 source._syncReadGate();
               };
               record.onEnd = function() {
                 if (!record.active) return;
                 var thrown, hasThrown = false;
                 try { source._removePipe(record); } catch (error) { thrown = error; hasThrown = true; }
                 if (shouldEnd && typeof destination.end === 'function') try { destination.end(); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } }
                 if (hasThrown) throw thrown;
               };
               this._pipeRecords.push(record);
               try {
                 this.on('data', record.onData);
                 if (!record.active) { this.removeListener('data', record.onData); return destination; }
                 this.on('end', record.onEnd);
                 if (!record.active) { this.removeListener('end', record.onEnd); return destination; }
                 if (typeof destination.emit === 'function') destination.emit('pipe', this);
               } catch (error) { try { this._removePipe(record); } catch (_) {} throw error; }
               return destination;
             };
             IncomingMessage.prototype.unpipe = function(destination) { var records = this._pipeRecords.slice(), thrown, hasThrown = false, source = this; records.forEach(function(record) { if (!record.active || (destination !== undefined && record.destination !== destination)) return; try { source._removePipe(record); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } } if (typeof record.destination.emit === 'function') try { record.destination.emit('unpipe', source); } catch (error) { if (!hasThrown) { thrown = error; hasThrown = true; } } }); if (hasThrown) throw thrown; return this; };
             function normalizeOptions(input, options) { var result = {}; if (typeof input === 'string' || input instanceof URL) { var url = input instanceof URL ? input : new URL(String(input)); result.protocol = url.protocol; result.hostname = url.hostname; if (url.port) result.port = Number(url.port); result.path = url.pathname + url.search; result.auth = url.username ? decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password) : undefined; } else if (input) Object.assign(result, input); if (options) Object.assign(result, options); var expectedProtocol = result._defaultProtocol || 'http:'; result.protocol = result.protocol || expectedProtocol; if (result.protocol !== expectedProtocol) throw new Error('Protocol "' + result.protocol + '" not supported. Expected "' + expectedProtocol + '"'); result.hostname = result.hostname || result.host || 'localhost'; result.port = result.port === undefined ? (expectedProtocol === 'https:' ? 443 : 80) : Number(result.port); result.path = result.path || '/'; result.method = String(result.method || 'GET').toUpperCase(); return result; }
             function ClientRequest(input, options, callback) { if (!(this instanceof ClientRequest)) return new ClientRequest(input, options, callback); EventEmitter.call(this); if (typeof options === 'function') { callback = options; options = undefined; } this._options = normalizeOptions(input, options); this.method = this._options.method; this.path = this._options.path; this.host = this._options.hostname; this.protocol = this._options.protocol; this.agent = this._options.agent === false ? undefined : (this._options.agent || this._options._globalAgent || globalAgent); this.socket = this.connection = null; this.aborted = false; this.destroyed = false; this.finished = false; this.writableEnded = false; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; if (this._options.headers) for (var name of Object.keys(this._options.headers)) this.setHeader(name, this._options.headers[name]); var defaultPort = this.protocol === 'https:' ? 443 : 80; if (!this.hasHeader('host')) this.setHeader('Host', this.host + (this._options.port === defaultPort ? '' : ':' + this._options.port)); if (this._options.auth && !this.hasHeader('authorization')) this.setHeader('Authorization', 'Basic ' + Buffer.from(this._options.auth).toString('base64')); if (typeof callback === 'function') this.once('response', callback); }
             function validateHeaderName(name, label) { var value = String(name); if (!value || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value)) { var error = new TypeError((label || 'Header name') + ' must be a valid HTTP token ["' + value + '"]'); error.code = 'ERR_INVALID_HTTP_TOKEN'; throw error; } return value; } function validateHeaderValue(name, value) { if (value === undefined) { var missing = new TypeError('Invalid value "undefined" for header "' + name + '"'); missing.code = 'ERR_HTTP_INVALID_HEADER_VALUE'; throw missing; } var values = Array.isArray(value) ? value : [value]; for (var item of values) if (/[^\t\x20-\x7e\x80-\xff]/.test(String(item))) { var error = new TypeError('Invalid character in header content ["' + name + '"]'); error.code = 'ERR_INVALID_CHAR'; throw error; } return value; }
             ClientRequest.prototype = Object.create(EventEmitter.prototype); ClientRequest.prototype.constructor = ClientRequest; ClientRequest.prototype.setHeader = function(name, value) { name = validateHeaderName(name); validateHeaderValue(name, value); var key = name.toLowerCase(); this._headers[key] = value; this._headerNames[key] = name; return this; }; ClientRequest.prototype.getHeader = function(name) { return this._headers[String(name).toLowerCase()]; }; ClientRequest.prototype.getHeaders = function() { return Object.assign({}, this._headers); }; ClientRequest.prototype.getHeaderNames = function() { return Object.keys(this._headers); }; ClientRequest.prototype.hasHeader = function(name) { return Object.prototype.hasOwnProperty.call(this._headers, String(name).toLowerCase()); }; ClientRequest.prototype.removeHeader = function(name) { var key = String(name).toLowerCase(); delete this._headers[key]; delete this._headerNames[key]; }; ClientRequest.prototype.flushHeaders = function() { return this; }; ClientRequest.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : ArrayBuffer.isView(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : chunk instanceof ArrayBuffer ? Buffer.from(chunk) : Buffer.from(String(chunk), encoding); this._chunks.push(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };
             function decodeChunked(body) { var offset = 0, output = []; while (offset < body.length) { var line = body.indexOf('\r\n', offset); if (line < 0) throw new Error('Parse Error: Invalid chunk size'); var size = parseInt(body.slice(offset, line).toString(), 16); if (!Number.isFinite(size)) throw new Error('Parse Error: Invalid chunk size'); offset = line + 2; if (size === 0) break; output.push(body.slice(offset, offset + size)); offset += size + 2; } return Buffer.concat(output); }
             function parseResponse(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? '' : buffer.slice(0, marker).toString(), body = marker < 0 ? buffer : buffer.slice(marker + 4), lines = head.split('\r\n'), status = (lines.shift() || '').match(/^HTTP\/(\d+\.\d+)\s+(\d+)(?:\s+(.*))?$/); if (!status) throw new Error('Parse Error: Invalid HTTP response'); var response = new IncomingMessage(socket); response.httpVersion = status[1]; response.httpVersionMajor = Number(status[1].split('.')[0]); response.httpVersionMinor = Number(status[1].split('.')[1]); response.statusCode = Number(status[2]); response.statusMessage = status[3] || ''; lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); response.rawHeaders.push(name, value); (response.headersDistinct[key] || (response.headersDistinct[key] = [])).push(value); if (key === 'set-cookie') response.headers[key] = response.headersDistinct[key].slice(); else response.headers[key] = response.headersDistinct[key].join(', '); }); if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) body = decodeChunked(body); return { response: response, body: body }; }
             ClientRequest.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var request = this, body = Buffer.concat(this._chunks); if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n'; var received = [], transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port }); var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions); this.emit('socket', socket); socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); }); socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); }); socket.on('data', function(data) { received.push(Buffer.from(data)); }); socket.on('end', function() { try { var parsed = parseResponse(Buffer.concat(received), socket), response = parsed.response; request.emit('response', response); var value = response._encoding ? parsed.body.toString(response._encoding) : parsed.body; if (parsed.body.length) response.emit('data', value); response.readable = false; response.emit('end'); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } }); return this; }; ClientRequest.prototype.abort = function() { this.aborted = true; this.emit('abort'); return this.destroy(); }; ClientRequest.prototype.destroy = function(error) { this.destroyed = true; if (this.socket) this.socket.destroy(error); return this; }; ClientRequest.prototype.setTimeout = function(timeout, callback) { this.timeout = Math.max(0, Number(timeout) || 0); if (typeof callback === 'function') this.once('timeout', callback); if (this.socket && this.socket.setTimeout) { var request = this; this.socket.setTimeout(this.timeout, function() { request.emit('timeout'); }); } return this; }; ClientRequest.prototype.setNoDelay = ClientRequest.prototype.setSocketKeepAlive = function() { return this; };
             ClientRequest.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (chunk !== undefined) this.write(chunk, encoding); if (this.writableEnded) return this; this.finished = this.writableEnded = true; var request = this, body = Buffer.concat(this._chunks); if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length)); if (!this.hasHeader('connection')) this.setHeader('Connection', 'close'); var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n'; var pending = Buffer.alloc(0), response = null, remaining = null, chunkSize = null, ended = false, transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port }); function finishResponse() { if (!response || ended) return; ended = true; response.complete = true; response.readable = false; response.emit('end'); } function emitBody(value) { if (!value.length || ended) return; if (remaining !== null) { var count = Math.min(remaining, value.length); if (count) response.emit('data', response._encoding ? value.subarray(0, count).toString(response._encoding) : value.subarray(0, count)); remaining -= count; if (remaining === 0) finishResponse(); return; } response.emit('data', response._encoding ? value.toString(response._encoding) : value); } function consume() { if (!response) { var marker = pending.indexOf(Buffer.from('\r\n\r\n')); if (marker < 0) return; var parsed = parseResponse(pending.subarray(0, marker + 4), request.socket); response = parsed.response; pending = pending.subarray(marker + 4); var length = response.headers['content-length']; remaining = length === undefined ? null : Math.max(0, Number(length)); request.emit('response', response); if (request.method === 'HEAD' || [101, 204, 205, 304].indexOf(response.statusCode) >= 0 || remaining === 0) { finishResponse(); pending = Buffer.alloc(0); return; } } if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) { while (!ended) { if (chunkSize === null) { var line = pending.indexOf(Buffer.from('\r\n')); if (line < 0) return; chunkSize = parseInt(pending.subarray(0, line).toString().split(';')[0], 16); if (!Number.isFinite(chunkSize)) throw new Error('Parse Error: Invalid chunk size'); pending = pending.subarray(line + 2); if (chunkSize === 0) { finishResponse(); return; } } if (pending.length < chunkSize + 2) return; emitBody(pending.subarray(0, chunkSize)); pending = pending.subarray(chunkSize + 2); chunkSize = null; } } else { var available = pending; pending = Buffer.alloc(0); emitBody(available); } } var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions); this.emit('socket', socket); socket.on('error', function(error) { request.destroyed = true; request.emit('error', error); }); socket.on('connect', function() { socket.end(Buffer.concat([Buffer.from(head), body])); request.emit('finish'); if (typeof callback === 'function') callback(); }); socket.on('data', function(data) { try { pending = Buffer.concat([pending, Buffer.from(data)]); consume(); } catch (error) { request.emit('error', error); socket.destroy(); } }); socket.on('end', function() { try { consume(); if (!response) throw new Error('Parse Error: Invalid HTTP response'); finishResponse(); request.destroyed = true; request.emit('close'); } catch (error) { request.emit('error', error); } }); return this; };
             ClientRequest.prototype.end = function(chunk, encoding, callback) {
               if (typeof chunk === 'function') { callback = chunk; chunk = undefined; }
               else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; }
               if (chunk !== undefined) this.write(chunk, encoding);
               if (this.writableEnded) return this;
               this.finished = this.writableEnded = true;
               var request = this, body = Buffer.concat(this._chunks);
               if (body.length && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) this.setHeader('Content-Length', String(body.length));
               if (!this.hasHeader('connection')) this.setHeader('Connection', 'close');
               var head = this.method + ' ' + this.path + ' HTTP/1.1\r\n' + Object.keys(this._headers).map(function(key) { return request._headerNames[key] + ': ' + request._headers[key]; }).join('\r\n') + '\r\n\r\n';
               var pending = Buffer.alloc(0), response = null, remaining = null, chunkSize = null, ended = false, upgraded = false, failed = false, closeQueued = false, inputEnded = false;
               function closeRequest() { if (closeQueued) return; closeQueued = true; request.destroyed = true; queueMicrotask(function() { request.emit('close'); }); }
               function failResponse(error) {
                 if (failed || upgraded) return;
                 failed = true;
                 var thrown, hasThrown = false;
                 if (response && !ended) try { response._abortBody(); } catch (reason) { thrown = reason; hasThrown = true; } if (response) { response._bodyResume = null; response._bodyReadGate = null; }
                 function notify(target, event, value) { try { if (event === 'error') target.emit(event, value); else target.emit(event); } catch (caught) { if (!hasThrown) { thrown = caught; hasThrown = true; } } }
                 if (response && !ended) { notify(response, 'aborted'); notify(response, 'error', error); notify(response, 'close'); }
                 notify(request, 'error', error);
                 closeRequest();
                 if (hasThrown) throw thrown;
               }
               function finishResponse() { if (!response || ended || failed || response.aborted || request.destroyed) return; ended = true; response.complete = true; response._finishBody(); }
               function emitBody(value) {
                 if (!value.length || ended || failed || response.aborted || request.destroyed) return 0;
                 if (remaining !== null) {
                   var count = Math.min(remaining, value.length), accepted = response._queueBody(value.subarray(0, count));
                   remaining -= accepted; if (remaining === 0) finishResponse(); return accepted;
                 }
                 return response._queueBody(value);
               }
               function parseTrailers(block) {
                 response.trailersDistinct = {};
                 if (!block) return;
                 block.split('\r\n').forEach(function(line) {
                   var colon = line.indexOf(':'); if (colon < 0) return;
                   var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim();
                   response.rawTrailers.push(name, value);
                   (response.trailersDistinct[key] || (response.trailersDistinct[key] = [])).push(value);
                   if (key === 'set-cookie') response.trailers[key] = response.trailersDistinct[key].slice();
                   else response.trailers[key] = response.trailersDistinct[key].join(', ');
                 });
               }
               function consume() {
                 while (!response) {
                   var marker = pending.indexOf(Buffer.from('\r\n\r\n')); if (marker < 0) return;
                   var parsed = parseResponse(pending.subarray(0, marker + 4), request.socket), candidate = parsed.response;
                   pending = pending.subarray(marker + 4);
                   if (candidate.statusCode >= 100 && candidate.statusCode < 200 && candidate.statusCode !== 101) {
                     var information = { statusCode: candidate.statusCode, statusMessage: candidate.statusMessage, httpVersion: candidate.httpVersion, httpVersionMajor: Number(candidate.httpVersion.split('.')[0]), httpVersionMinor: Number(candidate.httpVersion.split('.')[1]), headers: candidate.headers, rawHeaders: candidate.rawHeaders };
                     request.emit('information', information); if (candidate.statusCode === 100) request.emit('continue');
                     continue;
                   }
                   if (candidate.statusCode === 101) { upgraded = true; candidate.complete = true; var headBytes = pending; pending = Buffer.alloc(0); request.emit('upgrade', candidate, request.socket, headBytes); return; }
                   response = candidate; response.trailersDistinct = {}; response._bodyReadGate = updateGate; response._bodyResume = process;
                   var length = response.headers['content-length']; remaining = length === undefined ? null : Math.max(0, Number(length));
                   request.emit('response', response);
                   if (failed || response.aborted || request.destroyed) return;
                   if (request.method === 'HEAD' || [101, 204, 304].indexOf(response.statusCode) >= 0 || remaining === 0) { finishResponse(); pending = Buffer.alloc(0); return; }
                 }
                 if (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0) {
                   while (!ended) {
                     if (chunkSize === 0) {
                       if (pending.length >= 2 && pending[0] === 13 && pending[1] === 10) { pending = pending.subarray(2); parseTrailers(''); finishResponse(); return; }
                       var trailerEnd = pending.indexOf(Buffer.from('\r\n\r\n')); if (trailerEnd < 0) return;
                       parseTrailers(pending.subarray(0, trailerEnd).toString()); pending = pending.subarray(trailerEnd + 4); finishResponse(); return;
                     }
                     if (chunkSize === null) {
                       var line = pending.indexOf(Buffer.from('\r\n')); if (line < 0) return;
                       var sizeToken = pending.subarray(0, line).toString().split(';')[0].trim();
                       if (!/^[0-9a-f]+$/i.test(sizeToken)) throw new Error('Parse Error: Invalid chunk size');
                       chunkSize = parseInt(sizeToken, 16);
                       if (!Number.isSafeInteger(chunkSize)) throw new Error('Parse Error: Invalid chunk size');
                       pending = pending.subarray(line + 2); if (chunkSize === 0) continue;
                     }
                     if (chunkSize === -1) { if (pending.length < 2) return; if (pending[0] !== 13 || pending[1] !== 10) throw new Error('Parse Error: Invalid chunk terminator'); pending = pending.subarray(2); chunkSize = null; continue; }
                     if (!pending.length) return;
                     var accepted = emitBody(pending.subarray(0, Math.min(chunkSize, pending.length)));
                     pending = pending.subarray(accepted); chunkSize -= accepted;
                     if (!accepted) return;
                     if (chunkSize === 0) chunkSize = -1;
                   }
                 } else { if (pending.length) { var accepted = emitBody(pending); pending = pending.subarray(accepted); } }
               }
               var transport = this._options._transport || net, connectOptions = Object.assign({}, this._options, { host: this._options.hostname, port: this._options.port });
               var socket = this.socket = this.connection = transport.createConnection ? transport.createConnection(connectOptions) : transport.connect(connectOptions);
               this.emit('socket', socket);
               socket.on('error', function(error) { if (upgraded) { request.destroyed = true; request.emit('error', error); return; } failResponse(error); });
               socket.on('connect', function() { var packet = Buffer.concat([Buffer.from(head), body]); if (socket.encrypted) socket.end(packet); else socket.write(packet); request.emit('finish'); if (typeof callback === 'function') callback(); });
               function updateGate() { if (socket && typeof socket._setHttpReadPaused === 'function') socket._setHttpReadPaused(Boolean(response && !failed && !upgraded && (response._bodyBytes >= 16384 || response._blockedPipes()))); }
               function checkInputEnd() { if (!inputEnded || failed || upgraded) return; if (!response) throw new Error('Parse Error: Invalid HTTP response'); if (!ended && response._bodyBytes >= 16384 && pending.length) return; if (request.destroyed && !ended) { var stopped = new Error('aborted'); stopped.code = 'ECONNRESET'; throw stopped; } if (!ended && (String(response.headers['transfer-encoding'] || '').toLowerCase().indexOf('chunked') >= 0 || remaining > 0)) { var error = new Error('aborted'); error.code = 'ECONNRESET'; throw error; } if (!ended) finishResponse(); closeRequest(); }
               function process() { if (failed || upgraded) return; try { consume(); checkInputEnd(); updateGate(); } catch (error) { var thrown, hasThrown = false; try { failResponse(error); } catch (reason) { thrown = reason; hasThrown = true; } try { socket.destroy(); } catch (reason) { if (!hasThrown) { thrown = reason; hasThrown = true; } } if (hasThrown) throw thrown; } }
               socket.on('data', function(data) { if (upgraded || failed || request.destroyed) return; pending = Buffer.concat([pending, Buffer.from(data)]); process(); });
               socket.on('end', function() { if (upgraded || failed) return; inputEnded = true; process(); });
               socket.on('close', function() { if (upgraded) return; if (inputEnded && !failed && !ended && response && response._bodyBytes >= 16384 && pending.length) return; if (!failed && !ended) { if (!response && request.destroyed) { closeRequest(); return; } var error = new Error('aborted'); error.code = 'ECONNRESET'; failResponse(error); } else closeRequest(); });
               return this;
             };
             var endWithoutAgent = ClientRequest.prototype.end; ClientRequest.prototype.end = function() { if (this.agent && typeof this.agent.createConnection === 'function') { var agent = this.agent; this._options._transport = { createConnection: function(options) { return agent.createConnection(options); } }; } return endWithoutAgent.apply(this, arguments); };
             function request(input, options, callback) { return new ClientRequest(input, options, callback); } function get(input, options, callback) { var result = request(input, options, callback); result.end(); return result; }
             function ServerResponse(request) { EventEmitter.call(this); this.req = request; this.socket = this.connection = request.socket; this.statusCode = 200; this.statusMessage = null; this.sendDate = true; this.headersSent = false; this.finished = false; this.writableEnded = false; this._keepAlive = request.httpVersion === '1.1' && String(request.headers.connection || '').toLowerCase() !== 'close'; this._headers = Object.create(null); this._headerNames = Object.create(null); this._chunks = []; this._header = ''; this.assignSocket = function(socket) { this.socket = this.connection = socket; if (socket) socket._httpMessage = this; }; this.detachSocket = function(socket) { if (!socket || this.socket === socket) { if (socket) socket._httpMessage = null; this.socket = this.connection = null; } }; }
             ServerResponse.prototype = Object.create(EventEmitter.prototype); ServerResponse.prototype.constructor = ServerResponse;
             ServerResponse.prototype.setHeader = function(name, value) { if (this.headersSent) { var error = new Error('Cannot set headers after they are sent to the client'); error.code = 'ERR_HTTP_HEADERS_SENT'; throw error; } return ClientRequest.prototype.setHeader.call(this, name, value); };
             ServerResponse.prototype.getHeader = ClientRequest.prototype.getHeader; ServerResponse.prototype.getHeaders = ClientRequest.prototype.getHeaders; ServerResponse.prototype.getHeaderNames = ClientRequest.prototype.getHeaderNames; ServerResponse.prototype.hasHeader = ClientRequest.prototype.hasHeader;
             ServerResponse.prototype.removeHeader = function(name) { if (this.headersSent) { var error = new Error('Cannot remove headers after they are sent to the client'); error.code = 'ERR_HTTP_HEADERS_SENT'; throw error; } return ClientRequest.prototype.removeHeader.call(this, name); };
             ServerResponse.prototype.writeHead = function(statusCode, statusMessage, headers) { if (this.headersSent) { var error = new Error('Cannot write headers after they are sent to the client'); error.code = 'ERR_HTTP_HEADERS_SENT'; throw error; } this.statusCode = Number(statusCode); if (typeof statusMessage === 'object') { headers = statusMessage; statusMessage = undefined; } if (statusMessage !== undefined) this.statusMessage = String(statusMessage); if (headers) for (var name of Object.keys(headers)) ClientRequest.prototype.setHeader.call(this, name, headers[name]); this._wireStatusCode = this.statusCode; this._wireStatusMessage = this.statusMessage; this.headersSent = true; return this; };
             ServerResponse.prototype._head = function(streaming) { if (!this.headersSent) this.writeHead(this.statusCode); if (this._headerWritten) return Buffer.alloc(0); var status = this._wireStatusCode, hasBody = this.req.method !== 'HEAD' && !(status >= 100 && status < 200) && status !== 204 && status !== 205 && status !== 304; if ((status >= 100 && status < 200) || status === 204) { ClientRequest.prototype.removeHeader.call(this, 'Content-Length'); ClientRequest.prototype.removeHeader.call(this, 'Transfer-Encoding'); } else if (status === 205) { ClientRequest.prototype.removeHeader.call(this, 'Transfer-Encoding'); ClientRequest.prototype.setHeader.call(this, 'Content-Length', '0'); } if (streaming && hasBody && !this.hasHeader('content-length') && !this.hasHeader('transfer-encoding')) ClientRequest.prototype.setHeader.call(this, 'Transfer-Encoding', 'chunked'); if (!this.hasHeader('connection')) ClientRequest.prototype.setHeader.call(this, 'Connection', this._keepAlive ? 'keep-alive' : 'close'); this._wireHasBody = hasBody; this._wireChunked = hasBody && String(this.getHeader('transfer-encoding') || '').toLowerCase().indexOf('chunked') >= 0; var message = this._wireStatusMessage === null ? (STATUS_CODES[status] || '') : this._wireStatusMessage, response = 'HTTP/1.1 ' + status + ' ' + message + '\r\n' + Object.keys(this._headers).map(function(key) { var value = this._headers[key]; if (Array.isArray(value)) return value.map(function(item) { return this._headerNames[key] + ': ' + item; }, this).join('\r\n'); return this._headerNames[key] + ': ' + value; }, this).join('\r\n') + '\r\n\r\n'; this._headerWritten = this.headersSent = true; return Buffer.from(response); };
             ServerResponse.prototype.flushHeaders = function() { var head = this._head(true); if (head.length) this.socket.write(head); return this; };
             ServerResponse.prototype.write = function(chunk, encoding, callback) { if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) throw new Error('write after end'); var value = Buffer.isBuffer(chunk) ? chunk : ArrayBuffer.isView(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : chunk instanceof ArrayBuffer ? Buffer.from(chunk) : Buffer.from(String(chunk), encoding), head = this._head(true), packet = !this._wireHasBody || !value.length ? head : this._wireChunked ? Buffer.concat([head, Buffer.from(value.length.toString(16) + '\r\n'), value, Buffer.from('\r\n')]) : Buffer.concat([head, value]); this._streaming = true; if (packet.length) this.socket.write(packet, undefined, callback); else if (typeof callback === 'function') queueMicrotask(callback); return true; };
             ServerResponse.prototype.end = function(chunk, encoding, callback) { if (typeof chunk === 'function') { callback = chunk; chunk = undefined; } else if (typeof encoding === 'function') { callback = encoding; encoding = undefined; } if (this.writableEnded) return this; if (this._streaming || this._headerWritten || String(this.getHeader('transfer-encoding') || '').toLowerCase().indexOf('chunked') >= 0) { if (chunk !== undefined) this.write(chunk, encoding); var head = this._head(true); this.finished = this.writableEnded = true; var streamed = this, terminal = this._wireChunked ? Buffer.from('0\r\n\r\n') : Buffer.alloc(0), packet = head.length ? Buffer.concat([head, terminal]) : terminal, streamFinished = function() { if (typeof callback === 'function') callback(); streamed.emit('finish'); if (!streamed._keepAlive) streamed.emit('close'); }; if (this._keepAlive) { this.socket.write(packet, undefined, streamFinished); if (this._reuse) this._reuse(); } else this.socket.end(packet, streamFinished); return this; } var body = chunk === undefined ? Buffer.alloc(0) : Buffer.isBuffer(chunk) ? chunk : ArrayBuffer.isView(chunk) ? Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength) : chunk instanceof ArrayBuffer ? Buffer.from(chunk) : Buffer.from(String(chunk), encoding); var status = this.headersSent ? this._wireStatusCode : this.statusCode; if (!this.hasHeader('content-length') && !this.hasHeader('transfer-encoding') && !(status >= 100 && status < 200) && status !== 204 && status !== 304) ClientRequest.prototype.setHeader.call(this, 'Content-Length', String(body.length)); this.finished = this.writableEnded = true; var target = this, head = this._head(false), packet = this._wireHasBody ? Buffer.concat([head, body]) : head, finished = function() { if (typeof callback === 'function') callback(); target.emit('finish'); if (!target._keepAlive) target.emit('close'); }; if (this._keepAlive) { this.socket.write(packet, undefined, finished); if (this._reuse) this._reuse(); } else this.socket.end(packet, finished); return this; };
             ServerResponse.prototype.destroy = function(error) { this.socket.destroy(error); return this; };
             function parseRequest(buffer, socket) { var marker = buffer.indexOf(Buffer.from('\r\n\r\n')), head = marker < 0 ? buffer.toString() : buffer.slice(0, marker).toString(), body = marker < 0 ? Buffer.alloc(0) : buffer.slice(marker + 4), lines = head.split('\r\n'), start = (lines.shift() || '').match(/^(\S+)\s+(\S+)\s+HTTP\/(\d+\.\d+)$/); if (!start) throw new Error('Parse Error: Invalid HTTP request'); var message = new IncomingMessage(socket); message.method = start[1]; message.url = start[2]; message.httpVersion = start[3]; message.httpVersionMajor = Number(start[3].split('.')[0]); message.httpVersionMinor = Number(start[3].split('.')[1]); lines.forEach(function(line) { var colon = line.indexOf(':'); if (colon < 0) return; var name = line.slice(0, colon), key = name.toLowerCase(), value = line.slice(colon + 1).trim(); message.rawHeaders.push(name, value); (message.headersDistinct[key] || (message.headersDistinct[key] = [])).push(value); message.headers[key] = message.headersDistinct[key].join(', '); }); return { request: message, body: body }; }
             function Server(options, listener) {
               if (!(this instanceof Server)) return new Server(options, listener);
               EventEmitter.call(this);
               if (typeof options === 'function') { listener = options; options = {}; }
               options = options || {};
               this.requestTimeout = 300000; this.headersTimeout = 60000; this.keepAliveTimeout = 5000;
               this.maxHeadersCount = null; this.listening = false;
               var transport = options._transport || net, server = this, accept = function(socket) {
                 var pending = Buffer.alloc(0), active = null, terminal = false, inputEnded = false, draining = false, rerun = false, socketClosed = false;
                 function updateGate() { if (typeof socket._setHttpReadPaused !== 'function') return; var state = active, request = state && state.request; socket._setHttpReadPaused(Boolean(request && (request._bodyBytes >= 16384 || request._blockedPipes() || (state.bodyDone && (!state.responseReusable || request._bodyBytes || request._decoderTail))))); }
                 function fail(error, alreadyTerminal) {
                   if (terminal && !alreadyTerminal) return;
                   terminal = true;
                   var state = active; active = null; pending = Buffer.alloc(0);
                   var cleanupError; if (state) { if (!state.bodyDone) try { state.request._abortBody(); } catch (reason) { cleanupError = reason; } state.request._bodyResume = null; state.request._bodyReadGate = null; } updateGate();
                   var handled = false, thrown, hasThrown = false;
                   try { handled = server.emit('clientError', error, socket); }
                   catch (reason) { thrown = reason; hasThrown = true; }
                   if (!handled || hasThrown) try { socket.destroy(error); }
                   catch (reason) { if (!hasThrown) { thrown = reason; hasThrown = true; } }
                   if (cleanupError) throw cleanupError; if (hasThrown) throw thrown;
                 }
                 function waitForBody() { if (inputEnded) throw new Error('Parse Error: Premature end of request body'); }
                 function finishBody(state) {
                   if (state.bodyDone) return;
                   state.bodyDone = true; state.request.complete = true; state.request._finishBody(); updateGate();
                 }
                 function consume() {
                   if (terminal) return;
                   if (draining) { rerun = true; return; }
                   draining = true;
                   try {
                     while (!terminal) {
                       if (!active) {
                         if (!pending.length) return;
                         var marker = pending.indexOf(Buffer.from('\r\n\r\n'));
                         if (marker < 0) { if (inputEnded) throw new Error('Parse Error: Incomplete request headers'); return; }
                         var parsed = parseRequest(pending.subarray(0, marker + 4), socket), request = parsed.request;
                         var transfer = String(request.headers['transfer-encoding'] || '').toLowerCase().split(',').map(function(value) { return value.trim(); });
                         var connection = String(request.headers.connection || '').toLowerCase().split(',').map(function(value) { return value.trim(); });
                         var declared = request.headers['content-length'], mode = 'none', length = 0;
                         if (transfer[0] && declared !== undefined) throw new Error('Parse Error: Content-Length cannot be used with Transfer-Encoding');
                         if (transfer[0] && (transfer[transfer.length - 1] !== 'chunked' || transfer.slice(0, -1).some(function(value) { return value !== 'identity'; }))) throw new Error('Parse Error: Unsupported transfer encoding');
                         if (transfer[0]) mode = 'chunked';
                         else if (declared !== undefined) {
                           var token = String(declared).trim();
                           if (!/^[0-9]+$/.test(token)) throw new Error('Parse Error: Invalid content length');
                           length = Number(token);
                           if (!Number.isSafeInteger(length)) throw new Error('Parse Error: Invalid content length');
                           if (length) mode = 'length';
                         }
                         pending = pending.subarray(marker + 4);
                         if (request.headers.upgrade && connection.indexOf('upgrade') >= 0) {
                           terminal = true; request.complete = true;
                           var head = pending; pending = Buffer.alloc(0);
                           try { server.emit('upgrade', request, socket, head); }
                           catch (error) { fail(error, true); }
                           return;
                         }
                         var response = new ServerResponse(request);
                         let state = { request: request, response: response, mode: mode, remaining: length, chunkStage: 'size', chunkRemaining: 0, bodyDone: mode === 'none', responseReusable: false };
                         active = state; request._bodyReadGate = updateGate; request._bodyResume = consume;
                         if (state.bodyDone) request.complete = true;
                         response._reuse = function() { if (terminal || active !== state || state.responseReusable) return; state.responseReusable = true; queueMicrotask(consume); };
                         server.emit('request', request, response);
                         if (terminal || active !== state || socket.destroyed) return;
                         if (state.bodyDone) request._finishBody();
                       }
                       var current = active;
                       if (!current.bodyDone && current.mode === 'length') {
                         if (!pending.length) { waitForBody(); return; }
                         var count = Math.min(pending.length, current.remaining), accepted = current.request._queueBody(pending.subarray(0, count));
                         pending = pending.subarray(accepted); current.remaining -= accepted;
                         if (current.remaining === 0) finishBody(current);
                         if (!accepted) return;
                       } else if (!current.bodyDone && current.mode === 'chunked') {
                         if (current.chunkStage === 'size') {
                           var line = pending.indexOf(Buffer.from('\r\n'));
                           if (line < 0) { waitForBody(); return; }
                           var sizeToken = pending.subarray(0, line).toString().split(';')[0].trim();
                           if (!/^[0-9a-f]+$/i.test(sizeToken)) throw new Error('Parse Error: Invalid chunk size');
                           var size = parseInt(sizeToken, 16);
                           if (!Number.isSafeInteger(size)) throw new Error('Parse Error: Invalid chunk size');
                           pending = pending.subarray(line + 2); current.chunkRemaining = size;
                           current.chunkStage = size === 0 ? 'trailers' : 'data';
                         } else if (current.chunkStage === 'data') {
                           if (!pending.length) { waitForBody(); return; }
                           var amount = Math.min(pending.length, current.chunkRemaining), accepted = current.request._queueBody(pending.subarray(0, amount));
                           pending = pending.subarray(accepted); current.chunkRemaining -= accepted;
                           if (current.chunkRemaining === 0) current.chunkStage = 'dataCRLF';
                           if (!accepted) return;
                         } else if (current.chunkStage === 'dataCRLF') {
                           if (pending.length < 2) { waitForBody(); return; }
                           if (pending[0] !== 13 || pending[1] !== 10) throw new Error('Parse Error: Invalid chunk terminator');
                           pending = pending.subarray(2); current.chunkStage = 'size';
                         } else {
                           if (pending.length >= 2 && pending[0] === 13 && pending[1] === 10) pending = pending.subarray(2);
                           else {
                             var trailerEnd = pending.indexOf(Buffer.from('\r\n\r\n'));
                             if (trailerEnd < 0) { waitForBody(); return; }
                             pending = pending.subarray(trailerEnd + 4);
                           }
                           finishBody(current);
                         }
                       }
                       if (terminal || active !== current || socket.destroyed) return;
                       if (!current.bodyDone) continue;
                       if (inputEnded && !pending.length && !current.request._bodyBytes && !current.request._decoderTail && !current.request._blockedPipes() && !current.response.writableEnded) current.response._keepAlive = false;
                       if (!current.responseReusable || current.request._bodyBytes || current.request._decoderTail || current.request._blockedPipes()) return;
                       current.request._bodyReadGate = null; current.request._bodyResume = null;
                       active = null;
                     }
                   } catch (error) { if (terminal) throw error; fail(error); }
                   finally { draining = false; if (socketClosed && active && active.bodyDone && !active.request._bodyBytes && !active.request._decoderTail) { active.request._bodyResume = null; active.request._bodyReadGate = null; active = null; pending = Buffer.alloc(0); terminal = true; } updateGate(); if (inputEnded && !terminal && !active && !pending.length && socket.writable && !socket.destroyed) socket.end(); if (rerun && !terminal) { rerun = false; queueMicrotask(consume); } }
                 }
                 socket.on('data', function(chunk) { if (terminal) return; pending = Buffer.concat([pending, Buffer.from(chunk)]); consume(); });
                 socket.on('end', function() { if (terminal) return; inputEnded = true; consume(); });
                 socket.on('close', function() { if (terminal) return; socketClosed = true; if (inputEnded && active && !active.bodyDone && active.request._bodyBytes >= 16384 && pending.length) return; terminal = true; if (active && !active.bodyDone) active.request._abortBody(); active = null; pending = Buffer.alloc(0); });
               };
               this._net = transport.createServer(Object.assign({}, options, { allowHalfOpen: true }), accept);
               this._net.on('listening', function() { server.listening = true; server.emit('listening'); });
               this._net.on('close', function() { server.listening = false; server.emit('close'); });
               this._net.on('error', function(error) { server.emit('error', error); });
               this._net.on('connection', function(socket) { server.emit('connection', socket); });
               this._net.on('secureConnection', function(socket) { server.emit('secureConnection', socket); });
               if (typeof listener === 'function') this.on('request', listener);
             }
             Server.prototype = Object.create(EventEmitter.prototype); Server.prototype.constructor = Server; Server.prototype.listen = function() { this._net.listen.apply(this._net, arguments); return this; }; Server.prototype.close = function(callback) { this._net.close(callback); return this; }; Server.prototype.address = function() { return this._net.address(); }; Server.prototype.getConnections = function(callback) { return this._net.getConnections(callback); }; Server.prototype.setTimeout = function(milliseconds, callback) { this.timeout = Number(milliseconds); if (typeof callback === 'function') this.on('timeout', callback); return this; }; Server.prototype.closeAllConnections = function() { return this._net.closeAllConnections(); }; Server.prototype.closeIdleConnections = function() { return this._net.closeIdleConnections(); }; Server.prototype.ref = function() { this._net.ref(); return this; }; Server.prototype.unref = function() { this._net.unref(); return this; }; function createServer(options, listener) { return new Server(options, listener); }
             var METHODS = ['ACL','BIND','CHECKOUT','CONNECT','COPY','DELETE','GET','HEAD','LINK','LOCK','M-SEARCH','MERGE','MKACTIVITY','MKCALENDAR','MKCOL','MOVE','NOTIFY','OPTIONS','PATCH','POST','PROPFIND','PROPPATCH','PURGE','PUT','REBIND','REPORT','SEARCH','SOURCE','SUBSCRIBE','TRACE','UNBIND','UNLINK','UNLOCK','UNSUBSCRIBE']; var STATUS_CODES = { 200: 'OK', 201: 'Created', 202: 'Accepted', 204: 'No Content', 205: 'Reset Content', 301: 'Moved Permanently', 302: 'Found', 304: 'Not Modified', 400: 'Bad Request', 401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found', 500: 'Internal Server Error', 502: 'Bad Gateway', 503: 'Service Unavailable' };
             function Agent(options, transport, protocol) { if (!(this instanceof Agent)) return new Agent(options, transport, protocol); EventEmitter.call(this); options = options || {}; this.options = Object.assign({}, options); this.keepAlive = Boolean(options.keepAlive); this.keepAliveMsecs = options.keepAliveMsecs === undefined ? 1000 : Number(options.keepAliveMsecs); this.maxSockets = options.maxSockets === undefined ? Infinity : Number(options.maxSockets); this.maxFreeSockets = options.maxFreeSockets === undefined ? 256 : Number(options.maxFreeSockets); this.maxTotalSockets = options.maxTotalSockets === undefined ? Infinity : Number(options.maxTotalSockets); this.totalSocketCount = 0; this.requests = Object.create(null); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.protocol = protocol || 'http:'; this.defaultPort = this.protocol === 'https:' ? 443 : 80; this._transport = transport || net; }
             Agent.prototype = Object.create(EventEmitter.prototype); Agent.prototype.constructor = Agent; Agent.prototype.getName = function(options) { options = options || {}; var host = options.host || options.hostname || 'localhost', port = options.port || this.defaultPort, localAddress = options.localAddress || '', family = options.family || ''; return host + ':' + port + ':' + localAddress + ':' + family; }; Agent.prototype.createConnection = function(options, callback) { var agent = this, name = this.getName(options), socket = this._transport.createConnection ? this._transport.createConnection(options) : this._transport.connect(options); (this.sockets[name] || (this.sockets[name] = [])).push(socket); this.totalSocketCount++; socket.once('close', function() { agent.removeSocket(socket, options); }); if (typeof callback === 'function') socket.once(this.protocol === 'https:' ? 'secureConnect' : 'connect', function() { callback(null, socket); }); return socket; }; Agent.prototype.keepSocketAlive = function(socket) { socket.setKeepAlive(true, this.keepAliveMsecs); socket.unref(); return true; }; Agent.prototype.reuseSocket = function(socket) { socket.ref(); }; Agent.prototype.removeSocket = function(socket, options) { var name = this.getName(options), list = this.sockets[name] || [], index = list.indexOf(socket); if (index >= 0) list.splice(index, 1); if (!list.length) delete this.sockets[name]; this.totalSocketCount = Math.max(0, this.totalSocketCount - 1); }; Agent.prototype.destroy = function() { for (var group of [this.sockets, this.freeSockets]) for (var name of Object.keys(group)) for (var socket of group[name]) socket.destroy(); this.sockets = Object.create(null); this.freeSockets = Object.create(null); this.totalSocketCount = 0; };
             var globalAgent = new Agent({ keepAlive: true }, net, 'http:'); function setMaxIdleHTTPParsers() {}
             function createSecureModule(tls) { function SecureAgent(options) { Agent.call(this, options, tls, 'https:'); } SecureAgent.prototype = Object.create(Agent.prototype); SecureAgent.prototype.constructor = SecureAgent; var secureGlobalAgent = new SecureAgent({ keepAlive: true }); function secureOptions(options) { return Object.assign({}, options || {}, { _transport: tls, _defaultProtocol: 'https:', _globalAgent: secureGlobalAgent }); } function secureRequest(input, options, callback) { if (typeof options === 'function') { callback = options; options = undefined; } return new ClientRequest(input, secureOptions(options), callback); } function secureGet(input, options, callback) { var result = secureRequest(input, options, callback); result.end(); return result; } function secureCreateServer(options, listener) { if (typeof options === 'function') { listener = options; options = {}; } return new Server(Object.assign({}, options || {}, { _transport: tls }), listener); } return { request: secureRequest, get: secureGet, createServer: secureCreateServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: secureGlobalAgent, Agent: SecureAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers }; }
             module.exports = { request: request, get: get, createServer: createServer, ClientRequest: ClientRequest, IncomingMessage: IncomingMessage, ServerResponse: ServerResponse, Server: Server, Agent: Agent, METHODS: METHODS, STATUS_CODES: STATUS_CODES, maxHeaderSize: 16384, globalAgent: globalAgent, validateHeaderName: validateHeaderName, validateHeaderValue: validateHeaderValue, setMaxIdleHTTPParsers: setMaxIdleHTTPParsers, __createSecureModule: createSecureModule }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        _ => None,
    }
}
