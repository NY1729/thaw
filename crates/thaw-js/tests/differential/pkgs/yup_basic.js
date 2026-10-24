// pkg: yup
const yup = require('yup');
const schema = yup.object({
  name: yup.string().required().min(2),
  age: yup.number().required().positive().integer(),
  email: yup.string().email(),
  tags: yup.array().of(yup.string()).max(3),
});
async function check(v) {
  try {
    const r = await schema.validate(v, { abortEarly: false });
    console.log('ok', JSON.stringify(r));
  } catch (e) {
    console.log(e.name, e.errors.join(' | '));
  }
}
(async () => {
  await check({ name: 'Al', age: 30, email: 'a@b.co', tags: ['x'] });
  await check({ name: 'A', age: -1.5, email: 'nope', tags: ['a', 'b', 'c', 'd'] });
  await check({});
  await check({ name: 'Bob', age: '42' });
  console.log(schema.isValidSync({ name: 'Zed', age: 1 }), schema.isValidSync({ name: 'Z', age: 1 }));
  console.log(JSON.stringify(schema.cast({ name: 'Q', age: '7', extra: 1 }, { stripUnknown: true })));
  const s2 = yup.string().trim().lowercase().default('dflt');
  console.log(s2.cast('  HeLLo '), s2.cast(undefined), await s2.validate(' ABC '));
  const dates = yup.object({ n: yup.number().when('k', { is: true, then: (s) => s.required(), otherwise: (s) => s.optional() }), k: yup.boolean() });
  console.log(await dates.isValid({ k: true }), await dates.isValid({ k: false }));
  const m = yup.mixed().oneOf(['a', 'b']);
  console.log(m.isValidSync('a'), m.isValidSync('c'));
  try { await yup.string().required('custom msg').validate(''); } catch (e) { console.log(e.message, e.path, e.type); }
  console.log(await yup.number().validate('12.5'), await yup.boolean().validate('true'));
  const lazyP = yup.string().test('len', 'must be 3', (v) => v.length === 3);
  console.log(await lazyP.isValid('abc'), await lazyP.isValid('abcd'));
  const slow = yup.string().test('slow', 'slow fail', (v) => new Promise((r) => setTimeout(() => r(v === 'ok'), 5)));
  console.log(await slow.isValid('ok'), await slow.isValid('no'));
})();
