#!/usr/bin/env node
// Enumerates the export surface of every module in tools/parity-aot/modules.txt under Node 22 (the thaw-js
// builtin allowlist). Prints TSV: module, export, kind, arity, vtype, alias.
// Usage: node tools/parity-aot/module_inv.js [OUT_FILE]   (default: stdout)
// Kinds: function | class | constant (UPPER_SNAKE primitive) | primitive | object | accessor.
// Accessors are never invoked. arity = Function.length ('-' for non-functions).
// alias = "module:export" of the first export (in sorted order) holding the same object, else '-'.
'use strict';
const fs = require('fs');
const path = require('path');

if (process.versions.node.split('.')[0] !== '22') {
  console.error(`module_inv.js: needs Node 22 (got ${process.versions.node}); the table is pinned to it`);
  process.exit(2);
}

const HERE = __dirname;
const HEADER = ['module', 'export', 'kind', 'arity', 'vtype', 'alias'];
const UPPER = /^[A-Z][A-Z0-9_]*$/;
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

function keyName(k) {
  if (typeof k === 'string') return k;
  const d = k.description;
  if (d === undefined) return '@@';
  return '@@' + (d.startsWith('Symbol.') ? d.slice(7) : d);
}

function isClass(f) {
  if (/^class[\s{]/.test(Function.prototype.toString.call(f))) return true;
  const p = f.prototype;
  return p !== null && typeof p === 'object' && Reflect.ownKeys(p).some((k) => k !== 'constructor');
}

function kindOf(key, v) {
  if (typeof v === 'function') return isClass(v) ? 'class' : 'function';
  if (v === null || typeof v !== 'object') return UPPER.test(key) ? 'constant' : 'primitive';
  return 'object';
}

const modules = fs.readFileSync(path.join(HERE, 'modules.txt'), 'utf8').split('\n').map((s) => s.trim()).filter(Boolean);
const specs = [...modules].sort(cmp);
const firstOwner = new Map(); // object or function -> "module:export" of its first occurrence
const rows = [];
const kinds = {};

for (const spec of specs) {
  const mod = require(spec);
  const exps = Reflect.ownKeys(mod).map((k) => [keyName(k), k]).sort((a, b) => cmp(a[0], b[0]));
  for (const [name, k] of exps) {
    const d = Object.getOwnPropertyDescriptor(mod, k);
    let kind;
    let vtype = '-';
    let arity = '-';
    let alias = '-';
    if (!('value' in d)) {
      kind = 'accessor';
    } else {
      const v = d.value;
      kind = kindOf(name, v);
      vtype = v === null ? 'null' : typeof v;
      if (typeof v === 'function') arity = String(v.length);
      if (v !== null && (typeof v === 'object' || typeof v === 'function')) {
        const at = `${spec}:${name}`;
        if (firstOwner.has(v)) alias = firstOwner.get(v);
        else firstOwner.set(v, at);
      }
    }
    kinds[kind] = (kinds[kind] || 0) + 1;
    const cells = [spec, name, kind, arity, vtype, alias];
    for (const c of cells) {
      if (/[\t\r\n]/.test(c)) throw new Error(`control character in cell ${JSON.stringify(c)}`);
    }
    rows.push(cells);
  }
}

const text = [HEADER.join('\t'), ...rows.map((r) => r.join('\t'))].join('\n') + '\n';
const summary = Object.entries(kinds).sort(([a], [b]) => cmp(a, b)).map(([k, n]) => `${k}=${n}`).join(' ');
console.error(`module_inv: ${specs.length} modules, ${rows.length} exports (${summary})`);
if (process.argv[2]) fs.writeFileSync(process.argv[2], text);
else process.stdout.write(text);
