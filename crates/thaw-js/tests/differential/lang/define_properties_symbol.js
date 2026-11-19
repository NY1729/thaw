const S = Symbol('s');
function Reply() { this.request = { [S]: 'ctx' }; }
Object.defineProperties(Reply.prototype, {
  [S]: { get() { return this.request[S]; } },
  plain: { get() { return 1; } },
});
const r = new Reply();
console.log(r[S], r.plain, Object.getOwnPropertySymbols(Reply.prototype).length);
const c = Object.create({}, { [S]: { value: 4, enumerable: true }, k: { value: 5 } });
console.log(c[S], c.k);
