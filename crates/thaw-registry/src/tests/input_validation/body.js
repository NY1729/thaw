var fs = require('node:fs'), path = require('node:path'), url = require('node:url'), util = require('node:util'), qs = require('node:querystring'), r = [];
function c(f) { try { f(); r.push('ok'); } catch (e) { r.push(e.code + ':' + e.message); } }
[['a'], new Date(0), { a: 1 }, 5, null, 5n, new Map()].forEach(function (v) {
  c(function () { fs.statSync(v); }); c(function () { fs.renameSync('/nonexistent-r75-a', v); }); c(function () { fs.writeFileSync('/nonexistent-r75/x', v); }); c(function () { path.basename(v); }); c(function () { url.fileURLToPath(v); }); c(function () { util.stripVTControlCharacters(v); }); r.push(JSON.stringify(qs.parse(v)), fs.existsSync(v));
});
c(function () { fs.statSync(new Uint8Array([97])); }); c(function () { fs.statSync(new URL('http://x/a')); }); c(function () { fs.statSync('a\0b'); });
return r;
