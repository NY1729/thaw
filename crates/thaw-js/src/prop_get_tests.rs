use super::*;

/// Run `src` and return the console.log lines (or the uncaught error text).
fn run(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out,
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out
        }
        Err(Ctl::Unsupported(f)) => vec![format!("unsupported: {f}")],
        Err(_) => vec!["stray completion".into()],
    }
}

fn one(src: &str) -> String {
    run(src).join("|")
}

// Guards for the property-get fast path: each expectation matches Node.

#[test]
fn prop_get_accessors_run_with_receiver_this() {
    assert_eq!(
        one("const base = { get v() { return this.n * 2; }, set v(x) { this.n = x; } }; const d = Object.create(base); d.n = 4; console.log(d.v, base.v, d.n);"),
        "8 NaN 4"
    );
    assert_eq!(
        one("const o = { set w(x) {} }; console.log(o.w, 'w' in o);"),
        "undefined true"
    );
    assert_eq!(
        one("const o = { get g() { throw new Error('boom'); } }; try { o.g } catch (e) { console.log(e.message) }"),
        "boom"
    );
    assert_eq!(
        one("Object.defineProperty(Number.prototype, 'twice', { get() { return this * 2; }, configurable: true }); console.log((4).twice);"),
        "8"
    );
    assert_eq!(
        one("Object.defineProperty(String.prototype, 'first', { get() { return this[0]; }, configurable: true }); console.log('xyz'.first);"),
        "x"
    );
}

#[test]
fn prop_get_class_getters_and_statics() {
    assert_eq!(
        one("class A { get x() { return 'A'; } } class B extends A { get x() { return 'B' + super.x; } } console.log(new B().x);"),
        "BA"
    );
    assert_eq!(
        one("class S { static get k() { return this.name + '!'; } } class T extends S {} console.log(S.k, T.k);"),
        "S! T!"
    );
    assert_eq!(
        one("class P { constructor() { this.v = 1; } get dbl() { return this.v * 2; } m() { return 'm'; } } const i = new P(); console.log(i.dbl, i.m(), typeof i.m, typeof P.prototype.m, 'dbl' in i);"),
        "2 m function function true"
    );
    assert_eq!(
        one("class MA extends Array {} const m = new MA(); m.push(4); console.log(m[0], m.length, m[1]);"),
        "4 1 undefined"
    );
}

#[test]
fn prop_get_prototype_chain_and_shadowing() {
    assert_eq!(
        one("const a = { p: 1 }; const b = Object.create(a); const c = Object.create(b); console.log(c.p, c.q, b.p); b.p = 2; console.log(c.p, a.p);"),
        "1 undefined 1|2 1"
    );
    assert_eq!(
        one("const bp = {}; for (let i = 0; i < 12; i++) bp['p' + i] = i; const bc = Object.create(bp); bc.own = 1; console.log(bc.p11, bc.own, bc.p12);"),
        "11 1 undefined"
    );
    assert_eq!(
        one("const big = {}; for (let i = 0; i < 12; i++) big['k' + i] = i; Object.defineProperty(big, 'g', { get() { return 'G' + this.k3; } }); console.log(big.k11, big.g, big.k0, big.nope);"),
        "11 G3 0 undefined"
    );
    assert_eq!(
        one("const q = Object.setPrototypeOf({}, { x: 1 }); console.log(q.x, ({}).__proto__ === Object.prototype, Object.create(null).__proto__, Object.getPrototypeOf(q).x);"),
        "1 true undefined 1"
    );
}

#[test]
fn prop_get_array_index_length_and_holes() {
    assert_eq!(
        one("const a = [1, , 3]; a.extra = 'x'; console.log(a[0], a[1], a[2], a[3], a.length, a.extra, a['-1'], a[1.5], a['1e0']);"),
        "1 undefined 3 undefined 3 x undefined undefined undefined"
    );
    assert_eq!(
        one("const a = []; a.push(1, 2); a.length = 1; console.log(a.length, a[1], a[0]);"),
        "1 undefined 1"
    );
    assert_eq!(
        one("const a2 = ['p', 'q']; console.log(a2['0'], a2['1'], a2['2'], a2['99999999999999999999']);"),
        "p q undefined undefined"
    );
    assert_eq!(
        one("const n = { 1: 'x', '01': 'y' }; console.log(n[1], n['1'], n['01']);"),
        "x x y"
    );
}

#[test]
fn prop_get_typed_arrays_and_array_buffers() {
    assert_eq!(
        one("const u = new Uint8Array([7, 8, 9]); console.log(u.length, u.byteLength, u.byteOffset, u[2], u[3], u.buffer === u.buffer, u.buffer.byteLength, u.foo);"),
        "3 3 0 9 undefined true 3 undefined"
    );
    assert_eq!(
        one("const f = new Float64Array(new ArrayBuffer(32), 8, 2); console.log(f.length, f.byteLength, f.byteOffset, f[1], f[2]);"),
        "2 16 8 0 undefined"
    );
    assert_eq!(
        one("console.log(new ArrayBuffer(12).byteLength, new ArrayBuffer(4).length);"),
        "12 undefined"
    );
}

#[test]
fn prop_get_string_wrapper_and_primitives() {
    assert_eq!(
        one("const s = 'héllo'; console.log(s.length, s[1], s[5], s.foo, typeof s.toUpperCase, s[0].length);"),
        "5 é undefined undefined function 1"
    );
    assert_eq!(
        one("const e = 'a\u{1F600}'; console.log(e.length, e[1] === '\\ud83d', e[2] === '\\ude00');"),
        "3 true true"
    );
    assert_eq!(
        one("const w = new String('ab'); console.log(w.length, w[0], w[2], typeof w.charAt);"),
        "2 a undefined function"
    );
    assert_eq!(
        one("console.log((5).x, (5).toFixed(1), true.y, typeof (5).toString);"),
        "undefined 5.0 undefined function"
    );
}

#[test]
fn prop_get_symbols_and_well_known() {
    assert_eq!(
        one("const s = Symbol('k'); const o = { [s]: 'own' }; const p = Object.create({ [s]: 'inh' }); console.log(o[s], p[s], Symbol.iterator in [], [][Symbol.iterator] === Array.prototype[Symbol.iterator], ({})[Symbol('z')]);"),
        "own inh true true undefined"
    );
}

#[test]
fn prop_get_proxy_traps_and_forwarding() {
    assert_eq!(
        one("const t = { a: 1 }; const p = new Proxy(t, { get(tg, k, r) { return k === 'a' ? 'trap' : Reflect.get(tg, k, r); } }); console.log(p.a, p.b, p.c === undefined);"),
        "trap undefined true"
    );
    assert_eq!(
        one("const p2 = new Proxy({ z: 9 }, {}); console.log(p2.z, p2.y);"),
        "9 undefined"
    );
    assert_eq!(
        one("const p3 = new Proxy({}, { get(t, k, r) { return r === p3 ? 'recv-ok' : 'bad'; } }); console.log(p3.anything);"),
        "recv-ok"
    );
    assert_eq!(
        one("const pt = new Proxy({ get g() { return this.k; }, k: 'K' }, {}); console.log(pt.g);"),
        "K"
    );
    assert_eq!(
        one("const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); try { proxy.x } catch (e) { console.log(e instanceof TypeError); }"),
        "true"
    );
}

#[test]
fn prop_get_globals_closures_and_errors() {
    assert_eq!(
        one("globalThis.gv = 7; console.log(gv, typeof gx, globalThis.gv);"),
        "7 undefined 7"
    );
    assert_eq!(
        one("function f(a, b = 1, c) {} const g = (x, y) => x; console.log(f.name, f.length, g.length, (function () {}).name === '', (() => 1).length);"),
        "f 1 2 true 0"
    );
    assert_eq!(
        one("function q() {} Object.defineProperty(q, 'name', { value: 'Z' }); console.log(q.name, q.length);"),
        "Z 0"
    );
    assert_eq!(one("class K {} console.log(K.name, K.length);"), "K 0");
    assert_eq!(
        one("try { null.x } catch (e) { console.log(e.message) }"),
        "Cannot read properties of null (reading 'x')"
    );
    assert_eq!(
        one("try { undefined.y } catch (e) { console.log(e.message) }"),
        "Cannot read properties of undefined (reading 'y')"
    );
}
