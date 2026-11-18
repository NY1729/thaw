// Async yup validation workload for the cycle-collector gate (gc_yup_tests.rs, `gc_yup_async_transcript_equal`). Runs
// unchanged under thaw-js and under Node 22 (the oracle, with `__gc` a no-op). The expression's value is the run's
// promise; the transcript (one line per result) is left in `globalThis.__transcript`.
(async function () {
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
  // Async custom tests: each calls __gc() while its validation is pending, then awaits.
  const mid = yup.object({
    a: yup.string().test('gc-a', 'bad a', async function (v) {
      __gc();
      await null;
      const junk = { x: [{}] };
      junk.x[0].self = junk;
      return v !== 'bad';
    }),
    b: yup.number().test('gc-b', 'b negative', async (v) => {
      __gc();
      await null;
      return v === undefined || v >= 0;
    }),
  });
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
  function describe(e) {
    return 'err:' + e.name + ':' + e.message + ':' + JSON.stringify(e.errors) + ':' + e.path + ':' + e.type;
  }
  let acc = 0;
  const N = 3000;
  for (let i = 0; i < N; i++) {
    const r = rnd();
    const val = Object.assign({}, inputs[i % inputs.length]);
    if (i % 7 === 0) val.name = 'n'.repeat(i % 5);
    const res = await schema
      .validate(val, { abortEarly: false })
      .then((v) => 'ok:' + JSON.stringify(v), (e) => describe(e));
    out.push(i + '|valid|' + res);
    const batch = [val, { name: 'Zed', age: i % 4 }, {}, { name: 'Q', age: -i }, { name: 'Ann', age: 3, tags: ['x'] }];
    const all = await Promise.all(
      batch.map((x) =>
        schema.validate(x, { abortEarly: false }).then(
          (v) => 'ok',
          (e) => 'err:' + (e.errors ? e.errors.length : e.name)
        )
      )
    );
    out.push(i + '|all|' + all.join(','));
    if (i % 200 === 0) {
      const m = midInputs[i % midInputs.length];
      const mr = await mid
        .validate(m, { abortEarly: false })
        .then((v) => 'ok:' + JSON.stringify(v), (e) => describe(e));
      out.push(i + '|mid|' + mr);
    }
    if (i % 100 === 99 || r < 0.03) __gc();
    acc = (acc * 31 + out.length) % 1000003;
  }
  out.push('checksum|' + acc);
  globalThis.__transcript = out.join('\n');
})();
