// pkg: yup
const yup = require('yup');
const show = (e) => console.log(e.name, e.path, e.type, JSON.stringify(e.errors), e.inner ? e.inner.map((i) => i.path).join() : '');
(async () => {
  const addr = yup.object({ street: yup.string().required(), zip: yup.string().matches(/^\d{5}$/, 'bad zip') });
  const user = yup.object({
    id: yup.number().integer().min(1).required(),
    name: yup.string().trim().min(1),
    addr,
    friends: yup.array().of(yup.object({ id: yup.number().required() })).min(1),
    role: yup.string().oneOf(['admin', 'user']).default('user'),
    born: yup.date(),
    pw: yup.string().min(4),
    pw2: yup.string().oneOf([yup.ref('pw')], 'passwords differ'),
  });
  try {
    await user.validate({ id: 0, name: ' x ', addr: { street: '', zip: 'abc' }, friends: [{}], role: 'root', pw: 'abcd', pw2: 'abce' }, { abortEarly: false });
  } catch (e) { show(e); console.log(JSON.stringify(e.errors)); }
  const ok = await user.validate({ id: '5', name: '  Ann ', addr: { street: 'Main', zip: '12345' }, friends: [{ id: 1 }], born: '2020-02-03T00:00:00Z', pw: 'secret', pw2: 'secret' });
  console.log(JSON.stringify(ok));
  console.log(ok.born instanceof Date, ok.born.getTime());
  console.log(JSON.stringify(user.describe().fields.role));
  console.log(JSON.stringify(yup.string().min(2).max(5).describe()));
  try { await yup.object({ a: yup.number() }).noUnknown().strict().validate({ a: 1, b: 2 }); } catch (e) { show(e); }
  try { await yup.number().strict().validate('1'); } catch (e) { console.log(e.message); }
  console.log(await yup.number().transform((v) => v * 2).validate(21));
  const tup = yup.tuple([yup.string(), yup.number()]);
  console.log(JSON.stringify(await tup.validate(['a', 1])), tup.isValidSync(['a', 'b']));
  const lz = yup.lazy((v) => (typeof v === 'number' ? yup.number().min(5) : yup.string()));
  console.log(lz.isValidSync(3), lz.isValidSync(7), lz.isValidSync('s'));
  console.log(yup.array().of(yup.number()).cast(['1', '2']), yup.string().nullable().cast(null), yup.mixed().notRequired().isValidSync(undefined));
  const e = new yup.ValidationError('boom', 1, 'f');
  console.log(e instanceof Error, e.name, e.message, e.value, e.path, yup.ValidationError.isError(e));
  console.log(await yup.string().required().isValid(null), await yup.string().nullable().isValid(null), await yup.string().defined().isValid(undefined));
  console.log(yup.object({ a: yup.string() }).partial !== undefined, JSON.stringify(yup.object({ a: yup.string().default('x') }).getDefault()));
})();
