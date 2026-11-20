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

/// Writes `files` into a fresh directory for `case`, runs the worker on `entry` (a path string, or a `file:` URL object
/// when `via_url`) and returns the main thread's console line with the directory shown as `{DIR}` (as Node 22 prints).
fn run_file_worker(case: &str, files: &[(&str, &str)], entry: &str, via_url: bool) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("thaw-esm-entry-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
    let path = dir.join(entry).display().to_string();
    let spec = if via_url {
        format!("new URL({:?})", format!("file://{path}"))
    } else {
        format!("{path:?}")
    };
    let src = format!(
        "const {{ Worker }} = __thaw_worker_threads_exports;
        const log = [];
        const w = new Worker({spec});
        w.on('online', () => log.push('online'));
        w.on('message', (m) => log.push('message:' + JSON.stringify(m)));
        w.on('error', (e) => log.push('error:' + e.name + ':' + e.code + ':' + e.message));
        w.on('exit', (c) => {{ log.push('exit:' + c); console.log(log.join(' | ')); }});"
    );
    let dir_text = dir.display().to_string();
    run_main(&src)
        .into_iter()
        .map(|line| line.replace(&dir_text, "{DIR}"))
        .collect()
}

const GRAPH: [(&str, &str); 3] = [
    (
        "w.mjs",
        r#"import d, { a } from './dep.mjs';
import cjs, { x } from './cjs.cjs';
import { parentPort } from 'node:worker_threads';
parentPort.postMessage({ a, d: d(), cjs, x, req: typeof require, strict: (function(){return this===undefined})() });
"#,
    ),
    (
        "dep.mjs",
        "export const a = 1;\nexport default function d() { return 'dflt'; }\n",
    ),
    ("cjs.cjs", "exports.x = 7;\nmodule.exports.y = 8;\n"),
];

#[test]
fn esm_file_entry_imports_sibling_named_default_and_cjs() {
    assert_eq!(
        run_file_worker("graph", &GRAPH, "w.mjs", false),
        vec![
            r#"online | message:{"a":1,"d":"dflt","cjs":{"x":7,"y":8},"x":7,"req":"undefined","strict":true} | exit:0"#
        ]
    );
}

#[test]
fn esm_file_entry_as_file_url_object_runs_the_same_graph() {
    assert_eq!(
        run_file_worker("graph-url", &GRAPH, "w.mjs", true),
        vec![
            r#"online | message:{"a":1,"d":"dflt","cjs":{"x":7,"y":8},"x":7,"req":"undefined","strict":true} | exit:0"#
        ]
    );
}

#[test]
fn esm_type_module_package_js_entry_and_dependency() {
    let files = [
        ("package.json", "{\"type\":\"module\"}\n"),
        (
            "w.js",
            "import d, { a } from './dep.js';\nimport { parentPort } from 'node:worker_threads';\nparentPort.postMessage([a, d()]);\n",
        ),
        (
            "dep.js",
            "export const a = 1;\nexport default function d() { return 'dflt'; }\n",
        ),
    ];
    assert_eq!(
        run_file_worker("pkg", &files, "w.js", false),
        vec![r#"online | message:[1,"dflt"] | exit:0"#]
    );
}

#[test]
fn esm_reexports_named_star_and_namespace() {
    let files = [
        (
            "rw.mjs",
            "import { b, s, ns } from './re.mjs';\nimport v from './expr.mjs';\nimport { parentPort } from 'node:worker_threads';\nparentPort.postMessage({ b, s, ns: [ns.a, typeof ns.default], v });\n",
        ),
        (
            "re.mjs",
            "export { a as b } from './dep.mjs';\nexport * from './star.mjs';\nexport * as ns from './dep.mjs';\n",
        ),
        (
            "dep.mjs",
            "export const a = 1;\nexport default function d() { return 'dflt'; }\n",
        ),
        ("star.mjs", "export const s = 's';\n"),
        ("expr.mjs", "export default 40 + 2;\n"),
    ];
    assert_eq!(
        run_file_worker("reexport", &files, "rw.mjs", false),
        vec![r#"online | message:{"b":1,"s":"s","ns":[1,"function"],"v":42} | exit:0"#]
    );
}

#[test]
fn esm_import_meta_url_filename_dirname_for_entry_and_dependency() {
    let files = [
        (
            "mt.mjs",
            "import { info } from './mdep.mjs';\nimport { parentPort } from 'node:worker_threads';\nparentPort.postMessage({ entry: { url: import.meta.url, filename: import.meta.filename, dirname: import.meta.dirname }, dep: info });\n",
        ),
        (
            "mdep.mjs",
            "export const info = { url: import.meta.url, filename: import.meta.filename, dirname: import.meta.dirname };\n",
        ),
    ];
    assert_eq!(
        run_file_worker("meta", &files, "mt.mjs", false),
        vec![
            r#"online | message:{"entry":{"url":"file://{DIR}/mt.mjs","filename":"{DIR}/mt.mjs","dirname":"{DIR}"},"dep":{"url":"file://{DIR}/mdep.mjs","filename":"{DIR}/mdep.mjs","dirname":"{DIR}"}} | exit:0"#
        ]
    );
}

#[test]
fn esm_missing_dependency_fails_at_link_before_any_module_runs() {
    let files = [
        (
            "missing.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport './ok.mjs';\nimport './nope.mjs';\n",
        ),
        (
            "ok.mjs",
            "import { parentPort } from 'node:worker_threads';\nparentPort.postMessage('ok-ran');\n",
        ),
    ];
    assert_eq!(
        run_file_worker("missing", &files, "missing.mjs", false),
        vec!["online | error:Error:ERR_MODULE_NOT_FOUND:Cannot find module '{DIR}/nope.mjs' imported from {DIR}/missing.mjs | exit:1"]
    );
}

/// Node 22 prints `SyntaxError:undefined:Unexpected token '='`. The parser's own message differs (swc's wording, no V8
/// text mapping yet: a follow-up), so this pins the event shape and the error name and code only.
#[test]
fn esm_dependency_syntax_error_is_an_error_event() {
    let files = [
        ("bad.mjs", "export const = ;\n"),
        ("syn.mjs", "import './bad.mjs';\n"),
    ];
    let lines = run_file_worker("syntax", &files, "syn.mjs", false);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("online | error:SyntaxError:undefined:")
            && lines[0].ends_with(" | exit:1"),
        "{}",
        lines[0]
    );
}

#[test]
fn esm_dependency_throwing_on_evaluation_is_an_error_event() {
    let files = [
        ("boom.mjs", "throw new TypeError('dep-boom');\n"),
        ("thr.mjs", "import './boom.mjs';\n"),
    ];
    assert_eq!(
        run_file_worker("throws", &files, "thr.mjs", false),
        vec!["online | error:TypeError:undefined:dep-boom | exit:1"]
    );
}
