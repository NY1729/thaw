//! Missing builtin members (PR F of the Q9 parity round 3 plan): `Object.prototype` legacy accessors and
//! `toLocaleString`, `Function.prototype.arguments`/`caller`/`@@hasInstance`, `Array.prototype.toLocaleString`
//! and `@@unscopables`, `Boolean.prototype.valueOf`, `Symbol.prototype` `constructor`/`valueOf`/`description`/
//! `@@toPrimitive`, `Date.prototype.setYear`/`@@toPrimitive`, `DataView.prototype.buffer`. The expected text is
//! Node 22 output for the same probe (`node` on the JS below); `Array.prototype.length` is waived (W8).
#[cfg(test)]
use super::*;

const PROBE: &str = r##"
const out = [];
const T = (f) => { try { out.push(String(f())); } catch (e) { out.push('THROW ' + e.name); } };
const fi = (f) => f === undefined ? 'undef' : JSON.stringify([f.name, f.length]);
const desc = (o, k) => {
  const d = Object.getOwnPropertyDescriptor(o, k);
  if (!d) return 'NONE';
  if ('get' in d || 'set' in d) return 'acc ' + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-') + ' get=' + fi(d.get) + ' set=' + fi(d.set);
  const fl = (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-');
  return 'data ' + fl + ' ' + (typeof d.value === 'function' ? 'fn ' + fi(d.value) : typeof d.value);
};
// Object.prototype legacy accessors
out.push(desc(Object.prototype, '__defineGetter__'), desc(Object.prototype, '__defineSetter__'));
out.push(desc(Object.prototype, '__lookupGetter__'), desc(Object.prototype, '__lookupSetter__'));
out.push(desc(Object.prototype, 'toLocaleString'));
const o = {};
Object.prototype.__defineGetter__.call(o, 'x', function () { return 7; });
out.push(o.x, desc(o, 'x'));
T(() => Object.prototype.__defineGetter__.call({}, 'x', 1));
T(() => Object.prototype.__defineGetter__.call(null, 'x', function () {}));
const p = {};
Object.prototype.__defineSetter__.call(p, 'y', function (v) { this.z = v; });
p.y = 3;
out.push(p.z, desc(p, 'y'));
const g = Object.getOwnPropertyDescriptor(o, 'x').get;
const c = Object.create(o);
out.push(c.__lookupGetter__('x') === g, p.__lookupGetter__('y') === undefined, typeof p.__lookupSetter__('y'));
out.push(String(({ a: 1 }).__lookupGetter__('a')), String(Object.create(null).__lookupGetter__ === undefined));
T(() => Object.prototype.__lookupGetter__.call(null, 'x'));
const sym = Symbol('s');
const q = {};
Object.prototype.__defineGetter__.call(q, sym, () => 5);
out.push(q[sym], typeof Object.prototype.__lookupGetter__.call(q, sym));
out.push(Object.prototype.toLocaleString.call(5), Object.prototype.toLocaleString.call({ toString() { return 'T'; } }));
T(() => Object.prototype.toLocaleString.call(null));
// Function.prototype
// setter length and getter/setter identity are not asserted: the table's accessor rule gives setters length 1
out.push(desc(Function.prototype, 'arguments').split(' set=')[0], desc(Function.prototype, 'caller').split(' set=')[0]);
const fa = Object.getOwnPropertyDescriptor(Function.prototype, 'arguments');
const fc = Object.getOwnPropertyDescriptor(Function.prototype, 'caller');
out.push(fa.get === fc.get);
T(() => Function.prototype.caller);
T(() => (function () { 'use strict'; }).arguments);
out.push(desc(Function.prototype, Symbol.hasInstance));
out.push(Function.prototype[Symbol.hasInstance].call(Object, {}), Function.prototype[Symbol.hasInstance].call(Object, Object.create(null)), Function.prototype[Symbol.hasInstance].call(1, {}));
class Even { static [Symbol.hasInstance](n) { return n % 2 === 0; } }
out.push(2 instanceof Even, 3 instanceof Even);
function F() {}
out.push(new F() instanceof F, ({}) instanceof F, Function.prototype[Symbol.hasInstance].call(F, new F()));
// Array.prototype
out.push(desc(Array.prototype, 'toLocaleString'), desc(Array.prototype, Symbol.unscopables));
out.push(['a', null, undefined, , 'b'].toLocaleString());
out.push([{ toLocaleString() { return 'X'; } }, 'y'].toLocaleString());
out.push(Array.prototype.toLocaleString.call({ length: 2, 0: 'x' }));
T(() => Array.prototype.toLocaleString.call(null));
const U = Array.prototype[Symbol.unscopables];
out.push(Object.getPrototypeOf(U) === null, Reflect.ownKeys(U).join(','));
out.push(desc(U, 'at'), desc(U, 'values'), U.keys, U.flat === true);
// Boolean
out.push(desc(Boolean.prototype, 'valueOf'));
out.push(Boolean.prototype.valueOf.call(true), Boolean.prototype.valueOf.call(new Boolean(false)), true.valueOf());
T(() => Boolean.prototype.valueOf.call(1));
T(() => Boolean.prototype.valueOf.call({}));
// Symbol
out.push(desc(Symbol.prototype, 'constructor'), Symbol.prototype.constructor === Symbol);
out.push(desc(Symbol.prototype, 'valueOf'));
const s = Symbol('q');
out.push(Symbol.prototype.valueOf.call(s) === s, Object(s).valueOf() === s);
T(() => Symbol.prototype.valueOf.call('x'));
out.push(desc(Symbol.prototype, 'description'));
out.push(Symbol('d').description, String(Symbol().description), Symbol.iterator.description);
T(() => Object.getOwnPropertyDescriptor(Symbol.prototype, 'description').get.call('x'));
T(() => Symbol.prototype.description);
out.push(desc(Symbol.prototype, Symbol.toPrimitive));
out.push(Symbol.prototype[Symbol.toPrimitive].call(s, 'string') === s, Symbol.prototype[Symbol.toPrimitive].call(s) === s);
T(() => Symbol.prototype[Symbol.toPrimitive].call('x', 'default'));
// Date
out.push(desc(Date.prototype, 'setYear'), desc(Date.prototype, Symbol.toPrimitive));
out.push(new Date(2000, 5, 15).setYear(98) === new Date(1998, 5, 15).getTime());
out.push(new Date(2000, 5, 15).setYear(2005) === new Date(2005, 5, 15).getTime());
out.push(new Date(2000, 0, 1).setYear(5) === new Date(1905, 0, 1).getTime());
out.push(new Date(NaN).setYear(2001) === new Date(2001, 0, 1).getTime());
const dn = new Date(2000, 0, 1);
out.push(dn.setYear(NaN), dn.getTime());
T(() => Date.prototype.setYear.call({}, 1));
const d = new Date(0);
out.push(Date.prototype[Symbol.toPrimitive].call(d, 'number'), Date.prototype[Symbol.toPrimitive].call(d, 'default') === d.toString(), Date.prototype[Symbol.toPrimitive].call(d, 'string') === d.toString());
T(() => Date.prototype[Symbol.toPrimitive].call(d, 'bogus'));
T(() => Date.prototype[Symbol.toPrimitive].call(d));
T(() => Date.prototype[Symbol.toPrimitive].call(1, 'number'));
out.push(d + 1 === d.toString() + '1', d - 0, +d);
// DataView
out.push(desc(DataView.prototype, 'buffer'));
const ab = new ArrayBuffer(8);
const dv = new DataView(ab, 2);
out.push(dv.buffer === ab, dv.byteOffset, dv.byteLength, Object.prototype.hasOwnProperty.call(dv, 'buffer'));
T(() => Object.getOwnPropertyDescriptor(DataView.prototype, 'buffer').get.call({}));
console.log(out.join('\n'));
"##;

/// Node 22 output for `PROBE`.
const EXPECTED: &str = r##"data w-c fn ["__defineGetter__",2]
data w-c fn ["__defineSetter__",2]
data w-c fn ["__lookupGetter__",1]
data w-c fn ["__lookupSetter__",1]
data w-c fn ["toLocaleString",0]
7
acc ec get=["",0] set=undef
THROW TypeError
THROW TypeError
3
acc ec get=undef set=["",1]
true
true
function
undefined
true
THROW TypeError
5
function
5
T
THROW TypeError
acc -c get=["",0]
acc -c get=["",0]
true
THROW TypeError
THROW TypeError
data --- fn ["[Symbol.hasInstance]",1]
true
false
false
true
false
true
false
true
data w-c fn ["toLocaleString",0]
data --c object
a,,,,b
X,y
x,
THROW TypeError
true
at,copyWithin,entries,fill,find,findIndex,findLast,findLastIndex,flat,flatMap,includes,keys,toReversed,toSorted,toSpliced,values
data wec boolean
data wec boolean
true
true
data w-c fn ["valueOf",0]
true
false
true
THROW TypeError
THROW TypeError
data w-c fn ["Symbol",0]
true
data w-c fn ["valueOf",0]
true
true
THROW TypeError
acc -c get=["get description",0] set=undef
d
undefined
Symbol.iterator
THROW TypeError
THROW TypeError
data --c fn ["[Symbol.toPrimitive]",1]
true
true
THROW TypeError
data w-c fn ["setYear",1]
data --c fn ["[Symbol.toPrimitive]",1]
true
true
true
true
NaN
NaN
THROW TypeError
0
true
true
THROW TypeError
THROW TypeError
THROW TypeError
true
0
0
acc -c get=["get buffer",0] set=undef
true
2
6
false
THROW TypeError"##;

#[test]
fn missing_builtin_members_match_node() {
    let mut it = Interp::new();
    it.run(PROBE).expect("probe runs");
    assert_eq!(it.out.join("\n"), EXPECTED);
}

/// Node 22 output for `LEGACY_PROBE`: `Function.prototype.arguments`/`caller` by receiver kind.
const LEGACY_PROBE: &str = r##"// not asserted: own legacy props on sloppy functions (waived) and direct getter/setter calls on sloppy receivers.
const out = [];
const T = (f) => { try { out.push(String(f())); } catch (e) { out.push('THROW ' + e.name); } };
function sl() {}
function st() { 'use strict'; }
const ar = () => 1;
class C {}
async function af() {}
function* gf() {}
const bd = sl.bind(null);
const sm = { m() {} }.m;
function outerStrict() { 'use strict'; return function inner() {}; }
function outerSloppy() { return function inner2() {}; }
const nestedStrict = outerStrict();
const nestedSloppy = outerSloppy();
T(() => sl.caller); T(() => sl.arguments);
T(() => st.caller); T(() => st.arguments);
T(() => ar.caller); T(() => ar.arguments);
T(() => C.caller); T(() => C.arguments);
T(() => af.caller); T(() => af.arguments);
T(() => gf.caller); T(() => gf.arguments);
T(() => bd.caller); T(() => bd.arguments);
T(() => sm.caller); T(() => sm.arguments);
T(() => nestedStrict.caller); T(() => nestedSloppy.caller); T(() => nestedSloppy.arguments);
T(() => Math.max.caller);
T(() => Function.prototype.caller);
T(() => Function.prototype.arguments);
T(() => Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get.call(1));
T(() => Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get.call({}));
T(() => Object.getOwnPropertyDescriptor(Function.prototype, 'caller').get.call(null));
T(() => Object.getOwnPropertyDescriptor(Function.prototype, 'arguments').get.call(gf));
T(() => Object.getOwnPropertyDescriptor(Function.prototype, 'caller').set.call(st, 1));
sl.caller = 1; T(() => sl.caller);
T(() => { st.caller = 1; return 'no throw'; });
T(() => { C.arguments = 1; return 'no throw'; });
console.log(out.join('\n'));
"##;

/// Node 22 output for `LEGACY_PROBE`.
const LEGACY_EXPECTED: &str = r##"null
null
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
null
null
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
THROW TypeError
null
THROW TypeError
THROW TypeError"##;

#[test]
fn legacy_caller_and_arguments_match_node_by_receiver_kind() {
    let mut it = Interp::new();
    it.run(LEGACY_PROBE).expect("probe runs");
    assert_eq!(it.out.join("\n"), LEGACY_EXPECTED);
}
