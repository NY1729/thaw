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

// Expectations are Node 22 (v22.23.1) output: ToString(object) runs ToPrimitive, and an exception
// thrown by a user `toString` / `valueOf` / `@@toPrimitive` propagates out of String(), template
// literals, `+` and array join alike.

#[test]
fn string_call_propagates_tostring_throw() {
    assert_eq!(
        one("try { String({toString(){throw new Error('x')}}); console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught x"
    );
}

#[test]
fn template_literal_propagates_tostring_throw() {
    assert_eq!(
        one("try { `${{toString(){throw new Error('t')}}}`; console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught t"
    );
}

#[test]
fn template_literal_propagates_toprimitive_throw() {
    assert_eq!(
        one("try { `${{[Symbol.toPrimitive](){throw new Error('p')}}}`; console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught p"
    );
}

#[test]
fn string_call_propagates_valueof_fallback_throw() {
    // toString returns an object, so ToPrimitive falls through to valueOf, which throws.
    assert_eq!(
        one("try { String({toString(){return {}}, valueOf(){throw new Error('v')}}); console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught v"
    );
}

#[test]
fn plus_concat_propagates_tostring_throw() {
    assert_eq!(
        one("try { '' + {toString(){throw new Error('c')}}; console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught c"
    );
}

#[test]
fn plus_and_template_reject_symbol_operands_but_string_keeps_them() {
    // Node: `${sym}`, `'' + sym` and `sym + ''` throw TypeError; String(sym) is 'Symbol(x)'.
    assert_eq!(
        one("var s = Symbol('x'); var r = []; [function () { return `${s}`; }, function () { return '' + s; }, function () { return s + ''; }].forEach(function (f) { try { f(); r.push('no'); } catch (e) { r.push(e.name); } }); console.log(r.join(','), String(s));"),
        "TypeError,TypeError,TypeError Symbol(x)"
    );
}

#[test]
fn array_join_propagates_element_tostring_throw() {
    assert_eq!(
        one("try { String([{toString(){throw new Error('e')}}]); console.log('no'); } catch (e) { console.log('caught', e.message); }"),
        "caught e"
    );
}

#[test]
fn string_of_object_without_primitive_conversion_throws_type_error() {
    assert_eq!(
        one("try { String(Object.create(null)); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot convert object to primitive value"
    );
}

#[test]
fn ordinary_objects_still_stringify() {
    assert_eq!(
        one("console.log(String({}), `${{}}`, String({toString(){return 'ok'}}), `a${[1,{toString(){return 'b'}}]}`, String([1,[2,3]]), String(function f(){}).slice(0, 10));"),
        "[object Object] [object Object] ok a1,b 1,2,3 function f"
    );
}
