#[test]
fn diagnostics_channels_publish_bind_stores_and_trace() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_diagnostics_channel");
    fs::write(
            dir.join("index.js"),
            "var diagnostics = require('node:diagnostics_channel');\n\
             module.exports = async function () {\n\
             \x20 var events = []; var work = diagnostics.channel('work'); var same = work === diagnostics.channel('work');\n\
             \x20 function subscriber(data, name) { events.push(name + ':' + data.value); } diagnostics.subscribe('work', subscriber); diagnostics.subscribe('work', subscriber);\n\
             \x20 var store = { run: function(value, callback) { events.push('store:' + value); return callback(); } }; work.bindStore(store, function(data) { return data.value * 2; });\n\
             \x20 var storeResult = work.runStores({ value: 2 }, function(left, right) { return left + right; }, null, 3, 4); work.publish({ value: 5 }); var removed = diagnostics.unsubscribe('work', subscriber); work.publish({ value: 6 }); work.unbindStore(store);\n\
             \x20 var trace = diagnostics.tracingChannel('operation'); ['start', 'end', 'asyncStart', 'asyncEnd', 'error'].forEach(function(name) { trace[name].subscribe(function(context) { events.push(name + ':' + (context.result || context.error && context.error.message || '')); }); });\n\
             \x20 var sync = trace.traceSync(function(value) { return value + 1; }, {}, null, 4); var promised = await trace.tracePromise(async function(value) { return value * 2; }, {}, null, 3); var failed; try { trace.traceSync(function() { throw new Error('bad'); }, {}); } catch (error) { failed = error.message; }\n\
             \x20 return [same, storeResult, removed, diagnostics.hasSubscribers('work'), sync, promised, failed, events];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_diagnostics_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseDiagnostics = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseDiagnostics").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,7,true,false,5,6,"bad",["store:4","store:10","work:5","store:12","start:","end:5","start:","asyncStart:6","asyncEnd:6","end:6","start:","error:bad","end:bad"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn async_hooks_preserve_storage_and_resource_scope() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_async_hooks");
    fs::write(
            dir.join("index.js"),
            "var hooks = require('node:async_hooks');\n\
             module.exports = async function () {\n\
             \x20 var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default', name: 'request' }); var events = [storage.getStore()]; var bound; var snapshot;\n\
             \x20 var result = await storage.run('outer', async function(value) { events.push(storage.getStore() + ':' + value); bound = hooks.AsyncLocalStorage.bind(function(suffix) { return storage.getStore() + suffix; }); snapshot = hooks.AsyncLocalStorage.snapshot(); await Promise.resolve(); events.push(storage.getStore()); var nested = storage.run('inner', function() { return storage.getStore(); }); events.push(nested + ':' + storage.getStore()); var exited = storage.exit(function() { return storage.getStore(); }); events.push(String(exited) + ':' + storage.getStore()); return 'done'; }, 4);\n\
             \x20 storage.enterWith('changed'); var rebound = bound('!'); var snapped = snapshot(function() { return storage.getStore(); });\n\
             \x20 var resource = new hooks.AsyncResource('work'); var outside = hooks.executionAsyncId(); var inside = resource.runInAsyncScope(function(left, right) { return [hooks.executionAsyncId(), hooks.triggerAsyncId(), hooks.executionAsyncResource() === resource, left + right]; }, null, 2, 3); var reboundResource = resource.bind(function() { return hooks.executionAsyncId(); })(); resource.emitDestroy();\n\
             \x20 storage.disable(); return [events, result, rebound, snapped, storage.getStore() === undefined, outside, inside, reboundResource, resource.asyncId(), resource.triggerAsyncId(), resource._destroyed, hooks.createHook({}).enable().disable().callbacks];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_async_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAsyncHooks = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseAsyncHooks").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["default","outer:4","outer","inner:outer","undefined:outer"],"done","outer!","outer",true,1,[2,1,true,5],2,2,1,true,{}]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn async_hooks_disable_releases_registration_and_run_reactivates_it() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_async_hooks_disable");
    fs::write(
        dir.join("index.js"),
        r#"var hooks = require('node:async_hooks');
            module.exports = function () {
              var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default' });
              var registered = globalThis.__thawAsyncLocalStorages;
              var initial = registered.has(storage);
              storage.disable();
              var released = !registered.has(storage);
              storage.disable();
              var repeated = !registered.has(storage);
              // The snapshot holds the disabled-time frame. Reusing this
              // storage later cannot borrow the invoking run's store.
              var withoutDisabled = hooks.AsyncLocalStorage.snapshot();
              var duringRun = storage.run('run', function () {
                return [registered.has(storage), storage.getStore(),
                        withoutDisabled(function () { return storage.getStore(); })];
              });
              var reusedByRun = registered.has(storage);
              storage.disable();
              storage.enterWith('enter');
              var reusedByEnter = registered.has(storage);
              var withEnabled = hooks.AsyncLocalStorage.snapshot();
              storage.enterWith('changed');
              var capturedEnabled = withEnabled(function () { return storage.getStore(); });
              var restored = storage.getStore();
              storage.disable();
              var oldSnapshot;
              var disabledInside = storage.run('old-root', function () {
                oldSnapshot = hooks.AsyncLocalStorage.snapshot();
                storage.disable();
                return storage.getStore() === undefined;
              });
              var restoredDisabled = storage.getStore() === undefined;
              var afterReactivation = storage.run('fresh', function () {
                return [storage.getStore(), oldSnapshot(function () { return storage.getStore(); })];
              });
              storage.disable();
              var oldWeakMap = globalThis.WeakMap, oldSet = globalThis.Set;
              var oldHas = oldSet.prototype.has, oldAdd = oldSet.prototype.add;
              var oldDelete = oldSet.prototype.delete;
              var intrinsicControl;
              try {
                globalThis.WeakMap = function () { throw new Error('changed WeakMap'); };
                globalThis.Set = function () { throw new Error('changed Set'); };
                oldSet.prototype.has = oldSet.prototype.add = oldSet.prototype.delete =
                  function () { throw new Error('changed Set method'); };
                var resilient = new hooks.AsyncLocalStorage({ defaultValue: 'default' });
                resilient.enterWith('one');
                var beforeReplacement = hooks.AsyncLocalStorage.snapshot();
                resilient.enterWith('two');
                intrinsicControl = [resilient.getStore(),
                  beforeReplacement(function () { return resilient.getStore(); })];
                resilient.disable();
                intrinsicControl.push(beforeReplacement(function () { return resilient.getStore(); }));
              } finally {
                globalThis.WeakMap = oldWeakMap; globalThis.Set = oldSet;
                oldSet.prototype.has = oldHas; oldSet.prototype.add = oldAdd;
                oldSet.prototype.delete = oldDelete;
              }
              return [initial, released, repeated, duringRun, reusedByRun,
                      reusedByEnter, capturedEnabled, restored, !registered.has(storage),
                      disabledInside, restoredDisabled, afterReactivation, intrinsicControl];
            };"#,
    ).unwrap();
    let empty_node_modules = temp_registry("builtin_async_hooks_disable_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }};\n\
         globalThis.exports = globalThis.module.exports;\n\
         globalThis.require = function(name) {{ throw new Error('unexpected require ' + name); }};\n\
         {bundle}\n\
         globalThis.exerciseAsyncHooksDisable = module.exports;\n"
    );
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"exerciseAsyncHooksDisable".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        r#"[true,true,true,[true,"run","default"],true,true,"enter","changed",true,true,true,["fresh","default"],["two","one",null]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn async_hooks_capture_reactions_and_host_callbacks_without_shared_store_state() {
    // Source-only regression: registration context, not settlement context,
    // owns pending reactions and internal await continuations.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_async_hooks_continuations");
    fs::write(dir.join("index.js"), r#"
        var hooks = require('node:async_hooks');
        module.exports = async function () {
          var storage = new hooks.AsyncLocalStorage({ defaultValue: 'default' });
          var resolveA, resolveB;
          var a = storage.run('A', async function () {
            await new Promise(function (resolve) { resolveA = resolve; });
            return storage.getStore();
          });
          var b = storage.run('B', async function () {
            await new Promise(function (resolve) { resolveB = resolve; });
            return storage.getStore();
          });
          var outside = storage.getStore();
          storage.enterWith('outside');
          resolveB(); resolveA();
          var overlap = await Promise.all([a, b]);
          var captured = storage.run('captured', function () {
            return [hooks.AsyncLocalStorage.bind(function () { return storage.getStore(); }),
                    hooks.AsyncLocalStorage.snapshot(),
                    new hooks.AsyncResource('captured')];
          });
          var saved = [captured[0](), captured[1](function () { return storage.getStore(); }),
                       captured[2].runInAsyncScope(function () { return storage.getStore(); })];
          var replaced = storage.run('seed', function () {
            storage.enterWith('first');
            var prior = hooks.AsyncLocalStorage.snapshot();
            for (var index = 0; index < 1000; index++) storage.enterWith('replacement-' + index);
            return [prior(function () { return storage.getStore(); }), storage.getStore()];
          });
          var exited = storage.run('present', function () {
            return storage.exit(function () { return storage.getStore(); });
          });
          var timer = new Promise(function (resolve) {
            storage.run('timer', function () { setTimeout(function () { resolve(storage.getStore()); }, 0); });
          });
          var tick = new Promise(function (resolve) {
            storage.run('tick', function () { process.nextTick(function () { resolve(storage.getStore()); }); });
          });
          var host = await Promise.all([timer, tick]);
          var resolveOld;
          var old = storage.run('old', async function () {
            await new Promise(function (resolve) { resolveOld = resolve; });
            return storage.getStore();
          });
          storage.disable();
          storage.enterWith('new');
          resolveOld();
          var invalidated = await old;
          return [outside, overlap, saved, replaced, exited === undefined, host, invalidated,
                  storage.getStore()];
        };
    "#).unwrap();
    let empty_node_modules = temp_registry("builtin_async_hooks_continuations_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseAsyncHooksContinuations = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(c"exerciseAsyncHooksContinuations".as_ptr(), c"[]".as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        r#"["default",["A","B"],["captured","captured","captured"],["first","replacement-999"],true,["timer","tick"],"default","new"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn tty_builtin_reports_capabilities_and_emits_ansi_sequences() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_tty");
    fs::write(
            dir.join("index.js"),
            "var tty = require('node:tty');\n\
             module.exports = async function () {\n\
             \x20 var input = new tty.ReadStream(0); var same = input.setRawMode(true) === input; input.setRawMode(false);\n\
             \x20 var output = new tty.WriteStream(1); var callbacks = []; output.write('text'); output.cursorTo(2, 3, function() { callbacks.push('cursor'); }); output.moveCursor(-1, 2, function() { callbacks.push('move'); }); output.clearLine(0); output.clearScreenDown(); await Promise.resolve();\n\
             \x20 return [tty.isatty(1), input.isTTY, input.isRaw, same, output.getWindowSize(), output.getColorDepth({ FORCE_COLOR: '3' }), output.getColorDepth({ TERM: 'xterm-256color' }), output.hasColors(256, { TERM: 'xterm-256color' }), output.hasColors(16, {}), output._output, callbacks.sort().join(',')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_tty_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTty = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseTty").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
            result,
            "[false,true,false,true,[80,24],24,8,true,false,\"text\\u001b[4;3H\\u001b[1D\\u001b[2B\\u001b[2K\\u001b[0J\",\"cursor,move\"]"
        );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn module_create_require_loads_relative_and_builtin_dependencies() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_module_create_require");
    fs::write(
            dir.join("index.js"),
            "import { createRequire, isBuiltin, builtinModules, registerHooks } from 'node:module';\n\
             const localRequire = createRequire('pkg/index.js');\n\
             export default function () {\n\
             \x20 const dependency = localRequire('./dependency'); const path = localRequire('node:path'); const hooks = registerHooks({}); hooks.deregister();\n\
             \x20 return [dependency.value, path.basename('/tmp/file.txt'), localRequire.resolve('./dependency'), Object.keys(localRequire.cache).length >= 3, isBuiltin('node:path'), isBuiltin('missing'), builtinModules.includes('stream'), isBuiltin('child_process'), isBuiltin('node:https'), builtinModules.includes('child_process'), builtinModules.includes('https'), hooks.active];\n\
             }",
        )
        .unwrap();
    fs::write(dir.join("dependency.js"), "module.exports = { value: 42 };").unwrap();
    let empty_node_modules = temp_registry("builtin_module_create_require_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseModule = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseModule").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[42,"file.txt","pkg/dependency.js",true,true,false,true,true,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}
