// util.types matrix: every util.types function (Node 22 lists 43) against a table of probe values.
// Each cell is `label=1|0` (or `label=throw:<Error>`). Two probes are left out because thaw-js cannot
// represent them yet: `arguments` (an arguments object is an Array here, so isArgumentsObject is false) and
// `Object(Symbol())` (a boxed symbol is the symbol itself here, so isSymbolObject/isBoxedPrimitive are false).
// Under thaw-js the module is `globalThis.__thaw_util_types` (the registry's util polyfill exports it).
const util = typeof require === 'function' ? require('util') : { types: globalThis.__thaw_util_types };

function values() {
  const v = [];
  const add = (label, make) => v.push([label, make]);
  add('plainObject', () => ({}));
  add('array', () => []);
  add('date', () => new Date(0));
  add('fakeDate', () => Object.create(Date.prototype));
  add('regexp', () => /a/g);
  add('map', () => new Map([[1, 2]]));
  add('set', () => new Set([1]));
  add('weakMap', () => new WeakMap());
  add('weakSet', () => new WeakSet());
  add('fakeMap', () => Object.create(Map.prototype));
  add('mapIterator', () => new Map([[1, 2]]).keys());
  add('setIterator', () => new Set([1]).values());
  add('arrayIterator', () => [1].values());
  add('promise', () => Promise.resolve(1));
  add('asyncFunction', () => async function af() {});
  add('asyncArrow', () => async () => {});
  add('generatorFunction', () => function* gf() {});
  add('asyncGeneratorFunction', () => async function* agf() {});
  add('plainFunction', () => function pf() {});
  add('generatorObject', () => (function* g() { yield 1; })());
  add('asyncGeneratorObject', () => (async function* ag() { yield 1; })());
  add('proxyArray', () => new Proxy([], {}));
  add('proxyObject', () => new Proxy({}, {}));
  add('proxyFunction', () => new Proxy(function () {}, {}));
  add('int8', () => new Int8Array(1));
  add('uint8', () => new Uint8Array(1));
  add('uint8clamped', () => new Uint8ClampedArray(1));
  add('int16', () => new Int16Array(1));
  add('uint16', () => new Uint16Array(1));
  add('int32', () => new Int32Array(1));
  add('uint32', () => new Uint32Array(1));
  add('float32', () => new Float32Array(1));
  add('float64', () => new Float64Array(1));
  add('bigint64', () => new BigInt64Array(1));
  add('biguint64', () => new BigUint64Array(1));
  add('fakeUint8', () => Object.create(Uint8Array.prototype));
  add('dataView', () => new DataView(new ArrayBuffer(4)));
  add('arrayBuffer', () => new ArrayBuffer(4));
  add('sharedArrayBuffer', () => new SharedArrayBuffer(4));
  add('fakeArrayBuffer', () => Object.create(ArrayBuffer.prototype));
  add('numberObject', () => new Number(1));
  add('stringObject', () => new String(''));
  add('booleanObject', () => new Boolean(false));
  add('bigintObject', () => Object(1n));
  add('fakeNumber', () => Object.create(Number.prototype));
  add('primitiveNumber', () => 1);
  add('primitiveString', () => 'x');
  add('primitiveBigint', () => 1n);
  add('primitiveSymbol', () => Symbol('p'));
  add('undefinedValue', () => undefined);
  add('nullValue', () => null);
  add('error', () => new Error('x'));
  add('typeError', () => new TypeError('x'));
  add('subclassError', () => { class E extends Error {} return new E('x'); });
  add('fakeError', () => Object.create(Error.prototype));
  add('fakeErrorObj', () => ({ name: 'Error', message: 'm' }));
  add('subclassDate', () => { class D extends Date {} return new D(0); });
  add('fakeTagDate', () => ({ [Symbol.toStringTag]: 'Date' }));
  add('fakeTagMap', () => ({ [Symbol.toStringTag]: 'Map' }));
  add('fakeTagArguments', () => ({ [Symbol.toStringTag]: 'Arguments' }));
  add('fakeTagAsyncFunction', () => ({ [Symbol.toStringTag]: 'AsyncFunction' }));
  add('fakeTagGenerator', () => ({ [Symbol.toStringTag]: 'Generator' }));
  add('fakeTagPromise', () => ({ [Symbol.toStringTag]: 'Promise' }));
  return v;
}

// sorted: Node's key order is its binding order, which the thaw polyfill does not reproduce (not part of the contract)
const fns = Object.keys(util.types).sort();
const vals = values();
for (const name of fns) {
  const f = util.types[name];
  const cells = vals.map(([label, make]) => {
    let r;
    try { r = f(make()); } catch (e) { r = 'throw:' + e.constructor.name; }
    return label + '=' + (typeof r === 'boolean' ? (r ? 1 : 0) : String(r));
  });
  console.log(name + ' ' + cells.join(' '));
}
console.log('count ' + fns.length);
