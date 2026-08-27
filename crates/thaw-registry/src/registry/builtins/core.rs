pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "util" => Some(
            "var inspectCustom = Symbol.for('nodejs.util.inspect.custom');\n\
             function inspect(value, options) {\n\
             \x20\x20if (value && typeof value[inspectCustom] === 'function') return String(value[inspectCustom](2, options || {}, inspect));\n\
             \x20\x20if (typeof value === 'string') return \"'\" + value.replace(/\\\\/g, '\\\\\\\\').replace(/'/g, \"\\\\'\") + \"'\";\n\
             \x20\x20if (typeof value === 'function') return '[Function' + (value.name ? ': ' + value.name : '') + ']';\n\
             \x20\x20if (typeof value === 'symbol' || typeof value === 'bigint') return String(value);\n\
             \x20\x20if (value instanceof Error) return value.stack || value.name + ': ' + value.message;\n\
             \x20\x20var seen = new Set();\n\
             \x20\x20function render(input) {\n\
             \x20\x20\x20\x20if (input === null || typeof input !== 'object') return typeof input === 'string' ? \"'\" + input + \"'\" : String(input);\n\
             \x20\x20\x20\x20if (seen.has(input)) return '[Circular]'; seen.add(input);\n\
             \x20\x20\x20\x20var result;\n\
             \x20\x20\x20\x20if (Array.isArray(input)) result = '[ ' + input.map(render).join(', ') + ' ]';\n\
             \x20\x20\x20\x20else if (input instanceof Date) result = isNaN(input.getTime()) ? 'Invalid Date' : input.toISOString();\n\
             \x20\x20\x20\x20else if (input instanceof RegExp) result = String(input);\n\
             \x20\x20\x20\x20else if (input instanceof Map) result = 'Map(' + input.size + ') { ' + Array.from(input).map(function(entry) { return render(entry[0]) + ' => ' + render(entry[1]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20else if (input instanceof Set) result = 'Set(' + input.size + ') { ' + Array.from(input).map(render).join(', ') + ' }';\n\
             \x20\x20\x20\x20else result = '{ ' + Object.keys(input).map(function(key) { return key + ': ' + render(input[key]); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20seen.delete(input); return result;\n\
             \x20\x20}\n\
             \x20\x20return render(value);\n\
             }\n\
             inspect.custom = inspectCustom; inspect.defaultOptions = {};\n\
             function format() {\n\
             \x20\x20var args = Array.prototype.slice.call(arguments); if (args.length === 0) return '';\n\
             \x20\x20if (typeof args[0] !== 'string') return args.map(inspect).join(' ');\n\
             \x20\x20var index = 1; var output = args[0].replace(/%[sdifjoOc%]/g, function(token) {\n\
             \x20\x20\x20\x20if (token === '%%') return '%'; if (index >= args.length) return token; var value = args[index++];\n\
             \x20\x20\x20\x20if (token === '%s') return String(value); if (token === '%d') return String(Number(value));\n\
             \x20\x20\x20\x20if (token === '%i') return String(parseInt(value, 10)); if (token === '%f') return String(parseFloat(value));\n\
             \x20\x20\x20\x20if (token === '%j') { try { return JSON.stringify(value); } catch (_) { return '[Circular]'; } }\n\
             \x20\x20\x20\x20if (token === '%c') return ''; return inspect(value);\n\
             \x20\x20});\n\
             \x20\x20while (index < args.length) { var extra = args[index++]; output += ' ' + (typeof extra === 'string' ? extra : inspect(extra)); } return output;\n\
             }\n\
             function formatWithOptions(options) { return format.apply(null, Array.prototype.slice.call(arguments, 1)); }\n\
             function inherits(constructor, superConstructor) { if (constructor === undefined || superConstructor === undefined) throw new TypeError('constructors are required'); constructor.super_ = superConstructor; Object.setPrototypeOf(constructor.prototype, superConstructor.prototype); }\n\
             var promisifyCustom = Symbol.for('nodejs.util.promisify.custom');\n\
             function promisify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function'); if (original[promisifyCustom]) return original[promisifyCustom];\n\
             \x20\x20function wrapped() { var self = this; var args = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { args.push(function(error) { if (error) reject(error); else { var values = Array.prototype.slice.call(arguments, 1); resolve(values.length > 1 ? values : values[0]); } }); original.apply(self, args); }); }\n\
             \x20\x20Object.setPrototypeOf(wrapped, Object.getPrototypeOf(original)); return wrapped;\n\
             }\n\
             promisify.custom = promisifyCustom;\n\
             function callbackify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function');\n\
             \x20\x20return function() { var args = Array.prototype.slice.call(arguments); var callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); Promise.resolve(original.apply(this, args)).then(function(value) { queueMicrotask(function() { callback(null, value); }); }, function(error) { queueMicrotask(function() { callback(error || new Error('Promise was rejected with a falsy value')); }); }); };\n\
             }\n\
             function deprecate(fn) { return function() { return fn.apply(this, arguments); }; }\n\
             function stripVTControlCharacters(value) { return String(value).replace(/[\\u001B\\u009B][[\\]()#;?]*(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]+)*)?\\u0007|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-nq-uy=><~]))/g, ''); }\n\
             function toUSVString(value) { return String(value).replace(/[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(^|[^\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]/g, function(match, prefix) { return (prefix || '') + '\\uFFFD'; }); }\n\
             function parseArgs(config) { config = config || {}; var args = Array.from(config.args === undefined ? (process.argv || []).slice(2) : config.args, String), definitions = config.options || {}, values = Object.create(null), positionals = [], tokens = [], short = Object.create(null); Object.keys(definitions).forEach(function(name) { var definition = definitions[name] || {}; if (definition.short) short[String(definition.short)] = name; if (definition.default !== undefined) values[name] = definition.multiple ? Array.from(definition.default) : definition.default; else if (definition.multiple) values[name] = []; }); function assign(name, value, index, inline) { var definition = definitions[name]; if (!definition) { if (config.strict !== false) { var error = new TypeError('Unknown option --' + name); error.code = 'ERR_PARSE_ARGS_UNKNOWN_OPTION'; throw error; } definition = { type: 'boolean' }; } if (definition.type === 'string') { if (value === undefined) { var error = new TypeError('Option --' + name + ' argument is missing'); error.code = 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE'; throw error; } value = String(value); } else value = value === undefined ? true : Boolean(value); if (definition.multiple) (values[name] || (values[name] = [])).push(value); else values[name] = value; tokens.push({ kind: 'option', index: index, name: name, rawName: inline || '--' + name, value: definition.type === 'string' ? value : undefined, inlineValue: Boolean(inline && inline.indexOf('=') >= 0) }); } for (var index = 0; index < args.length; index++) { var argument = args[index]; if (argument === '--') { tokens.push({ kind: 'option-terminator', index: index }); for (index++; index < args.length; index++) { positionals.push(args[index]); tokens.push({ kind: 'positional', index: index, value: args[index] }); } break; } if (argument.slice(0, 2) === '--') { var equal = argument.indexOf('='), name = argument.slice(2, equal < 0 ? undefined : equal), value = equal < 0 ? undefined : argument.slice(equal + 1); if (name.slice(0, 3) === 'no-' && config.allowNegative && definitions[name.slice(3)] && definitions[name.slice(3)].type === 'boolean') { assign(name.slice(3), false, index, argument); continue; } var definition = definitions[name]; if (equal < 0 && definition && definition.type === 'string') value = args[++index]; assign(name, value, equal < 0 && definition && definition.type === 'string' ? index - 1 : index, argument); } else if (argument[0] === '-' && argument.length > 1) { var letters = argument.slice(1); for (var letterIndex = 0; letterIndex < letters.length; letterIndex++) { var name = short[letters[letterIndex]] || letters[letterIndex], definition = definitions[name], value; if (definition && definition.type === 'string') { value = letters.slice(letterIndex + 1) || args[++index]; letterIndex = letters.length; } assign(name, value, index, '-' + letters[letterIndex]); } } else { if (!config.allowPositionals && config.strict !== false) { var error = new TypeError('Unexpected argument ' + argument); error.code = 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL'; throw error; } positionals.push(argument); tokens.push({ kind: 'positional', index: index, value: argument }); } } var result = { values: values, positionals: positionals }; if (config.tokens) result.tokens = tokens; return result; }\n\
             var types = globalThis.__thaw_util_types || (globalThis.__thaw_util_types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return Object.prototype.toString.call(value) === '[object Arguments]'; } });\n\
             module.exports = { inspect: inspect, format: format, formatWithOptions: formatWithOptions, inherits: inherits, promisify: promisify, callbackify: callbackify, deprecate: deprecate, stripVTControlCharacters: stripVTControlCharacters, toUSVString: toUSVString, parseArgs: parseArgs, TextEncoder: globalThis.TextEncoder, TextDecoder: globalThis.TextDecoder, types: types };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "util/types" => Some(
            "var tag = function(value) { return Object.prototype.toString.call(value); }; var types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isWeakMap: function(value) { return value instanceof WeakMap; }, isWeakSet: function(value) { return value instanceof WeakSet; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer); }, isArrayBufferView: function(value) { return ArrayBuffer.isView(value); }, isDataView: function(value) { return value instanceof DataView; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isUint8Array: function(value) { return value instanceof Uint8Array; }, isUint8ClampedArray: function(value) { return value instanceof Uint8ClampedArray; }, isUint16Array: function(value) { return value instanceof Uint16Array; }, isUint32Array: function(value) { return value instanceof Uint32Array; }, isInt8Array: function(value) { return value instanceof Int8Array; }, isInt16Array: function(value) { return value instanceof Int16Array; }, isInt32Array: function(value) { return value instanceof Int32Array; }, isFloat32Array: function(value) { return value instanceof Float32Array; }, isFloat64Array: function(value) { return value instanceof Float64Array; }, isBigInt64Array: function(value) { return typeof BigInt64Array === 'function' && value instanceof BigInt64Array; }, isBigUint64Array: function(value) { return typeof BigUint64Array === 'function' && value instanceof BigUint64Array; }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return tag(value) === '[object Arguments]'; }, isNumberObject: function(value) { return tag(value) === '[object Number]'; }, isStringObject: function(value) { return tag(value) === '[object String]'; }, isBooleanObject: function(value) { return tag(value) === '[object Boolean]'; }, isBigIntObject: function(value) { return tag(value) === '[object BigInt]'; }, isSymbolObject: function(value) { return tag(value) === '[object Symbol]'; }, isBoxedPrimitive: function(value) { return /\\[object (Number|String|Boolean|BigInt|Symbol)\\]/.test(tag(value)); }, isAsyncFunction: function(value) { return tag(value) === '[object AsyncFunction]'; }, isGeneratorFunction: function(value) { return tag(value) === '[object GeneratorFunction]'; }, isGeneratorObject: function(value) { return tag(value) === '[object Generator]'; }, isExternal: function() { return false; }, isProxy: function() { return false; }, isModuleNamespaceObject: function() { return false; }, isKeyObject: function() { return false; }, isCryptoKey: function() { return false; } }; module.exports = types; module.exports.default = types; module.exports.__esModule = true;\n",
        ),
        // Found necessary by a real ESM package (`has-flag`): `import
        // process from 'process'` -- Node exposes `process` as both a
        // global and a core module; this is the module half. `.default`
        // is set too so the ESM-interop convention `rewrite_esm_to_commonjs`
        // generates for a default import (`.__esModule ? .default : ...`)
        // finds the same object either way. Only the couple of fields a
        // real package has actually been found to read.
        "process" => Some(
            "var __thaw_process = globalThis.process || { argv: [], env: {}, platform: 'linux', version: '', versions: {}, cwd: function() { return '/'; }, nextTick: function(fn) { var args = Array.prototype.slice.call(arguments, 1); Promise.resolve().then(function() { fn.apply(undefined, args); }); } };\n\
             if (!__thaw_process.cwd) __thaw_process.cwd = function() { return '/'; };\n\
             module.exports = __thaw_process;\n\
             module.exports.default = __thaw_process;\n\
             module.exports.__esModule = true;\n",
        ),
        "child_process" => Some(
            r#"var EventEmitter = require('node:events'), streams = require('node:stream');
             function normalize(command, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; command = String(command); args = args.map(String); if (options.shell) { var shell = typeof options.shell === 'string' ? options.shell : '/bin/sh', joined = [command].concat(args).map(function(value) { return "'" + value.replace(/'/g, "'\\''") + "'"; }).join(' '); command = shell; args = ['-c', joined]; } var environment; if (options.env) { environment = {}; Object.keys(options.env).forEach(function(name) { if (options.env[name] !== undefined) environment[name] = String(options.env[name]); }); } var input = options.input === undefined ? undefined : Buffer.from(options.input, options.encoding).toString('hex'); return { command: command, args: args, options: { cwd: options.cwd === undefined ? undefined : String(options.cwd), env: environment, input: input, detached: Boolean(options.detached) }, original: options }; }
             function processError(record, normalized) { var error = new Error(record.error || ('Command failed: ' + normalized.command)); error.code = record.code || null; error.errno = record.errno === undefined ? null : record.errno; error.path = record.path || normalized.command; error.spawnargs = normalized.args.slice(); error.status = record.status === undefined ? null : record.status; error.signal = record.signal === undefined ? null : record.signal; return error; }
             function decode(record, options) { var encoding = options.encoding === undefined ? null : options.encoding, stdout = Buffer.from(record.stdout || '', 'hex'), stderr = Buffer.from(record.stderr || '', 'hex'); if (encoding && encoding !== 'buffer') { stdout = stdout.toString(encoding); stderr = stderr.toString(encoding); } return { pid: record.pid || 0, output: [null, stdout, stderr], stdout: stdout, stderr: stderr, status: record.status === undefined ? null : record.status, signal: record.signal === undefined ? null : record.signal, error: record.error ? processError(record, { command: record.path || '', args: [] }) : undefined }; }
             function spawnSync(command, args, options) { var normalized = normalize(command, args, options), record = JSON.parse(__thaw_child_process_sync(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options))), result = decode(record, normalized.original), maxBuffer = normalized.original.maxBuffer === undefined ? 1048576 : Number(normalized.original.maxBuffer); if (!result.error && result.stdout.length + result.stderr.length > maxBuffer) { var error = new Error('spawnSync ' + normalized.command + ' ENOBUFS'); error.code = 'ENOBUFS'; result.error = error; result.status = null; } return result; }
             function execFileSync(file, args, options) { var normalized = normalize(file, args, options), result = spawnSync(file, args, options); if (result.error || result.status !== 0) { var error = result.error || processError({ status: result.status, signal: result.signal }, normalized); error.stdout = result.stdout; error.stderr = result.stderr; error.output = result.output; error.pid = result.pid; throw error; } return result.stdout; }
             function execSync(command, options) { options = options || {}; return execFileSync(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false })); }
             var children = new Map(), signals = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGUSR1: 10, SIGUSR2: 12, SIGPIPE: 13, SIGALRM: 14, SIGTERM: 15 }; function signalName(signal) { if (signal === null || signal === undefined) return null; var names = Object.keys(signals); for (var index = 0; index < names.length; index++) if (signals[names[index]] === signal) return names[index]; return 'SIG' + signal; } function signalNumber(signal) { if (signal === undefined || signal === null) return signals.SIGTERM; if (typeof signal === 'string') { signal = signal.toUpperCase(); if (signals[signal] !== undefined) return signals[signal]; } else if (Number.isInteger(signal) && signal > 0) return signal; var error = new TypeError('Unknown signal: ' + signal); error.code = 'ERR_UNKNOWN_SIGNAL'; throw error; }
             function ChildProcess(normalized) { EventEmitter.call(this); this.pid = 0; this.spawnfile = normalized.command; this.spawnargs = [normalized.command].concat(normalized.args); this.killed = false; this.connected = false; this.exitCode = null; this.signalCode = null; this.detached = Boolean(normalized.original.detached); this._closed = false; this._spawnError = false; this._timedOut = false; var child = this; this._stdin = new streams.Writable({ write: function(chunk, encoding, callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, Buffer.from(chunk).toString('hex'), false); callback(); }, final: function(callback) { if (!child._closed) __thaw_child_process_stdin(child._handle, '', true); callback(); } }); this._stdout = new streams.PassThrough(); this._stderr = new streams.PassThrough(); var stdio = normalized.original.stdio === undefined ? 'pipe' : normalized.original.stdio, modes = Array.isArray(stdio) ? stdio.slice(0, 3) : [stdio, stdio, stdio]; while (modes.length < 3) modes.push('pipe'); modes = modes.map(function(mode) { mode = mode === null || mode === undefined ? 'pipe' : mode; if (mode !== 'pipe' && mode !== 'ignore' && mode !== 'inherit') { var error = new TypeError('The argument stdio is invalid. Received ' + mode); error.code = 'ERR_INVALID_ARG_VALUE'; throw error; } return mode; }); this._stdioModes = modes; this.stdin = modes[0] === 'pipe' ? this._stdin : null; this.stdout = modes[1] === 'pipe' ? this._stdout : null; this.stderr = modes[2] === 'pipe' ? this._stderr : null; this.stdio = [this.stdin, this.stdout, this.stderr]; this.channel = null; this._handle = __thaw_child_process_spawn(normalized.command, JSON.stringify(normalized.args), JSON.stringify(normalized.options)); children.set(this._handle, this); if (modes[0] !== 'pipe') this._stdin.end(); var timeout = Number(normalized.original.timeout || 0); if (timeout > 0 && Number.isFinite(timeout)) this._timeout = setTimeout(function() { if (!child._closed) { child._timedOut = true; child.kill(normalized.original.killSignal); } }, timeout); var signal = normalized.original.signal; if (signal && typeof signal.addEventListener === 'function') { this._abortSignal = signal; this._abortListener = function() { if (child._closed || child._aborted) return; child._aborted = true; var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = signal.reason; child.emit('error', error); child.kill(normalized.original.killSignal); }; signal.addEventListener('abort', this._abortListener, { once: true }); if (signal.aborted) queueMicrotask(this._abortListener); } }
             ChildProcess.prototype = Object.create(EventEmitter.prototype); ChildProcess.prototype.constructor = ChildProcess; ChildProcess.prototype.kill = function(signal) { if (this._closed) return false; this.killed = __thaw_child_process_kill(this._handle, signalNumber(signal)); return this.killed; }; ChildProcess.prototype.ref = function() { return this; }; ChildProcess.prototype.unref = function() { return this; }; ChildProcess.prototype.disconnect = function() { if (!this.connected) { var error = new Error('IPC channel is already disconnected'); error.code = 'ERR_IPC_DISCONNECTED'; throw error; } this.connected = false; this.channel = null; if (this.stdin && !this.stdin.writableEnded) this.stdin.end(); this.emit('disconnect'); return this; }; ChildProcess.prototype.send = function(message, sendHandle, options, callback) { if (typeof sendHandle === 'function') callback = sendHandle; else if (typeof options === 'function') callback = options; if (!this.connected || !this._ipc) { var error = new Error('Channel closed'); error.code = 'ERR_IPC_CHANNEL_CLOSED'; if (callback) { queueMicrotask(function() { callback(error); }); return false; } throw error; } var serialized; try { serialized = JSON.stringify(message); } catch (error) { if (callback) queueMicrotask(function() { callback(error); }); else throw error; return false; } this.stdin.write('__THAW_IPC__' + serialized + '\n', function(error) { if (callback) callback(error || null); }); return true; };
             function spawn(command, args, options) { return new ChildProcess(normalize(command, args, options)); }
             function fork(modulePath, args, options) { if (!Array.isArray(args)) { options = args || {}; args = []; } else options = options || {}; if (typeof modulePath !== 'string' && !(modulePath instanceof String)) { var pathError = new TypeError('The modulePath argument must be of type string'); pathError.code = 'ERR_INVALID_ARG_TYPE'; throw pathError; } var wrapper = "var prefix='__THAW_IPC__',buffer='';process.send=function(value,handle,options,callback){if(typeof handle==='function')callback=handle;else if(typeof options==='function')callback=options;try{process.stdout.write(prefix+JSON.stringify(value)+'\\n',function(){if(typeof callback==='function')callback(null);});return true;}catch(error){if(typeof callback==='function')callback(error);else throw error;return false;}};process.connected=true;process.disconnect=function(){if(!process.connected)return;process.connected=false;process.emit('disconnect');process.stdin.destroy();setTimeout(function(){process.exit(0);},0);};process.stdin.setEncoding('utf8');process.stdin.on('data',function(chunk){buffer+=chunk;for(;;){var end=buffer.indexOf('\\n');if(end<0)break;var line=buffer.slice(0,end);buffer=buffer.slice(end+1);if(line.indexOf(prefix)===0)process.emit('message',JSON.parse(line.slice(prefix.length)));}});process.stdin.on('end',process.disconnect);require(require('path').resolve(process.argv[1]));"; var childOptions = Object.assign({}, options, { stdio: 'pipe' }), execArgv = options.execArgv === undefined ? [] : Array.from(options.execArgv, String), child = spawn(options.execPath || process.execPath || '/usr/bin/node', execArgv.concat(['-e', wrapper, String(modulePath)]).concat(args.map(String)), childOptions); child.connected = true; child.channel = {}; child._ipc = true; child._ipcBuffer = ''; child._ipcOutputMode = options.silent ? 'pipe' : 'inherit'; if (!options.silent) { child.stdout = null; child.stderr = null; child.stdio = [child.stdin, null, null, child.channel]; } else child.stdio.push(child.channel); return child; }
             function execFile(file, args, options, callback) { if (typeof args === 'function') { callback = args; args = []; options = {}; } else if (!Array.isArray(args)) { callback = options; options = args || {}; args = []; } else if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; var child = spawn(file, args, options), stdout = [], stderr = [], encoding = options.encoding || 'utf8', maxBuffer = options.maxBuffer === undefined ? 1048576 : Number(options.maxBuffer); child.stdout.on('data', function(value) { stdout.push(Buffer.from(value)); }); child.stderr.on('data', function(value) { stderr.push(Buffer.from(value)); }); child.once('close', function(code, signal) { var out = Buffer.concat(stdout), err = Buffer.concat(stderr), decodedOut = encoding === 'buffer' ? out : out.toString(encoding), decodedErr = encoding === 'buffer' ? err : err.toString(encoding), error = null; if (out.length + err.length > maxBuffer) { error = new Error('stdout maxBuffer length exceeded'); error.code = 'ERR_CHILD_PROCESS_STDIO_MAXBUFFER'; } else if (code !== 0) { error = new Error('Command failed: ' + file + '\n' + decodedErr); error.code = code; error.killed = child.killed; error.signal = signal; error.cmd = file; } if (error) { error.stdout = decodedOut; error.stderr = decodedErr; } if (typeof callback === 'function') callback(error, decodedOut, decodedErr); }); child.stdin.end(); return child; }
             function exec(command, options, callback) { if (typeof options === 'function') { callback = options; options = {}; } options = options || {}; return execFile(options.shell || '/bin/sh', ['-c', String(command)], Object.assign({}, options, { shell: false }), callback); }
             if (!globalThis.__thaw_child_process_poll_installed) { globalThis.__thaw_child_process_poll_installed = true; var previousPlatformPoll = globalThis.__thaw_poll_platform_events; globalThis.__thaw_poll_platform_events = function() { if (previousPlatformPoll) previousPlatformPoll(); JSON.parse(__thaw_child_process_poll()).forEach(function(event) { var child = children.get(event.handle); if (!child) return; if (event.type === 'spawn') { child.pid = Number(event.pid); child.emit('spawn'); } else if (event.type === 'stdout') { var stdout = Buffer.from(event.value, 'hex'); if (child._ipc) { child._ipcBuffer += stdout.toString(); for (;;) { var end = child._ipcBuffer.indexOf('\n'); if (end < 0) break; var line = child._ipcBuffer.slice(0, end); child._ipcBuffer = child._ipcBuffer.slice(end + 1); if (line.indexOf('__THAW_IPC__') === 0) { try { child.emit('message', JSON.parse(line.slice(12))); } catch (error) { child.emit('error', error); } } else if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(line + '\n')); else child._stdout.write(Buffer.from(line + '\n')); } } else if (child._stdioModes[1] === 'pipe') child._stdout.write(stdout); else if (child._stdioModes[1] === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(stdout); } else if (event.type === 'stderr') { var stderr = Buffer.from(event.value, 'hex'); if (child._ipc && child._ipcOutputMode === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); else if (child._stdioModes[2] === 'pipe') child._stderr.write(stderr); else if (child._stdioModes[2] === 'inherit' && process.stderr && process.stderr.write) process.stderr.write(stderr); } else if (event.type === 'error') { child._spawnError = true; var error = new Error(event.message); error.code = event.code; error.path = child.spawnfile; error.spawnargs = child.spawnargs.slice(1); child.emit('error', error); } else if (event.type === 'exit') { child._closed = true; child.exitCode = event.code === null ? null : Number(event.code); child.signalCode = signalName(event.signal); if (child._ipcBuffer) { if (child._ipcOutputMode === 'inherit' && process.stdout && process.stdout.write) process.stdout.write(Buffer.from(child._ipcBuffer)); else child._stdout.write(Buffer.from(child._ipcBuffer)); } child._stdout.end(); child._stderr.end(); if (!child._stdin.writableEnded) child._stdin.end(); if (child.connected) { child.connected = false; child.channel = null; child.emit('disconnect'); } if (child._timeout) clearTimeout(child._timeout); if (child._abortSignal && child._abortListener) child._abortSignal.removeEventListener('abort', child._abortListener); if (!child._spawnError) child.emit('exit', child.exitCode, child.signalCode); child.emit('close', child.exitCode, child.signalCode); children.delete(event.handle); } }); }; }
             module.exports = { ChildProcess: ChildProcess, spawn: spawn, fork: fork, exec: exec, execFile: execFile, spawnSync: spawnSync, execFileSync: execFileSync, execSync: execSync }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "punycode" => Some(
            "var base = 36, tMin = 1, tMax = 26, skew = 38, damp = 700, initialBias = 72, initialN = 128, delimiter = '-'; function adapt(delta, points, first) { delta = first ? Math.floor(delta / damp) : delta >> 1; delta += Math.floor(delta / points); var k = 0; while (delta > Math.floor(((base - tMin) * tMax) / 2)) { delta = Math.floor(delta / (base - tMin)); k += base; } return k + Math.floor(((base - tMin + 1) * delta) / (delta + skew)); } function encodeDigit(value) { return String.fromCharCode(value + 22 + 75 * (value < 26)); } function decodeDigit(code) { if (code >= 48 && code <= 57) return code - 22; if (code >= 65 && code <= 90) return code - 65; if (code >= 97 && code <= 122) return code - 97; return base; } function codePoints(value) { return Array.from(String(value)).map(function(character) { return character.codePointAt(0); }); }\n\
             function encode(value) { var input = codePoints(value), output = [], n = initialN, delta = 0, bias = initialBias; input.forEach(function(point) { if (point < 128) output.push(String.fromCharCode(point)); }); var basic = output.length, handled = basic; if (basic) output.push(delimiter); while (handled < input.length) { var next = Infinity; input.forEach(function(point) { if (point >= n && point < next) next = point; }); delta += (next - n) * (handled + 1); n = next; input.forEach(function(point) { if (point < n) delta++; if (point === n) { var q = delta; for (var k = base;; k += base) { var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (q < threshold) break; output.push(encodeDigit(threshold + ((q - threshold) % (base - threshold)))); q = Math.floor((q - threshold) / (base - threshold)); } output.push(encodeDigit(q)); bias = adapt(delta, handled + 1, handled === basic); delta = 0; handled++; } }); delta++; n++; } return output.join(''); }\n\
             function decode(value) { var input = String(value), output = [], n = initialN, index = 0, bias = initialBias, i = 0, delimiterIndex = input.lastIndexOf(delimiter); if (delimiterIndex >= 0) { for (var basicIndex = 0; basicIndex < delimiterIndex; basicIndex++) { var basicCode = input.charCodeAt(basicIndex); if (basicCode >= 128) throw new RangeError('Illegal input'); output.push(basicCode); } index = delimiterIndex + 1; } while (index < input.length) { var oldI = i, weight = 1; for (var k = base;; k += base) { if (index >= input.length) throw new RangeError('Invalid input'); var digit = decodeDigit(input.charCodeAt(index++)); if (digit >= base) throw new RangeError('Invalid input'); i += digit * weight; var threshold = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias; if (digit < threshold) break; weight *= base - threshold; } var length = output.length + 1; bias = adapt(i - oldI, length, oldI === 0); n += Math.floor(i / length); i %= length; output.splice(i, 0, n); i++; } return String.fromCodePoint.apply(String, output); }\n\
             function mapDomain(value, callback) { var input = String(value), parts = input.split('@'), local = ''; if (parts.length > 1) local = parts.shift() + '@'; return local + parts.join('@').replace(/[\\u3002\\uFF0E\\uFF61]/g, '.').split('.').map(callback).join('.'); } function toASCII(value) { return mapDomain(value, function(label) { return /[^\\x00-\\x7F]/.test(label) ? 'xn--' + encode(label) : label; }); } function toUnicode(value) { return mapDomain(value, function(label) { return /^xn--/i.test(label) ? decode(label.slice(4).toLowerCase()) : label; }); } var ucs2 = { decode: codePoints, encode: function(points) { return String.fromCodePoint.apply(String, points); } }; module.exports = { version: '2.1.0', ucs2: ucs2, decode: decode, encode: encode, toASCII: toASCII, toUnicode: toUnicode }; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        // Found necessary chasing a real native addon's load path
        // (`bcrypt`, `utf-8-validate`): both depend on `node-gyp-build`,
        // which unconditionally does `require('path')`/`require('os')`/
        // `require('fs')` at the top of its own real, unmodified source
        // (`node-gyp-build.js`) before it ever gets to the actual
        // native-addon lookup. Pure string manipulation, no dependency on
        // any crate -- `path` never touches a real filesystem in Node
        // either (that's what `fs` is for).
        "path" => Some(
            "function __thaw_path_normalize(p) {\n\
             \x20\x20p = String(p); if (p === '') return '.'; var parts = p.split('/'); var out = []; var trailing = p.length > 1 && p.charAt(p.length - 1) === '/';\n\
             \x20\x20var abs = p.charAt(0) === '/';\n\
             \x20\x20for (var i = 0; i < parts.length; i++) {\n\
             \x20\x20\x20\x20var part = parts[i];\n\
             \x20\x20\x20\x20if (part === '' || part === '.') continue;\n\
             \x20\x20\x20\x20if (part === '..') { if (out.length && out[out.length - 1] !== '..') out.pop(); else if (!abs) out.push('..'); } else { out.push(part); }\n\
             \x20\x20}\n\
             \x20\x20var result = (abs ? '/' : '') + out.join('/'); if (result === '') result = abs ? '/' : '.'; if (trailing && result !== '/') result += '/'; return result;\n\
             }\n\
             function resolve() {\n\
             \x20\x20var p = '';\n\
             \x20\x20for (var i = 0; i < arguments.length; i++) {\n\
             \x20\x20\x20\x20var seg = String(arguments[i]);\n\
             \x20\x20\x20\x20if (seg.charAt(0) === '/') { p = seg; } else { p = p ? p + '/' + seg : seg; }\n\
             \x20\x20}\n\
             \x20\x20if (p.charAt(0) !== '/') p = (globalThis.process && process.cwd ? process.cwd() : '/') + '/' + p;\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20return n.charAt(0) === '/' ? n : '/' + n;\n\
             }\n\
             function join() {\n\
             \x20\x20return __thaw_path_normalize(Array.prototype.join.call(arguments, '/'));\n\
             }\n\
             function dirname(p) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20if (idx <= 0) return n.charAt(0) === '/' ? '/' : '.';\n\
             \x20\x20return n.substring(0, idx);\n\
             }\n\
             function basename(p, suffix) {\n\
             \x20\x20var n = __thaw_path_normalize(p);\n\
             \x20\x20var idx = n.lastIndexOf('/');\n\
             \x20\x20var base = idx === -1 ? n : n.substring(idx + 1); if (suffix && base.endsWith(String(suffix))) base = base.substring(0, base.length - String(suffix).length); return base;\n\
             }\n\
             function extname(p) { var base = basename(p); var index = base.lastIndexOf('.'); return index <= 0 ? '' : base.substring(index); }\n\
             function isAbsolute(p) { return String(p).charAt(0) === '/'; }\n\
             function relative(from, to) { var left = resolve(from).split('/').filter(Boolean); var right = resolve(to).split('/').filter(Boolean); var shared = 0; while (shared < left.length && shared < right.length && left[shared] === right[shared]) shared++; return left.slice(shared).map(function() { return '..'; }).concat(right.slice(shared)).join('/') || ''; }\n\
             function parse(p) { var root = isAbsolute(p) ? '/' : ''; var dir = dirname(p); var base = basename(p); var ext = extname(base); return { root: root, dir: dir, base: base, ext: ext, name: ext ? base.substring(0, base.length - ext.length) : base }; }\n\
             function format(value) { var dir = value.dir || value.root || ''; var base = value.base || String(value.name || '') + String(value.ext || ''); return dir ? (dir === '/' ? '/' : dir + '/') + base : base; }\n\
             function toNamespacedPath(p) { return p; }\n\
             var posix = { resolve: resolve, join: join, dirname: dirname, basename: basename, extname: extname, normalize: __thaw_path_normalize, relative: relative, isAbsolute: isAbsolute, parse: parse, format: format, toNamespacedPath: toNamespacedPath, sep: '/', delimiter: ':' };\n\
             function winInput(p) { return String(p).replace(/\\\\/g, '/').replace(/^([A-Za-z]):/, '/$1:'); }\n\
             function winOutput(p) { return String(p).replace(/^\\/([A-Za-z]:)/, '$1').replace(/\\//g, '\\\\'); }\n\
             var win32 = { resolve: function() { return winOutput(resolve.apply(null, Array.from(arguments, winInput))); }, join: function() { return winOutput(join.apply(null, Array.from(arguments, winInput))); }, dirname: function(p) { return winOutput(dirname(winInput(p))); }, basename: function(p, suffix) { return basename(winInput(p), suffix); }, extname: function(p) { return extname(winInput(p)); }, normalize: function(p) { return winOutput(__thaw_path_normalize(winInput(p))); }, relative: function(from, to) { return winOutput(relative(winInput(from), winInput(to))); }, isAbsolute: function(p) { return /^[A-Za-z]:[\\\\/]|^[\\\\/]{2}/.test(String(p)); }, parse: function(p) { var result = parse(winInput(p)); result.root = /^[A-Za-z]:/.test(String(p)) ? String(p).substring(0, 3) : result.root; result.dir = winOutput(result.dir); return result; }, format: function(value) { return winOutput(format(value)); }, toNamespacedPath: toNamespacedPath, sep: '\\\\', delimiter: ';' };\n\
             module.exports = posix; module.exports.posix = posix; module.exports.win32 = win32;\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "path/posix" => Some(
            "module.exports = require('node:path').posix; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "path/win32" => Some(
            "module.exports = require('node:path').win32; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "url" => Some(
            "function pathToFileURL(path) {\n\
             \x20\x20var value = String(path);\n\
             \x20\x20if (value.charAt(0) !== '/') value = '/' + value;\n\
             \x20\x20var encoded = value.split('/').map(function(part) { return encodeURIComponent(part); }).join('/');\n\
             \x20\x20return new globalThis.URL('file://' + encoded);\n\
             }\n\
             function fileURLToPath(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20if (url.protocol !== 'file:') throw new TypeError('URL must use the file: protocol');\n\
             \x20\x20if (url.hostname !== '' && url.hostname !== 'localhost') throw new TypeError('file URL host must be empty or localhost');\n\
             \x20\x20if (/%2f|%5c/i.test(url.pathname)) throw new TypeError('file URL path must not include encoded separators');\n\
             \x20\x20return decodeURIComponent(url.pathname);\n\
             }\n\
             function urlToHttpOptions(input) {\n\
             \x20\x20var url = input instanceof globalThis.URL ? input : new globalThis.URL(input);\n\
             \x20\x20var options = { protocol: url.protocol, hostname: url.hostname, hash: url.hash, search: url.search, pathname: url.pathname, path: url.pathname + url.search, href: url.href };\n\
             \x20\x20if (url.port !== '') options.port = Number(url.port);\n\
             \x20\x20if (url.username !== '' || url.password !== '') options.auth = decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password);\n\
             \x20\x20return options;\n\
             }\n\
             module.exports = { URL: globalThis.URL, URLSearchParams: globalThis.URLSearchParams, pathToFileURL: pathToFileURL, fileURLToPath: fileURLToPath, urlToHttpOptions: urlToHttpOptions };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "querystring" => Some(
            "function escape(value) { return encodeURIComponent(String(value)); }\n\
             function unescape(value) {\n\
             \x20\x20try { return decodeURIComponent(String(value).replace(/\\+/g, ' ')); } catch (_) { return String(value); }\n\
             }\n\
             function primitive(value) {\n\
             \x20\x20return value === null || value === undefined ? '' : (typeof value === 'string' || typeof value === 'number' || typeof value === 'bigint' || typeof value === 'boolean' ? String(value) : '');\n\
             }\n\
             function stringify(object, separator, assignment, options) {\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var encoder = options && typeof options.encodeURIComponent === 'function' ? options.encodeURIComponent : escape;\n\
             \x20\x20if (object === null || typeof object !== 'object') return '';\n\
             \x20\x20var fields = [];\n\
             \x20\x20Object.keys(object).forEach(function(key) {\n\
             \x20\x20\x20\x20var values = Array.isArray(object[key]) ? object[key] : [object[key]];\n\
             \x20\x20\x20\x20if (values.length === 0) return;\n\
             \x20\x20\x20\x20values.forEach(function(value) { fields.push(encoder(key) + assignment + encoder(primitive(value))); });\n\
             \x20\x20});\n\
             \x20\x20return fields.join(separator);\n\
             }\n\
             function parse(text, separator, assignment, options) {\n\
             \x20\x20var result = Object.create(null);\n\
             \x20\x20var source = String(text);\n\
             \x20\x20separator = separator === undefined ? '&' : String(separator);\n\
             \x20\x20assignment = assignment === undefined ? '=' : String(assignment);\n\
             \x20\x20var decoder = options && typeof options.decodeURIComponent === 'function' ? options.decodeURIComponent : unescape;\n\
             \x20\x20var maxKeys = options && options.maxKeys !== undefined ? Number(options.maxKeys) : 1000;\n\
             \x20\x20var fields = source === '' ? [] : source.split(separator);\n\
             \x20\x20if (maxKeys > 0) fields = fields.slice(0, maxKeys);\n\
             \x20\x20fields.forEach(function(field) {\n\
             \x20\x20\x20\x20var index = field.indexOf(assignment);\n\
             \x20\x20\x20\x20var key = decoder(index < 0 ? field : field.substring(0, index));\n\
             \x20\x20\x20\x20var value = decoder(index < 0 ? '' : field.substring(index + assignment.length));\n\
             \x20\x20\x20\x20if (!Object.prototype.hasOwnProperty.call(result, key)) result[key] = value;\n\
             \x20\x20\x20\x20else if (Array.isArray(result[key])) result[key].push(value);\n\
             \x20\x20\x20\x20else result[key] = [result[key], value];\n\
             \x20\x20});\n\
             \x20\x20return result;\n\
             }\n\
             module.exports = { stringify: stringify, encode: stringify, parse: parse, decode: parse, escape: escape, unescape: unescape };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "sys" => Some(
            "module.exports = require('node:util');\n",
        ),
        "domain" => Some(
            r#"var EventEmitter = require('node:events'), stack = [];
             function Domain() { if (!(this instanceof Domain)) return new Domain(); EventEmitter.call(this); this.members = []; }
             Domain.prototype = Object.create(EventEmitter.prototype); Domain.prototype.constructor = Domain;
             Domain.prototype.enter = function() { stack.push(module.exports.active); module.exports.active = this; if (globalThis.process) process.domain = this; return this; };
             Domain.prototype.exit = function() { if (module.exports.active !== this) return this; module.exports.active = stack.length ? stack.pop() : null; if (globalThis.process) process.domain = module.exports.active || null; return this; };
             Domain.prototype.add = function(emitter) { if (!emitter || (typeof emitter !== 'object' && typeof emitter !== 'function')) throw new TypeError('emitter must be an object'); if (emitter.domain && emitter.domain !== this && emitter.domain.remove) emitter.domain.remove(emitter); if (this.members.indexOf(emitter) < 0) this.members.push(emitter); emitter.domain = this; return this; };
             Domain.prototype.remove = function(emitter) { var index = this.members.indexOf(emitter); if (index >= 0) this.members.splice(index, 1); if (emitter && emitter.domain === this) emitter.domain = null; return this; };
             Domain.prototype._handle = function(error) { if (!(error instanceof Error)) error = new Error(String(error)); error.domain = this; error.domainThrown = true; if (this.listenerCount('error')) { this.emit('error', error); return; } throw error; };
             Domain.prototype.run = function(fn) { var args = Array.prototype.slice.call(arguments, 1); this.enter(); try { return fn.apply(undefined, args); } catch (error) { return this._handle(error); } finally { this.exit(); } };
             Domain.prototype.bind = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function bound() { var args = arguments, receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } bound.domain = domain; return bound; };
             Domain.prototype.intercept = function(fn) { if (typeof fn !== 'function') throw new TypeError('callback must be a function'); var domain = this; function intercepted(error) { if (error) return domain._handle(error); var args = Array.prototype.slice.call(arguments, 1), receiver = this; return domain.run(function() { return fn.apply(receiver, args); }); } intercepted.domain = domain; return intercepted; };
             Domain.prototype.dispose = function() { this.exit(); this.members.slice().forEach(this.remove, this); this.removeAllListeners(); return this; };
             function create() { return new Domain(); }
             module.exports = { Domain: Domain, create: create, createDomain: create, active: null }; module.exports.default = module.exports; module.exports.__esModule = true;
"#,
        ),
        "events" => Some(
            "function EventEmitter() {\n\
             \x20\x20if (!(this instanceof EventEmitter)) return new EventEmitter();\n\
             \x20\x20this._events = Object.create(null); this._maxListeners = undefined;\n\
             }\n\
             EventEmitter.prototype._add = function(event, listener, prepend, once) {\n\
             \x20\x20if (typeof listener !== 'function') throw new TypeError('listener must be a function');\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name] || (this._events[name] = []);\n\
             \x20\x20var entry = { listener: listener, once: Boolean(once) };\n\
             \x20\x20if (prepend) list.unshift(entry); else list.push(entry);\n\
             \x20\x20return this;\n\
             };\n\
             EventEmitter.prototype.addListener = EventEmitter.prototype.on = function(event, listener) { return this._add(event, listener, false, false); };\n\
             EventEmitter.prototype.once = function(event, listener) { return this._add(event, listener, false, true); };\n\
             EventEmitter.prototype.prependListener = function(event, listener) { return this._add(event, listener, true, false); };\n\
             EventEmitter.prototype.prependOnceListener = function(event, listener) { return this._add(event, listener, true, true); };\n\
             EventEmitter.prototype.emit = function(event) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name];\n\
             \x20\x20if (!list || list.length === 0) {\n\
             \x20\x20\x20\x20if (name === 'error') { var error = arguments[1]; throw error instanceof Error ? error : new Error('Unhandled error event'); }\n\
             \x20\x20\x20\x20return false;\n\
             \x20\x20}\n\
             \x20\x20var args = Array.prototype.slice.call(arguments, 1);\n\
             \x20\x20list.slice().forEach(function(entry) {\n\
             \x20\x20\x20\x20if (entry.once) this.removeListener(name, entry.listener);\n\
             \x20\x20\x20\x20entry.listener.apply(this, args);\n\
             \x20\x20}, this);\n\
             \x20\x20return true;\n\
             };\n\
             EventEmitter.prototype.removeListener = EventEmitter.prototype.off = function(event, listener) {\n\
             \x20\x20var name = typeof event === 'symbol' ? event : String(event); var list = this._events[name]; if (!list) return this;\n\
             \x20\x20for (var index = list.length - 1; index >= 0; index--) if (list[index].listener === listener || list[index].listener.listener === listener) { list.splice(index, 1); break; }\n\
             \x20\x20if (list.length === 0) delete this._events[name]; return this;\n\
             };\n\
             EventEmitter.prototype.removeAllListeners = function(event) { if (event === undefined) this._events = Object.create(null); else delete this._events[typeof event === 'symbol' ? event : String(event)]; return this; };\n\
             EventEmitter.prototype.listeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener.listener || entry.listener; }); };\n\
             EventEmitter.prototype.rawListeners = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)] || []; return list.map(function(entry) { return entry.listener; }); };\n\
             EventEmitter.prototype.listenerCount = function(event) { var list = this._events[typeof event === 'symbol' ? event : String(event)]; return list ? list.length : 0; };\n\
             EventEmitter.prototype.eventNames = function() { return Reflect.ownKeys(this._events); };\n\
             EventEmitter.prototype.setMaxListeners = function(value) { value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); this._maxListeners = value; return this; }; EventEmitter.prototype.getMaxListeners = function() { return this._maxListeners === undefined ? EventEmitter.defaultMaxListeners : this._maxListeners; }; EventEmitter.defaultMaxListeners = 10;\n\
             EventEmitter.listenerCount = function(emitter, event) { return emitter.listenerCount(event); };\n\
             function once(emitter, event, options) {\n\
             \x20\x20return new Promise(function(resolve, reject) {\n\
             \x20\x20\x20\x20function cleanup() { emitter.removeListener(event, done); emitter.removeListener('error', failed); if (options && options.signal) options.signal.removeEventListener('abort', aborted); }\n\
             \x20\x20\x20\x20function done() { var values = Array.prototype.slice.call(arguments); cleanup(); resolve(values); }\n\
             \x20\x20\x20\x20function failed(error) { cleanup(); reject(error); }\n\
             \x20\x20\x20\x20function aborted() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; cleanup(); reject(error); }\n\
             \x20\x20\x20\x20if (options && options.signal && options.signal.aborted) { aborted(); return; } emitter.once(event, done); if (event !== 'error') emitter.once('error', failed); if (options && options.signal) options.signal.addEventListener('abort', aborted, { once: true });\n\
             \x20\x20});\n\
             }\n\
             function on(emitter, event, options) { options = options || {}; var values = [], waiters = [], ended = false, failure; function received() { var value = Array.prototype.slice.call(arguments); if (waiters.length) waiters.shift().resolve({ value: value, done: false }); else values.push(value); } function fail(error) { failure = error; finish(); } function finish() { if (ended) return; ended = true; emitter.removeListener(event, received); if (event !== 'error') emitter.removeListener('error', fail); while (waiters.length) { var waiter = waiters.shift(); if (failure) waiter.reject(failure); else waiter.resolve({ value: undefined, done: true }); } } emitter.on(event, received); if (event !== 'error') emitter.on('error', fail); if (options.signal) { var abort = function() { var error = new Error('The operation was aborted'); error.name = 'AbortError'; error.code = 'ABORT_ERR'; error.cause = options.signal.reason; failure = error; finish(); }; if (options.signal.aborted) abort(); else options.signal.addEventListener('abort', abort, { once: true }); } return { next: function() { if (values.length) return Promise.resolve({ value: values.shift(), done: false }); if (failure) return Promise.reject(failure); if (ended) return Promise.resolve({ value: undefined, done: true }); return new Promise(function(resolve, reject) { waiters.push({ resolve: resolve, reject: reject }); }); }, return: function() { finish(); return Promise.resolve({ value: undefined, done: true }); }, throw: function(error) { failure = error; finish(); return Promise.reject(error); }, [Symbol.asyncIterator]: function() { return this; } }; }\n\
             function getEventListeners(emitter, event) { return typeof emitter.listeners === 'function' ? emitter.listeners(event) : []; } function getMaxListeners(emitter) { return typeof emitter.getMaxListeners === 'function' ? emitter.getMaxListeners() : EventEmitter.defaultMaxListeners; } function setMaxListeners(value) { var emitters = Array.prototype.slice.call(arguments, 1); value = Number(value); if (!Number.isFinite(value) || value < 0) throw new RangeError('The value of n is out of range'); if (!emitters.length) EventEmitter.defaultMaxListeners = value; else emitters.forEach(function(emitter) { emitter.setMaxListeners(value); }); }\n\
             module.exports = EventEmitter;\n\
             module.exports.EventEmitter = EventEmitter;\n\
             module.exports.once = once;\n\
             module.exports.on = on; module.exports.getEventListeners = getEventListeners; module.exports.getMaxListeners = getMaxListeners; module.exports.setMaxListeners = setMaxListeners; module.exports.errorMonitor = Symbol.for('events.errorMonitor'); module.exports.captureRejectionSymbol = Symbol.for('nodejs.rejection');\n\
             module.exports.default = EventEmitter;\n\
             module.exports.__esModule = true;\n",
        ),
        "assert" | "assert/strict" => Some(
            "function AssertionError(options) {\n\
             \x20\x20options = options || {}; this.name = 'AssertionError'; this.code = 'ERR_ASSERTION';\n\
             \x20\x20this.actual = options.actual; this.expected = options.expected; this.operator = options.operator;\n\
             \x20\x20this.generatedMessage = options.message === undefined;\n\
             \x20\x20this.message = options.message === undefined ? 'Expected values to satisfy ' + (options.operator || 'assertion') : String(options.message);\n\
             \x20\x20if (Error.captureStackTrace) Error.captureStackTrace(this, options.stackStartFn || AssertionError);\n\
             }\n\
             AssertionError.prototype = Object.create(Error.prototype); AssertionError.prototype.constructor = AssertionError;\n\
             function failure(actual, expected, message, operator, start) { throw new AssertionError({ actual: actual, expected: expected, message: message, operator: operator, stackStartFn: start }); }\n\
             function deep(actual, expected, seen) {\n\
             \x20\x20if (Object.is(actual, expected)) return true;\n\
             \x20\x20if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;\n\
             \x20\x20if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;\n\
             \x20\x20seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);\n\
             \x20\x20if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();\n\
             \x20\x20if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;\n\
             \x20\x20if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }\n\
             \x20\x20var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;\n\
             \x20\x20for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !deep(actual[key], expected[key], seen)) return false; }\n\
             \x20\x20return true;\n\
             }\n\
             function ok(value, message) { if (!value) failure(value, true, message, '==', ok); }\n\
             function equal(actual, expected, message) { if (actual != expected) failure(actual, expected, message, '==', equal); }\n\
             function notEqual(actual, expected, message) { if (actual == expected) failure(actual, expected, message, '!=', notEqual); }\n\
             function strictEqual(actual, expected, message) { if (!Object.is(actual, expected)) failure(actual, expected, message, 'strictEqual', strictEqual); }\n\
             function notStrictEqual(actual, expected, message) { if (Object.is(actual, expected)) failure(actual, expected, message, 'notStrictEqual', notStrictEqual); }\n\
             function deepStrictEqual(actual, expected, message) { if (!deep(actual, expected)) failure(actual, expected, message, 'deepStrictEqual', deepStrictEqual); }\n\
             function notDeepStrictEqual(actual, expected, message) { if (deep(actual, expected)) failure(actual, expected, message, 'notDeepStrictEqual', notDeepStrictEqual); }\n\
             function fail(message) { failure(undefined, undefined, message || 'Failed', 'fail', fail); }\n\
             function matches(error, expected) { if (expected === undefined) return true; if (expected instanceof RegExp) return expected.test(String(error && error.message || error)); if (typeof expected === 'function') return error instanceof expected || expected(error) === true; return true; }\n\
             function throws(block, expected, message) { var caught; try { block(); } catch (error) { caught = error; } if (caught === undefined || !matches(caught, expected)) failure(caught, expected, message, 'throws', throws); return caught; }\n\
             function doesNotThrow(block, expected, message) { try { block(); } catch (error) { if (matches(error, expected)) failure(error, undefined, message, 'doesNotThrow', doesNotThrow); throw error; } }\n\
             ok.AssertionError = AssertionError; ok.ok = ok; ok.equal = equal; ok.notEqual = notEqual; ok.strictEqual = strictEqual; ok.notStrictEqual = notStrictEqual;\n\
             ok.deepEqual = deepStrictEqual; ok.notDeepEqual = notDeepStrictEqual; ok.deepStrictEqual = deepStrictEqual; ok.notDeepStrictEqual = notDeepStrictEqual;\n\
             ok.fail = fail; ok.throws = throws; ok.doesNotThrow = doesNotThrow; ok.strict = ok; ok.default = ok; ok.__esModule = true;\n\
             module.exports = ok;\n",
        ),
        _ => None,
    }
}
