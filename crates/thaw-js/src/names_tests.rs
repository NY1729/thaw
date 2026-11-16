//! Function `name`/`length` family: typed-array statics and `set` shapes, and the `name` a method,
//! accessor, class member or key-value function takes from a computed symbol key (`[desc]`, `get [desc]`).
//! Expected lines are recorded from Node 22 (`node -e`), one `console.log` per line.

use super::*;

/// Run `src` and return its console.log lines.
fn lines(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.clone(),
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
}

fn expect(src: &str, want: &[&str]) {
    let got = lines(src);
    let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
}

#[test]
fn typed_from_and_of_are_inherited_from_the_typed_array_constructor() {
    expect(
        r#"
const TA = Object.getPrototypeOf(Uint8Array);
const desc = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); return x ? JSON.stringify([x.value, x.writable, x.enumerable, x.configurable]) : 'none'; };
console.log('from', Uint8Array.from.name, Uint8Array.from.length, Object.getOwnPropertyNames(Uint8Array).includes('from'), TA.from === Uint8Array.from, Int8Array.from === Uint8Array.from, desc(TA, 'from'));
console.log('of', Uint8Array.of.name, Uint8Array.of.length, Object.getOwnPropertyNames(Uint8Array).includes('of'), desc(TA, 'of'));
"#,
        &[
            "from from 1 false true true [null,true,false,true]",
            "of of 0 false [null,true,false,true]",
        ],
    );
}

#[test]
fn typed_set_has_node_name_and_length() {
    expect(
        r#"
const desc = (o, k) => { const x = Object.getOwnPropertyDescriptor(o, k); return x ? JSON.stringify([x.value, x.writable, x.enumerable, x.configurable]) : 'none'; };
const TA = Object.getPrototypeOf(Uint8Array);
console.log('set', Uint8Array.prototype.set.name, Uint8Array.prototype.set.length, desc(Uint8Array.prototype.set, 'name'), desc(Uint8Array.prototype.set, 'length'));
console.log('setown', Object.getOwnPropertyNames(Uint8Array.prototype).includes('set'), desc(TA.prototype, 'set'));
"#,
        &[
            r#"set set 1 ["set",false,false,true] [1,false,false,true]"#,
            "setown false [null,true,false,true]",
        ],
    );
}

#[test]
fn computed_symbol_method_names_use_the_bracketed_description() {
    expect(
        r#"
const d = Symbol('d');
const emp = Symbol();
const o1 = { [d]() {} };
const o2 = { [emp]() {} };
const o5 = { async [d]() {}, *[d]() {}, async *[d]() {} };
const o6 = { [Symbol.iterator]() {}, [Symbol.for('f')]() {} };
console.log('obj method', o1[d].name, JSON.stringify(o2[emp].name));
console.log('obj async', o5[d].name);
console.log('wk', o6[Symbol.iterator].name, o6[Symbol.for('f')].name);
"#,
        &[
            "obj method [d] \"\"",
            "obj async [d]",
            "wk [Symbol.iterator] [f]",
        ],
    );
}

#[test]
fn computed_symbol_accessor_names_carry_the_get_and_set_prefix() {
    expect(
        r#"
const d = Symbol('d');
const o3 = { get [d]() { return 1; } };
const o4 = { set [d](v) {} };
const o6 = { get [Symbol.asyncIterator]() { return 1; } };
console.log('obj getter', Object.getOwnPropertyDescriptor(o3, d).get.name, Object.getOwnPropertyDescriptor(o4, d).set.name);
console.log('wk getter', Object.getOwnPropertyDescriptor(o6, Symbol.asyncIterator).get.name);
"#,
        &[
            "obj getter get [d] set [d]",
            "wk getter get [Symbol.asyncIterator]",
        ],
    );
}

#[test]
fn key_value_functions_under_symbol_keys_take_the_bracketed_name() {
    expect(
        r#"
const d = Symbol('d');
const emp = Symbol();
const o7 = { [d]: function () {}, [Symbol('x y')]: () => 1, [Symbol.toPrimitive]: class {}, [emp]: function () {} };
console.log('kv', o7[d].name, o7[Symbol.toPrimitive].name, Object.getOwnPropertySymbols(o7).map((s) => JSON.stringify(o7[s].name)).join(','));
const o8 = { [d]: function named() {} };
console.log('kv named', o8[d].name);
"#,
        &[
            "kv [d] [Symbol.toPrimitive] \"[d]\",\"[x y]\",\"[Symbol.toPrimitive]\",\"\"",
            "kv named named",
        ],
    );
}

#[test]
fn class_members_under_symbol_keys_take_the_bracketed_name() {
    expect(
        r#"
const d = Symbol('d');
const emp = Symbol();
class C {
  [d]() {}
  static [emp]() {}
  get [Symbol.toStringTag]() { return 'C'; }
  set [Symbol.toStringTag](v) {}
  static async *[Symbol.asyncIterator]() {}
  [Symbol.iterator]() {}
  static get [Symbol.species]() { return 1; }
  *[Symbol.for('gen')]() {}
}
console.log('class', C.prototype[d].name, JSON.stringify(C[emp].name), C.prototype[Symbol.iterator].name, C[Symbol.asyncIterator].name);
console.log('class acc', Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag).get.name, Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag).set.name, Object.getOwnPropertyDescriptor(C, Symbol.species).get.name, C.prototype[Symbol.for('gen')].name);
class G { static [d]() {} }
console.log('static', G[d].name);
"#,
        &[
            "class [d] \"\" [Symbol.iterator] [Symbol.asyncIterator]",
            "class acc get [Symbol.toStringTag] set [Symbol.toStringTag] get [Symbol.species] [gen]",
            "static [d]",
        ],
    );
}

#[test]
fn class_fields_under_symbol_keys_take_the_bracketed_name() {
    expect(
        r#"
const d = Symbol('d');
const emp = Symbol();
class F { [d] = () => 1; static [emp] = function () {}; static [Symbol.iterator] = () => 2; }
console.log('fields', new F()[d].name, JSON.stringify(F[emp].name), F[Symbol.iterator].name);
"#,
        &["fields [d] \"\" [Symbol.iterator]"],
    );
}

#[test]
fn typed_array_constructor_is_the_abstract_parent_of_every_typed_array() {
    expect(
        r#"
const TA = Object.getPrototypeOf(Uint8Array);
console.log('chain', Object.getPrototypeOf(Float64Array) === TA, TA.prototype.constructor === TA, new Uint8Array(2) instanceof TA, TA.name, TA.length);
try { new TA(); } catch (e) { console.log('abs', e.constructor.name, e.message); }
try { TA(); } catch (e) { console.log('abs call', e.constructor.name, e.message); }
console.log('from-this', Uint8Array.from.call(Int8Array, [1, 2]).constructor.name, Uint8Array.from([3, 4], (x) => x * 2).join(','), Uint8Array.of(7, 8).join(','));
console.log('set-len', Uint8Array.prototype.set.length, Object.getOwnPropertyNames(Uint8Array.prototype.set).join(','));
console.log('at', TA.prototype.at.name, TA.prototype.at.length, 'prototype' in TA.prototype.at, 'prototype' in TA.prototype.set);
"#,
        &[
            "chain true true true TypedArray 0",
            "abs TypeError Abstract class TypedArray not directly constructable",
            "abs call TypeError Abstract class TypedArray not directly constructable",
            "from-this Int8Array 6,8 7,8",
            "set-len 1 length,name",
            "at at 1 false false",
        ],
    );
}

#[test]
fn private_member_and_field_functions_take_the_hash_name() {
    expect(
        r#"
class A { #m() {} #f = () => 1; static #s = function () {}; getM() { return this.#m; } getF() { return this.#f; } static getS() { return A.#s; } }
const a = new A();
console.log('priv', a.getM().name, a.getF().name, A.getS().name);
"#,
        &["priv #m #f #s"],
    );
}

#[test]
fn field_functions_under_symbol_and_plain_keys_take_their_key_name() {
    expect(
        r#"
const emp = Symbol();
class B { [emp] = () => 1; static [Symbol.for('q')] = () => 2; x = () => 3; static y = function () {}; }
console.log('fields2', new B()[emp].name === '', B[Symbol.for('q')].name, new B().x.name, B.y.name);
const o = { [emp]: () => 1, [Symbol.for('')]() {}, get [Symbol()]() { return 1; } };
console.log('kv2', JSON.stringify(o[emp].name), Object.getOwnPropertySymbols(o).map((s) => JSON.stringify(o[s].name ?? Object.getOwnPropertyDescriptor(o, s).get.name)).join(','));
"#,
        &["fields2 true [q] x y", "kv2 \"\" \"\",\"[]\",\"get \""],
    );
}
