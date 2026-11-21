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

// PR-5: dynamic `import()` and dependency top-level await. Each expected sequence is what Node 22.23.1 prints (probe
// fixtures: thaw-tmp/tmp-run/pr5, harness.mjs / gen.mjs; `{DIR}` is the case directory).

#[test]
fn esm_dynamic_import_of_the_worker_threads_builtin_namespace() {
    let src = "import { parentPort } from 'node:worker_threads'; const wt = await import('node:worker_threads'); parentPort.postMessage([typeof wt.parentPort, typeof wt.default.parentPort, wt.parentPort === parentPort, typeof wt.default.Worker]);";
    assert_eq!(
        run_modules(&[src]),
        vec![r#"online | message:["object","object",true,"function"] | exit:0"#]
    );
}

/// Node: `"function:function"`. The worker realm serves `node:path` from thaw-node-builtins (worker_esm.js `load`).
#[test]
fn esm_dynamic_import_of_a_builtin_namespace() {
    let src = "import { parentPort } from 'node:worker_threads'; const p = await import('node:path'); parentPort.postMessage(typeof p.join + ':' + typeof p.default.join);";
    assert_eq!(
        run_modules(&[src]),
        vec![r#"online | message:"function:function" | exit:0"#]
    );
}

#[test]
fn esm_builtin_imports_resolve_with_or_without_the_prefix_to_one_namespace() {
    let src = "import { parentPort } from 'node:worker_threads'; import path from 'node:path'; import { join } from 'path'; const p = await import('node:path'); const q = await import('path'); parentPort.postMessage([typeof join, join('a', 'b'), path.join('x', 'y'), typeof p.join, typeof p.default.join, p.default === path, p.default === q.default, (await import('node:path')) === p]);";
    assert_eq!(
        run_modules(&[src]),
        vec![
            r#"online | message:["function","a/b","x/y","function","function",true,true,true] | exit:0"#
        ]
    );
}

#[test]
fn esm_import_of_an_unknown_builtin_rejects_with_its_code() {
    let src = "import { parentPort } from 'node:worker_threads'; try { await import('node:nonexistent'); parentPort.postMessage('no throw'); } catch (e) { parentPort.postMessage(e.code); }";
    assert_eq!(
        run_modules(&[src]),
        vec![r#"online | message:"ERR_UNKNOWN_BUILTIN_MODULE" | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_an_esm_file_returns_its_namespace() {
    let files = [
        ("dep.mjs", "export const a = 1;\nexport default function d() { return 'dflt'; }\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nconst ns = await import('./dep.mjs');\nparentPort.postMessage({ a: ns.a, d: ns.default(), keys: Object.keys(ns) });\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-esm", &files, "w.mjs", false),
        vec![r#"online | message:{"a":1,"d":"dflt","keys":["a","default"]} | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_a_cjs_file_is_default_plus_names() {
    let files = [
        ("cjs.cjs", "exports.x = 7;\nmodule.exports.y = 8;\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nconst ns = await import('./cjs.cjs');\nparentPort.postMessage({ x: ns.x, y: ns.y, def: ns.default });\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-cjs", &files, "w.mjs", false),
        vec![r#"online | message:{"x":7,"y":8,"def":{"x":7,"y":8}} | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_a_missing_file_uncaught_is_an_error_event() {
    let files = [(
        "w.mjs",
        "import { parentPort } from 'node:worker_threads';\nawait import('./nope.mjs');\n",
    )];
    assert_eq!(
        run_file_worker("dyn-missing", &files, "w.mjs", false),
        vec!["online | error:Error:ERR_MODULE_NOT_FOUND:Cannot find module '{DIR}/nope.mjs' imported from {DIR}/w.mjs | exit:1"]
    );
}

#[test]
fn esm_dynamic_import_of_a_missing_file_caught_posts_the_code() {
    let files = [(
        "w.mjs",
        "import { parentPort } from 'node:worker_threads';\ntry { await import('./nope.mjs'); parentPort.postMessage('no throw'); } catch (e) { parentPort.postMessage(e.code); }\n",
    )];
    assert_eq!(
        run_file_worker("dyn-missing-caught", &files, "w.mjs", false),
        vec![r#"online | message:"ERR_MODULE_NOT_FOUND" | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_a_syntax_error_caught_posts_the_name() {
    let files = [
        ("bad.mjs", "export const = ;\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\ntry { await import('./bad.mjs'); parentPort.postMessage('no throw'); } catch (e) { parentPort.postMessage(e.name); }\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-syntax", &files, "w.mjs", false),
        vec![r#"online | message:"SyntaxError" | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_a_throwing_module_uncaught_is_an_error_event() {
    let files = [
        ("boom.mjs", "throw new TypeError('dep-boom');\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nawait import('./boom.mjs');\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-throws", &files, "w.mjs", false),
        vec!["online | error:TypeError:undefined:dep-boom | exit:1"]
    );
}

#[test]
fn esm_dynamic_import_of_a_module_in_the_static_graph_returns_the_same_namespace() {
    let files = [
        ("dep.mjs", "export const a = 1;\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport * as s from './dep.mjs';\nconst d = await import('./dep.mjs');\nparentPort.postMessage(s === d);\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-same", &files, "w.mjs", false),
        vec![r#"online | message:true | exit:0"#]
    );
}

#[test]
fn esm_dependency_with_top_level_await_finishes_before_the_importer_body() {
    let files = [
        (
            "tla.mjs",
            "await new Promise((r) => setTimeout(r, 5));\nglobalThis.order = (globalThis.order || []).concat('dep');\nexport const v = 1;\n",
        ),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport { v } from './tla.mjs';\nparentPort.postMessage({ v, order: globalThis.order.concat('body') });\n",
        ),
    ];
    assert_eq!(
        run_file_worker("tla-dep", &files, "w.mjs", false),
        vec![r#"online | message:{"v":1,"order":["dep","body"]} | exit:0"#]
    );
}

#[test]
fn esm_sibling_dependencies_with_top_level_await_keep_nodes_order() {
    // Node evaluates async siblings concurrently: both start before either resumes, and the shorter timer finishes first
    let files = [
        (
            "sa.mjs",
            "globalThis.order = (globalThis.order || []).concat('a0');\nawait new Promise((r) => setTimeout(r, 20));\nglobalThis.order = globalThis.order.concat('a1');\nexport const n = 'a';\n",
        ),
        (
            "sb.mjs",
            "globalThis.order = (globalThis.order || []).concat('b0');\nawait new Promise((r) => setTimeout(r, 5));\nglobalThis.order = globalThis.order.concat('b1');\nexport const n = 'b';\n",
        ),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport { n as na } from './sa.mjs';\nimport { n as nb } from './sb.mjs';\nparentPort.postMessage({ na, nb, order: globalThis.order });\n",
        ),
    ];
    assert_eq!(
        run_file_worker("tla-siblings", &files, "w.mjs", false),
        vec![r#"online | message:{"na":"a","nb":"b","order":["a0","b0","b1","a1"]} | exit:0"#]
    );
}

#[test]
fn esm_dynamic_import_of_a_top_level_await_dependency_sees_its_value() {
    let files = [
        (
            "tla.mjs",
            "await new Promise((r) => setTimeout(r, 5));\nexport const v = 1;\n",
        ),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport { v } from './tla.mjs';\nconst d = await import('./tla.mjs');\nparentPort.postMessage([v, d.v]);\n",
        ),
    ];
    assert_eq!(
        run_file_worker("dyn-tla", &files, "w.mjs", false),
        vec!["online | message:[1,1] | exit:0"]
    );
}

/// Synchronous graphs keep depth-first source order (guards the sync path of the ordered evaluation).
#[test]
fn esm_sync_graph_evaluates_depth_first_in_source_order() {
    let files = [
        ("d1.mjs", "globalThis.order = (globalThis.order || []).concat('d1');\n"),
        (
            "m1.mjs",
            "import './d1.mjs';\nglobalThis.order = globalThis.order.concat('m1');\n",
        ),
        ("m2.mjs", "globalThis.order = (globalThis.order || []).concat('m2');\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport './m1.mjs';\nimport './m2.mjs';\nparentPort.postMessage(globalThis.order);\n",
        ),
    ];
    assert_eq!(
        run_file_worker("sync-dfs", &files, "w.mjs", false),
        vec![r#"online | message:["d1","m1","m2"] | exit:0"#]
    );
}

/// A synchronous sibling runs while an async sibling awaits; the importer body runs after both.
#[test]
fn esm_sync_sibling_runs_while_an_async_sibling_awaits() {
    let files = [
        (
            "sa.mjs",
            "globalThis.order = (globalThis.order || []).concat('a0');\nawait new Promise((r) => setTimeout(r, 20));\nglobalThis.order = globalThis.order.concat('a1');\n",
        ),
        ("sc.mjs", "globalThis.order = (globalThis.order || []).concat('c');\n"),
        (
            "w.mjs",
            "import { parentPort } from 'node:worker_threads';\nimport './sa.mjs';\nimport './sc.mjs';\nglobalThis.order = globalThis.order.concat('body');\nparentPort.postMessage(globalThis.order);\n",
        ),
    ];
    assert_eq!(
        run_file_worker("sync-behind-async", &files, "w.mjs", false),
        vec![r#"online | message:["a0","c","a1","body"] | exit:0"#]
    );
}
