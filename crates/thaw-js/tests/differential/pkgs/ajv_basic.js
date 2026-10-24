// pkg: ajv
const Ajv = require('ajv');
const ajv = new Ajv({ allErrors: true });
const schema = {
  type: 'object',
  properties: {
    name: { type: 'string', minLength: 2, maxLength: 10 },
    age: { type: 'integer', minimum: 0, maximum: 150 },
    email: { type: 'string', pattern: '^[^@]+@[^@]+$' },
    tags: { type: 'array', items: { type: 'string' }, uniqueItems: true, maxItems: 3 },
    role: { enum: ['admin', 'user'] },
    addr: { type: 'object', properties: { zip: { type: 'string', pattern: '^\\d{5}$' } }, required: ['zip'], additionalProperties: false },
    score: { type: 'number', multipleOf: 0.5, exclusiveMaximum: 100 },
    nested: { anyOf: [{ type: 'string' }, { type: 'number', minimum: 10 }] },
    flag: { const: true },
  },
  required: ['name', 'age'],
  additionalProperties: false,
};
const validate = ajv.compile(schema);
const cases = [
  { name: 'Al', age: 30 },
  { name: 'A', age: -1 },
  { name: 'Bob', age: 1.5, extra: 1 },
  { age: 3 },
  { name: 'Carol', age: 20, email: 'c@d.e', tags: ['a', 'b'], role: 'admin', addr: { zip: '12345' }, score: 99.5, nested: 11, flag: true },
  { name: 'Dan', age: 20, email: 'nope', tags: ['a', 'a', 'b', 'c'], role: 'root', addr: { zip: 'abc', y: 1 }, score: 100.2, nested: 5, flag: false },
  'not an object', null, [], 42,
];
for (const c of cases) {
  const ok = validate(c);
  console.log(ok, ok ? '' : JSON.stringify(validate.errors.map((e) => [e.instancePath, e.keyword, e.message, e.schemaPath])));
}
console.log(JSON.stringify(ajv.validate({ type: 'array', items: [{ type: 'number' }, { type: 'string' }], minItems: 2, maxItems: 2 }, [1, 'x'])), ajv.errorsText(ajv.errors));
console.log(ajv.validate({ type: 'string', minLength: 3 }, 'x'), ajv.errorsText(ajv.errors, { separator: ' & ' }));
const v2 = new Ajv({ coerceTypes: true, useDefaults: true, removeAdditional: true });
const data = { n: '42', b: 'true', extra: 1 };
const check = v2.compile({ type: 'object', properties: { n: { type: 'number' }, b: { type: 'boolean' }, d: { type: 'string', default: 'dflt' } }, additionalProperties: false });
console.log(check(data), JSON.stringify(data));
const withRef = new Ajv();
withRef.addSchema({ $id: 'http://x.org/defs.json', definitions: { pos: { type: 'integer', minimum: 1 } } });
const rv = withRef.compile({ $ref: 'http://x.org/defs.json#/definitions/pos' });
console.log(rv(5), rv(0), rv('a'));
const kw = new Ajv(); kw.addKeyword({ keyword: 'even', type: 'number', validate: (s, d) => d % 2 === 0 });
console.log(kw.validate({ even: true }, 4), kw.validate({ even: true }, 5));
try { new Ajv().compile({ type: 'nonsense' }); } catch (e) { console.log(e.constructor.name, e.message.slice(0, 60)); }
console.log(Ajv.name, typeof ajv.compile, typeof ajv.addFormat);
(async () => { const av = new Ajv(); const f = av.compile({ $async: true, type: 'string' }); console.log(await f('ok')); try { await f(1); } catch (e) { console.log(e.name, e.errors.length); } })();
