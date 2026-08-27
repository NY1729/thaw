pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "tty" => Some(
            "function isatty(fd) { return false; }\n\
             function ReadStream(fd, options) { if (!(this instanceof ReadStream)) return new ReadStream(fd, options); this.fd = Number(fd); this.isRaw = false; this.isTTY = true; this.readable = true; this.destroyed = false; }\n\
             ReadStream.prototype.setRawMode = function(mode) { this.isRaw = Boolean(mode); return this; }; ReadStream.prototype.ref = function() { return this; }; ReadStream.prototype.unref = function() { return this; };\n\
             function WriteStream(fd) { if (!(this instanceof WriteStream)) return new WriteStream(fd); this.fd = Number(fd); this.isTTY = true; this.columns = 80; this.rows = 24; this.writable = true; this.destroyed = false; this._output = ''; }\n\
             WriteStream.prototype.write = function(value, callback) { this._output += String(value); if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.getColorDepth = function(environment) { var env = environment || globalThis.process && process.env || {}; if (env.FORCE_COLOR === '0' || env.NO_COLOR !== undefined) return 1; if (env.FORCE_COLOR === '3' || env.COLORTERM === 'truecolor') return 24; if (env.FORCE_COLOR === '2' || /256color/i.test(env.TERM || '')) return 8; if (env.FORCE_COLOR === '1' || /color|ansi|xterm|screen/i.test(env.TERM || '')) return 4; return 1; };\n\
             WriteStream.prototype.hasColors = function(count, environment) { if (typeof count === 'object') { environment = count; count = 16; } count = count === undefined ? 16 : Number(count); return Math.pow(2, this.getColorDepth(environment)) >= count; };\n\
             WriteStream.prototype._ansi = function(sequence, callback) { this._output += sequence; if (typeof callback === 'function') queueMicrotask(callback); return true; };\n\
             WriteStream.prototype.clearLine = function(direction, callback) { var sequence = Number(direction) < 0 ? '\\u001b[1K' : Number(direction) > 0 ? '\\u001b[0K' : '\\u001b[2K'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.clearScreenDown = function(callback) { return this._ansi('\\u001b[0J', callback); };\n\
             WriteStream.prototype.cursorTo = function(x, y, callback) { if (typeof y === 'function') { callback = y; y = undefined; } var sequence = y === undefined ? '\\u001b[' + (Number(x) + 1) + 'G' : '\\u001b[' + (Number(y) + 1) + ';' + (Number(x) + 1) + 'H'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.moveCursor = function(dx, dy, callback) { var sequence = ''; dx = Number(dx); dy = Number(dy); if (dx < 0) sequence += '\\u001b[' + -dx + 'D'; else if (dx > 0) sequence += '\\u001b[' + dx + 'C'; if (dy < 0) sequence += '\\u001b[' + -dy + 'A'; else if (dy > 0) sequence += '\\u001b[' + dy + 'B'; return this._ansi(sequence, callback); };\n\
             WriteStream.prototype.getWindowSize = function() { return [this.columns, this.rows]; }; WriteStream.prototype.ref = function() { return this; }; WriteStream.prototype.unref = function() { return this; };\n\
             module.exports = { isatty: isatty, ReadStream: ReadStream, WriteStream: WriteStream }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "module" => Some(
            "var builtinModules = ['_http_agent','_http_client','_http_common','_http_incoming','_http_outgoing','_http_server','_stream_duplex','_stream_passthrough','_stream_readable','_stream_transform','_stream_wrap','_stream_writable','_tls_common','_tls_wrap','assert','assert/strict','async_hooks','buffer','cluster','console','constants','crypto','dgram','diagnostics_channel','dns','dns/promises','domain','events','fs','fs/promises','http','http2','inspector','inspector/promises','module','net','os','path','path/posix','path/win32','perf_hooks','process','punycode','querystring','readline','readline/promises','repl','stream','stream/consumers','stream/promises','stream/web','string_decoder','sys','test','test/reporters','timers','timers/promises','tls','trace_events','tty','url','util','util/types','v8','vm','wasi','worker_threads','zlib']; var builtinSet = new Set(builtinModules);\n\
             function isBuiltin(name) { var value = String(name); return builtinSet.has(value.replace(/^node:/, '')); }\n\
             function createRequire(filename) { if (typeof globalThis.__thaw_bundle_create_require !== 'function') throw new Error('createRequire is only available inside a Thaw bundle'); return globalThis.__thaw_bundle_create_require(filename); }\n\
             function Module(id, parent) { if (!(this instanceof Module)) return new Module(id, parent); this.id = id === undefined ? '' : String(id); this.path = this.id; this.exports = {}; this.filename = null; this.loaded = false; this.parent = parent || null; this.children = []; this.paths = []; if (parent && parent.children) parent.children.push(this); }\n\
             Module.builtinModules = builtinModules; Module.isBuiltin = isBuiltin; Module.createRequire = createRequire; Module._cache = {}; Module._extensions = { '.js': function() {}, '.json': function() {}, '.node': function() {} };\n\
             function syncBuiltinESMExports() {} function findSourceMap() { return undefined; }\n\
             function SourceMap(payload) { if (!(this instanceof SourceMap)) return new SourceMap(payload); this.payload = payload || {}; } SourceMap.prototype.findEntry = function(line, column) { return { generatedLine: Number(line), generatedColumn: Number(column || 0), originalSource: undefined, originalLine: undefined, originalColumn: undefined, name: undefined }; }; SourceMap.prototype.findOrigin = function(line, column) { return this.findEntry(line, column); };\n\
             function register() { return undefined; } function registerHooks(hooks) { var active = true; return { deregister: function() { active = false; }, get active() { return active; }, hooks: hooks }; }\n\
             Object.assign(Module, { Module: Module, createRequire: createRequire, builtinModules: builtinModules, isBuiltin: isBuiltin, syncBuiltinESMExports: syncBuiltinESMExports, findSourceMap: findSourceMap, SourceMap: SourceMap, register: register, registerHooks: registerHooks });\n\
             module.exports = Module; module.exports.default = Module; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}
