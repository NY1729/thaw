#!/usr/bin/env node
// Generates the builtin descriptor table (crates/thaw-js/src/builtin_table.tsv) from Node 22.
// Usage: node tools/parity/gen_builtin_table.js [OUT_FILE]   (default: stdout)
// Descriptors only: no getter is invoked (except the getter of a root global, which is needed to reach it),
// and primitive values are recorded as typeof only, so the output is the same on every machine.
'use strict';
const fs = require('fs');
const path = require('path');

if (process.versions.node.split('.')[0] !== '22') {
  console.error(`gen_builtin_table.js: needs Node 22 (got ${process.versions.node}); the table is pinned to it`);
  process.exit(2);
}

// Depth 0 = a root (global or intrinsic), 1 = its members and .prototype, 2 = their prototypes.
const MAX_DEPTH = 2;
// Children of these owners are not walked (their contents depend on the machine or the build).
// Their own rows are still emitted as data.
const DENY = new Set([
  'process.env', 'process.versions', 'process.config', 'process.release', 'process.features',
  'process.mainModule', 'process.report', 'process.allowedNodeEnvironmentFlags',
]);
const HEADER = ['owner', 'key', 'kind', 'flags', 'vtype', 'length', 'name', 'hasPrototype', 'alias', 'tag'];

const desc = (o, k) => Object.getOwnPropertyDescriptor(o, k);
const isObj = (v) => v !== null && (typeof v === 'object' || typeof v === 'function');

function keyName(k) {
  if (typeof k === 'string') return k;
  const d = k.description;
  if (d === undefined) return '@@';
  return '@@' + (d.startsWith('Symbol.') ? d.slice(7) : d);
}

// Intrinsics that are not reachable as globals, reached through the language's own prototype chains.
function intrinsicRoots() {
  const GF = Object.getPrototypeOf(function* () {});
  const AGF = Object.getPrototypeOf(async function* () {});
  const ArrayIt = Object.getPrototypeOf([][Symbol.iterator]());
  return [
    ['%TypedArray%', Object.getPrototypeOf(Int8Array)],
    ['%GeneratorFunction%', GF.constructor],
    ['%GeneratorPrototype%', GF.prototype],
    ['%AsyncFunction%', Object.getPrototypeOf(async function () {}).constructor],
    ['%AsyncGeneratorFunction%', AGF.constructor],
    ['%AsyncGeneratorPrototype%', AGF.prototype],
    ['%AsyncIteratorPrototype%', Object.getPrototypeOf(AGF.prototype)],
    ['%IteratorPrototype%', Object.getPrototypeOf(ArrayIt)],
    ['%ArrayIteratorPrototype%', ArrayIt],
  ];
}

// Roots: Node's global names (node_globals.json, a checked-in snapshot) that are objects or functions.
function globalRoots() {
  const names = JSON.parse(fs.readFileSync(path.join(__dirname, 'node_globals.json'), 'utf8'));
  const roots = [];
  for (const [name, type] of names) {
    if (type !== 'function' && type !== 'object') continue;
    if (!Object.prototype.hasOwnProperty.call(globalThis, name)) continue;
    const d = desc(globalThis, name);
    const v = 'value' in d ? d.value : globalThis[name];
    if (!isObj(v) || v === globalThis) continue;
    roots.push([name, v]);
  }
  return roots;
}

// owner path -> rows; a row keeps the value object (for alias identity) and is formatted later.
const owners = new Map();
const seen = new Set();
const occurrences = new Map(); // object -> count of (row value or root) references

function visit(obj, p, depth) {
  seen.add(obj);
  const rows = [];
  owners.set(p, rows);
  const keys = Reflect.ownKeys(obj).map((k) => [keyName(k), k]).sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  for (let i = 1; i < keys.length; i++) {
    if (keys[i][0] === keys[i - 1][0]) throw new Error(`duplicate key ${keys[i][0]} on ${p}`);
  }
  for (const [kn, k] of keys) {
    const d = desc(obj, k);
    const row = { owner: p, key: kn, kind: 'data', flags: '', vtype: '-', length: '-', name: '-', hasPrototype: '-', obj: null, tag: '-' };
    if ('value' in d) {
      const v = d.value;
      row.flags = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');
      row.vtype = v === null ? 'object' : typeof v;
      if (isObj(v)) row.obj = v;
      if (typeof v === 'function') {
        row.length = dataValue(v, 'length');
        row.name = dataValue(v, 'name');
        row.hasPrototype = Object.prototype.hasOwnProperty.call(v, 'prototype') ? '1' : '0';
      }
      if (kn === '@@toStringTag' && typeof v === 'string') row.tag = v;
      const child = row.obj;
      if (child && !seen.has(child) && depth < MAX_DEPTH && !Array.isArray(child) && child !== globalThis) {
        const cp = p + '.' + kn;
        if (!DENY.has(cp)) visit(child, cp, depth + 1);
      }
    } else {
      row.kind = 'accessor';
      row.flags = '-' + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');
      const g = d.get ? dataValue(d.get, 'name') : '-';
      const s = d.set ? dataValue(d.set, 'name') : '-';
      row.name = g + '|' + s;
    }
    rows.push(row);
  }
}

function dataValue(fn, key) {
  const d = desc(fn, key);
  return d && 'value' in d ? String(d.value) : '-';
}

const roots = [...globalRoots(), ...intrinsicRoots()];
for (const [name, obj] of roots) {
  if (seen.has(obj)) { occurrences.set(obj, (occurrences.get(obj) || 0) + 1); continue; }
  visit(obj, name, 0);
  occurrences.set(obj, (occurrences.get(obj) || 0) + 1);
}

// Sorted owners and rows; alias ids are numbered in output order, only for objects reached twice or more.
const allRows = [];
for (const p of [...owners.keys()].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))) {
  for (const r of owners.get(p)) {
    if (r.obj) occurrences.set(r.obj, (occurrences.get(r.obj) || 0) + 1);
    allRows.push(r);
  }
}
const aliasId = new Map();
let nextAlias = 1;
const lines = [HEADER.join('\t')];
for (const r of allRows) {
  let alias = '-';
  if (r.obj && occurrences.get(r.obj) >= 2) {
    if (!aliasId.has(r.obj)) aliasId.set(r.obj, nextAlias++);
    alias = String(aliasId.get(r.obj));
  }
  const cells = [r.owner, r.key, r.kind, r.flags, r.vtype, r.length, r.name, r.hasPrototype, alias, r.tag];
  for (const c of cells) {
    if (/[\t\r\n]/.test(c)) throw new Error(`control character in cell ${JSON.stringify(c)}`);
  }
  lines.push(cells.join('\t'));
}
const text = lines.join('\n') + '\n';
if (process.argv[2]) fs.writeFileSync(process.argv[2], text);
else process.stdout.write(text);
