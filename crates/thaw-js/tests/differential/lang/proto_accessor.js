const o = Object.create({ inherited: 1 });
console.log(o.__proto__.inherited, Object.keys(o), '__proto__' in {}, Object.prototype.hasOwnProperty('__proto__'));
const p = {}; p.__proto__ = { viaSetter: 2 }; console.log(p.viaSetter, Object.keys(p));
const parsed = JSON.parse('{"__proto__": {"x": 1}}'); console.log(Object.keys(parsed), parsed.x, Object.getPrototypeOf(parsed) === Object.prototype);
console.log(JSON.stringify(Object.getOwnPropertyNames(Object.prototype).includes('__proto__')));
