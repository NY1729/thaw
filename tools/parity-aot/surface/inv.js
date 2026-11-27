#!/usr/bin/env node
// Recursive Node 22 inventory: every public node:* module and the Node/web globals -> <OUT>/<module>.json.
// Usage: node inv.js OUT_DIR [module ...]   (default: all of require('module').builtinModules, plus globals.json)
// Entry: {name, kind: function|class|getter|object|value, typeof, arity, static[], proto[], instance[], props[]}.
// Member: {name, kind, typeof, arity, from?}. Accessors are never invoked. Underscore names and symbols are not listed.
'use strict';
const fs = require('fs'), path = require('path'), vm = require('vm');
if (process.versions.node.split('.')[0] !== '22') { console.error('needs Node 22, got ' + process.versions.node); process.exit(2); }
const recipes = JSON.parse(fs.readFileSync(path.join(__dirname, 'recipes.json'), 'utf8'));
const SKIP = new Set(['constructor', 'length', 'name', 'prototype', 'caller', 'arguments', '__proto__']);
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
const isClass = (f) => /^class[\s{]/.test(Function.prototype.toString.call(f)) ||
  (f.prototype && typeof f.prototype === 'object' && Reflect.ownKeys(f.prototype).some((k) => k !== 'constructor'));
const ok = (k) => typeof k === 'string' && !SKIP.has(k) && !k.startsWith('_');

function describe(owner, key, from) {
  const d = Object.getOwnPropertyDescriptor(owner, key);
  const m = { name: key };
  if (from) m.from = from;
  if (!('value' in d)) { m.kind = 'getter'; m.typeof = 'accessor'; m.set = !!d.set; return m; }
  const v = d.value;
  m.typeof = v === null ? 'null' : typeof v;
  m.kind = typeof v === 'function' ? (isClass(v) ? 'class' : 'function') : (v !== null && typeof v === 'object' ? 'object' : 'value');
  m.arity = typeof v === 'function' ? v.length : null;
  return m;
}

// own keys along the prototype chain (stop at the Object/Function prototype); first definition wins
function chain(start, stops, label) {
  const seen = new Map();
  for (let o = start; o && !stops.includes(o); o = Object.getPrototypeOf(o)) {
    const from = o === start ? undefined : label(o);
    for (const k of Reflect.ownKeys(o)) if (ok(k) && !seen.has(k)) seen.set(k, describe(o, k, from));
  }
  return [...seen.values()].sort((a, b) => cmp(a.name, b.name));
}

function objectProps(o, depth, seen) {
  if (seen.has(o)) return [];
  seen.add(o);
  const out = [];
  for (const k of Reflect.ownKeys(o).filter(ok).sort(cmp).slice(0, 300)) {
    const m = describe(o, k);
    if (m.kind === 'object' && depth > 1) m.props = objectProps(Object.getOwnPropertyDescriptor(o, k).value, depth - 1, seen);
    out.push(m);
  }
  return out;
}

function entry(id, mod, name, d) {
  const e = describe(mod, name);
  if (!('value' in d)) return e;
  const v = d.value;
  if (e.kind === 'class' || e.kind === 'function') {
    e.static = chain(v, [Function.prototype, Object.getPrototypeOf(Function.prototype)], (o) => o.name || '?');
    if (v.prototype) e.proto = chain(v.prototype, [Object.prototype], (o) => (o.constructor && o.constructor.name) || '?');
    const r = recipes[id];
    let inst;
    try {
      if (r && r.js !== undefined) inst = r.js === null ? null : new Function('M', 'require', 'return ' + r.js)(mod, require);
      else if (e.kind === 'class') { inst = new v(); e.construct = 'ok'; }
    } catch (_) { inst = null; if (e.kind === 'class') e.construct = 'throws'; }
    if (inst && typeof inst === 'object') {
      const protoNames = new Set((e.proto || []).map((m) => m.name));
      e.instance = Reflect.ownKeys(inst).filter((k) => ok(k) && !protoNames.has(k)).sort(cmp).map((k) => describe(inst, k));
      try { (inst.destroy || inst.close || (() => {})).call(inst); } catch (_) {}
    }
  } else if (e.kind === 'object' && v) e.props = objectProps(v, 2, new Set());
  return e;
}

const out = process.argv[2];
if (!out) { console.error('usage: inv.js OUT_DIR [module ...]'); process.exit(2); }
fs.mkdirSync(out, { recursive: true });
const want = process.argv.slice(3);
const mods = (want.length ? want : require('module').builtinModules.filter((m) => !m.startsWith('_') && !m.includes('/internal')).map((m) => 'node:' + m.replace(/^node:/, '')).concat('globals'))
  .map((m) => (m === 'globals' ? m : m.startsWith('node:') ? m : 'node:' + m));
const total = {};
for (const spec of [...new Set(mods)].sort(cmp)) {
  let ents = [];
  try {
    if (spec === 'globals') {
      const es = new Set(vm.runInNewContext('Object.getOwnPropertyNames(globalThis)'));
      for (const k of Object.getOwnPropertyNames(globalThis).filter((k) => !es.has(k) && ok(k)).sort(cmp)) {
        const d = Object.getOwnPropertyDescriptor(globalThis, k);
        ents.push(entry('globals.' + k, globalThis, k, d));
      }
    } else {
      const mod = require(spec);
      for (const k of Reflect.ownKeys(mod).filter((k) => typeof k === 'string' && k !== 'default').sort(cmp)) {
        ents.push(entry(spec + '.' + k, mod, k, Object.getOwnPropertyDescriptor(mod, k)));
      }
    }
  } catch (e) { fs.writeFileSync(path.join(out, spec.replace(/[:/]/g, '_') + '.json'), JSON.stringify({ module: spec, error: String(e.message), exports: [] }) + '\n'); continue; }
  fs.writeFileSync(path.join(out, spec.replace(/[:/]/g, '_') + '.json'), JSON.stringify({ module: spec, node: process.version, exports: ents }, null, 1) + '\n');
  total[spec] = ents.length;
}
console.error('inv: ' + Object.keys(total).length + ' modules, ' + Object.values(total).reduce((a, b) => a + b, 0) + ' exports');
process.exit(0);
