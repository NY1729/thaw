pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "url" => Some(
            "function pathToFileURL(path) {\n\
             \x20\x20var raw = String(path), value = require('node:path').resolve(raw);\n\
             \x20\x20if (raw.charAt(raw.length - 1) === '/' && value.charAt(value.length - 1) !== '/') value += '/';\n\
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
             function queryObject(search) {\n\
             \x20\x20var result = Object.create(null);\n\
             \x20\x20new globalThis.URLSearchParams(search || '').forEach(function(value, key) { if (!Object.prototype.hasOwnProperty.call(result, key)) result[key] = value; else if (Array.isArray(result[key])) result[key].push(value); else result[key] = [result[key], value]; });\n\
             \x20\x20return result;\n\
             }\n\
             function queryString(query) {\n\
             \x20\x20var fields = [];\n\
             \x20\x20Object.keys(query).forEach(function(key) { (Array.isArray(query[key]) ? query[key] : [query[key]]).forEach(function(value) { var text = typeof value === 'string' || typeof value === 'number' || typeof value === 'bigint' || typeof value === 'boolean' ? String(value) : ''; fields.push(encodeURIComponent(key) + '=' + encodeURIComponent(text)); }); });\n\
             \x20\x20return fields.join('&');\n\
             }\n\
             function punycodeToUnicode(input) {\n\
             \x20\x20return input.split('.').map(function(label) {\n\
             \x20\x20\x20\x20if (label.slice(0, 4) !== 'xn--') return label;\n\
             \x20\x20\x20\x20var text = label.slice(4), basic = text.lastIndexOf('-'), output = basic > 0 ? text.slice(0, basic).split('').map(function(ch) { return ch.charCodeAt(0); }) : [], index = 0, n = 128, bias = 72, position = basic > 0 ? basic + 1 : 0;\n\
             \x20\x20\x20\x20function digit(code) { return code - 48 < 10 ? code - 22 : code - 65 < 26 ? code - 65 : code - 97 < 26 ? code - 97 : 36; }\n\
             \x20\x20\x20\x20function adapt(delta, count, first) { var k = 0; delta = first ? Math.floor(delta / 700) : delta >> 1; delta += Math.floor(delta / count); for (; delta > 455; k += 36) delta = Math.floor(delta / 35); return Math.floor(k + 36 * delta / (delta + 38)); }\n\
             \x20\x20\x20\x20while (position < text.length) {\n\
             \x20\x20\x20\x20\x20\x20var previous = index;\n\
             \x20\x20\x20\x20\x20\x20for (var weight = 1, k = 36; ; k += 36) { var d = digit(text.charCodeAt(position++)); index += d * weight; var t = k <= bias ? 1 : k >= bias + 26 ? 26 : k - bias; if (d < t) break; weight *= 36 - t; }\n\
             \x20\x20\x20\x20\x20\x20var length = output.length + 1; bias = adapt(index - previous, length, previous === 0); n += Math.floor(index / length); index %= length; output.splice(index++, 0, n);\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20return String.fromCodePoint.apply(null, output);\n\
             \x20\x20}).join('.');\n\
             }\n\
             function parse(input, parseQuery) {\n\
             \x20\x20var source = String(input);\n\
             \x20\x20var absolute = /^[A-Za-z][A-Za-z0-9+.-]*:/.test(source);\n\
             \x20\x20var url = new globalThis.URL(source, 'http://thaw.invalid');\n\
             \x20\x20return { protocol: absolute ? url.protocol : null, slashes: absolute && source.indexOf('//') >= 0, auth: url.username || url.password ? decodeURIComponent(url.username) + ':' + decodeURIComponent(url.password) : null, host: absolute ? url.host : null, port: absolute && url.port ? url.port : null, hostname: absolute ? url.hostname : null, hash: url.hash || null, search: url.search || null, query: parseQuery ? queryObject(url.search) : url.search ? url.search.slice(1) : null, pathname: url.pathname, path: url.pathname + url.search, href: source };\n\
             }\n\
             function format(input) {\n\
             \x20\x20if (input instanceof globalThis.URL) return input.href;\n\
             \x20\x20if (typeof input === 'string') return input;\n\
             \x20\x20var protocol = input.protocol || '';\n\
             \x20\x20if (protocol && protocol.charAt(protocol.length - 1) !== ':') protocol += ':';\n\
             \x20\x20var authority = input.host || input.hostname || '';\n\
             \x20\x20if (!input.host && input.port) authority += ':' + input.port;\n\
             \x20\x20if (input.auth) authority = encodeURI(input.auth).replace(/%3A/i, ':') + '@' + authority;\n\
             \x20\x20var slashes = (input.slashes || /^(https?|ftp|gopher|file|wss?):$/.test(protocol)) && (authority || input.slashes) ? '//' : protocol === 'file:' ? '//' : '';\n\
             \x20\x20var pathname = input.pathname || '';\n\
             \x20\x20if (slashes && authority && pathname && pathname.charAt(0) !== '/') pathname = '/' + pathname;\n\
             \x20\x20var search = input.search ? input.search : (input.query && typeof input.query === 'object' ? queryString(input.query) : typeof input.query === 'string' ? input.query : '');\n\
             \x20\x20if (search && search.charAt(0) !== '?') search = '?' + search;\n\
             \x20\x20var hash = input.hash || '';\n\
             \x20\x20if (hash && hash.charAt(0) !== '#') hash = '#' + hash;\n\
             \x20\x20return protocol + slashes + authority + pathname.replace(/[?#]/g, encodeURIComponent) + search.replace('#', '%23') + hash;\n\
             }\n\
             function resolve(from, to) { var resolved = new globalThis.URL(String(to), new globalThis.URL(String(from), 'resolve://')); return resolved.protocol === 'resolve:' ? resolved.pathname + resolved.search + resolved.hash : resolved.href; }\n\
             function domainToASCII(domain) { try { return new globalThis.URL('ws://' + domain).hostname; } catch (_) { return ''; } }\n\
             function domainToUnicode(domain) { var ascii = domainToASCII(domain); return punycodeToUnicode(ascii); }\n\
             module.exports = { URL: globalThis.URL, URLSearchParams: globalThis.URLSearchParams, resolve: resolve, domainToASCII: domainToASCII, domainToUnicode: domainToUnicode, parse: parse, format: format, pathToFileURL: pathToFileURL, fileURLToPath: fileURLToPath, urlToHttpOptions: urlToHttpOptions };\n\
             module.exports.default = module.exports;\n\
             module.exports.__esModule = true;\n",
        ),
        "querystring" => Some(
            "function escape(value) { return encodeURIComponent(String(value)); }\n\
             function unescape(value, decodeSpaces) {\n\
             \x20\x20var source = String(value);\n\
             \x20\x20try { return decodeURIComponent(source); } catch (_) {\n\
             \x20\x20\x20\x20var bytes = new Uint8Array(source.length), length = 0;\n\
             \x20\x20\x20\x20function hex(code) { return code >= 48 && code <= 57 ? code - 48 : code >= 65 && code <= 70 ? code - 55 : code >= 97 && code <= 102 ? code - 87 : -1; }\n\
             \x20\x20\x20\x20for (var index = 0; index < source.length; index++) {\n\
             \x20\x20\x20\x20\x20\x20var code = source.charCodeAt(index);\n\
             \x20\x20\x20\x20\x20\x20if (code === 37 && index + 2 < source.length) { var high = hex(source.charCodeAt(index + 1)), low = hex(source.charCodeAt(index + 2)); if (high >= 0 && low >= 0) { bytes[length++] = high * 16 + low; index += 2; continue; } }\n\
             \x20\x20\x20\x20\x20\x20bytes[length++] = code === 43 && decodeSpaces ? 32 : code;\n\
             \x20\x20\x20\x20}\n\
             \x20\x20\x20\x20return new TextDecoder('utf-8', { ignoreBOM: true }).decode(bytes.subarray(0, length));\n\
             \x20\x20}\n\
             }\n\
             function primitive(value) {\n\
             \x20\x20if (typeof value === 'number' && !Number.isFinite(value)) return '';\n\
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
             \x20\x20var decoder = options && typeof options.decodeURIComponent === 'function' ? options.decodeURIComponent : unescape, customDecoder = decoder !== unescape;\n\
             \x20\x20function decodePart(value) { var source = String(value).replace(/\\+/g, customDecoder ? '%20' : ' '); try { return decoder(source); } catch (_) { return unescape(source, true); } }\n\
             \x20\x20var maxKeys = options && options.maxKeys !== undefined ? Number(options.maxKeys) : 1000;\n\
             \x20\x20var fields = source === '' ? [] : source.split(separator);\n\
             \x20\x20if (maxKeys > 0) fields = fields.slice(0, maxKeys);\n\
             \x20\x20fields.forEach(function(field) {\n\
             \x20\x20\x20\x20if (field === '') return;\n\
             \x20\x20\x20\x20var index = field.indexOf(assignment);\n\
             \x20\x20\x20\x20var key = decodePart(index < 0 ? field : field.substring(0, index));\n\
             \x20\x20\x20\x20var value = decodePart(index < 0 ? '' : field.substring(index + assignment.length));\n\
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
        _ => None,
    }
}
