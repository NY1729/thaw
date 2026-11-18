// yup validation workload for the cycle-collector safety run. Runs unchanged under thaw-js (the test in
// gc_yup_tests.rs) and under Node (with `__gc` a no-op). Returns the transcript: one line per validation.
(function () {
  const yup = require('yup');
  const out = [];
  let seed = 12345;
  function rnd() {
    seed = (seed * 16807) % 2147483647;
    return seed / 2147483647;
  }
  const schema = yup.object({
    name: yup.string().required().min(2),
    age: yup.number().integer().min(0).required(),
    tags: yup.array(yup.string()).default([]),
  });
  globalThis.__schema = schema;
  // Mid-validation collections: the custom tests call __gc() while a validation is executing.
  const mid = yup.object({
    a: yup.string().test('gc-a', 'bad a', function (v) {
      __gc();
      const junk = { x: [{}] };
      junk.x[0].self = junk;
      return v !== 'bad';
    }),
    b: yup.number().test('gc-b', 'b negative', (v) => {
      __gc();
      return v === undefined || v >= 0;
    }),
  });
  globalThis.__mid = mid;
  const midInputs = [
    { a: 'ok', b: 1 },
    { a: 'bad', b: 2 },
    { a: 'fine', b: -1 },
    { a: 'bad', b: -3 },
  ];
  const inputs = [
    { name: 'Al', age: 3 },
    { name: 'A', age: -1 },
    {},
    { name: 'Bo', age: 5, tags: ['x'] },
    { name: 'Cy', age: 2.5 },
    { name: 'Dee', age: '7' },
    { name: '', age: 4, tags: ['a', 1] },
    { name: 'Eve', age: null },
    { name: 'Fay', age: 0, tags: [] },
    { name: 'G', age: 9, tags: null },
    { name: 'Hal', age: 10, extra: 1 },
    { name: 'Ivy', age: -5, tags: ['q'] },
    { name: 'Jo', age: 'abc' },
    { name: 'nnn', age: 1 },
    { name: null, age: 1 },
    { name: 'Kim', age: 44, tags: ['x', 'y', 'z'] },
  ];
  function show(fn) {
    try {
      return 'ok:' + JSON.stringify(fn());
    } catch (e) {
      return 'err:' + e.name + ':' + e.message + ':' + JSON.stringify(e.errors) + ':' + e.path + ':' + e.type;
    }
  }
  let acc = 0;
  const N = 3000;
  for (let i = 0; i < N; i++) {
    const r = rnd();
    const val = Object.assign({}, inputs[i % inputs.length]);
    if (i % 7 === 0) val.name = 'n'.repeat(i % 5);
    out.push(i + '|valid|' + show(() => schema.validateSync(val)));
    out.push(i + '|isValid|' + schema.isValidSync(val));
    out.push(i + '|abort|' + show(() => schema.validateSync(val, { abortEarly: false })));
    if (i % 50 === 0) {
      const m = midInputs[i % midInputs.length];
      out.push(i + '|mid|' + show(() => mid.validateSync(m, { abortEarly: false })));
      out.push(i + '|midValid|' + mid.isValidSync(m));
    }
    if (i % 100 === 99 || r < 0.03) __gc();
    acc = (acc * 31 + out.length) % 1000003;
  }
  out.push('checksum|' + acc);
  return out.join('\n');
})()
