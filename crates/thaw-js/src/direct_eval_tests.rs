//! Golden tests for direct `eval`: each `tests/eval/*.js` probe runs under the interpreter and must
//! print what Node 22 prints (its `.txt` next to it). The `.txt` files come from `node <probe>.js`.
//! Stack-trace lines are compared by their message part only: V8 frame text is shape-only.
use super::*;

/// The probes run as main CommonJS modules, as Node runs them: sloppy, inside the module wrapper.
const SCOPE_JS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/eval/eval_scope.js");
const SCOPE_TXT: &str = include_str!("../tests/eval/eval_scope.txt");
const ERRORS_JS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/eval/eval_errors.js");
const ERRORS_TXT: &str = include_str!("../tests/eval/eval_errors.txt");
const MISC_JS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/eval/eval_misc.js");
const MISC_TXT: &str = include_str!("../tests/eval/eval_misc.txt");

/// The console lines the probe at `path` prints (event loop included).
fn probe_output(path: &str) -> Vec<String> {
    let mut it = Box::new(Interp::new());
    let ran = it.run_main_file(path);
    it.flush_partial();
    let mut out = it.out.clone();
    if let Err(Ctl::Throw(v)) = ran {
        out.push(format!("Uncaught {}", it.to_str(&v)));
    }
    out
}

/// Each line cut to its message: frame lines (`    at ...`) and the `|    at ` tail of a probe's
/// stack summary are dropped.
fn normalize(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !l.starts_with("    at "))
        .map(|l| {
            l.split("|    at ")
                .next()
                .unwrap_or_default()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// A probe line thaw-js is known not to match yet. `Message` lines only need the same error class
/// (V8 and the SWC parser word the same early error differently); `Divergence` lines may differ in any
/// way. Each entry must still mismatch, or the test fails until the entry is removed.
enum Gap {
    Message(&'static str, &'static str),
    Divergence(&'static str, &'static str),
}

use Gap::{Divergence, Message};

const SCOPE_GAPS: &[Gap] = &[
    Message("yield-in-eval THROW SyntaxError: Unexpected number", "parser message text"),
    Message("field-arguments THROW SyntaxError: 'arguments' is not allowed in class field initializer or static initialization block", "parser message text"),
    Divergence("tdz THROW ReferenceError: Cannot access 'tz' before initialization", "let TDZ is not modelled: an unhoisted let reads as not defined (pre-existing, not eval-specific)"),
    Message("labels THROW SyntaxError: Undefined label 'a'", "parser message text"),
    Message("return-in-eval THROW SyntaxError: Illegal return statement", "parser message text"),
    Message("strict-arguments-assign THROW SyntaxError: Unexpected eval or arguments in strict mode", "parser message text"),
    Message("strict-eval-name THROW SyntaxError: Unexpected eval or arguments in strict mode", "parser message text"),
];

const ERRORS_GAPS: &[Gap] = &[
    Message(
        "SyntaxError: Unexpected end of input",
        "parser message text",
    ),
    Divergence(
        "SyntaxError: Identifier 'a' has already been declared",
        "a duplicate let is not an early error in script code (SWC parse_script)",
    ),
    Message("SyntaxError: Unexpected number", "parser message text"),
    Message("SyntaxError: Unexpected token '}'", "parser message text"),
    Message(
        "SyntaxError: Invalid or unexpected token",
        "parser message text",
    ),
    Message("SyntaxError: Unexpected token '{'", "parser message text"),
    Message(
        "SyntaxError: Unexpected identifier 'b'",
        "parser message text",
    ),
    Message(
        "SyntaxError: Invalid left-hand side in assignment",
        "parser message text",
    ),
    Message(
        "SyntaxError: 'super' keyword unexpected here",
        "parser message text",
    ),
    Message(
        "SyntaxError: await is only valid in async functions and the top level bodies of modules",
        "parser message text",
    ),
    Message(
        "SyntaxError: Illegal break statement",
        "parser message text",
    ),
    Divergence(
        "SyntaxError: Duplicate __proto__ fields are not allowed in object literals",
        "a duplicate __proto__ is not an early error in script code (SWC parse_script)",
    ),
    Message(
        "SyntaxError: Strict mode code may not include a with statement",
        "parser message text",
    ),
    Message(
        "SyntaxError: Octal literals are not allowed in strict mode.",
        "parser message text",
    ),
    Message(
        "SyntaxError: Illegal return statement",
        "parser message text",
    ),
];

/// The error class word (`SyntaxError`, `TypeError`, ...) in an expected probe line, if any.
fn error_class(line: &str) -> Option<&str> {
    let end = line.find("Error: ")? + "Error".len();
    let start = line[..end]
        .rfind(|c: char| !c.is_ascii_alphabetic())
        .map_or(0, |i| i + 1);
    Some(&line[start..end])
}

fn check_golden(path: &str, golden: &str, base: &[Gap]) {
    let gaps: Vec<&Gap> = base.iter().collect();
    let actual = normalize(&probe_output(path));
    let expected: Vec<String> = golden.lines().map(str::to_string).collect();
    let expected = normalize(&expected);
    let mut used = vec![false; gaps.len()];
    let mut diffs = Vec::new();
    for i in 0..actual.len().max(expected.len()) {
        let (a, e) = (
            actual.get(i).map_or("", String::as_str),
            expected.get(i).map_or("", String::as_str),
        );
        if a == e {
            continue;
        }
        let known = gaps.iter().position(|g| match g {
            Message(line, _) | Divergence(line, _) => *line == e,
        });
        match known {
            Some(k) if matches!(gaps[k], Divergence(..)) => used[k] = true,
            Some(k) if error_class(e).is_some_and(|c| a.contains(c)) => used[k] = true,
            _ => diffs.push(format!("line {}: node {e:?} / thaw-js {a:?}", i + 1)),
        }
    }
    for (k, g) in gaps.iter().enumerate() {
        let (line, reason) = match g {
            Message(l, r) | Divergence(l, r) => (*l, *r),
        };
        // an entry for a line this golden does not have is not stale
        if !used[k] && expected.iter().any(|e| e == line) {
            diffs.push(format!("stale gap entry (now matches; {reason}): {line:?}"));
        }
    }
    assert!(
        diffs.is_empty(),
        "eval golden mismatch:\n{}",
        diffs.join("\n")
    );
}

#[test]
fn direct_eval_scope_golden() {
    check_golden(SCOPE_JS, SCOPE_TXT, SCOPE_GAPS);
}

#[test]
fn direct_eval_errors_golden() {
    check_golden(ERRORS_JS, ERRORS_TXT, ERRORS_GAPS);
}

#[test]
fn direct_eval_misc_golden() {
    check_golden(MISC_JS, MISC_TXT, &[]);
}
