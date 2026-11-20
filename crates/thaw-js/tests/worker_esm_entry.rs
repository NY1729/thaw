//! A `Worker` whose entry is an ES module given as a `data:text/javascript` URL object (PR-3a): strict mode, the
//! CommonJS names undefined, `import.meta.url`, top-level await, and the string form that stays an ERR_WORKER_PATH.
//! Each expected event sequence is what Node 22.23.1 prints for the same program.
use thaw_js::Interp;

/// Runs `src` on the main thread and its event loop; returns the main thread's console lines.
fn run_main(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    it.run(src).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.clone()
}

/// The main script: runs each worker module source in turn and logs its events (`online`, `message:<json>`,
/// `error:<name>:<code>:<message>`, `exit:<code>`) joined by ` | `, one console line per module.
fn run_modules(modules: &[&str]) -> Vec<String> {
    let list = modules
        .iter()
        .map(|m| format!("{m:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let src = format!(
        "const {{ Worker }} = __thaw_worker_threads_exports;
        const run = (src) => new Promise((resolve) => {{
          const log = [];
          const w = new Worker(new URL('data:text/javascript,' + encodeURIComponent(src)));
          w.on('online', () => log.push('online'));
          w.on('message', (m) => log.push('message:' + JSON.stringify(m)));
          w.on('error', (e) => log.push('error:' + e.name + ':' + e.code + ':' + e.message));
          w.on('exit', (c) => {{ log.push('exit:' + c); resolve(log.join(' | ')); }});
        }});
        (async () => {{ for (const m of [{list}]) console.log(await run(m)); }})();"
    );
    run_main(&src)
}

#[test]
fn esm_entry_is_strict_so_an_undeclared_assignment_throws() {
    let src = "import { parentPort } from 'node:worker_threads'; try { undeclared = 1; parentPort.postMessage('no throw'); } catch (e) { parentPort.postMessage(e.name); }";
    assert_eq!(
        run_modules(&[src]),
        vec![r#"online | message:"ReferenceError" | exit:0"#]
    );
}

#[test]
fn esm_entry_has_no_commonjs_names_and_no_this() {
    let src = "import { parentPort } from 'node:worker_threads'; parentPort.postMessage([typeof require, typeof module, typeof __filename, typeof exports, typeof __dirname, String(this)]);";
    assert_eq!(
        run_modules(&[src]),
        vec![
            r#"online | message:["undefined","undefined","undefined","undefined","undefined","undefined"] | exit:0"#
        ]
    );
}

#[test]
fn esm_entry_import_meta_url_is_its_data_url() {
    let src = "import { parentPort } from 'node:worker_threads'; parentPort.postMessage([typeof import.meta.url, import.meta.url.startsWith('data:text/javascript,')]);";
    assert_eq!(
        run_modules(&[src]),
        vec![r#"online | message:["string",true] | exit:0"#]
    );
}

#[test]
fn esm_entry_top_level_await_resolves_before_the_worker_exits() {
    let src = "import { parentPort } from 'node:worker_threads'; const v = await Promise.resolve(42); parentPort.postMessage(v);";
    assert_eq!(run_modules(&[src]), vec!["online | message:42 | exit:0"]);
}

#[test]
fn data_url_string_worker_path_still_throws_synchronously() {
    let src = "try { new (__thaw_worker_threads_exports.Worker)('data:text/javascript,1'); console.log('no throw'); } catch (e) { console.log(e.name + ' ' + e.code); }";
    assert_eq!(run_main(src), vec!["TypeError ERR_WORKER_PATH"]);
}
