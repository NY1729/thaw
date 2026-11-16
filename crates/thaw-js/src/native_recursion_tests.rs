//! Recursive natives (JSON, join, the parser front door, RegExp compile, inspect) on a configured stack: deep
//! input must throw a catchable error, not abort the process. Inputs are built iteratively in JS (200k levels).
//! Expected classes and messages were checked against Node 22 (see each test).

use super::*;

/// Stack of the test thread; `configure_stack` gets the same size, so the guard trips well before the real end.
const SMALL_STACK: usize = 16 << 20;

/// Runs `src` on a fresh Interp configured for `SMALL_STACK` and returns its console lines.
fn run_small_stack(src: String) -> Vec<String> {
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || {
            // The natives under test are the point, whatever THAW_JS_COMPILE says: `check` mode runs every call twice.
            let mut it = Interp::with_compile_mode(crate::compile::CompileMode::Off);
            it.configure_stack(SMALL_STACK);
            match it.run(&src) {
                Ok(_) => {}
                Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
                Err(_) => panic!("stray completion"),
            }
            std::mem::take(&mut it.out)
        })
        .expect("thread spawns")
        .join()
        .expect("thread joins")
}

/// `setup` declares the deep input; `trigger` must throw `class` whose message ends with `tail` (the whole message
/// for a stack overflow). The trigger runs twice, and a plain call afterwards proves the interpreter still works.
fn throws_then_recovers(setup: &str, trigger: &str, class: &str, tail: &str) {
    let src = format!(
        "{setup}\n\
         for (let i = 0; i < 2; i++) {{ try {{ {trigger}; console.log('no throw'); }} catch (e) {{ console.log(e.name, e.message.slice(-{})); }} }}\n\
         function ok() {{ return 7; }}\n\
         console.log(ok());",
        tail.len()
    );
    assert_eq!(
        run_small_stack(src),
        [
            format!("{class} {tail}"),
            format!("{class} {tail}"),
            "7".to_string()
        ]
    );
}

const OVERFLOW: &str = "Maximum call stack size exceeded";
const DEEP_ARRAY: &str = "let a = []; for (let i = 0; i < 200000; i++) a = [a];";
const DEEP_OBJECT: &str = "let o = {}; for (let i = 0; i < 200000; i++) o = { c: o };";
const BRACKETS: &str = "const s = '['.repeat(100000) + ']'.repeat(100000);";

#[test]
fn json_stringify_deep_array_throws_range_error() {
    // node 22: RangeError Maximum call stack size exceeded
    throws_then_recovers(DEEP_ARRAY, "JSON.stringify(a)", "RangeError", OVERFLOW);
}

#[test]
fn json_stringify_deep_object_throws_range_error() {
    throws_then_recovers(DEEP_OBJECT, "JSON.stringify(o)", "RangeError", OVERFLOW);
}

#[test]
fn json_stringify_to_json_recursion_throws_range_error() {
    throws_then_recovers(
        "const t = { toJSON() { return { x: this }; } };",
        "JSON.stringify(t)",
        "RangeError",
        OVERFLOW,
    );
}

#[test]
fn json_parse_deep_input_throws_range_error() {
    // node 22 parses 200k levels; thaw's parser caps nesting at 5000, documented as a RangeError
    throws_then_recovers(
        "const s = '['.repeat(200000) + ']'.repeat(200000);",
        "JSON.parse(s)",
        "RangeError",
        OVERFLOW,
    );
}

#[test]
fn array_join_deep_array_throws_range_error() {
    // node 22: RangeError Maximum call stack size exceeded
    throws_then_recovers(DEEP_ARRAY, "a.join()", "RangeError", OVERFLOW);
}

#[test]
fn string_of_deep_array_throws_range_error() {
    throws_then_recovers(DEEP_ARRAY, "String(a)", "RangeError", OVERFLOW);
}

#[test]
fn structured_clone_deep_array_throws_range_error() {
    throws_then_recovers(DEEP_ARRAY, "structuredClone(a)", "RangeError", OVERFLOW);
}

#[test]
fn flat_infinity_deep_array_throws_range_error() {
    throws_then_recovers(DEEP_ARRAY, "a.flat(Infinity)", "RangeError", OVERFLOW);
}

#[test]
fn regexp_with_100k_capture_groups_is_too_many_captures() {
    // node 22: SyntaxError Invalid regular expression: /(((...)))/: Too many captures
    throws_then_recovers(
        "const s = '('.repeat(100000) + ')'.repeat(100000);",
        "new RegExp(s)",
        "SyntaxError",
        "Too many captures",
    );
}

#[test]
fn regexp_with_deep_non_capturing_groups_throws_range_error() {
    throws_then_recovers(
        "const s = '(?:'.repeat(100000) + ')'.repeat(100000);",
        "new RegExp(s)",
        "RangeError",
        OVERFLOW,
    );
}

#[test]
fn eval_of_100k_nested_brackets_throws_range_error() {
    // node 22: RangeError Maximum call stack size exceeded
    throws_then_recovers(BRACKETS, "eval(s)", "RangeError", OVERFLOW);
}

#[test]
fn function_constructor_of_100k_nested_brackets_throws_range_error() {
    throws_then_recovers(
        BRACKETS,
        "new Function('return ' + s)",
        "RangeError",
        OVERFLOW,
    );
}

#[test]
fn console_log_of_deep_object_prints_placeholder_and_continues() {
    // node 22 prints `{ c: { c: [Object] } }`; thaw prints nested levels until the guard, then `[Object]`
    let out = run_small_stack(format!(
        "{DEEP_OBJECT}\nconsole.log(o);\nconsole.log('after');"
    ));
    assert_eq!(out.len(), 2, "one line for the object, one after it");
    assert!(out[0].contains("[Object]"), "placeholder at the guard");
    assert_eq!(out[1], "after");
}

/// The parser front-door scan counts only code brackets: regex, string and comment text is not nesting.
#[test]
fn nesting_scan_ignores_brackets_in_regex_string_and_comment_text() {
    assert!(!too_nested(&"x = /\\(/; y = /[(]/;".repeat(3000)));
    assert!(!too_nested(&format!("s = '{}';", "(".repeat(5000))));
    assert!(!too_nested(&format!("// {}\nz = 1;", "(".repeat(5000))));
    assert!(!too_nested(&format!("/* {} */ z = 1;", "[".repeat(5000))));
    assert!(too_nested(&format!(
        "{}{}",
        "(".repeat(2500),
        ")".repeat(2500)
    )));
}
