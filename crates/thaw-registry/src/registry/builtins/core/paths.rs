pub(super) fn source(name: &str) -> Option<&'static str> {
    match name {
        "path" => Some(r#"
function posixNormalize(value) {
  var path = String(value); if (path === '') return '.';
  var absolute = path.charAt(0) === '/', trailing = path.length > 1 && path.charAt(path.length - 1) === '/';
  var input = path.split('/'), output = [];
  for (var i = 0; i < input.length; i++) {
    var part = input[i]; if (!part || part === '.') continue;
    if (part === '..') { if (output.length && output[output.length - 1] !== '..') output.pop(); else if (!absolute) output.push('..'); }
    else output.push(part);
  }
  var result = (absolute ? '/' : '') + output.join('/'); if (!result) result = absolute ? '/' : '.';
  if (trailing && result !== '/') result += '/'; return result;
}
function posixResolve() {
  var args = Array.prototype.map.call(arguments, String), path = '', absolute = false;
  for (var i = args.length - 1; i >= 0 && !absolute; i--) { var part = args[i]; if (!part) continue; path = part + '/' + path; absolute = part.charAt(0) === '/'; }
  if (!absolute) path = (globalThis.process && process.cwd ? process.cwd() : '/') + '/' + path;
  path = posixNormalize(path); return path.length > 1 && path.charAt(path.length - 1) === '/' ? path.slice(0, -1) : path;
}
function posixJoin() {
  var parts = []; for (var i = 0; i < arguments.length; i++) if (arguments[i] !== '') parts.push(arguments[i]);
  return posixNormalize(Array.prototype.join.call(parts, '/'));
}
function posixLex(path) {
  path = String(path); var end = path.length; while (end > 1 && path.charAt(end - 1) === '/') end--;
  var root = path.charAt(0) === '/' ? '/' : '', sep = path.lastIndexOf('/', end - 1), base = path.substring(sep + 1, end);
  var dir = sep < 0 ? '' : sep === 0 ? '/' : sep === 1 && path.charAt(0) === '/' && path.charAt(1) === '/' ? '//' : path.substring(0, sep);
  if (!base && root) dir = path.substring(0, end);
  return { root: root, dir: dir, base: base };
}
function posixDirname(path) { var lex = posixLex(path); return lex.dir || '.'; }
function posixBasename(path, suffix) { path = String(path); var base = posixLex(path).base; if (suffix) { suffix = String(suffix); if (suffix === path) base = ''; else if (suffix && base.length > suffix.length && base.endsWith(suffix)) base = base.slice(0, -suffix.length); } return base; }
function extension(base) { if (base === '..' || base === '.' || !base) return ''; var dot = base.lastIndexOf('.'); return dot <= 0 ? '' : base.slice(dot); }
function posixExtname(path) { return extension(posixLex(path).base); }
function posixParse(path) { path = String(path); var lex = posixLex(path), dir = lex.dir === '//' && path.slice(0, 2) === '//' && path.charAt(2) !== '/' ? '/' : lex.dir, ext = lex.root && lex.dir === lex.root && lex.base === '..' ? '.' : extension(lex.base); return { root: lex.root, dir: dir, base: lex.base, ext: ext, name: ext ? lex.base.slice(0, -ext.length) : lex.base }; }
function formatPath(value, separator) {
  var dir = value.dir || value.root || '', base = value.base;
  if (!base) { var name = String(value.name || ''), ext = String(value.ext || ''); if (ext && ext.charAt(0) !== '.') ext = '.' + ext; base = name + ext; }
  return dir ? (dir === value.root ? dir + base : dir + separator + base) : base;
}
function posixIsAbsolute(path) { return String(path).charAt(0) === '/'; }
function posixRelative(from, to) { var left = posixResolve(from).split('/').filter(Boolean), right = posixResolve(to).split('/').filter(Boolean), shared = 0; while (shared < left.length && shared < right.length && left[shared] === right[shared]) shared++; return left.slice(shared).map(function() { return '..'; }).concat(right.slice(shared)).join('/'); }
function toNamespacedPath(path) { return path; }
var posix = { resolve: posixResolve, join: posixJoin, dirname: posixDirname, basename: posixBasename, extname: posixExtname, normalize: posixNormalize, relative: posixRelative, isAbsolute: posixIsAbsolute, parse: posixParse, format: function(value) { return formatPath(value, '/'); }, toNamespacedPath: toNamespacedPath, sep: '/', delimiter: ':' };
function isWinSep(char) { return char === '/' || char === '\\'; }
function winScan(path) {
  path = String(path); var normalized = path.replace(/\\/g, '/'), drive = /^([A-Za-z]:)/.exec(normalized);
  if (drive) { var absolute = normalized.charAt(2) === '/', rootEnd = absolute ? 3 : 2; return { device: drive[1], root: normalized.slice(0, rootEnd), rootEnd: rootEnd, absolute: absolute, unc: false, tail: normalized.slice(rootEnd) }; }
  if (normalized.slice(0, 2) === '//') {
    var serverStart = 2; while (normalized.charAt(serverStart) === '/') serverStart++;
    var serverEnd = normalized.indexOf('/', serverStart);
    if (serverEnd > serverStart) {
      var shareStart = serverEnd; while (normalized.charAt(shareStart) === '/') shareStart++;
      var shareEnd = normalized.indexOf('/', shareStart);
      if (shareEnd < 0) shareEnd = normalized.length;
      if (shareEnd > shareStart && serverStart === 2) {
        var afterRoot = shareEnd; if (normalized.charAt(afterRoot) === '/') afterRoot++;
        var device = '//' + normalized.slice(serverStart, serverEnd) + '/' + normalized.slice(shareStart, shareEnd);
        return { device: device, root: device + '/', rootEnd: shareEnd + (normalized.charAt(shareEnd) === '/' ? 1 : 0), absolute: true, unc: true, share: normalized.slice(shareStart, shareEnd), serverEnd: serverEnd, shareEnd: shareEnd, tail: normalized.slice(afterRoot) };
      }
    }
  }
  var rooted = normalized.charAt(0) === '/'; var rootEnd = 0; if (rooted) { rootEnd = 1; while (normalized.charAt(rootEnd) === '/') rootEnd++; }
  return { device: '', root: rooted ? '/' : '', rootEnd: rootEnd, absolute: rooted, unc: false, tail: normalized.slice(rootEnd) };
}
function winTailNormalize(tail, absolute) {
  var parts = tail.split('/'), out = [];
  for (var i = 0; i < parts.length; i++) { var part = parts[i]; if (!part || part === '.') continue; if (part === '..') { if (out.length && out[out.length - 1] !== '..') out.pop(); else if (!absolute) out.push('..'); } else out.push(part); }
  return out;
}
function winNormalize(path) {
  path = String(path); if (!path) return '.';
  var root = winScan(path), trailing = /[\\/]$/.test(path), parts = winTailNormalize(root.tail, root.absolute), tail = parts.join('/'), result;
  if (root.root) {
    if (root.device && !root.unc && !root.absolute) result = root.device + (tail || '.');
    else result = root.root + tail;
  } else result = tail || '.';
  if (trailing && (result !== '.' || path !== '') && !/[\/]$/.test(result)) result += '/';
  return result.replace(/\//g, '\\');
}
function winOutput(path) { return String(path).replace(/\//g, '\\'); }
function winJoin() {
  var parts = []; for (var i = 0; i < arguments.length; i++) { var part = String(arguments[i]); if (part !== '') parts.push(part); }
  if (!parts.length) return '.';
  var preserveUnc = /^[\\/]{2}[^\\/]/.test(parts[0]), joined = parts.join('/').replace(/\\/g, '/');
  if (!preserveUnc) joined = joined.replace(/^\/{2,}/, '/');
  return winNormalize(joined);
}
function winResolve() {
  var args = Array.prototype.map.call(arguments, String), device = '', absolute = false, tail = '';
  for (var i = args.length - 1; i >= -1 && !(absolute && device); i--) {
    var value = i >= 0 ? args[i] : '';
    if (i < 0 || !value) {
      if (i < 0) {
        var cwd;
        if (device) {
          var driveCwd = globalThis.process && process.env ? process.env['=' + device] : undefined;
          cwd = driveCwd || (globalThis.process && process.cwd ? process.cwd() : '/');
          if (cwd === undefined || (String(cwd).slice(0, 2).toLowerCase() !== device.toLowerCase() && String(cwd).charAt(2) === '\\')) cwd = device + '\\';
        } else cwd = globalThis.process && process.cwd ? process.cwd() : '/';
        var current = winScan(String(cwd));
        if (current.device && !device) device = current.device;
        if (!absolute && (!current.device || current.device.toLowerCase() === device.toLowerCase())) { tail = current.tail + '/' + tail; absolute = current.absolute; }
      }
      continue;
    }
    var parsed = winScan(value);
    if (parsed.device) { if (device && device.toLowerCase() !== parsed.device.toLowerCase()) continue; if (!device) device = parsed.device; }
    if (parsed.absolute) { if (!absolute) { tail = parsed.tail + '/' + tail; absolute = true; } }
    else if (!absolute) tail = parsed.tail + '/' + tail;
  }
  var normalized = winTailNormalize(tail, absolute), root = device ? (device.slice(0, 2) === '//' ? device + '/' : device + (absolute ? '/' : '')) : (absolute ? '/' : '');
  var result = root + normalized.join('/'); if (!result) result = '.';
  return winOutput(result);
}
function winLex(path) {
  path = String(path); var normalized = path.replace(/\\/g, '/'), scan = winScan(path), rootEnd = scan.unc || scan.device ? scan.rootEnd : scan.absolute ? 1 : 0, end = normalized.length;
  while (end > rootEnd && normalized.charAt(end - 1) === '/') end--;
  if (scan.unc && end <= scan.shareEnd) return { scan: scan, end: end, baseStart: end, base: '', root: rawWinRoot(path, scan), dir: rawWinRoot(path, scan) };
  var sep = normalized.lastIndexOf('/', end - 1), baseStart = Math.max(sep + 1, rootEnd), base = normalized.slice(baseStart, end);
  var dir = sep < 0 ? (scan.device ? path.slice(0, 2) : '') : sep < rootEnd ? rawWinRoot(path, scan) : path.slice(0, sep);
  var root = rawWinRoot(path, scan); if (!base && root) dir = root;
  return { scan: scan, end: end, baseStart: baseStart, base: base, root: root, dir: dir };
}
function rawWinRoot(path, scan) {
  if (!scan.root) return '';
  var end = scan.rootEnd;
  if (!scan.device && !scan.unc) end = Math.min(end, 1);
  return path.slice(0, end);
}
function winDirname(path) { var lex = winLex(path); return lex.dir || '.'; }
function winBasename(path, suffix) {
  path = String(path); var lex = winLex(path), scan = lex.scan, base = lex.base;
  if (scan.unc && lex.end <= scan.shareEnd + 1) base = scan.share;
  if (suffix) { suffix = String(suffix); if (suffix === path) base = ''; else if (suffix && base.length > suffix.length && base.endsWith(suffix)) base = base.slice(0, -suffix.length); } return base;
}
function winExtname(path) { var lex = winLex(path); return lex.scan.unc && lex.end <= lex.scan.shareEnd + 1 ? extension(lex.scan.share) : extension(lex.base); }
function winParse(path) { var lex = winLex(path), ext = extension(lex.base); return { root: lex.root, dir: lex.dir, base: lex.base, ext: ext, name: ext ? lex.base.slice(0, -ext.length) : lex.base }; }
function winIsAbsolute(path) { path = String(path); return isWinSep(path.charAt(0)) || /^[A-Za-z]:[\\/]/.test(path); }
/*
 * Adapted from Node.js v22.20.0 lib/path.js.
 * Copyright Joyent, Inc. and other Node contributors.
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to
 * deal in the Software without restriction, including without limitation the
 * rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
 * sell copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
 * FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
 * DEALINGS IN THE SOFTWARE.
 */
function winRelative(from, to) {
  var fromPath = winResolve(from), toPath = winResolve(to);
  if (fromPath === toPath) return '';
  var fromLower = fromPath.toLowerCase(), toLower = toPath.toLowerCase();
  if (fromLower === toLower) return '';
  if (fromPath.length !== fromLower.length || toPath.length !== toLower.length) {
    var fromParts = fromPath.split('\\'), toParts = toPath.split('\\');
    if (fromParts[fromParts.length - 1] === '') fromParts.pop();
    if (toParts[toParts.length - 1] === '') toParts.pop();
    var count = Math.min(fromParts.length, toParts.length), shared = 0;
    while (shared < count && fromParts[shared].toLowerCase() === toParts[shared].toLowerCase()) shared++;
    if (shared === 0) return toPath;
    if (shared === count) {
      if (toParts.length > count) return toParts.slice(shared).join('\\');
      if (fromParts.length > count) return '..\\'.repeat(fromParts.length - 1 - shared) + '..';
      return '';
    }
    return '..\\'.repeat(fromParts.length - shared) + toParts.slice(shared).join('\\');
  }

  var fromStart = 0, fromEnd = fromLower.length;
  while (fromStart < fromEnd && fromLower.charAt(fromStart) === '\\') fromStart++;
  while (fromEnd - 1 > fromStart && fromLower.charAt(fromEnd - 1) === '\\') fromEnd--;
  var toStart = 0, toEnd = toLower.length;
  while (toStart < toEnd && toLower.charAt(toStart) === '\\') toStart++;
  while (toEnd - 1 > toStart && toLower.charAt(toEnd - 1) === '\\') toEnd--;
  var fromLength = fromEnd - fromStart, toLength = toEnd - toStart;
  var length = Math.min(fromLength, toLength), lastCommonSep = -1, i = 0;
  for (; i < length; i++) {
    var fromChar = fromLower.charAt(fromStart + i);
    if (fromChar !== toLower.charAt(toStart + i)) break;
    if (fromChar === '\\') lastCommonSep = i;
  }
  if (i !== length) {
    if (lastCommonSep === -1) return toPath;
  } else {
    if (toLength > length) {
      if (toLower.charAt(toStart + i) === '\\') return toPath.slice(toStart + i + 1);
      if (i === 2) return toPath.slice(toStart + i);
    }
    if (fromLength > length) {
      if (fromLower.charAt(fromStart + i) === '\\') lastCommonSep = i;
      else if (i === 2) lastCommonSep = 3;
    }
    if (lastCommonSep === -1) lastCommonSep = 0;
  }
  var out = '';
  for (i = fromStart + lastCommonSep + 1; i <= fromEnd; i++) {
    if (i === fromEnd || fromLower.charAt(i) === '\\') out += out ? '\\..' : '..';
  }
  toStart += lastCommonSep;
  if (out) return out + toPath.slice(toStart, toEnd);
  if (toPath.charAt(toStart) === '\\') toStart++;
  return toPath.slice(toStart, toEnd);
}
var win32 = { resolve: winResolve, join: winJoin, dirname: winDirname, basename: winBasename, extname: winExtname, normalize: winNormalize, relative: winRelative, isAbsolute: winIsAbsolute, parse: winParse, format: function(value) { return String(formatPath(value, '\\')); }, toNamespacedPath: toNamespacedPath, sep: '\\', delimiter: ';' };
posix.posix = posix; posix.win32 = win32; win32.posix = posix; win32.win32 = win32;
module.exports = posix; module.exports.default = module.exports; module.exports.__esModule = true;
"#),
        "path/posix" => Some(
            "module.exports = require('node:path').posix; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        "path/win32" => Some(
            "module.exports = require('node:path').win32; module.exports.default = module.exports; module.exports.__esModule = true;\n",
        ),
        _ => None,
    }
}
