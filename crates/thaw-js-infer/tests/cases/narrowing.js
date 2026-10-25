function describe(x) {
  if (typeof x === 'number') { return 'n' + (x * 2); }
  if (typeof x === 'string') { return 's' + x.length; }
  return 'other';
}
function lenOr(v, d) {
  if (v === undefined) { return d; }
  return v.length;
}
function mode(c) { return c ? 'fast' : 'slow'; }
var k = mode(true);
console.log(describe(3), describe('abc'), describe(true));
console.log(lenOr('hello', 0), lenOr(undefined, 7), k, mode(false));
