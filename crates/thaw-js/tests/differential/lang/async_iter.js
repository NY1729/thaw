async function* ag(){ yield 1; await null; yield 2; try { yield 3; } finally { console.log('agfin'); } }
(async () => {
  for await (const v of ag()) { console.log('v', v); if (v === 3) break; }
  for await (const v of [Promise.resolve('p1'), 'p2']) console.log(v);
  const it = ag(); console.log(await it.next(), await it.return('x'), await it.next());
  const src = { [Symbol.asyncIterator](){ let i=0; return { next: async () => i<2 ? {value:i++, done:false} : {done:true} }; } };
  for await (const v of src) console.log('src', v);
  async function* d(){ yield* ag2(); } async function* ag2(){ yield 'a'; yield 'b'; }
  const r=[]; for await (const v of d()) r.push(v); console.log(r);
  class K { async *[Symbol.asyncIterator](){ yield 'k'; } } for await (const v of new K()) console.log(v);
  console.log(Object.prototype.toString.call(ag()));
})();
console.log('sync end');
