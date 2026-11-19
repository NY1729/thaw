//! Symbol identity across the Interps of one thread (symbol_table.rs). A registry package and its subpaths
//! are separate Interps that share objects as the same Rc, so a symbol key made in one must name the same
//! symbol in the other.

use super::*;

fn value(it: &mut Interp, src: &str) -> Value {
    match it.run(src) {
        Ok(v) => v,
        Err(e) => panic!("script failed: {src}: {e:?}"),
    }
}

fn text(it: &mut Interp, src: &str) -> String {
    let v = value(it, src);
    it.to_str(&v).to_string()
}

/// Hands `v` to `it` as the binding `name`, the way a second package receives a shared object.
fn hand_over(it: &mut Interp, name: &str, v: Value) {
    it.global.declare(name, v, true);
}

#[test]
fn registered_symbol_key_resolves_in_another_interp() {
    let mut a = Interp::runtime();
    let mut b = Interp::runtime();
    let table = value(&mut a, "({[Symbol.for('drizzle:Columns')]: 7})");
    hand_over(&mut b, "table", table.clone());
    let sym = value(&mut b, "Symbol.for('drizzle:Columns')");
    let key = b.key_of(&sym);
    let got = b.get(&table, &key).expect("get");
    assert!(matches!(got, Value::Number(n) if n == 7.0));
}

#[test]
fn unregistered_symbol_key_resolves_in_another_interp() {
    let mut a = Interp::runtime();
    let mut b = Interp::runtime();
    let obj = value(
        &mut a,
        "(function () { const s = Symbol('x'); return { [s]: 5, sym: s }; })()",
    );
    hand_over(&mut b, "obj", obj);
    assert_eq!(
        text(&mut b, "Object.getOwnPropertySymbols(obj)[0].description"),
        "x"
    );
    assert_eq!(
        text(&mut b, "obj[Object.getOwnPropertySymbols(obj)[0]]"),
        "5"
    );
    assert_eq!(
        text(
            &mut b,
            "String(Object.getOwnPropertySymbols(obj)[0] === obj.sym)"
        ),
        "true"
    );
}

#[test]
fn symbols_minted_in_two_interps_never_share_an_id() {
    let mut a = Interp::runtime();
    let mut b = Interp::runtime();
    let mut keys = std::collections::HashSet::new();
    for _ in 0..4 {
        let sa = value(&mut a, "Symbol('a')");
        let ka = a.key_of(&sa);
        assert!(keys.insert(ka.to_string()), "duplicate key {ka}");
        let sb = value(&mut b, "Symbol('b')");
        let kb = b.key_of(&sb);
        assert!(keys.insert(kb.to_string()), "duplicate key {kb}");
    }
    // the well-known symbols are one set per thread
    let ia = value(&mut a, "Symbol.iterator");
    let ib = value(&mut b, "Symbol.iterator");
    assert_eq!(a.key_of(&ia), b.key_of(&ib));
}

#[test]
fn shared_symbols_outlive_the_interp_that_made_them() {
    let mut a = Interp::runtime();
    let mut b = Interp::runtime();
    let obj = value(&mut a, "({[Symbol.for('keep')]: 9, [Symbol('local')]: 1})");
    hand_over(&mut b, "obj", obj);
    drop(a);
    assert_eq!(text(&mut b, "String(obj[Symbol.for('keep')])"), "9");
    assert_eq!(
        text(&mut b, "Object.getOwnPropertySymbols(obj)[1].description"),
        "local"
    );
    assert_eq!(
        text(&mut b, "Symbol.iterator.toString()"),
        "Symbol(Symbol.iterator)"
    );
}
