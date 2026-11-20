#!/usr/bin/env node
// Generates the Node 22 name universe read by crates/thaw-js/src/scan_names.rs.
// Usage: node tools/scan-names/gen.js [OUT_FILE]   (default: stdout)
//   Regenerate the committed data with:
//   node tools/scan-names/gen.js crates/thaw-js/src/data/node22_names.txt
// Sections: `methods` = every string key reachable on the builtin modules' exports, their classes and
// prototypes, globalThis, a diagnostics_channel Channel/TracingChannel and a forked child's `channel`;
// `globals` = own string keys of globalThis; `members` = `G.P` for G in MEMBER_ROOTS (own + prototype
// chain keys); `owners` = the builtin modules that reach each key (`*` = reachable from globalThis).
// Descriptors only: no getter is invoked. Needs Node 22 (the data is pinned to it).
'use strict';
const fs = require('fs');
const cp = require('child_process');

if (process.versions.node.split('.')[0] !== '22') {
  console.error(`gen.js: needs Node 22 (got ${process.versions.node}); the data is pinned to it`);
  process.exit(2);
}

const MAX_DEPTH = 4;
const MEMBER_ROOTS = ['process', 'Symbol', 'crypto', 'globalThis', 'navigator'];
// Built-in modules that only load with the `node:` scheme.
const SCHEME_ONLY = ['node:test', 'node:test/reporters', 'node:sea', 'node:sqlite'];

const methods = new Set();
// The environment's keys depend on the machine that runs the generator: never walk them.
const seen = new WeakSet([process.env]);
// Keys that are file paths (require.cache, Module._cache) or contain control bytes are not API names.
const PATH_OR_CONTROL = /[\x00-\x1f/\\]/;
let dropped = 0;

// `into` collects the string keys and `visited` holds the objects already walked (the owners walks pass their own).
// `symbols` = false skips symbol-keyed properties (the globalThis owners walk: see `#section owners` below).
function walk(o, d, into = methods, visited = seen, symbols = true) {
  if (o === null || (typeof o !== 'object' && typeof o !== 'function') || visited.has(o) || d > MAX_DEPTH) return;
  visited.add(o);
  for (const k of Reflect.ownKeys(o)) {
    if (typeof k !== 'string' && !symbols) continue;
    if (typeof k === 'string') into.add(k);
    const dsc = Object.getOwnPropertyDescriptor(o, k);
    const v = dsc && 'value' in dsc ? dsc.value : undefined;
    if (typeof v === 'function' || (typeof v === 'object' && v !== null)) walk(v, d + 1, into, visited, symbols);
  }
  walk(Object.getPrototypeOf(o), d + 1, into, visited, symbols);
  if (typeof o === 'function') walk(o.prototype, d + 1, into, visited, symbols);
}

function keysOf(o) {
  const out = new Set();
  for (let p = o; p !== null && p !== undefined; p = Object.getPrototypeOf(p)) {
    for (const k of Reflect.ownKeys(p)) if (typeof k === 'string') out.add(k);
  }
  return out;
}

for (const m of require('module').builtinModules) {
  try { walk(require(m), 0); } catch { /* module refuses to load here: skip */ }
}
for (const m of SCHEME_ONLY) {
  try { walk(require(m), 0); } catch { /* not available on this build: skip */ }
}
walk(globalThis, 0);
const dc = require('diagnostics_channel');
walk(dc.tracingChannel('thaw-scan-names'), 0);
walk(dc.channel('thaw-scan-names'), 0);

// A forked child is the only place Node exposes `channel` with IPC control methods (refCounted etc.).
const child = cp.spawn(process.execPath, ['-e', 'setTimeout(() => {}, 60000)'], {
  stdio: ['ignore', 'ignore', 'ignore', 'ipc'],
});
if (child.channel) walk(child.channel, 0);
else console.error('gen.js: warning: child has no IPC channel; child.channel names missing');
child.kill();

// Owners (`#section owners`, `name<TAB>module,module`): the modules whose exports reach a key, each module walked
// with its own visited set. A key reachable from globalThis gets `*` (always a finding). A key no module reaches
// has no line (the scanner then flags it). Objects that a global getter returns are never walked (getters are
// not invoked), so `process.stdin/stdout/stderr` (net/tty/fs streams) are covered by the `process` alias: every
// net-owned key also counts as owned by the modules that hand out net.Socket instances, and by `process`.
const OWNER_ALIASES = ['http', 'https', 'http2', 'tls', 'child_process', 'cluster', 'worker_threads', 'repl', 'dgram', 'process'];
const owners = new Map();
const addOwner = (name, mod) => {
  if (!owners.has(name)) owners.set(name, new Set());
  owners.get(name).add(mod);
};
for (const m of [...require('module').builtinModules, ...SCHEME_ONLY]) {
  let exported;
  try { exported = require(m); } catch { continue; }
  const reached = new Set();
  walk(exported, 0, reached, new WeakSet([process.env]));
  for (const n of reached) addOwner(n, m.replace(/^node:/, ''));
}
// Symbol-keyed globals are skipped: undici keeps its global dispatcher under `Symbol.for('undici.globalDispatcher.1')`,
// whose `connect` (the HTTP CONNECT method) would otherwise make every `connect` a finding. A package reaches that
// object only by the registered symbol, so this is the one unmodelled route (reported in the change notes).
// The plain getters of these globals are read (they return fixed objects, no side effects) so their keys count as
// reachable from globalThis; no other getter is invoked.
const GETTER_GLOBALS = ['process', 'Buffer', 'performance', 'navigator', 'crypto'];
const fromGlobal = new Set();
walk(globalThis, 0, fromGlobal, new WeakSet([process.env]), false);
for (const root of GETTER_GLOBALS) {
  const value = globalThis[root];
  if (value !== null && (typeof value === 'object' || typeof value === 'function')) {
    walk(value, 0, fromGlobal, new WeakSet([process.env]), false);
  }
}
for (const n of fromGlobal) addOwner(n, '*');
for (const mods of owners.values()) {
  if (mods.has('net')) for (const alias of OWNER_ALIASES) mods.add(alias);
}
const ownerLines = [...owners.keys()]
  .filter((n) => !PATH_OR_CONTROL.test(n) && !n.startsWith('#'))
  .sort()
  .map((n) => `${n}\t${[...owners.get(n)].sort().join(',')}`);

const globals = Object.getOwnPropertyNames(globalThis);
const members = [];
for (const root of MEMBER_ROOTS) {
  const obj = globalThis[root];
  if (obj === undefined || obj === null) {
    console.error(`gen.js: warning: ${root} is not present; no members emitted for it`);
    continue;
  }
  for (const k of [...keysOf(obj)].sort()) members.push(`${root}.${k}`);
}

const keep = (list) => [...new Set(list)].filter((n) => {
  if (PATH_OR_CONTROL.test(n)) { dropped++; return false; }
  if (n.startsWith('#')) throw new Error(`gen.js: name starts with '#', not representable: ${JSON.stringify(n)}`);
  return true;
}).sort();
const methodList = keep(methods);
const globalList = keep(globals);
const memberList = keep(members);

const out = [
  `# node: ${process.version}`,
  '# generated by tools/scan-names/gen.js; do not edit by hand. Regenerate with:',
  '#   node tools/scan-names/gen.js crates/thaw-js/src/data/node22_names.txt',
  `# counts: methods=${methodList.length} globals=${globalList.length} members=${memberList.length} owners=${ownerLines.length}`,
  '#section methods',
  ...methodList,
  '#section globals',
  ...globalList,
  '#section members',
  ...memberList,
  '#section owners',
  ...ownerLines,
  '',
].join('\n');

const target = process.argv[2];
if (target) fs.writeFileSync(target, out);
else process.stdout.write(out);
console.error(`gen.js: ${process.version} methods=${methodList.length} globals=${globalList.length} members=${memberList.length} owners=${ownerLines.length} dropped=${dropped}`);
