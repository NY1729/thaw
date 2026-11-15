use super::*;

/// Run `src` as the main script, then the event loop; return the stdout lines joined with `|`.
fn run_loop(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("script runs");
    it.run_event_loop().expect("loop runs");
    it.out.join("|")
}

// Legacy `RegExp.$1`-style statics. Expected values were recorded with Node v22.23.1.

#[test]
fn regexp_statics_after_exec_like_node() {
    assert_eq!(
        run_loop(
            "/(a)(b)?/.test(\"xxab yy\"); console.log([RegExp.$1,RegExp.$2,RegExp.lastMatch,RegExp.lastParen,RegExp['$&'],RegExp.leftContext,RegExp.rightContext,RegExp.input].join(';'));"
        ),
        "a;b;ab;b;ab;xx; yy;xxab yy"
    );
}

#[test]
fn regexp_statics_initial_empty_and_failed_match_keeps() {
    assert_eq!(
        run_loop(
            "console.log([RegExp.$1, RegExp.lastMatch, RegExp.input].join(';'));
\"abc\".replace(/(b)/, \"X\");
const fail = /zz/.exec(\"abc\");
console.log([fail === null, RegExp.$1, RegExp.lastMatch, RegExp.input].join(';'));"
        ),
        ";;|true;b;b;abc"
    );
}

#[test]
fn regexp_statics_unmatched_group_is_empty_string() {
    assert_eq!(
        run_loop(
            "/(?<n>x)(y)?/.exec(\"x\"); console.log([RegExp.$1, RegExp.$2 === \"\", RegExp.lastParen === \"\", RegExp.$9 === \"\"].join(';'));"
        ),
        "x;true;true;true"
    );
}

#[test]
fn regexp_statics_replace_fn_sees_last_match() {
    // replace(/g, fn) runs every match first, so RegExp.$1 is the last match inside each callback.
    assert_eq!(
        run_loop("console.log(\"a1b2\".replace(/(\\d)/g, (m, d) => d + RegExp.$1));"),
        "a12b22"
    );
}

#[test]
fn regexp_statics_string_match_global_keeps_last_match() {
    assert_eq!(
        run_loop("\"xaybz\".match(/(\\w)/g); console.log(RegExp.$1, RegExp.lastMatch);"),
        "z z"
    );
}

#[test]
fn regexp_statics_search_updates_on_hit_only() {
    assert_eq!(
        run_loop(
            "const i = \"hello world\".search(/(o)/); const j = \"abc\".search(/(z)/); console.log([i, j, RegExp.$1, RegExp.lastMatch].join(';'));"
        ),
        "4;-1;o;o"
    );
}

#[test]
fn regexp_statics_split_updates_with_last_accepted_match() {
    assert_eq!(
        run_loop(
            "console.log(\"qr\".split(/(r)/).join(',') + ';' + RegExp.$1);
console.log(\"a1b2c\".split(/(\\d)/).join(',') + ';' + RegExp.$1 + ';' + RegExp.lastMatch);"
        ),
        "q,r,;r|a,1,b,2,c;2;2"
    );
}

#[test]
fn regexp_statics_string_pattern_replace_does_not_update() {
    assert_eq!(
        run_loop(
            "\"abc\".replace(/(b)/, \"X\"); \"q\".replace(\"q\", \"Y\"); console.log(RegExp.$1);"
        ),
        "b"
    );
}

#[test]
fn regexp_input_setter_only_input_is_writable() {
    assert_eq!(
        run_loop(
            "/k/.exec(\"k\"); RegExp.input = \"mmm\"; RegExp.lastMatch = \"q\"; RegExp.leftContext = \"L\"; RegExp.$1 = \"z\";
console.log([RegExp.$_, RegExp.lastMatch, JSON.stringify(RegExp.leftContext), JSON.stringify(RegExp.rightContext), RegExp.$1 === \"\"].join(';'));"
        ),
        "mmm;k;\"\";\"\";true"
    );
}

#[test]
fn regexp_static_descriptors_like_node() {
    assert_eq!(
        run_loop(
            r#"const keys = ['input','$_','lastMatch','$&','lastParen','$+','leftContext','$`','rightContext',"$'",'$1','$2','$3','$4','$5','$6','$7','$8','$9'];
console.log(keys.map(k => { const d = Object.getOwnPropertyDescriptor(RegExp, k); return [typeof d.get, typeof d.set, d.enumerable, d.configurable, d.get.name, d.get.length, d.set.name, d.set.length].join(';'); }).join('|'));
console.log(Object.keys(RegExp).length);
class X extends RegExp {}
/(q)/.exec("q"); console.log(X.$1 === RegExp.$1, globalThis.RegExp.$1, Object.getOwnPropertyDescriptor(RegExp,'$1').get.call({}));"#
        ),
        "function;function;false;true;get input;0;set input;1|function;function;false;true;get $_;0;set $_;1|function;function;false;true;get lastMatch;0;set lastMatch;1|function;function;false;true;get $&;0;set $&;1|function;function;false;true;get lastParen;0;set lastParen;1|function;function;false;true;get $+;0;set $+;1|function;function;false;true;get leftContext;0;set leftContext;1|function;function;false;true;get $`;0;set $`;1|function;function;false;true;get rightContext;0;set rightContext;1|function;function;false;true;get $';0;set $';1|function;function;false;true;get $1;0;set $1;1|function;function;false;true;get $2;0;set $2;1|function;function;false;true;get $3;0;set $3;1|function;function;false;true;get $4;0;set $4;1|function;function;false;true;get $5;0;set $5;1|function;function;false;true;get $6;0;set $6;1|function;function;false;true;get $7;0;set $7;1|function;function;false;true;get $8;0;set $8;1|function;function;false;true;get $9;0;set $9;1|0|true q q"
    );
}

#[test]
fn regexp_statics_are_per_realm() {
    // Two realms in one process: a match in one leaves the other's statics empty (node `vm` contexts: true).
    let mut a = Interp::new();
    a.run("/(x)/.exec('x');").expect("runs");
    a.run_event_loop().expect("loop runs");
    let mut b = Interp::new();
    b.run("console.log(RegExp.$1 === '');").expect("runs");
    b.run_event_loop().expect("loop runs");
    assert_eq!(b.out.join("|"), "true");
}
