function plain(a, b) { return a + b; }
const arrow = (x) => x * 2;
class K { m(a) { return a; } static s() {} }
const o = { meth(a) { return a; }, get g() { return 1; } };
console.log(plain.toString());
console.log(arrow.toString());
console.log(K.toString());
console.log(new K().m.toString(), '|', o.meth.toString());
console.log(String(async function af() {}), '|', String(function* gf() { yield 1; }));
console.log(/\[native code\]/.test(Math.max.toString()), /\[native code\]/.test(plain.toString()), Math.max.toString());
console.log(Object.toString(), Map.toString(), Symbol.toString());
console.log(Function.prototype.toString.call(class A {}), String(Array.prototype.push));
