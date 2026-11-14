use super::*;

/// Run `src` as a global script; return the stdout and stderr lines, each list joined with `|`.
fn run(src: &str) -> (String, String) {
    let mut it = Interp::new();
    match it.run_parsed(src, thaw_parser::parse_javascript_script(src), true) {
        Ok(_) => (it.out.join("|"), it.err.join("|")),
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
}

// `console` is one instance of the `Console` class (web/console.js), as in Node 22 where
// `console instanceof console.Console`. Expected values were recorded with Node v22.23.1.

#[test]
fn global_console_methods_are_own_enumerable_named_and_lengthed_like_node() {
    let (out, _) = run(r#"
for (const n of ['log','info','debug','warn','error','dir','time','timeEnd','timeLog','trace','assert','clear','count','countReset','group','groupEnd','groupCollapsed','table','dirxml','profile','profileEnd','timeStamp','context','createTask']) {
  const d = Object.getOwnPropertyDescriptor(console, n);
  console.log([n, typeof d.value, d.enumerable, d.writable, d.configurable, d.value.name, d.value.length].join(';'));
}
"#);
    assert_eq!(
        out,
        [
            "log;function;true;true;true;log;0",
            "info;function;true;true;true;info;0",
            "debug;function;true;true;true;debug;0",
            "warn;function;true;true;true;warn;0",
            "error;function;true;true;true;error;0",
            "dir;function;true;true;true;dir;0",
            "time;function;true;true;true;time;0",
            "timeEnd;function;true;true;true;timeEnd;0",
            "timeLog;function;true;true;true;timeLog;0",
            "trace;function;true;true;true;trace;0",
            "assert;function;true;true;true;assert;0",
            "clear;function;true;true;true;clear;0",
            "count;function;true;true;true;count;0",
            "countReset;function;true;true;true;countReset;0",
            "group;function;true;true;true;group;0",
            "groupEnd;function;true;true;true;groupEnd;0",
            "groupCollapsed;function;true;true;true;groupCollapsed;0",
            "table;function;true;true;true;table;0",
            "dirxml;function;true;true;true;dirxml;0",
            "profile;function;true;true;true;profile;0",
            "profileEnd;function;true;true;true;profileEnd;0",
            "timeStamp;function;true;true;true;timeStamp;0",
            "context;function;true;true;true;context;1",
            "createTask;function;true;true;true;createTask;0",
        ]
        .join("|")
    );
}

#[test]
fn global_console_private_members_have_node_shapes() {
    let (out, _) = run(r#"
const ac = Object.getOwnPropertyDescriptor(console, '_stdout');
console.log([typeof ac.get, typeof ac.set, ac.enumerable, ac.configurable].join(';'));
console.log(console._stdout === process.stdout, console._stderr === process.stderr);
for (const n of ['_stdoutErrorHandler','_stderrErrorHandler','_ignoreErrors','_times']) { const d = Object.getOwnPropertyDescriptor(console, n); console.log([n, d.enumerable, d.writable, d.configurable, typeof d.value].join(';')); }
console.log([console._stdoutErrorHandler.name === '', console._stdoutErrorHandler.length, console._ignoreErrors].join(';'));
console.log(console instanceof console.Console, console.Console.name, console.Console.length);
"#);
    assert_eq!(
        out,
        [
            "function;function;false;true",
            "true true",
            "_stdoutErrorHandler;false;true;true;function",
            "_stderrErrorHandler;false;true;true;function",
            "_ignoreErrors;false;true;true;boolean",
            "_times;false;true;true;object",
            "true;1;true",
            "true Console 1",
        ]
        .join("|")
    );
}

#[test]
fn count_group_and_profile_members_write_like_node() {
    let (out, err) = run(r#"
console.count(); console.count(); console.count('a'); console.countReset(); console.count();
console.group('G'); console.log('in'); console.count(); console.groupEnd(); console.log('out');
console.dirxml('x', 2); console.groupCollapsed('C'); console.log('cc'); console.groupEnd();
console.profile(); console.profileEnd(); console.timeStamp('m');
console.log(console.createTask('t').run(() => 42));
console.log(typeof console.context('x'));
"#);
    assert_eq!(
        out,
        [
            "default: 1",
            "default: 2",
            "a: 1",
            "default: 1",
            "G",
            "  in",
            "  default: 2",
            "out",
            "x 2",
            "C",
            "  cc",
            "42",
            "object",
        ]
        .join("|")
    );
    assert_eq!(err, "");
}

#[test]
fn log_info_debug_write_stdout_and_warn_error_write_stderr_with_group_indent() {
    let (out, err) = run(r#"
console.log('l'); console.info('i'); console.debug('d'); console.warn('w'); console.error('e');
console.assert(1 === 2, 'x %s', 'y'); console.assert(true, 'no');
console.group('G'); console.warn('gw'); console.error('ge'); console.log('gl'); console.groupEnd();
console.warn('after');
"#);
    assert_eq!(out, "l|i|d|G|  gl");
    assert_eq!(err, "w|e|Assertion failed: x y|  gw|  ge|after");
}

#[test]
fn time_and_time_log_write_labelled_stdout_lines() {
    let (out, err) = run("console.time('x'); console.timeLog('x', 'extra'); console.timeEnd('x');");
    let lines: Vec<&str> = out.split('|').collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(
        lines[0].starts_with("x: ") && lines[0].ends_with("ms extra"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("x: ") && lines[1].ends_with("ms"),
        "{}",
        lines[1]
    );
    assert_eq!(err, "");
}

#[test]
fn table_on_global_console_writes_the_node_table_to_stdout() {
    let (out, err) = run("console.table([{a:1,b:'x'},{a:2}]);");
    assert_eq!(
        out,
        [
            "┌─────────┬───┬─────┐",
            "│ (index) │ a │ b   │",
            "├─────────┼───┼─────┤",
            "│ 0       │ 1 │ 'x' │",
            "│ 1       │ 2 │     │",
            "└─────────┴───┴─────┘",
        ]
        .join("|")
    );
    assert_eq!(err, "");
}

// Console prelude functions are never analysed or compiled (the hot `console.log` path stays on the AST), while a
// user function in the same realm still compiles. `compiled()` counts only compiled functions.
#[test]
fn console_methods_are_never_compiled_but_user_functions_are() {
    let mut it = Interp::with_compile_mode(crate::compile::CompileMode::On);
    it.compile_after = 1;
    let src = "function user(n) { let s = 0; for (let i = 0; i < 3; i++) s += n; return s; }\n\
               let t = 0;\n\
               for (let k = 0; k < 200; k++) { console.log('x', k); t = user(t); }\n\
               console.log(t);";
    assert!(it.run_sloppy(src).is_ok());
    assert_eq!(it.out.len(), 201);
    assert_eq!(it.out.last().map(String::as_str), Some("0"));
    assert_eq!(it.cstats.compiled(), 1, "only `user` may compile");
}
