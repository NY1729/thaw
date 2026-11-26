// A RegExp stored in an `any` object keeps its methods.
const o = { re: /a(b)/g, list: [/x/i], nested: { r: /\d+/ } };
console.log(o.re.test('zab'), o.re.lastIndex);
console.log(o.list[0].test('X'), o.nested.r.exec('ab12')[0]);
const m = o.re.exec('ab');
console.log(m === null ? 'null' : m[1]);
console.log('a1b22'.replace(o.nested.r, '#'), o.re.source, o.re.flags);
function use(x) { return x.re.test('ab'); }
console.log(use({ re: /ab/ }));
const g = { re: /\d/g };
let n = 0;
while (g.re.test('a1b2c3')) n++;
console.log(n);
let mm; const words = { re: /\w/g }; const out = [];
while ((mm = words.re.exec('abc')) !== null) out.push(mm[0]);
console.log(out.join());
