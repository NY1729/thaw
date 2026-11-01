pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "util" => Some(
            "var inspectCustom = Symbol.for('nodejs.util.inspect.custom');\n\
             function inspect(value, options) {\n\
             \x20\x20if (typeof options === 'boolean') options = { showHidden: options, depth: arguments[2], colors: arguments[3] };\n\
             \x20\x20var merged = {}, defaults = inspect.defaultOptions, given = options || {};\n\
             \x20\x20Object.keys(defaults).forEach(function(key) { merged[key] = defaults[key]; });\n\
             \x20\x20Object.keys(given).forEach(function(key) { if (given[key] !== undefined) merged[key] = given[key]; });\n\
             \x20\x20options = merged;\n\
             \x20\x20var depth = options.depth === null ? Infinity : options.depth, seen = [], refs = new Map();\n\
             \x20\x20var STYLES = { number: [33, 39], bigint: [33, 39], boolean: [33, 39], string: [32, 39], symbol: [32, 39], undefined: [90, 39], null: [1, 22], special: [36, 39], date: [35, 39], regexp: [31, 39] };\n\
             \x20\x20function paint(text, kind) { if (!options.colors) return text; var style = STYLES[kind]; return '\\u001b[' + style[0] + 'm' + text + '\\u001b[' + style[1] + 'm'; }\n\
             \x20\x20function quote(text) {\n\
             \x20\x20\x20\x20var mark = \"'\";\n\
             \x20\x20\x20\x20if (text.indexOf(\"'\") !== -1) { if (text.indexOf('\"') === -1) mark = '\"'; else if (text.indexOf('`') === -1 && text.indexOf('${') === -1) mark = '`'; }\n\
             \x20\x20\x20\x20var out = '';\n\
             \x20\x20\x20\x20for (var i = 0; i < text.length; i++) {\n\
             \x20\x20\x20\x20\x20\x20var ch = text[i], code = text.charCodeAt(i);\n\
             \x20\x20\x20\x20\x20\x20if (ch === mark || ch === '\\\\') out += '\\\\' + ch;\n\
             \x20\x20\x20\x20\x20\x20else if (ch === '\\n') out += '\\\\n'; else if (ch === '\\t') out += '\\\\t'; else if (ch === '\\r') out += '\\\\r'; else if (ch === '\\b') out += '\\\\b'; else if (ch === '\\f') out += '\\\\f';\n\
             \x20\x20\x20\x20\x20\x20else if (code < 32 || code === 127) out += '\\\\x' + (code < 16 ? '0' : '') + code.toString(16).toUpperCase();\n\
             \x20\x20\x20\x20\x20\x20else out += ch;\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20return mark + out + mark;\n\
             \x20\x20}\n\
             \x20\x20function renderString(text) {\n\
             \x20\x20\x20\x20var rest = 0;\n\
             \x20\x20\x20\x20if (text.length > options.maxStringLength) { rest = text.length - options.maxStringLength; text = text.slice(0, options.maxStringLength); }\n\
             \x20\x20\x20\x20return paint(quote(text), 'string') + (rest ? '... ' + rest + ' more character' + (rest > 1 ? 's' : '') : '');\n\
             \x20\x20}\n\
             \x20\x20function groupDigits(text) {\n\
             \x20\x20\x20\x20if (!options.numericSeparator || /[^0-9.\\-]/.test(text)) return text;\n\
             \x20\x20\x20\x20var parts = text.split('.'), integer = parts[0].replace(/\\B(?=(\\d{3})+(?!\\d))/g, '_');\n\
             \x20\x20\x20\x20return parts.length > 1 ? integer + '.' + parts[1].replace(/(\\d{3})(?=\\d)/g, '$1_') : integer;\n\
             \x20\x20}\n\
             \x20\x20function identifierKey(key) { return /^[a-zA-Z_][a-zA-Z_0-9]*$/.test(key) ? key : paint(quote(key), 'string'); }\n\
             \x20\x20function constructorName(object) {\n\
             \x20\x20\x20\x20for (var pointer = object; pointer; pointer = Object.getPrototypeOf(pointer)) {\n\
             \x20\x20\x20\x20\x20\x20var descriptor = Object.getOwnPropertyDescriptor(pointer, 'constructor');\n\
             \x20\x20\x20\x20\x20\x20if (descriptor && typeof descriptor.value === 'function' && descriptor.value.name !== '') return descriptor.value.name;\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20return null;\n\
             \x20\x20}\n\
             \x20\x20function isEmptyContainer(input) { if (Object.keys(input).length !== 0 || Object.getOwnPropertySymbols(input).length !== 0) return false; return Array.isArray(input) ? input.length === 0 : input instanceof Map || input instanceof Set ? input.size === 0 : true; }\n\
             \x20\x20function indented(value, remaining) { indentationLvl += 2; try { return render(value, remaining); } finally { indentationLvl -= 2; } }\n\
             \x20\x20function renderProperty(owner, key, remaining) {\n\
             \x20\x20\x20\x20var descriptor = Object.getOwnPropertyDescriptor(owner, key);\n\
             \x20\x20\x20\x20if (!descriptor) return '';\n\
             \x20\x20\x20\x20if (Object.prototype.hasOwnProperty.call(descriptor, 'value')) return indented(descriptor.value, remaining);\n\
             \x20\x20\x20\x20var getter = descriptor.get, setter = descriptor.set, evaluate = getter && (options.getters === true || options.getters === 'get' && !setter || options.getters === 'set' && setter);\n\
             \x20\x20\x20\x20if (evaluate) {\n\
             \x20\x20\x20\x20\x20\x20var got; try { got = getter.call(owner); } catch (_) { return paint('[Inspection threw]', 'special'); }\n\
             \x20\x20\x20\x20\x20\x20return paint(setter ? '[Getter/Setter:' : '[Getter:', 'special') + ' ' + render(got, remaining) + paint(']', 'special');\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20return paint(getter && setter ? '[Getter/Setter]' : getter ? '[Getter]' : '[Setter]', 'special');\n\
             \x20\x20}\n\
             \x20\x20function entriesOf(input, remaining, skipIndices) {\n\
             \x20\x20\x20\x20var keys = Object.keys(input).filter(function(key) { return !skipIndices || !/^(0|[1-9][0-9]*)$/.test(key); });\n\
             \x20\x20\x20\x20if (options.showHidden) Object.getOwnPropertyNames(input).forEach(function(key) { if (keys.indexOf(key) < 0 && !(skipIndices && (key === 'length' || /^(0|[1-9][0-9]*)$/.test(key))) && !(typeof input === 'function' && (key === 'length' || key === 'name' || key === 'prototype'))) keys.push(key); });\n\
             \x20\x20\x20\x20if (options.sorted) keys.sort(typeof options.sorted === 'function' ? options.sorted : undefined);\n\
             \x20\x20\x20\x20var out = keys.map(function(key) {\n\
             \x20\x20\x20\x20\x20\x20var enumerable = Object.prototype.propertyIsEnumerable.call(input, key), label = identifierKey(key);\n\
             \x20\x20\x20\x20\x20\x20return (enumerable ? label : '[' + label + ']') + ': ' + renderProperty(input, key, remaining);\n\
             \x20\x20\x20\x20});\n\
             \x20\x20\x20\x20Object.getOwnPropertySymbols(input).forEach(function(symbol) {\n\
             \x20\x20\x20\x20\x20\x20if (Object.prototype.propertyIsEnumerable.call(input, symbol)) out.push('[' + paint(String(symbol), 'symbol') + ']: ' + renderProperty(input, symbol, remaining));\n\
             \x20\x20\x20\x20});\n\
             \x20\x20\x20\x20return out;\n\
             \x20\x20}\n\
               var indentationLvl = 0, currentDepth = 0;\n\
               function isBelowBreakLength(output, start, base) {\n\
                 var total = output.length + start;\n\
                 if (total + output.length > options.breakLength) return false;\n\
                 for (var i = 0; i < output.length; i++) { total += output[i].length; if (total > options.breakLength) return false; }\n\
                 return base === '' || base.indexOf('\\n') < 0;\n\
               }\n\
               function groupArrayElements(output, value) {\n\
                 var totalLength = 0, maxLength = 0, i = 0, outputLength = output.length;\n\
                 if (options.maxArrayLength < output.length) outputLength--;\n\
                 var separatorSpace = 2, dataLen = new Array(outputLength);\n\
                 for (; i < outputLength; i++) { var len = options.colors ? output[i].replace(/\\u001b\\[\\d\\d?m/g, '').length : output[i].length; dataLen[i] = len; totalLength += len + separatorSpace; if (maxLength < len) maxLength = len; }\n\
                 var actualMax = maxLength + separatorSpace;\n\
                 if (actualMax * 3 + indentationLvl < options.breakLength && (totalLength / actualMax > 5 || maxLength <= 6)) {\n\
                   var averageBias = Math.sqrt(actualMax - totalLength / output.length), biasedMax = Math.max(actualMax - 3 - averageBias, 1);\n\
                   var columns = Math.min(Math.round(Math.sqrt(2.5 * biasedMax * outputLength) / biasedMax), Math.floor((options.breakLength - indentationLvl) / actualMax), options.compact * 4, 15);\n\
                   if (columns <= 1) return output;\n\
                   var tmp = [], maxLineLength = [];\n\
                   for (var c = 0; c < columns; c++) { var lineLength = 0; for (var j = c; j < output.length; j += columns) { if (dataLen[j] > lineLength) lineLength = dataLen[j]; } maxLineLength.push(lineLength + separatorSpace); }\n\
                   var padStart = true;\n\
                   if (value !== undefined) { for (var k = 0; k < output.length; k++) { if (typeof value[k] !== 'number' && typeof value[k] !== 'bigint') { padStart = false; break; } } }\n\
                   for (var row = 0; row < outputLength; row += columns) {\n\
                     var max = Math.min(row + columns, outputLength), str = '', col = row;\n\
                     for (; col < max - 1; col++) { var padding = maxLineLength[col - row] + output[col].length - dataLen[col], cell = output[col] + ', '; str += padStart ? cell.padStart(padding, ' ') : cell.padEnd(padding, ' '); }\n\
                     if (padStart) str += output[col].padStart(maxLineLength[col - row] + output[col].length - dataLen[col] - separatorSpace, ' '); else str += output[col];\n\
                     tmp.push(str);\n\
                   }\n\
                   if (options.maxArrayLength < output.length) tmp.push(output[outputLength]);\n\
                   output = tmp;\n\
                 }\n\
                 return output;\n\
               }\n\
               function wrap(prefix, open, items, close, level, arrayValue) {\n\
                 var brace0 = (prefix ? prefix + ' ' : '') + open;\n\
                 if (!items.length) return brace0 + close;\n\
                 var compact = options.compact;\n\
                 if (compact !== true) {\n\
                   if (typeof compact === 'number' && compact >= 1) {\n\
                     var entries = items.length, grouped = items;\n\
                     if (arrayValue !== undefined && entries > 6) grouped = groupArrayElements(grouped, arrayValue);\n\
                     if (currentDepth - level < compact && entries === grouped.length) {\n\
                       var start = grouped.length + indentationLvl + brace0.length + 10;\n\
                       if (isBelowBreakLength(grouped, start, '')) { var joined = grouped.join(', '); if (joined.indexOf('\\n') < 0) return brace0 + ' ' + joined + ' ' + close; }\n\
                     }\n\
                     items = grouped;\n\
                   }\n\
                   var indentation = '\\n' + ' '.repeat(indentationLvl);\n\
                   return brace0 + indentation + '  ' + items.join(',' + indentation + '  ') + indentation + close;\n\
                 }\n\
                 if (isBelowBreakLength(items, 0, '')) return brace0 + ' ' + items.join(', ') + ' ' + close;\n\
                 var pad = ' '.repeat(indentationLvl);\n\
                 return brace0 + (brace0.length === 1 ? ' ' : '\\n' + pad + '  ') + items.join(',\\n' + pad + '  ') + ' ' + close;\n\
               }\n\
             \x20\x20function render(input, remaining) {\n\
             \x20\x20\x20\x20if (input !== null && (typeof input === 'object' || typeof input === 'function') && options.customInspect !== false) {\n\
             \x20\x20\x20\x20\x20\x20var custom = input[inspectCustom];\n\
             \x20\x20\x20\x20\x20\x20if (typeof custom === 'function' && custom !== inspect) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20var hookOptions = Object.assign({}, options, { stylize: function(text, kind) { return paint(String(text), kind); } }); var result = custom.call(input, remaining, hookOptions, inspect);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (result !== input) return typeof result === 'string' ? result : render(result, remaining);\n\
             \x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20if (input === null) return paint('null', 'null');\n\
             \x20\x20\x20\x20if (input === undefined) return paint('undefined', 'undefined');\n\
             \x20\x20\x20\x20if (typeof input === 'bigint') return paint(groupDigits(String(input)) + 'n', 'bigint');\n\
             \x20\x20\x20\x20if (typeof input === 'number') return paint(Object.is(input, -0) ? '-0' : groupDigits(String(input)), 'number');\n\
             \x20\x20\x20\x20if (typeof input === 'boolean') return paint(String(input), 'boolean');\n\
             \x20\x20\x20\x20if (typeof input === 'string') return renderString(input);\n\
             \x20\x20\x20\x20if (typeof input === 'symbol') return paint(String(input), 'symbol');\n\
             \x20\x20\x20\x20var base = '';\n\
             \x20\x20\x20\x20if (typeof input === 'function') {\n\
             \x20\x20\x20\x20\x20\x20var source = Function.prototype.toString.call(input), kind = /^class[\\s{]/.test(source) ? 'class' : 'Function';\n\
             \x20\x20\x20\x20\x20\x20if (kind === 'class') { var parent = Object.getPrototypeOf(input); base = '[class ' + (input.name || '(anonymous)') + (parent && parent.name ? ' extends ' + parent.name : '') + ']'; }\n\
             \x20\x20\x20\x20\x20\x20else { var tag = input.constructor && input.constructor.name && input.constructor.name !== 'Function' ? input.constructor.name : 'Function'; base = '[' + tag + (input.name ? ': ' + input.name : ' (anonymous)') + ']'; }\n\
             \x20\x20\x20\x20\x20\x20base = paint(base, 'special');\n\
             \x20\x20\x20\x20\x20\x20var extra = entriesOf(input, remaining - 1, false);\n\
             \x20\x20\x20\x20\x20\x20return extra.length ? (remaining < 0 ? base : base + ' { ' + extra.join(', ') + ' }') : base;\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20if (typeof input !== 'object') return String(input);\n\
             \x20\x20\x20\x20if (input instanceof Error) { var header = input.message ? input.name + ': ' + input.message : String(input.name), frames = String(input.stack || '').split('\\n').filter(function(line) { return /^\\s+at /.test(line) && !/thaw[A-Z]|eval_script/.test(line); }); return String(input.stack || '').indexOf(header) === 0 && frames.length === String(input.stack).split('\\n').length - 1 ? input.stack : [header].concat(frames).join('\\n'); }\n\
             \x20\x20\x20\x20if (input instanceof Date) return paint(isNaN(input.getTime()) ? 'Invalid Date' : input.toISOString(), 'date');\n\
             \x20\x20\x20\x20if (input instanceof RegExp) return paint(String(input), 'regexp');\n\
             \x20\x20\x20\x20if (ArrayBuffer.isView(input) && !(input instanceof DataView)) { if (typeof Buffer !== 'undefined' && Buffer.isBuffer(input)) { var hex = []; for (var b = 0; b < input.length && b < 50; b++) hex.push((input[b] < 16 ? '0' : '') + input[b].toString(16)); return '<Buffer' + (hex.length ? ' ' + hex.join(' ') : '') + (input.length > 50 ? ' ... ' + (input.length - 50) + ' more byte' + (input.length - 50 > 1 ? 's' : '') : '') + '>'; } var typedItems = [], typedMax = Math.min(input.length, options.maxArrayLength); for (var t = 0; t < typedMax; t++) typedItems.push(render(input[t], remaining - 1)); if (input.length > typedMax) typedItems.push('... ' + (input.length - typedMax) + ' more item' + (input.length - typedMax > 1 ? 's' : '')); if (options.showHidden) typedItems.push('[BYTES_PER_ELEMENT]: ' + input.BYTES_PER_ELEMENT, '[length]: ' + input.length, '[byteLength]: ' + input.byteLength, '[byteOffset]: ' + input.byteOffset, '[buffer]: ' + render(input.buffer, remaining - 1)); return input[Symbol.toStringTag] + '(' + input.length + ') ' + (typedItems.length ? '[ ' + typedItems.join(', ') + ' ]' : '[]'); }\n\
             \x20\x20\x20\x20if (remaining < 0 && !(Array.isArray(input) ? input.length === 0 && Object.keys(input).length === 0 : Object.getPrototypeOf(input) === Object.prototype && Object.keys(input).length === 0 && Object.getOwnPropertySymbols(input).length === 0)) { var shortName = constructorName(input); return paint('[' + (Array.isArray(input) ? 'Array' : input instanceof Map ? 'Map' : input instanceof Set ? 'Set' : shortName || 'Object') + ']', 'special'); }\n\
             \x20\x20\x20\x20if (seen.indexOf(input) >= 0) { if (!refs.has(input)) refs.set(input, refs.size + 1); return paint('[Circular *' + refs.get(input) + ']', 'special'); }\n\
             \x20\x20\x20\x20seen.push(input);\n\
             \x20\x20\x20\x20if (!isEmptyContainer(input)) currentDepth = seen.length - 1;\n\
             \x20\x20\x20\x20try {\n\
             \x20\x20\x20\x20\x20\x20var name = constructorName(input), nullProto = Object.getPrototypeOf(input) === null, result;\n\
             \x20\x20\x20\x20\x20\x20if (Array.isArray(input)) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20var items = [], holes = 0, shown = 0, length = input.length;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20for (var index = 0; index < length && shown < options.maxArrayLength; index++) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if (!Object.prototype.hasOwnProperty.call(input, index)) { holes++; continue; }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20if (holes) { items.push(paint('<' + holes + ' empty item' + (holes > 1 ? 's' : '') + '>', 'undefined')); shown++; holes = 0; if (shown >= options.maxArrayLength) { index--; continue; } }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20items.push(renderProperty(input, String(index), remaining - 1)); shown++;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (holes && shown < options.maxArrayLength) { items.push(paint('<' + holes + ' empty item' + (holes > 1 ? 's' : '') + '>', 'undefined')); index = length; }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (index < length) { var more = length - index; items.push('... ' + more + ' more item' + (more > 1 ? 's' : '')); }\n\
             \x20\x20\x20\x20\x20\x20\x20\x20items = items.concat(entriesOf(input, remaining - 1, true));\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (options.showHidden) items.push('[length]: ' + length);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20result = wrap(name && name !== 'Array' ? name + '(' + length + ')' : '', '[', items, ']', seen.length - 1, input);\n\
             \x20\x20\x20\x20\x20\x20} else if (input instanceof Map) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20result = wrap('Map(' + input.size + ')', '{', Array.from(input).map(function(entry) { return indented(entry[0], remaining - 1) + ' => ' + indented(entry[1], remaining - 1); }).concat(entriesOf(input, remaining - 1, false)), '}', seen.length - 1);\n\
             \x20\x20\x20\x20\x20\x20} else if (input instanceof Set) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20result = wrap('Set(' + input.size + ')', '{', Array.from(input).map(function(item) { return indented(item, remaining - 1); }).concat(entriesOf(input, remaining - 1, false)), '}', seen.length - 1);\n\
             \x20\x20\x20\x20\x20\x20} else {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20var tagName = input[Symbol.toStringTag], label = nullProto ? '[Object: null prototype]' : name && name !== 'Object' ? name : '';\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (typeof tagName === 'string' && tagName !== '' && tagName !== name && !(options.showHidden ? Object.prototype.hasOwnProperty : Object.prototype.propertyIsEnumerable).call(input, Symbol.toStringTag)) label = (label || 'Object') + ' [' + tagName + ']';\n\
             \x20\x20\x20\x20\x20\x20\x20\x20result = wrap(label, '{', entriesOf(input, remaining - 1, false), '}', seen.length - 1);\n\
             \x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20if (refs.has(input)) result = '<ref *' + refs.get(input) + '> ' + result;\n\
             \x20\x20\x20\x20\x20\x20return result;\n\
             \x20\x20\x20\x20} finally { seen.pop(); }\n\
             \x20\x20}\n\
             \x20\x20return render(value, depth);\n\
             }\n\
             inspect.custom = inspectCustom; inspect.defaultOptions = { showHidden: false, depth: 2, colors: false, customInspect: true, showProxy: false, maxArrayLength: 100, maxStringLength: 10000, breakLength: 80, compact: 3, sorted: false, getters: false, numericSeparator: false };\n\
             function fmtNumber(n) { return Object.is(n, -0) ? '-0' : String(n); }\n\
             function hasBuiltInToString(value) {\n\
             \x20\x20if (typeof value.toString !== 'function') return true;\n\
             \x20\x20if (Object.prototype.hasOwnProperty.call(value, 'toString')) return false;\n\
             \x20\x20var pointer = value; do { pointer = Object.getPrototypeOf(pointer); } while (pointer && !Object.prototype.hasOwnProperty.call(pointer, 'toString'));\n\
             \x20\x20if (!pointer) return true; var descriptor = Object.getOwnPropertyDescriptor(pointer, 'constructor');\n\
             \x20\x20return descriptor !== undefined && typeof descriptor.value === 'function' && globalThis[descriptor.value.name] === descriptor.value;\n\
             }\n\
             function format() {\n\
             \x20\x20var args = Array.prototype.slice.call(arguments), first = args[0], a = 0, str = '', join = '';\n\
             \x20\x20if (typeof first === 'string') {\n\
             \x20\x20\x20\x20if (args.length === 1) return first;\n\
             \x20\x20\x20\x20var lastPos = 0, tempStr, tempArg;\n\
             \x20\x20\x20\x20for (var i = 0; i < first.length - 1; i++) {\n\
             \x20\x20\x20\x20\x20\x20if (first.charCodeAt(i) !== 37) continue;\n\
             \x20\x20\x20\x20\x20\x20var next = first.charCodeAt(++i);\n\
             \x20\x20\x20\x20\x20\x20if (a + 1 !== args.length) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20switch (next) {\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 115: tempArg = args[++a]; tempStr = typeof tempArg === 'number' ? fmtNumber(tempArg) : typeof tempArg === 'bigint' ? String(tempArg) + 'n' : (typeof tempArg !== 'object' || tempArg === null || !hasBuiltInToString(tempArg)) ? String(tempArg) : inspect(tempArg, { depth: 0 }); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 106: tempArg = args[++a]; try { tempStr = JSON.stringify(tempArg); } catch (_) { tempStr = '[Circular]'; } break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 100: tempArg = args[++a]; tempStr = typeof tempArg === 'bigint' ? String(tempArg) + 'n' : typeof tempArg === 'symbol' ? 'NaN' : fmtNumber(Number(tempArg)); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 79: tempStr = inspect(args[++a]); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 111: tempStr = inspect(args[++a], { showHidden: true, depth: 4 }); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 105: tempArg = args[++a]; tempStr = typeof tempArg === 'bigint' ? String(tempArg) + 'n' : typeof tempArg === 'symbol' ? 'NaN' : fmtNumber(parseInt(tempArg)); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 102: tempArg = args[++a]; tempStr = typeof tempArg === 'symbol' ? 'NaN' : fmtNumber(parseFloat(tempArg)); break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 99: a += 1; tempStr = ''; break;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20case 37: str += first.slice(lastPos, i); lastPos = i + 1; continue;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20default: continue;\n\
             \x20\x20\x20\x20\x20\x20\x20\x20}\n\
             \x20\x20\x20\x20\x20\x20\x20\x20if (lastPos !== i - 1) str += first.slice(lastPos, i - 1);\n\
             \x20\x20\x20\x20\x20\x20\x20\x20str += tempStr; lastPos = i + 1;\n\
             \x20\x20\x20\x20\x20\x20} else if (next === 37) { str += first.slice(lastPos, i); lastPos = i + 1; }\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20if (lastPos !== 0) { a++; join = ' '; if (lastPos < first.length) str += first.slice(lastPos); }\n\
             \x20\x20}\n\
             \x20\x20while (a < args.length) { var value = args[a]; str += join; str += typeof value !== 'string' ? inspect(value) : value; join = ' '; a++; }\n\
             \x20\x20return str;\n\
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
             \x20\x20var args = Array.from(config.args === undefined ? (process.argv || []).slice(2) : config.args, String), definitions = config.options || {}, values = Object.create(null), positionals = [], tokens = [], short = Object.create(null), allowPositionals = config.allowPositionals === undefined ? config.strict === false : config.allowPositionals;\n\
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
             \x20\x20function addPositional(value, index) { if (!allowPositionals) { var error = new TypeError('Unexpected argument ' + value); error.code = 'ERR_PARSE_ARGS_UNEXPECTED_POSITIONAL'; throw error; } positionals.push(value); tokens.push({ kind: 'positional', index: index, value: value }); }\n\
             \x20\x20for (var index = 0; index < args.length; index++) {\n\
             \x20\x20\x20\x20var argument = args[index];\n\
             \x20\x20\x20\x20if (argument === '--') { tokens.push({ kind: 'option-terminator', index: index }); for (index++; index < args.length; index++) addPositional(args[index], index); break; }\n\
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
             \x20\x20\x20\x20} else addPositional(argument, index);\n\
             \x20\x20}\n\
             \x20\x20Object.keys(definitions).forEach(function(name) { if (!Object.prototype.hasOwnProperty.call(values, name)) { var definition = definitions[name] || {}, defaultValue = definition.default; if (defaultValue !== undefined) values[name] = definition.multiple ? Array.from(defaultValue) : defaultValue; } });\n\
             \x20\x20var result = { values: values, positionals: positionals }; if (config.tokens) result.tokens = tokens; return result;\n\
             }\n\
             function strictDeep(actual, expected, seen) {\n\
             if (Object.is(actual, expected)) return true;\n\
             if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;\n\
             if (Object.getPrototypeOf(actual) !== Object.getPrototypeOf(expected)) return false;\n\
             seen = seen || new Map(); if (seen.get(actual) === expected) return true; seen.set(actual, expected);\n\
             if (actual instanceof Date) return expected instanceof Date && actual.getTime() === expected.getTime();\n\
             if (actual instanceof RegExp) return expected instanceof RegExp && actual.source === expected.source && actual.flags === expected.flags;\n\
             if ((actual instanceof Number || actual instanceof String || actual instanceof Boolean) && !Object.is(actual.valueOf(), expected.valueOf())) return false;\n\
             if (actual instanceof Error) { if (actual.name !== expected.name || actual.message !== expected.message || ('cause' in actual) !== ('cause' in expected) || !strictDeep(actual.cause, expected.cause, seen) || !strictDeep(actual.errors, expected.errors, seen)) return false; }\n\
             if (Array.isArray(actual) && actual.length !== expected.length) return false;\n\
             if (actual instanceof ArrayBuffer || actual instanceof DataView) { if (actual.byteLength !== expected.byteLength) return false; var left = actual instanceof DataView ? new Uint8Array(actual.buffer, actual.byteOffset, actual.byteLength) : new Uint8Array(actual), right = expected instanceof DataView ? new Uint8Array(expected.buffer, expected.byteOffset, expected.byteLength) : new Uint8Array(expected); for (var at = 0; at < left.length; at++) if (left[at] !== right[at]) return false; return true; }\n\
             if (ArrayBuffer.isView(actual)) { if (!ArrayBuffer.isView(expected) || actual.length !== expected.length) return false; for (var i = 0; i < actual.length; i++) if (!Object.is(actual[i], expected[i])) return false; return true; }\n\
             if (actual instanceof Map) { if (actual.size !== expected.size) return false; var usedKeys = new Set(); for (var entry of actual) { if (expected.has(entry[0]) && strictDeep(entry[1], expected.get(entry[0]), seen)) { usedKeys.add(entry[0]); continue; } var found = false; for (var other of expected) { if (usedKeys.has(other[0])) continue; if (strictDeep(entry[0], other[0], seen) && strictDeep(entry[1], other[1], seen)) { usedKeys.add(other[0]); found = true; break; } } if (!found) return false; } }\n\
             if (actual instanceof Set) { if (actual.size !== expected.size) return false; var usedValues = new Set(); for (var item of actual) { if (expected.has(item)) { usedValues.add(item); continue; } var matched = false; for (var candidate of expected) { if (usedValues.has(candidate)) continue; if (strictDeep(item, candidate, seen)) { usedValues.add(candidate); matched = true; break; } } if (!matched) return false; } }\n\
             var left = Object.keys(actual); var right = Object.keys(expected); if (left.length !== right.length) return false;\n\
             for (var index = 0; index < left.length; index++) { var key = left[index]; if (!Object.prototype.hasOwnProperty.call(expected, key) || !strictDeep(actual[key], expected[key], seen)) return false; }\n\
             return true;\n\
             }\n\
             function isDeepStrictEqual(left, right) { return strictDeep(left, right); }\n\
             var styleCodes = { reset: [0, 0], bold: [1, 22], dim: [2, 22], italic: [3, 23], underline: [4, 24], blink: [5, 25], inverse: [7, 27], hidden: [8, 28], strikethrough: [9, 29], doubleunderline: [21, 24], black: [30, 39], red: [31, 39], green: [32, 39], yellow: [33, 39], blue: [34, 39], magenta: [35, 39], cyan: [36, 39], white: [37, 39], gray: [90, 39], grey: [90, 39], blackBright: [90, 39], redBright: [91, 39], greenBright: [92, 39], yellowBright: [93, 39], blueBright: [94, 39], magentaBright: [95, 39], cyanBright: [96, 39], whiteBright: [97, 39], bgBlack: [40, 49], bgRed: [41, 49], bgGreen: [42, 49], bgYellow: [43, 49], bgBlue: [44, 49], bgMagenta: [45, 49], bgCyan: [46, 49], bgWhite: [47, 49], bgGray: [100, 49], bgGrey: [100, 49], bgBlackBright: [100, 49], bgRedBright: [101, 49], bgGreenBright: [102, 49], bgYellowBright: [103, 49], bgBlueBright: [104, 49], bgMagentaBright: [105, 49], bgCyanBright: [106, 49], bgWhiteBright: [107, 49] };\n\
             function styleText(format, text, options) {\n\
             \x20\x20if (typeof text !== 'string') { var textError = new TypeError('The \"text\" argument must be of type string. Received ' + (text === null ? 'null' : typeof text === 'object' ? 'an instance of ' + (text.constructor && text.constructor.name) : 'type ' + typeof text + ' (' + String(text) + ')')); textError.code = 'ERR_INVALID_ARG_TYPE'; throw textError; }\n\
             \x20\x20var names = Array.isArray(format) ? format : [format], left = '', right = '';\n\
             \x20\x20names.forEach(function(name) { var code = styleCodes[name]; if (!code) { var formatError = new TypeError('The argument \\'format\\' must be one of: ' + Object.keys(styleCodes).map(function(key) { return \"'\" + key + \"'\"; }).join(', ') + '. Received ' + format); formatError.code = 'ERR_INVALID_ARG_VALUE'; throw formatError; } left += '\\u001b[' + code[0] + 'm'; right = '\\u001b[' + code[1] + 'm' + right; });\n\
             \x20\x20var stream = options && options.stream !== undefined ? options.stream : process.stdout;\n\
             \x20\x20if (!(options && options.validateStream === false) && !(stream && stream.isTTY && (typeof stream.hasColors !== 'function' || stream.hasColors()))) return text;\n\
             \x20\x20return left + text + right;\n\
             }\n\
             var systemErrorNames = {'-1':'EPERM','-2':'ENOENT','-3':'ESRCH','-4':'EINTR','-5':'EIO','-6':'ENXIO','-7':'E2BIG','-8':'ENOEXEC','-9':'EBADF','-10':'ECHILD','-11':'EWOULDBLOCK','-12':'ENOMEM','-13':'EACCES','-14':'EFAULT','-16':'EBUSY','-17':'EEXIST','-18':'EXDEV','-19':'ENODEV','-20':'ENOTDIR','-21':'EISDIR','-22':'EINVAL','-23':'ENFILE','-24':'EMFILE','-25':'ENOTTY','-26':'ETXTBSY','-27':'EFBIG','-28':'ENOSPC','-29':'ESPIPE','-30':'EROFS','-31':'EMLINK','-32':'EPIPE','-33':'EDOM','-34':'ERANGE','-35':'EDEADLK','-36':'ENAMETOOLONG','-37':'ENOLCK','-38':'ENOSYS','-39':'ENOTEMPTY','-40':'ELOOP','-42':'ENOMSG','-43':'EIDRM','-60':'ENOSTR','-61':'ENODATA','-62':'ETIME','-63':'ENOSR','-67':'ENOLINK','-71':'EPROTO','-72':'EMULTIHOP','-74':'EBADMSG','-75':'EOVERFLOW','-84':'EILSEQ','-88':'ENOTSOCK','-89':'EDESTADDRREQ','-90':'EMSGSIZE','-91':'EPROTOTYPE','-92':'ENOPROTOOPT','-93':'EPROTONOSUPPORT','-95':'EOPNOTSUPP','-97':'EAFNOSUPPORT','-98':'EADDRINUSE','-99':'EADDRNOTAVAIL','-100':'ENETDOWN','-101':'ENETUNREACH','-102':'ENETRESET','-103':'ECONNABORTED','-104':'ECONNRESET','-105':'ENOBUFS','-106':'EISCONN','-107':'ENOTCONN','-110':'ETIMEDOUT','-111':'ECONNREFUSED','-113':'EHOSTUNREACH','-114':'EALREADY','-115':'EINPROGRESS','-116':'ESTALE','-122':'EDQUOT','-125':'ECANCELED'}; function getSystemErrorName(code) { return systemErrorNames[code] || 'Unknown system error ' + code; }\n\
             function isArray(value) { return Array.isArray(value); }\n\
             var types = globalThis.__thaw_util_types;\n\
             module.exports = { inspect: inspect, format: format, formatWithOptions: formatWithOptions, inherits: inherits, promisify: promisify, callbackify: callbackify, deprecate: deprecate, debuglog: debuglog, isDeepStrictEqual: isDeepStrictEqual, styleText: styleText, getSystemErrorName: getSystemErrorName, isArray: isArray, stripVTControlCharacters: stripVTControlCharacters, toUSVString: toUSVString, parseArgs: parseArgs, TextEncoder: globalThis.TextEncoder, TextDecoder: globalThis.TextDecoder, types: types };\n\
             module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "util/types" => Some(
            "var types = globalThis.__thaw_util_types; module.exports = types; module.exports.default = types; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}
