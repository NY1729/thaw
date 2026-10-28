outer: for (let i = 0; i < 3; i++) { inner: for (let j = 0; j < 3; j++) { if (j === 2) continue outer; if (i === 2) break outer; console.log(i, j); } }
let n = 0; w: while (true) { do { n++; if (n > 3) break w; continue w; } while (false); }
console.log(n);
a: { b: { console.log('x'); break a; } console.log('unreached'); }
sw: switch (2) { case 2: for (const x of [1, 2]) { if (x === 2) break sw; console.log('x', x); } default: console.log('no'); }
f: for (const k in { p: 1, q: 2 }) { for (;;) { console.log(k); continue f; } }
try { throw 1; } catch { console.log('no binding'); }
try { try { throw new Error('a'); } finally { console.log('f1'); } } catch ({ message }) { console.log(message); }
const r = (() => { try { return 'try'; } finally { console.log('fin'); } })(); console.log(r);
console.log((() => { for (const x of [1, 2, 3]) { try { if (x === 2) return x; } finally { console.log('f', x); } } })());
var v1 = 1; { var v1 = 2; } console.log(v1, typeof hoisted, hoisted2()); var hoisted = 1; function hoisted2() { return 'h2'; }
console.log([1, 2, 3].includes(2, 2), [1, 2, 3, 2].indexOf(2, 2), [1, 2, 3, 2].lastIndexOf(2, -2), 'abcabc'.endsWith('b', 5));
console.log(`${1}${'a'}${[1, 2]}${{}}`, String.raw`\n${1}`, 'x'.concat(1), 0.1 * 3, 1e21, 123456789012345680000, 2 ** 53 + 2, (0.000001234).toString(), 1e-7);
console.log((1234.5678).toFixed(2), (2.5).toFixed(0), (1.005).toFixed(2), (-1.5).toFixed(0), (0.5).toFixed(0), (1e21).toFixed(2), (12.3456).toExponential(2), (255).toString(16), (-0).toFixed(1), (123.456).toPrecision(4), (0.00001234).toPrecision(2), (1234567.891).toLocaleString('en-US'), (0.5).toLocaleString(undefined, { maximumFractionDigits: 0 }), (1.005).toLocaleString(undefined, { maximumFractionDigits: 2 }));
