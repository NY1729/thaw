use super::*;

/// Run `src` and join its console.log lines with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

/// Function-kind tags: generator, async and async generator functions print their own tag.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(tag(function*(){}), tag(async function(){}), tag(async function*(){}));
///   console.log(tag(GF), tag(AF), tag(AGF));
///   console.log(GF[Symbol.toStringTag], AF[Symbol.toStringTag], AGF[Symbol.toStringTag]);
#[test]
fn generator_function_tags_match_node() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(tag(function*(){}), tag(async function(){}), tag(async function*(){}));
console.log(tag(GF), tag(AF), tag(AGF));
console.log(GF[Symbol.toStringTag], AF[Symbol.toStringTag], AGF[Symbol.toStringTag]);
"#),
        "[object GeneratorFunction] [object AsyncFunction] [object AsyncGeneratorFunction]|[object GeneratorFunction] [object AsyncFunction] [object AsyncGeneratorFunction]|GeneratorFunction AsyncFunction AsyncGeneratorFunction"
    );
}

/// The function kinds inherit from Function.prototype and own their %...Function.prototype% objects.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(Object.getPrototypeOf(GF) === Function.prototype, Object.getPrototypeOf(AF) === Function.prototype, Object.getPrototypeOf(AGF) === Function.prototype);
///   console.log(Object.getPrototypeOf(function*(){}) === GF, Object.getPrototypeOf(async function(){}) === AF, Object.getPrototypeOf(async function*(){}) === AGF);
///   console.log(GF.prototype === GP, GP[Symbol.toStringTag], GP.constructor === GF, AGF.prototype === AGP, AGP[Symbol.toStringTag]);
#[test]
fn generator_function_intrinsic_prototype_chain() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(Object.getPrototypeOf(GF) === Function.prototype, Object.getPrototypeOf(AF) === Function.prototype, Object.getPrototypeOf(AGF) === Function.prototype);
console.log(Object.getPrototypeOf(function*(){}) === GF, Object.getPrototypeOf(async function(){}) === AF, Object.getPrototypeOf(async function*(){}) === AGF);
console.log(GF.prototype === GP, GP[Symbol.toStringTag], GP.constructor === GF, AGF.prototype === AGP, AGP[Symbol.toStringTag]);
"#),
        "true true true|true true true|true Generator true true AsyncGenerator"
    );
}

/// %GeneratorPrototype% has constructor, next, return, throw and the Generator tag with Node's flags.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(d(GF, 'prototype'), d(GF, Symbol.toStringTag), d(GF, 'constructor'), d(GP, 'constructor'), d(GP, Symbol.toStringTag), d(GP, 'next'), d(GP, 'return'), d(GP, 'throw'));
///   console.log(Object.getPrototypeOf(GP) === ITP, typeof GP.next, GP.next.length, GP.next.name, GP.return.name, GP.throw.name);
///   console.log(JSON.stringify(Object.getOwnPropertyNames(GP).sort()), JSON.stringify(Object.getOwnPropertyNames(GF).sort()));
///   console.log(Object.getPrototypeOf(function*(){}).prototype.__proto__ === ITP);
#[test]
fn generator_prototype_surface_matches_node() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(d(GF, 'prototype'), d(GF, Symbol.toStringTag), d(GF, 'constructor'), d(GP, 'constructor'), d(GP, Symbol.toStringTag), d(GP, 'next'), d(GP, 'return'), d(GP, 'throw'));
console.log(Object.getPrototypeOf(GP) === ITP, typeof GP.next, GP.next.length, GP.next.name, GP.return.name, GP.throw.name);
console.log(JSON.stringify(Object.getOwnPropertyNames(GP).sort()), JSON.stringify(Object.getOwnPropertyNames(GF).sort()));
console.log(Object.getPrototypeOf(function*(){}).prototype.__proto__ === ITP);
"#),
        "data;object;--c data;string;--c data;function;--c data;object;--c data;string;--c data;function;w-c data;function;w-c data;function;w-c|true function 1 next return throw|[\"constructor\",\"next\",\"return\",\"throw\"] [\"constructor\",\"prototype\"]|true"
    );
}

/// GeneratorFunction, AsyncFunction and AsyncGeneratorFunction are reachable only through .constructor.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(typeof GeneratorFunction, typeof AsyncFunction, typeof AsyncGeneratorFunction);
///   console.log((function*(){}).constructor.name, (async function(){}).constructor.name, (async function*(){}).constructor.name);
///   console.log(GF.constructor.name, GF.constructor.length, Object.getPrototypeOf(GF.constructor) === Function, GF.constructor.prototype === GF, d(GF.constructor, 'prototype'));
///   console.log([...new GF.constructor('a', 'yield a; yield a + 1')(10)].join());
#[test]
fn generator_constructors_are_not_globals() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(typeof GeneratorFunction, typeof AsyncFunction, typeof AsyncGeneratorFunction);
console.log((function*(){}).constructor.name, (async function(){}).constructor.name, (async function*(){}).constructor.name);
console.log(GF.constructor.name, GF.constructor.length, Object.getPrototypeOf(GF.constructor) === Function, GF.constructor.prototype === GF, d(GF.constructor, 'prototype'));
console.log([...new GF.constructor('a', 'yield a; yield a + 1')(10)].join());
"#),
        "undefined undefined undefined|GeneratorFunction AsyncFunction AsyncGeneratorFunction|GeneratorFunction 1 true true data;object;---|10,11"
    );
}

/// A generator object's chain is instance -> fn.prototype -> %GeneratorPrototype% -> %IteratorPrototype%.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((function*(){})()))) === ITP);
///   console.log(Object.getPrototypeOf(Object.getPrototypeOf((function*(){})())) === ITP);
///   const it = (function*(){ yield 1; })(); console.log(tag(it), JSON.stringify(it.next()), it[Symbol.iterator]() === it);
#[test]
fn generator_instances_reach_iterator_prototype() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((function*(){})()))) === ITP);
console.log(Object.getPrototypeOf(Object.getPrototypeOf((function*(){})())) === ITP);
const it = (function*(){ yield 1; })(); console.log(tag(it), JSON.stringify(it.next()), it[Symbol.iterator]() === it);
"#),
        "true|false|[object Generator] {\"value\":1,\"done\":false} true"
    );
}

/// Array iterators read the live array (index and length) on every next(), like V8.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   const a = [1]; const i = a.values(); i.next(); a.push(2); console.log(JSON.stringify(i.next()), JSON.stringify(i.next()));
///   const b = [1,2]; const e = b.entries(); e.next(); b.push(3); console.log(JSON.stringify(e.next()), JSON.stringify(e.next()), JSON.stringify(e.next()));
///   const k = [1]; const kk = k.keys(); kk.next(); k.push(9); console.log(JSON.stringify(kk.next()), JSON.stringify(kk.next()));
///   const c = [1,2,3]; const it2 = c.values(); it2.next(); c.length = 1; console.log(JSON.stringify(it2.next()));
#[test]
fn array_iterator_sees_mutation_lazily() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
const a = [1]; const i = a.values(); i.next(); a.push(2); console.log(JSON.stringify(i.next()), JSON.stringify(i.next()));
const b = [1,2]; const e = b.entries(); e.next(); b.push(3); console.log(JSON.stringify(e.next()), JSON.stringify(e.next()), JSON.stringify(e.next()));
const k = [1]; const kk = k.keys(); kk.next(); k.push(9); console.log(JSON.stringify(kk.next()), JSON.stringify(kk.next()));
const c = [1,2,3]; const it2 = c.values(); it2.next(); c.length = 1; console.log(JSON.stringify(it2.next()));
"#),
        "{\"value\":2,\"done\":false} {\"done\":true}|{\"value\":[1,2],\"done\":false} {\"value\":[2,3],\"done\":false} {\"done\":true}|{\"value\":1,\"done\":false} {\"done\":true}|{\"done\":true}"
    );
}

/// Buffer instances carry the TypedArray tag of their [[TypedArrayName]], not the constructor name.
///
/// Node 22 (`node -e` of the script below, one console.log per line, joined with `|`):
///
///   const tag = o => Object.prototype.toString.call(o);
///   const GF = Object.getPrototypeOf(function*(){});
///   const AF = Object.getPrototypeOf(async function(){});
///   const AGF = Object.getPrototypeOf(async function*(){});
///   const GP = GF.prototype;
///   const AGP = AGF.prototype;
///   const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
///   const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
///   console.log(tag(Buffer.alloc(1)), tag(new Uint8Array(1)), tag(new (class X extends Uint8Array {})(1)), tag(Buffer.from('ab')));
///   console.log(Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get.call(Buffer.alloc(1)));
///   console.log(Object.prototype.toString.call(new Float64Array(1)), tag(new BigInt64Array(1)));
///   console.log(Object.getPrototypeOf(Buffer.alloc(1)).constructor.name, Buffer.alloc(1)[Symbol.toStringTag]);
#[test]
fn buffer_instance_tag_is_uint8array() {
    assert_eq!(
        one(r#"const tag = o => Object.prototype.toString.call(o);
const GF = Object.getPrototypeOf(function*(){});
const AF = Object.getPrototypeOf(async function(){});
const AGF = Object.getPrototypeOf(async function*(){});
const GP = GF.prototype;
const AGP = AGF.prototype;
const ITP = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));
const d = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); if (!x) return 'absent'; return ('value' in x ? 'data;' + typeof x.value + ';' : 'accessor;') + (x.writable ? 'w' : '-') + (x.enumerable ? 'e' : '-') + (x.configurable ? 'c' : '-'); };
console.log(tag(Buffer.alloc(1)), tag(new Uint8Array(1)), tag(new (class X extends Uint8Array {})(1)), tag(Buffer.from('ab')));
console.log(Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag).get.call(Buffer.alloc(1)));
console.log(Object.prototype.toString.call(new Float64Array(1)), tag(new BigInt64Array(1)));
console.log(Object.getPrototypeOf(Buffer.alloc(1)).constructor.name, Buffer.alloc(1)[Symbol.toStringTag]);
"#),
        "[object Uint8Array] [object Uint8Array] [object Uint8Array] [object Uint8Array]|Uint8Array|[object Float64Array] [object BigInt64Array]|Buffer Uint8Array"
    );
}
