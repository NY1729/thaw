//! Worker stdin below the JS class (W5): the `__thaw_worker_stdin` native reaches a worker that reads `process.stdin`,
//! and a bare end (empty hex, end flag) ends it. Pattern of worker_runtime.rs.
use thaw_js::Interp;

/// Spawns `source` with `{ stdin: true }`, runs `stdin_calls` (with the handle as `h`), and returns the events the main
/// script logged, joined by `|`.
fn run_worker_with_stdin(source: &str, stdin_calls: &str) -> String {
    let main = format!(
        r#"const seen = [];
        globalThis.__thaw_worker_dispatch = (json) => {{
          for (const e of JSON.parse(json)) {{
            if (e.type === 'message') seen.push('message:' + __thaw_worker_decode(e.value));
            else if (e.type === 'exit') {{ seen.push('exit:' + e.code); console.log(seen.join('|')); }}
          }}
        }};
        const h = __thaw_worker_spawn({source:?}, JSON.stringify({{ stdin: true }}));
        {stdin_calls}"#
    );
    let mut it = Interp::new();
    it.run(&main).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

#[test]
fn raw_stdin_native_delivers_bytes_and_a_bare_end_exits_the_worker() {
    let source = "let s = ''; process.stdin.setEncoding('utf8'); process.stdin.on('data', (c) => { s += c; }); process.stdin.on('end', () => { __thaw_worker_send(__thaw_worker_encode(s)); });";
    assert_eq!(
        run_worker_with_stdin(
            source,
            "__thaw_worker_stdin(h, '616263', false); __thaw_worker_stdin(h, '', true);"
        ),
        "message:abc|exit:0"
    );
}
