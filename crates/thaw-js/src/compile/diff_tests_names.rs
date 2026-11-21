//! Differential tests of ES NamedEvaluation for anonymous functions: the compiled object-literal path
//! must name a function value exactly as the AST interpreter does (`lib.rs` `name_anonymous_fn`). Each
//! program runs under `off` (reference), `on` and `check`; output and ok-ness must match, and the
//! compiled-function count must reach the minimum. Explicit expectations are the Node output.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

fn same(src: &str, min_compiled: u64) -> Vec<String> {
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    assert!(
        !want.is_empty() || !ok_want,
        "reference printed nothing: {src}"
    );
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, it) = run_mode(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            it.cstats.compiled() >= min_compiled,
            "{mode:?}: compiled {} < {min_compiled}: {src}",
            it.cstats.compiled()
        );
    }
    want
}

/// `{ k: function(){}, k: () => 1, k: (fn) }`: anonymous functions, arrows and parenthesized ones take the key.
#[test]
fn names_object_literal_values_take_their_key() {
    let got = same(
        r#"
        function mk() {
            return {
                describe: function () {},
                go: () => 1,
                paren: (function () {}),
                arrowParen: (() => 2),
            };
        }
        const o = mk();
        console.log(JSON.stringify([o.describe.name, o.go.name, o.paren.name, o.arrowParen.name]));
        console.log(o.describe);
        console.log(o.paren);
        "#,
        1,
    );
    assert_eq!(
        got,
        vec![
            r#"["describe","go","paren","arrowParen"]"#.to_string(),
            "[Function: describe]".to_string(),
            "[Function: paren]".to_string(),
        ]
    );
}

/// Negative controls: a named function expression keeps its name, `(0, fn)` stays anonymous, a
/// computed key names by its string, a numeric key names by its digits, and `{ a: fn }` keeps `fn`'s name.
#[test]
fn names_negative_controls_and_computed_keys() {
    let got = same(
        r#"
        function named() {}
        function mk(k) {
            return {
                inner: function inner() {},
                seq: (0, function () {}),
                [k]: function () {},
                7: () => 1,
                keep: named,
                shorthand: named,
            };
        }
        const o = mk("dyn");
        console.log(JSON.stringify([o.inner.name, o.seq.name, o.dyn.name, o[7].name]));
        console.log(JSON.stringify([o.keep.name, o.shorthand.name]));
        const p = mk("other");
        console.log(JSON.stringify([p.other.name, p.inner.name]));
        "#,
        1,
    );
    assert_eq!(
        got,
        vec![
            r#"["inner","","dyn","7"]"#.to_string(),
            r#"["named","named"]"#.to_string(),
            r#"["other","inner"]"#.to_string(),
        ]
    );
}

/// Nested and array-element positions: only the object-literal property value is named.
#[test]
fn names_nested_objects_and_array_elements() {
    let got = same(
        r#"
        function mk() {
            return { outer: { inner: () => 1 }, list: [function () {}, { x: function () {} }] };
        }
        const o = mk();
        console.log(JSON.stringify([o.outer.inner.name, o.list[0].name, o.list[1].x.name]));
        "#,
        1,
    );
    assert_eq!(got, vec![r#"["inner","","x"]"#.to_string()]);
}

/// Direct, unnamed function values (returned, passed) are not named by any key.
#[test]
fn names_direct_function_values_stay_anonymous() {
    let got = same(
        r#"
        function mk() { return function () {}; }
        function take(f) { return f.name; }
        console.log(JSON.stringify([mk().name, take(() => 3), take(function () {})]));
        "#,
        1,
    );
    assert_eq!(got, vec![r#"["","",""]"#.to_string()]);
}

/// Variable declarations and assignments: a parity check of the names (explicit Node values are in
/// `names_identifier_targets_as_node_does`).
#[test]
fn names_declarations_and_assignments_match_the_interpreter() {
    same(
        r#"
        function decl() {
            const f = function () {};
            let g = () => 1;
            return JSON.stringify([f.name, g.name]);
        }
        function assign() {
            let f;
            f = function () {};
            let g;
            g ||= () => 2;
            return JSON.stringify([f.name, g.name]);
        }
        console.log(decl());
        console.log(assign());
        console.log(decl());
        console.log(assign());
        "#,
        1,
    );
}

/// Default parameter values and class fields: differential only (interpreter parity; see the note above).
#[test]
fn names_default_params_and_class_fields_match_the_interpreter() {
    same(
        r#"
        function p(cb = function () {}) { return cb.name; }
        class C { f = function () {}; }
        function mk() { return new C().f.name; }
        console.log(p());
        console.log(p());
        console.log(mk());
        console.log(mk());
        "#,
        0,
    );
}

/// Identifier targets (var/let/const, `=`, `||=`, destructuring and parameter defaults) name the function
/// on both the compiled and the interpreted path; member targets do not. Node 22 is the reference.
#[test]
fn names_identifier_targets_as_node_does() {
    let got = same(
        r#"
        function decl() { const f = function () {}; let g = () => 1; var h = class {}; return [f.name, g.name, h.name].join(); }
        function asg() { let f; f = function () {}; let g; g ||= () => 2; let i; i &&= () => 3; let m = {}; m.p = () => 1; return [f.name, g.name, String(i), m.p.name].join(); }
        function des() { var {x = () => 1} = {}; var [y = class {}] = []; return [x.name, y.name].join(); }
        function par(a = () => 1, b = function () {}) { return [a.name, b.name].join(); }
        for (let n = 0; n < 3; n++) { console.log(decl()); console.log(asg()); console.log(des()); console.log(par()); }
        "#,
        1,
    );
    let one = ["f,g,h", "f,g,undefined,", "x,y", "a,b"];
    let want: Vec<String> = (0..3).flat_map(|_| one.map(String::from)).collect();
    assert_eq!(got, want);
}
