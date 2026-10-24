class Svc {
  constructor() { this.n = 0; }
  async inc(by) { await null; this.n += by; return this.n; }
  static async make() { const s = new Svc(); await s.inc(1); return s; }
}
const o = { async m(x) { return x * 2; }, f: async (y) => (await y) + 1 };
(async () => {
  const s = await Svc.make();
  console.log(await s.inc(2), await o.m(4), await o.f(Promise.resolve(9)));
  const out = [];
  for (const x of [1, 2, 3]) out.push(await o.m(x));
  console.log(out);
  let i = 0;
  while (i < 3) { await null; i++; }
  console.log(i);
  console.log(await Promise.all([1, 2, 3].map(async (x) => { await null; return x * x; })));
  for await (const v of [Promise.resolve('p1'), 'p2']) console.log(v);
})();
