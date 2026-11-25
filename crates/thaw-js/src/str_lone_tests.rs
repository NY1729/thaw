//! Lone-surrogate encoding: every real code point U+10F800..=U+10FFFF must stay distinct from the lone
//! surrogate units the string layer keeps at the same chars (it stores the real ones as two lone chars).
use super::*;
use crate::builtins::{from_scalar, str_code_points, str_units, to_scalar, units_str};

fn run(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("\n"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

/// Node 22 output of the script (`node probe.js`), covering length, codePointAt, equality, isWellFormed,
/// JSON, Map/Set/object keys, iteration, slicing across the pair and re-joining.
#[test]
fn high_plane_code_points_match_node() {
    let script = r#"
const F = String.fromCodePoint, C = String.fromCharCode, L = F(0x10ffff), S = 'a' + L + 'b';
const show = (n, v) => console.log(n + '=' + JSON.stringify(v));
show('len', [L.length, '\u{10ffff}'.length, F(0x10f800).length, S.length]);
show('cp', [F(0x10f800).codePointAt(0) === 0x10f800, L.codePointAt(0), L.codePointAt(1), L.charCodeAt(0), L.charCodeAt(1)]);
show('eq', [C(0xd800) === F(0x10f800), C(0xdbfe, 0xdc00) === F(0x10f800), C(0xdbff) + C(0xdfff) === L, '\udbff\udfff' === L]);
show('wf', [L.isWellFormed(), L.toWellFormed().length, C(0xd800).isWellFormed(), C(0xd800).toWellFormed() === '�']);
show('json', [JSON.stringify(L).length, JSON.stringify(C(0xd800)), JSON.parse('"\\udbff\\udfff"') === L, JSON.parse(JSON.stringify(S)) === S, JSON.parse('"\\ud800"').length]);
const m = new Map([[C(0xdfff), 1]]);
show('map', [m.has(L), m.has(C(0xdfff)), new Set([L, C(0xdfff), F(0x10f800)]).size]);
const o = {}; o[C(0xdfff)] = 1; o[L] = 2;
show('key', [Object.keys(o).length, L in o, o[L], o[C(0xdfff)]]);
const forof = []; for (const c of S) forof.push(c.length);
show('iter', [forof, [...S].map(c => c.length), Array.from(L + F(0x10f800)).map(c => c.codePointAt(0)), S.split('').length]);
show('slice', [S.slice(1, 2).charCodeAt(0), S.slice(2, 3).charCodeAt(0), S.slice(1, 2) + S.slice(2, 3) === L, S.substring(1, 3) === L, S.charAt(1).length, S.at(2).charCodeAt(0), S.slice(1, 3) === L, S.substr(2, 1).length]);
show('concat', [(C(0xdbfe) + C(0xdc00)).codePointAt(0), (C(0xd83d) + C(0xde00)) === '\u{1f600}', (C(0xd83d) + C(0xde00)).length, `${L}`.length, (L + L).length, (L + C(0xdc00)).length]);
show('search', [S.indexOf('b'), S.lastIndexOf('b'), S.indexOf(L), S.includes(L), S.indexOf(C(0xdfff))]);
show('misc', [structuredClone(L).length, eval("'" + L + "'").length, L.padEnd(3, 'x').length, L.normalize().length, L.toUpperCase().length, L.concat('').length, S.replace('b', 'c').length]);
show('lit', ['\ud800'.length, '\u{10ffff}' === L, `\u{10ffff}` === L, ({ '\u{10ffff}': 1 })[L], String.raw`\u{10ffff}${L}`.length]);
"#;
    let expected = r#"
len=[2,2,2,4]
cp=[true,1114111,57343,56319,57343]
eq=[false,true,true,true]
wf=[true,2,false,true]
json=[4,"\"\\ud800\"",true,true,1]
map=[false,true,3]
key=[2,true,2,1]
iter=[[1,2,1],[1,2,1],[1114111,1112064],4]
slice=[56319,57343,true,true,1,57343,true,1]
concat=[1112064,true,2,2,4,3]
search=[3,3,1,true,2]
misc=[2,2,3,2,2,2,4]
lit=[1,true,true,1,12]
"#;
    assert_eq!(run(script), expected.trim());
}

/// Every one of the 2048 real code points round-trips and is distinct from every lone surrogate unit.
#[test]
fn every_high_plane_code_point_is_injective() {
    let mut seen = std::collections::HashSet::new();
    for cp in 0x10F800u32..=0x10FFFF {
        let real = char::from_u32(cp).unwrap().to_string();
        let stored = from_scalar(&real).into_owned();
        assert_eq!(str_units(&stored).len(), 2, "{cp:x}");
        assert_eq!(to_scalar(&stored), real, "{cp:x}");
        assert_eq!(str_code_points(&stored), vec![stored.clone()], "{cp:x}");
        let v = cp - 0x10000;
        let units = [(0xD800 + (v >> 10)) as u16, (0xDC00 + (v & 0x3FF)) as u16];
        assert_eq!(str_units(&stored), units);
        assert_eq!(units_str(&units), stored);
        // slicing in the middle and joining back
        let (a, b) = (units_str(&units[..1]), units_str(&units[1..]));
        let mut joined = a.clone();
        crate::builtins::push_joined(&mut joined, &b);
        assert_eq!(joined, stored);
        assert!(seen.insert(stored));
    }
    for unit in 0xD800u32..0xE000 {
        let lone = units_str(&[unit as u16]);
        assert_eq!(str_units(&lone), vec![unit as u16]);
        assert!(
            !seen.contains(&lone),
            "{unit:x} collides with a real code point"
        );
    }
}

/// A raw real U+10FFFF in a source file is stored as its two lone chars once the host enters it with `from_scalar`.
#[test]
fn raw_high_plane_char_in_source_is_two_units() {
    let src = from_scalar(
        "console.log('\u{10ffff}'.length, '\u{10ffff}'.codePointAt(0), `\u{10f800}`.length);",
    );
    assert_eq!(run(&src), "2 1114111 2");
}

/// Ordinary sources (ASCII, BMP, astral below U+10F800) take the borrowed fast path: no rewrite, so spans and
/// `Function.prototype.toString` are untouched.
#[test]
fn ordinary_sources_are_not_rewritten() {
    for src in [
        "function f() { return 1; }",
        "var s = 'é€😀';",
        "var t = '\u{10f7ff}';",
    ] {
        assert!(
            matches!(
                crate::builtins::to_source(src),
                std::borrow::Cow::Borrowed(_)
            ),
            "{src}"
        );
        assert!(
            matches!(from_scalar(src), std::borrow::Cow::Borrowed(_)),
            "{src}"
        );
    }
    assert_eq!(
        run("const k = eval('(function k(){ return \"é😀\"; })'); console.log(k.toString())"),
        "function k(){ return \"é😀\"; }"
    );
}
