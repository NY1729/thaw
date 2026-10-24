// pkg: js-yaml
const yaml = require('js-yaml');
const show = (v) => console.log(JSON.stringify(v));
const doc = `
name: Thaw
version: 1.5
count: 42
neg: -7
hex: 0xFF
oct: 0o17
exp: 1.5e3
inf: .inf
nan: .nan
yes: true
no: false
nothing: null
tilde: ~
date: 2020-01-02
datetime: 2020-01-02T03:04:05Z
str: "quoted: \\"x\\" \\n line"
single: 'it''s'
multi: |
  line one
  line two
folded: >
  folded
  text
list:
  - a
  - b: 1
    c: [1, 2, {d: e}]
  - - nested
    - seq
map: {x: 1, y: [2, 3]}
anchors:
  base: &b {p: 1, q: 2}
  copy: *b
  merged:
    <<: *b
    q: 3
`;
const loaded = yaml.load(doc);
show(Object.keys(loaded));
for (const k of Object.keys(loaded)) {
  const v = loaded[k];
  if (v instanceof Date) console.log(k, 'Date', v.toISOString());
  else if (v instanceof Uint8Array) console.log(k, 'bytes', Array.from(v).join());
  else if (typeof v === 'number' && !isFinite(v)) console.log(k, String(v));
  else if (typeof v === 'number' && v !== v) console.log(k, 'NaN');
  else console.log(k, JSON.stringify(v));
}
const out = yaml.dump({ a: 1, b: [1, 2, { c: 'x y' }], d: 'multi\nline', e: null, f: true, g: 'true', h: '123', i: new Date(Date.UTC(2020, 0, 2)), j: { k: {} }, l: [], m: 'with: colon', n: -0.5, o: 'é', p: ' lead' });
console.log(out);
console.log(yaml.dump([1, [2, 3], { x: 1 }], { indent: 4, flowLevel: 1 }));
console.log(yaml.dump({ z: 1, a: 2 }, { sortKeys: true }), yaml.dump('plain'), yaml.dump(null), yaml.dump(3.0), yaml.dump({ a: undefined }, { skipInvalid: true }));
console.log(JSON.stringify(yaml.loadAll('a: 1\n---\nb: 2\n---\n- 3')));
for (const bad of ['a: [1, 2', 'a: b: c', '\tx: 1', 'a: *missing', '{a: 1}}', 'key: "unterminated']) {
  try { yaml.load(bad); console.log('no error', bad); } catch (e) { console.log(e.name, e.reason, e.mark && e.mark.line, e.mark && e.mark.column); }
}
console.log(yaml.load('- 1\n- 2', { json: true }).length, JSON.stringify(yaml.load('a: 1\na: 2', { json: true })));
try { yaml.load('a: 1\na: 2'); } catch (e) { console.log(e.reason); }
console.log(JSON.stringify(yaml.load('x: !!float 3\ny: !!str 5\nz: !!int "7"')));
console.log(typeof yaml.Type, typeof yaml.Schema, typeof yaml.DEFAULT_SCHEMA, yaml.CORE_SCHEMA !== undefined, yaml.JSON_SCHEMA !== undefined);
console.log(yaml.dump({ long: 'x'.repeat(100) + ' ' + 'y'.repeat(30) }, { lineWidth: 40 }));

for (const t of ['!!binary aGVsbG8=', '!!set {a, b}', '!!omap [ {k1: 1} ]', '!!timestamp 2020-01-02']) { try { console.log(JSON.stringify(yaml.load('x: ' + t))); } catch (e) { console.log('ERR', e.reason); } }
