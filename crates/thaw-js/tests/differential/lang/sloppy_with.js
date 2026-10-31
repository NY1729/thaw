globalThis.y = 5;
var o = { a: 1, b: 0, say(x) { return this === o ? 'T' + x : 'F'; }, [Symbol.unscopables]: { hid: true }, hid: 'in' };
globalThis.hid = "outer";
var f = new Function('o', 'with (o) { b = a + y; return typeof a + typeof zz + say(b) + hid; }');
console.log(f(o), o.b);
console.log(new Function('with ({ k: 3 }) { return k * 2 }')());
console.log(eval('1 + 1'), 'with (x) {');
var tpl = new Function('locals', "var __out = []; with (locals || {}) { __out.push(name.toUpperCase()); } return __out.join('');");
console.log(tpl({ name: 'ejs' }));
