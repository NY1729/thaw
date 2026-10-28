// pkg: qs
const qs = require('qs');
const log = (...a) => console.log(...a.map((x) => (typeof x === 'string' ? x : JSON.stringify(x))));
log(qs.stringify({ a: 1, b: [1, 2], c: { d: 'x y' } }), qs.stringify({ a: [1, 2] }, { arrayFormat: 'brackets' }), qs.stringify({ a: [1, 2] }, { arrayFormat: 'indices' }), qs.stringify({ a: [1, 2] }, { arrayFormat: 'comma' }), qs.stringify({ a: 'b', c: 'd' }, { delimiter: ';' }));
log(qs.parse('a=1&b[]=2&b[]=3&c[d]=x'), qs.parse('a=b&a=c'), qs.parse('a[b][c]=1'), qs.parse('a%5Bb%5D=c'), qs.parse('?x=1', { ignoreQueryPrefix: true }), qs.parse('a=1;b=2', { delimiter: ';' }));
log(qs.parse('a[]=1&a[]=2', { arrayLimit: 1 }), qs.parse('a.b=1', { allowDots: true }), qs.parse('a=%E4%BD%A0'), qs.stringify({ a: '你' }), qs.stringify({ a: null, b: undefined, c: '' }), qs.stringify({ a: null }, { skipNulls: true }), qs.stringify({ a: 1 }, { addQueryPrefix: true }));
log(qs.stringify({ a: [{ b: 1 }, { b: 2 }] }), qs.parse('a[0][b]=1&a[1][b]=2'), qs.stringify({ d: new Date(0) }), qs.stringify({ b: 'x', a: 'y' }, { sort: (x, y) => x.localeCompare(y) }), qs.stringify({ a: 'b c' }, { encode: false }), qs.parse('a[__proto__]=x'), qs.parse('a=1&a=2', { duplicates: 'first' }));
