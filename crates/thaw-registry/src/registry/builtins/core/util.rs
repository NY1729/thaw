pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "util" => Some(
            "var inspectCustom = Symbol.for('nodejs.util.inspect.custom');\n\
             function inspect(value, options) {\n\
             \x20\x20options = options || {};\n\
             \x20\x20var depth = options.depth === undefined ? 2 : options.depth === null ? Infinity : options.depth, seen = new Set();\n\
             \x20\x20function renderProperty(owner, key, remaining) {\n\
             \x20\x20\x20\x20var descriptor = Object.getOwnPropertyDescriptor(owner, key);\n\
             \x20\x20\x20\x20if (!descriptor) return '';\n\
             \x20\x20\x20\x20if (Object.prototype.hasOwnProperty.call(descriptor, 'value')) return render(descriptor.value, remaining);\n\
             \x20\x20\x20\x20var getter = descriptor.get, setter = descriptor.set, evaluate = getter && (options.getters === true || options.getters === 'get' && !setter || options.getters === 'set' && setter);\n\
             \x20\x20\x20\x20if (evaluate) { var value; try { value = getter.call(owner); } catch (_) { return '[Inspection threw]'; } return render(value, remaining); }\n\
             \x20\x20\x20\x20return getter && setter ? '[Getter/Setter]' : getter ? '[Getter]' : setter ? '[Setter]' : '';\n\
             \x20\x20}\n\
             \x20\x20function render(input, remaining) {\n\
             \x20\x20\x20\x20if (input && options.customInspect !== false) { var custom = input[inspectCustom]; if (typeof custom === 'function') return String(custom.call(input, remaining, options, inspect)); }\n\
             \x20\x20if (input === null) return 'null';\n\
             \x20\x20if (typeof input === 'string') return \"'\" + input.replace(/\\\\/g, '\\\\\\\\').replace(/'/g, \"\\\\'\") + \"'\";\n\
             \x20\x20if (typeof input === 'function') return '[Function' + (input.name ? ': ' + input.name : '') + ']';\n\
             \x20\x20if (typeof input === 'symbol' || typeof input === 'bigint') return String(input);\n\
             \x20\x20if (typeof input !== 'object') return String(input);\n\
             \x20\x20\x20\x20if (input instanceof Error) return input.stack || input.name + ': ' + input.message;\n\
             \x20\x20\x20\x20if (input instanceof Date) return isNaN(input.getTime()) ? 'Invalid Date' : input.toISOString();\n\
             \x20\x20\x20\x20if (input instanceof RegExp) return String(input);\n\
             \x20\x20\x20\x20if (remaining < 0) return Array.isArray(input) ? '[Array]' : input instanceof Map ? '[Map]' : input instanceof Set ? '[Set]' : '[Object]';\n\
             \x20\x20\x20\x20if (seen.has(input)) return '[Circular]'; seen.add(input);\n\
             \x20\x20\x20\x20try {\n\
             \x20\x20\x20\x20\x20\x20if (Array.isArray(input)) { var items = []; for (var index = 0; index < input.length; index++) items.push(renderProperty(input, String(index), remaining - 1)); return '[ ' + items.join(', ') + ' ]'; }\n\
             \x20\x20\x20\x20\x20\x20if (input instanceof Map) return 'Map(' + input.size + ') { ' + Array.from(input).map(function(entry) { return render(entry[0], remaining - 1) + ' => ' + render(entry[1], remaining - 1); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20\x20\x20if (input instanceof Set) return 'Set(' + input.size + ') { ' + Array.from(input).map(function(value) { return render(value, remaining - 1); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20\x20\x20return '{ ' + Object.keys(input).map(function(key) { return key + ': ' + renderProperty(input, key, remaining - 1); }).join(', ') + ' }';\n\
             \x20\x20\x20\x20} finally { seen.delete(input); }\n\
             \x20\x20}\n\
             \x20\x20return render(value, depth);\n\
             }\n\
             inspect.custom = inspectCustom; inspect.defaultOptions = {};\n\
             function format() {\n\
             \x20\x20var args = Array.prototype.slice.call(arguments); if (args.length === 0) return '';\n\
             \x20\x20if (typeof args[0] !== 'string') return args.map(inspect).join(' ');\n\
             \x20\x20var index = 1; var output = args[0].replace(/%[sdifjoOc%]/g, function(token) {\n\
             \x20\x20\x20\x20if (token === '%%') return '%'; if (index >= args.length) return token; var value = args[index++];\n\
             \x20\x20\x20\x20if (typeof value === 'symbol' && (token === '%d' || token === '%i' || token === '%f')) return 'NaN';\n\
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
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function');\n\
             \x20\x20if (original[promisifyCustom]) { var custom = original[promisifyCustom]; if (typeof custom !== 'function') { var error = new TypeError('util.promisify.custom must be a function'); error.code = 'ERR_INVALID_ARG_TYPE'; throw error; } Object.defineProperty(custom, promisifyCustom, { value: custom, enumerable: false, writable: false, configurable: true }); return custom; }\n\
             \x20\x20function wrapped() { var self = this; var args = Array.prototype.slice.call(arguments); return new Promise(function(resolve, reject) { args.push(function(error, value) { if (error) reject(error); else resolve(value); }); original.apply(self, args); }); }\n\
             \x20\x20Object.setPrototypeOf(wrapped, Object.getPrototypeOf(original));\n\
             \x20\x20Object.defineProperty(wrapped, promisifyCustom, { value: wrapped, enumerable: false, writable: false, configurable: true }); return wrapped;\n\
             }\n\
             promisify.custom = promisifyCustom;\n\
             function callbackifyError(reason) { var error = new Error('Promise was rejected with falsy value'); error.code = 'ERR_FALSY_VALUE_REJECTION'; error.reason = reason; return error; }\n\
             function callbackifyReason(error) { return error ? error : callbackifyError(error); }\n\
             function callbackify(original) {\n\
             \x20\x20if (typeof original !== 'function') throw new TypeError('original must be a function');\n\
             \x20\x20return function() { var self = this, args = Array.prototype.slice.call(arguments), callback = args.pop(); if (typeof callback !== 'function') throw new TypeError('callback must be a function'); Promise.resolve(original.apply(self, args)).then(function(value) { queueMicrotask(function() { callback.call(self, null, value); }); }, function(error) { queueMicrotask(function() { callback.call(self, callbackifyReason(error)); }); }); };\n\
             }\n\
             function deprecate(fn) { return function() { return fn.apply(this, arguments); }; }\n\
             function debuglog(section) { var enabled = String(process.env.NODE_DEBUG || '').toUpperCase().split(/[ ,]+/).indexOf(String(section).toUpperCase()) >= 0; var logger = enabled ? function() { console.error(String(section).toUpperCase() + ' ' + format.apply(null, arguments)); } : function() {}; logger.enabled = enabled; return logger; }\n\
             function stripVTControlCharacters(value) { return String(value).replace(/[\\u001B\\u009B][[\\]()#;?]*(?:(?:[a-zA-Z\\d]*(?:;[-a-zA-Z\\d\\/#&.:=?%@~_]+)*)?\\u0007|(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-nq-uy=><~]))/g, ''); }\n\
             function toUSVString(value) { return String(value).replace(/[\\uD800-\\uDFFF]/gu, '\\uFFFD'); }\n\
             function parseArgs(config) {\n\
             \x20\x20config = config || {};\n\
             \x20\x20var args = Array.from(config.args === undefined ? (process.argv || []).slice(2) : config.args, String), definitions = config.options || {}, values = Object.create(null), positionals = [], tokens = [], short = Object.create(null);\n\
             \x20\x20Object.keys(definitions).forEach(function(name) { var definition = definitions[name] || {}; if (definition.short) short[String(definition.short)] = name; });\n\
             \x20\x20function assign(name, value, optionIndex, rawName, inlineValue) {\n\
             \x20\x20\x20\x20var definition = definitions[name];\n\
             \x20\x20\x20\x20if (!definition) { if (config.strict !== false) { var error = new TypeError('Unknown option --' + name); error.code = 'ERR_PARSE_ARGS_UNKNOWN_OPTION'; throw error; } definition = { type: 'boolean' }; }\n\
             \x20\x20\x20\x20var strict = config.strict !== false, supplied = typeof value === 'string';\n\
             \x20\x20\x20\x20if (strict && definition.type === 'boolean' && supplied) { var error = new TypeError('Option --' + name + ' does not take an argument'); error.code = 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE'; throw error; }\n\
             \x20\x20\x20\x20if (strict && definition.type === 'string' && (value === undefined || (!inlineValue && supplied && value.length > 1 && value.charAt(0) === '-'))) { var error = new TypeError(value === undefined ? 'Option --' + name + ' argument is missing' : 'Option --' + name + ' argument is ambiguous'); error.code = 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE'; throw error; }\n\
             \x20\x20\x20\x20if (definition.type === 'string' && value !== undefined) value = String(value);\n\
             \x20\x20\x20\x20else if (value === undefined) value = true;\n\
             \x20\x20\x20\x20else if (strict || !supplied) value = Boolean(value);\n\
             \x20\x20\x20\x20if (definition.multiple) (values[name] || (values[name] = [])).push(value); else values[name] = value;\n\
             \x20\x20\x20\x20tokens.push({ kind: 'option', index: optionIndex, name: name, rawName: rawName, value: definition.type === 'string' || (!strict && supplied) ? (supplied ? value : undefined) : undefined, inlineValue: inlineValue });\n\
             \x20\x20}\n\
             \x20\x20for (var index = 0; index < args.length; index++) {\n\
             \x20\x20\x20\x20var argument = args[index];\n\
             \x20\x20\x20\x20if (argument === '--') { tokens.push({ kind: 'option-terminator', index: index }); for (index++; index < args.length; index++) { positionals.push(args[index]); tokens.push({ kind: 'positional', index: index, value: args[index] }); } break; }\n\
             \x20\x20\x20\x20if (argument.slice(0, 2) === '--') {\n\
             \x20\x20\x20\x20\x20\x20var optionIndex = index, equal = argument.indexOf('='), name = argument.slice(2, equal < 0 ? undefined : equal), value = equal < 0 ? undefined : argument.slice(equal + 1), rawName = '--' + name, definition = definitions[name];\n\
             \x20\x20\x20\x20\x20\x20if (name.slice(0, 3) === 'no-' && config.allowNegative && definitions[name.slice(3)] && definitions[name.slice(3)].type === 'boolean') {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (equal < 0) { assign(name.slice(3), false, optionIndex, argument, undefined); continue; }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (config.strict !== false) { assign(name.slice(3), value, optionIndex, rawName, true); continue; }\n\
             \x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20if (equal < 0 && definition && definition.type === 'string' && index + 1 < args.length) value = args[++index];\n\
             \x20\x20\x20\x20\x20\x20assign(name, value, optionIndex, rawName, equal < 0 ? (value === undefined ? undefined : false) : true);\n\
             \x20\x20\x20\x20} else if (argument[0] === '-' && argument.length > 1) {\n\
             \x20\x20\x20\x20\x20\x20var letters = argument.slice(1);\n\
             \x20\x20\x20\x20\x20\x20for (var letterIndex = 0; letterIndex < letters.length; letterIndex++) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20var optionIndex = index, character = letters.charAt(letterIndex), name = short[character] || character, definition = definitions[name], value, inlineValue = undefined;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (definition && definition.type === 'string') {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20value = letters.slice(letterIndex + 1);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if (value !== '') inlineValue = true; else { value = args[index + 1]; if (value !== undefined) { index++; inlineValue = false; } }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20assign(name, value, optionIndex, '-' + character, inlineValue); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20assign(name, undefined, optionIndex, '-' + character, undefined);\n\
             \x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20} else { if (!config.allowPositionals && config.strict !== false) { var error = new TypeError('Unexpected argument ' + argument); error.code = 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL'; throw error; } positionals.push(argument); tokens.push({ kind: 'positional', index: index, value: argument }); }\n\
             \x20\x20}\n\
             \x20\x20Object.keys(definitions).forEach(function(name) { if (!Object.prototype.hasOwnProperty.call(values, name)) { var definition = definitions[name] || {}, defaultValue = definition.default; if (defaultValue !== undefined) values[name] = definition.multiple ? Array.from(defaultValue) : defaultValue; } });\n\
             \x20\x20var result = { values: values, positionals: positionals }; if (config.tokens) result.tokens = tokens; return result;\n\
             }\n\
             var types = globalThis.__thaw_util_types || (globalThis.__thaw_util_types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return Object.prototype.toString.call(value) === '[object Arguments]'; } });\n\
             module.exports = { inspect: inspect, format: format, formatWithOptions: formatWithOptions, inherits: inherits, promisify: promisify, callbackify: callbackify, deprecate: deprecate, debuglog: debuglog, stripVTControlCharacters: stripVTControlCharacters, toUSVString: toUSVString, parseArgs: parseArgs, TextEncoder: globalThis.TextEncoder, TextDecoder: globalThis.TextDecoder, types: types };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "util/types" => Some(
            "var tag = function(value) { return Object.prototype.toString.call(value); }; var types = { isDate: function(value) { return value instanceof Date; }, isRegExp: function(value) { return value instanceof RegExp; }, isMap: function(value) { return value instanceof Map; }, isSet: function(value) { return value instanceof Set; }, isWeakMap: function(value) { return value instanceof WeakMap; }, isWeakSet: function(value) { return value instanceof WeakSet; }, isPromise: function(value) { return value instanceof Promise; }, isArrayBuffer: function(value) { return value instanceof ArrayBuffer; }, isAnyArrayBuffer: function(value) { return value instanceof ArrayBuffer || (typeof SharedArrayBuffer === 'function' && value instanceof SharedArrayBuffer); }, isArrayBufferView: function(value) { return ArrayBuffer.isView(value); }, isDataView: function(value) { return value instanceof DataView; }, isTypedArray: function(value) { return ArrayBuffer.isView(value) && !(value instanceof DataView); }, isUint8Array: function(value) { return value instanceof Uint8Array; }, isUint8ClampedArray: function(value) { return value instanceof Uint8ClampedArray; }, isUint16Array: function(value) { return value instanceof Uint16Array; }, isUint32Array: function(value) { return value instanceof Uint32Array; }, isInt8Array: function(value) { return value instanceof Int8Array; }, isInt16Array: function(value) { return value instanceof Int16Array; }, isInt32Array: function(value) { return value instanceof Int32Array; }, isFloat32Array: function(value) { return value instanceof Float32Array; }, isFloat64Array: function(value) { return value instanceof Float64Array; }, isBigInt64Array: function(value) { return typeof BigInt64Array === 'function' && value instanceof BigInt64Array; }, isBigUint64Array: function(value) { return typeof BigUint64Array === 'function' && value instanceof BigUint64Array; }, isNativeError: function(value) { return value instanceof Error; }, isArgumentsObject: function(value) { return tag(value) === '[object Arguments]'; }, isNumberObject: function(value) { return tag(value) === '[object Number]'; }, isStringObject: function(value) { return tag(value) === '[object String]'; }, isBooleanObject: function(value) { return tag(value) === '[object Boolean]'; }, isBigIntObject: function(value) { return tag(value) === '[object BigInt]'; }, isSymbolObject: function(value) { return tag(value) === '[object Symbol]'; }, isBoxedPrimitive: function(value) { return /\\[object (Number|String|Boolean|BigInt|Symbol)\\]/.test(tag(value)); }, isAsyncFunction: function(value) { return tag(value) === '[object AsyncFunction]'; }, isGeneratorFunction: function(value) { return tag(value) === '[object GeneratorFunction]'; }, isGeneratorObject: function(value) { return tag(value) === '[object Generator]'; }, isExternal: function() { return false; }, isProxy: function() { return false; }, isModuleNamespaceObject: function() { return false; }, isKeyObject: function() { return false; }, isCryptoKey: function() { return false; } }; module.exports = types; module.exports.default = types; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}
