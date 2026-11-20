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
    let hrefs = modules
        .iter()
        .map(|m| format!("'data:text/javascript,' + encodeURIComponent({m:?})"))
        .collect::<Vec<_>>();
    run_hrefs(&hrefs)
}

/// Runs each JS expression in `hrefs` (evaluating to a `data:` href) as `new Worker(new URL(href))`, one console
/// line per worker.
fn run_hrefs(hrefs: &[String]) -> Vec<String> {
    let list = hrefs.join(",");
    let src = format!(
        "const {{ Worker }} = __thaw_worker_threads_exports;
        const run = (href) => new Promise((resolve) => {{
          const log = [];
          const w = new Worker(new URL(href));
          w.on('online', () => log.push('online'));
          w.on('message', (m) => log.push('message:' + JSON.stringify(m)));
          w.on('error', (e) => log.push('error:' + e.name + ':' + e.code + ':' + e.message));
          w.on('exit', (c) => {{ log.push('exit:' + c); resolve(log.join(' | ')); }});
        }});
        (async () => {{ for (const h of [{list}]) console.log(await run(h)); }})();"
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
fn esm_entry_process_exit_code_becomes_the_worker_exit_code() {
    assert_eq!(
        run_modules(&["process.exitCode = 5;"]),
        vec!["online | exit:5"]
    );
}

#[test]
fn esm_entry_throw_is_an_error_event_then_exit_1() {
    assert_eq!(
        run_modules(&["throw new Error('boom');"]),
        vec!["online | error:Error:undefined:boom | exit:1"]
    );
}

#[test]
fn esm_entry_rejected_top_level_await_is_an_error_event_then_exit_1() {
    assert_eq!(
        run_modules(&["await Promise.reject(new TypeError('tla-reject'));"]),
        vec!["online | error:TypeError:undefined:tla-reject | exit:1"]
    );
}

#[test]
fn esm_entry_non_javascript_data_url_is_an_error_event_not_a_sync_throw() {
    assert_eq!(
        run_hrefs(&["'data:text/plain,hi'".to_string()]),
        vec!["online | error:RangeError:ERR_UNKNOWN_MODULE_FORMAT:Unknown module format: text/plain for URL data:text/plain,hi | exit:1"]
    );
}

#[test]
fn esm_entry_base64_data_url_runs_its_module() {
    // base64 of: import {parentPort} from 'node:worker_threads'; parentPort.postMessage(1)
    assert_eq!(
        run_hrefs(&["'data:text/javascript;base64,aW1wb3J0IHtwYXJlbnRQb3J0fSBmcm9tICdub2RlOndvcmtlcl90aHJlYWRzJzsgcGFyZW50UG9ydC5wb3N0TWVzc2FnZSgxKQ=='".to_string()]),
        vec!["online | message:1 | exit:0"]
    );
}

#[test]
fn data_url_string_worker_path_still_throws_synchronously() {
    let src = "try { new (__thaw_worker_threads_exports.Worker)('data:text/javascript,1'); console.log('no throw'); } catch (e) { console.log(e.name + ' ' + e.code); }";
    assert_eq!(run_main(src), vec!["TypeError ERR_WORKER_PATH"]);
}
