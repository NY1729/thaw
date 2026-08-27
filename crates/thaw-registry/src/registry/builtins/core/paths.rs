pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
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
        _ => None,
    }
}
