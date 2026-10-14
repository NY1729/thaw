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
        r#"[true,7,true,true,5,6,"bad",["store:4","work:2","work:2","work:5","work:5","work:6","start:","end:5","start:","end:","asyncStart:6","asyncEnd:6","start:","error:bad","end:bad"]]"#
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

#[test]
fn child_process_exec_custom_promisify_preserves_child_and_outputs() {
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_child_process_promisify");
        fs::write(
            dir.join("index.js"),
            r#"
var cp = require('node:child_process'), util = require('node:util');
module.exports = async function() {
  var children = [], records = [], customSymbol = Symbol.for('nodejs.util.promisify.custom');
  function close(child) {
    var record = { child: child, closed: new Promise(function(resolve) {
      child.once('close', function(code, signal) { resolve([code, signal]); });
    }) };
    records.push(record);
    return record;
  }
  function track(promise) { children.push(promise.child); return { promise: promise, child: promise.child, record: close(promise.child) }; }
  function callbackChild(child) { children.push(child); return { child: child, record: close(child) }; }
  try {
    var execCustom = cp.exec[customSymbol], fileCustom = cp.execFile[customSymbol];
    var symbols = [typeof execCustom === 'function', typeof fileCustom === 'function',
      Object.getOwnPropertyDescriptor(cp.exec, customSymbol).enumerable === false,
      Object.getOwnPropertyDescriptor(cp.execFile, customSymbol).enumerable === false,
      util.promisify(cp.exec) === execCustom, util.promisify(cp.execFile) === fileCustom,
      util.promisify(execCustom) === execCustom, util.promisify(fileCustom) === fileCustom];

    var execRun = track(util.promisify(cp.exec)('printf exec-out; printf exec-err >&2'));
    var execAccess = execRun.child instanceof cp.ChildProcess && !!execRun.child.stdout && !!execRun.child.stderr;
    var execValue = await execRun.promise, execClosed = await execRun.record.closed;
    var execSuccess = [execValue.stdout, execValue.stderr, execAccess, execRun.promise.child === execRun.child,
      execClosed[0], execRun.child.exitCode];

    var omittedRun = track(util.promisify(cp.execFile)('/bin/echo', { encoding: 'buffer' }));
    var omittedAccess = omittedRun.child.stdout instanceof require('node:stream').PassThrough;
    var omittedValue = await omittedRun.promise, omittedClosed = await omittedRun.record.closed;
    var omittedArgs = [Buffer.isBuffer(omittedValue.stdout), Array.from(omittedValue.stdout),
      omittedValue.stderr.length, omittedAccess, omittedClosed[0]];

    var explicitRun = track(util.promisify(cp.execFile)('/bin/sh', ['-c', 'printf file-out; printf file-err >&2'], { encoding: 'buffer' }));
    var explicitValue = await explicitRun.promise, explicitClosed = await explicitRun.record.closed;
    var explicitArgs = [Buffer.isBuffer(explicitValue.stdout), Array.from(explicitValue.stdout),
      Buffer.isBuffer(explicitValue.stderr), Array.from(explicitValue.stderr), explicitClosed[0]];

    var failedExecRun = track(execCustom('printf exec-bad-out; printf exec-bad-err >&2; exit 7'));
    var failedExecError = await failedExecRun.promise.then(function() { return null; }, function(error) { return error; });
    var failedExecClosed = await failedExecRun.record.closed;
    var failedExec = [failedExecError.code, failedExecError.stdout, failedExecError.stderr,
      failedExecError.stdout === 'exec-bad-out' && failedExecError.stderr === 'exec-bad-err',
      failedExecClosed[0], failedExecRun.child.exitCode];

    var failedFileRun = track(fileCustom('/bin/sh', ['-c', 'printf file-bad-out; printf file-bad-err >&2; exit 8']));
    var failedFileError = await failedFileRun.promise.then(function() { return null; }, function(error) { return error; });
    var failedFileClosed = await failedFileRun.record.closed;
    var failedFile = [failedFileError.code, failedFileError.stdout, failedFileError.stderr,
      failedFileError.stdout === 'file-bad-out' && failedFileError.stderr === 'file-bad-err',
      failedFileClosed[0], failedFileRun.child.exitCode];

    var missingRun = track(fileCustom('/thaw/missing-promisify-executable', []));
    var missingError = await missingRun.promise.then(function() { return null; }, function(error) { return error; });
    var missingClosed = await missingRun.record.closed;
    var missing = [missingError.code, missingError.stdout, missingError.stderr,
      missingClosed[0], missingRun.child.exitCode];

    var callbackExecCount = 0, callbackExecResolve;
    var callbackExecPromise = new Promise(function(resolve) { callbackExecResolve = resolve; });
    var callbackExecChild = cp.exec('printf callback-out; printf callback-err >&2', { encoding: 'utf8' }, function(error, stdout, stderr) {
      callbackExecCount++; callbackExecResolve([error === null, stdout, stderr]);
    });
    var callbackExec = callbackChild(callbackExecChild), callbackExecAccess = callbackExecChild instanceof cp.ChildProcess;
    var callbackExecResult = await callbackExecPromise, callbackExecClosed = await callbackExec.record.closed;

    var callbackFileCount = 0, callbackFileResolve;
    var callbackFilePromise = new Promise(function(resolve) { callbackFileResolve = resolve; });
    var callbackFileChild = cp.execFile('/bin/printf', ['callback-file'], { encoding: 'utf8' }, function(error, stdout, stderr) {
      callbackFileCount++; callbackFileResolve([error === null, stdout, stderr]);
    });
    var callbackFile = callbackChild(callbackFileChild), callbackFileAccess = callbackFileChild instanceof cp.ChildProcess;
    var callbackFileResult = await callbackFilePromise, callbackFileClosed = await callbackFile.record.closed;

    var setupError;
    try { fileCustom('/bin/true', [], { stdio: 'invalid' }); } catch (error) { setupError = error; }
    return [symbols, execSuccess, omittedArgs, explicitArgs, failedExec, failedFile, missing,
      [callbackExecAccess, callbackExecCount, callbackExecResult, callbackExecClosed[0]],
      [callbackFileAccess, callbackFileCount, callbackFileResult, callbackFileClosed[0]],
      setupError && setupError.code === 'ERR_INVALID_ARG_VALUE'];
  } finally {
    children.forEach(function(child) { if (!child._closed) child.kill('SIGKILL'); });
    await Promise.all(records.map(function(record) { return record.closed; }));
  }
};
"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_promisify_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildPromisify = module.exports;");
        let source = CString::new(script).unwrap();
        let loaded = thaw_quickjs::thaw_js_load(source.as_ptr()) == 1;
        let result = if loaded {
            let result_ptr = thaw_quickjs::thaw_js_call(
                CString::new("exerciseChildPromisify").unwrap().as_ptr(),
                CString::new("[]").unwrap().as_ptr(),
            );
            Some(unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned())
        } else {
            None
        };
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        assert_eq!(file_count, 7);
        assert!(loaded);
        assert_eq!(result.as_deref(), Some(r#"[[true,true,true,true,true,true,true,true],["exec-out","exec-err",true,true,0,0],[true,[10],0,true,0],[true,[102,105,108,101,45,111,117,116],true,[102,105,108,101,45,101,114,114],0],[7,"exec-bad-out","exec-bad-err",true,7,7],[8,"file-bad-out","file-bad-err",true,8,8],["ENOENT","","",null,null],[true,1,[true,"callback-out","callback-err"],0],[true,1,[true,"callback-file",""],0],true]"#));
    })
    .join()
    .unwrap();
}

#[test]
fn child_process_sync_max_buffer_counts_native_output_bytes() {
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_child_process_max_buffer_bytes");
        fs::write(
            dir.join("index.js"),
            r#"
var cp = require('node:child_process');
module.exports = function() {
  var utf8Script = 'printf "\\342\\230\\203"';
  var utf8Low = cp.spawnSync('/bin/sh', ['-c', utf8Script], { encoding: 'utf8', maxBuffer: 2 });
  var utf8At = cp.spawnSync('/bin/sh', ['-c', utf8Script], { encoding: 'utf8', maxBuffer: 3 });
  var hexAt = cp.spawnSync('/bin/sh', ['-c', 'printf ab'], { encoding: 'hex', maxBuffer: 2 });
  var hexLow = cp.spawnSync('/bin/sh', ['-c', 'printf ab'], { encoding: 'hex', maxBuffer: 1 });
  var bufferAt = cp.spawnSync('/bin/sh', ['-c', 'printf ABC'], { encoding: 'buffer', maxBuffer: 3 });
  var bufferLow = cp.spawnSync('/bin/sh', ['-c', 'printf ABC'], { encoding: 'buffer', maxBuffer: 2 });
  var stderrOnly = cp.spawnSync('/bin/sh', ['-c', 'printf ee >&2'], { encoding: 'utf8', maxBuffer: 2 });
  var combined = cp.spawnSync('/bin/sh', ['-c', 'printf o; printf e >&2'], { encoding: 'utf8', maxBuffer: 1 });
  var fileError;
  try { cp.execFileSync('/bin/sh', ['-c', 'printf o; printf e >&2'], { encoding: 'utf8', maxBuffer: 1 }); }
  catch (error) { fileError = [error.code, error.stdout, error.stderr, error.output, error.pid > 0]; }
  var execError;
  try { cp.execSync('printf x; printf y >&2', { encoding: 'utf8', maxBuffer: 1 }); }
  catch (error) { execError = [error.code, error.stdout, error.stderr, error.output, error.pid > 0]; }
  var fileAt = cp.execFileSync('/bin/sh', ['-c', utf8Script], { encoding: 'utf8', maxBuffer: 3 });
  var execAt = cp.execSync(utf8Script, { encoding: 'utf8', maxBuffer: 3 });
  var missing = cp.spawnSync('/thaw/missing-max-buffer-executable', [], { encoding: 'utf8', maxBuffer: -1 });
  var nonzero = cp.spawnSync('/bin/sh', ['-c', 'printf x; exit 6'], { encoding: 'utf8', maxBuffer: 2 });
  return [
    [utf8Low.error.code, utf8Low.status, utf8Low.stdout, utf8Low.stderr],
    [utf8At.status, utf8At.stdout],
    [hexAt.status, hexAt.stdout],
    [hexLow.error.code, hexLow.status, hexLow.stdout],
    [bufferAt.status, Buffer.isBuffer(bufferAt.stdout), Array.from(bufferAt.stdout)],
    [bufferLow.error.code, bufferLow.status, Buffer.isBuffer(bufferLow.stdout), Array.from(bufferLow.stdout)],
    [stderrOnly.status, stderrOnly.stdout, stderrOnly.stderr],
    [combined.error.code, combined.status, combined.stdout, combined.stderr],
    fileError, execError, fileAt, execAt,
    [missing.error.code, missing.status],
    [nonzero.status, nonzero.error === undefined, nonzero.stdout]
  ];
};
"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_max_buffer_bytes_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildMaxBufferBytes = module.exports;");
        let source = CString::new(script).unwrap();
        let loaded = thaw_quickjs::thaw_js_load(source.as_ptr()) == 1;
        let result = if loaded {
            let result_ptr = thaw_quickjs::thaw_js_call(
                CString::new("exerciseChildMaxBufferBytes").unwrap().as_ptr(),
                CString::new("[]").unwrap().as_ptr(),
            );
            Some(unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned())
        } else {
            None
        };
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        assert_eq!(file_count, 6);
        assert!(loaded);
        assert_eq!(
            result.as_deref(),
            Some(r#"[["ENOBUFS",0,"☃",""],[0,"☃"],[0,"6162"],["ENOBUFS",0,"6162"],[0,true,[65,66,67]],["ENOBUFS",0,true,[65,66,67]],[0,"","ee"],["ENOBUFS",0,"o","e"],["ENOBUFS","o","e",[null,"o","e"],true],["ENOBUFS","x","y",[null,"x","y"],true],"☃","☃",["ENOENT",null],[6,true,"x"]]"#)
        );
    })
    .join()
    .unwrap();
}

#[test]
fn child_process_async_exec_limits_each_pipe_and_preserves_null_encoding() {
    std::thread::spawn(|| {
        use std::ffi::{CStr, CString};

        let dir = temp_registry("builtin_child_process_async_output_contract");
        fs::write(
            dir.join("index.js"),
            r#"
var cp = require('node:child_process'), util = require('node:util');
module.exports = async function() {
  var children = [], records = [];
  function close(child) {
    var record = { child: child, closed: new Promise(function(resolve) {
      child.once('close', function(code, signal) { resolve([code, signal]); });
    }) };
    records.push(record);
    return record;
  }
  function callbackCall(start) {
    var count = 0, resolveResult;
    var result = new Promise(function(resolve) { resolveResult = resolve; });
    var child = start(function(error, stdout, stderr) {
      count++;
      resolveResult([error, stdout, stderr]);
    });
    children.push(child);
    return { child: child, result: result, closed: close(child).closed, count: function() { return count; } };
  }
  function promiseCall(promise) {
    children.push(promise.child);
    return { promise: promise, child: promise.child, closed: close(promise.child).closed };
  }
  try {
    var balancedCallback = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', 'printf abc; printf def >&2'], { encoding: 'utf8', maxBuffer: 3 }, done);
    });
    var balancedCallbackResult = await balancedCallback.result, balancedCallbackClose = await balancedCallback.closed;
    var balancedPromise = promiseCall(util.promisify(cp.exec)('printf abc; printf def >&2', { encoding: 'utf8', maxBuffer: 3 }));
    var balancedPromiseChildVisible = balancedPromise.promise.child === balancedPromise.child && !!balancedPromise.child.stdout;
    var balancedPromiseValue = await balancedPromise.promise, balancedPromiseClose = await balancedPromise.closed;

    var stdoutOverflow = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', 'printf abcd; printf x >&2'], { encoding: 'utf8', maxBuffer: 3 }, done);
    });
    var stdoutErrorResult = await stdoutOverflow.result, stdoutClose = await stdoutOverflow.closed;
    var stderrOverflow = promiseCall(util.promisify(cp.exec)('printf x; printf abcd >&2', { encoding: 'utf8', maxBuffer: 3 }));
    var stderrError = await stderrOverflow.promise.then(function() { return null; }, function(error) { return error; });
    var stderrClose = await stderrOverflow.closed;

    var utf8Command = 'printf "\\342\\230\\203"';
    var utf8Overflow = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', utf8Command], { encoding: 'utf8', maxBuffer: 2 }, done);
    });
    var utf8ErrorResult = await utf8Overflow.result, utf8OverflowClose = await utf8Overflow.closed;
    var utf8Boundary = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', utf8Command], { encoding: 'utf8', maxBuffer: 3 }, done);
    });
    var utf8BoundaryResult = await utf8Boundary.result, utf8BoundaryClose = await utf8Boundary.closed;
    var hexBoundary = promiseCall(util.promisify(cp.execFile)('/bin/printf', ['ab'], { encoding: 'hex', maxBuffer: 2 }));
    var hexValue = await hexBoundary.promise, hexClose = await hexBoundary.closed;

    var invalidCommand = 'printf "\\377"; printf "\\376" >&2';
    var nullExec = callbackCall(function(done) {
      return cp.exec(invalidCommand, { encoding: null, maxBuffer: 1 }, done);
    });
    var nullExecResult = await nullExec.result, nullExecClose = await nullExec.closed;
    var nullFile = promiseCall(util.promisify(cp.execFile)('/bin/sh', ['-c', invalidCommand], { encoding: null, maxBuffer: 1 }));
    var nullFileValue = await nullFile.promise, nullFileClose = await nullFile.closed;
    var bufferFile = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', invalidCommand], { encoding: 'buffer', maxBuffer: 1 }, done);
    });
    var bufferResult = await bufferFile.result, bufferClose = await bufferFile.closed;
    var utf8Default = callbackCall(function(done) {
      return cp.execFile('/bin/sh', ['-c', utf8Command], { maxBuffer: 3 }, done);
    });
    var utf8DefaultResult = await utf8Default.result, utf8DefaultClose = await utf8Default.closed;
    var failedNull = promiseCall(util.promisify(cp.execFile)('/bin/sh', ['-c', 'printf "\\377"; printf "\\376" >&2; exit 5'], { encoding: null, maxBuffer: 1 }));
    var failedNullError = await failedNull.promise.then(function() { return null; }, function(error) { return error; });
    var failedNullClose = await failedNull.closed;

    return [
      [balancedCallbackResult[0] === null, balancedCallbackResult[1], balancedCallbackResult[2],
        balancedCallback.count(), balancedCallbackClose[0]],
      [balancedPromiseValue.stdout, balancedPromiseValue.stderr, balancedPromiseChildVisible,
        balancedPromiseClose[0]],
      [stdoutErrorResult[0].code, stdoutErrorResult[0].message.indexOf('stdout') >= 0,
        stdoutErrorResult[0].stdout === stdoutErrorResult[1], stdoutErrorResult[0].stderr === stdoutErrorResult[2],
        stdoutErrorResult[1], stdoutErrorResult[2], stdoutOverflow.count(), stdoutClose[0]],
      [stderrError.code, stderrError.message.indexOf('stderr') >= 0, stderrError.stdout, stderrError.stderr, stderrClose[0]],
      [utf8ErrorResult[0].code, utf8ErrorResult[1], utf8ErrorResult[2], utf8OverflowClose[0]],
      [utf8BoundaryResult[0] === null, utf8BoundaryResult[1], utf8BoundaryClose[0]],
      [hexValue.stdout, hexValue.stdout.length, hexClose[0]],
      [nullExecResult[0] === null, Buffer.isBuffer(nullExecResult[1]), Array.from(nullExecResult[1]),
        Buffer.isBuffer(nullExecResult[2]), Array.from(nullExecResult[2]), nullExecClose[0]],
      [Buffer.isBuffer(nullFileValue.stdout), Array.from(nullFileValue.stdout),
        Buffer.isBuffer(nullFileValue.stderr), Array.from(nullFileValue.stderr), nullFileClose[0]],
      [bufferResult[0] === null, Buffer.isBuffer(bufferResult[1]), Array.from(bufferResult[1]),
        Buffer.isBuffer(bufferResult[2]), Array.from(bufferResult[2]), bufferClose[0]],
      [utf8DefaultResult[0] === null, utf8DefaultResult[1], utf8DefaultClose[0]],
      [failedNullError.code, Buffer.isBuffer(failedNullError.stdout), Array.from(failedNullError.stdout),
        Buffer.isBuffer(failedNullError.stderr), Array.from(failedNullError.stderr),
        failedNullClose[0] === 5]
    ];
  } finally {
    children.forEach(function(child) { if (!child._closed) child.kill('SIGKILL'); });
    await Promise.all(records.map(function(record) { return record.closed; }));
  }
};
"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_child_process_async_output_contract_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildAsyncOutput = module.exports;");
        let source = CString::new(script).unwrap();
        let loaded = thaw_quickjs::thaw_js_load(source.as_ptr()) == 1;
        let result = if loaded {
            let result_ptr = thaw_quickjs::thaw_js_call(
                CString::new("exerciseChildAsyncOutput").unwrap().as_ptr(),
                CString::new("[]").unwrap().as_ptr(),
            );
            Some(unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy().into_owned())
        } else {
            None
        };
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        assert_eq!(file_count, 7);
        assert!(loaded);
        assert_eq!(
            result.as_deref(),
            Some(r#"[[true,"abc","def",1,0],["abc","def",true,0],["ERR_CHILD_PROCESS_STDIO_MAXBUFFER",true,true,true,"abc","",1,0],["ERR_CHILD_PROCESS_STDIO_MAXBUFFER",true,"x","abc",0],["ERR_CHILD_PROCESS_STDIO_MAXBUFFER","☃","",0],[true,"☃",0],["6162",4,0],[true,true,[255],true,[254],0],[true,[255],true,[254],0],[true,true,[255],true,[254],0],[true,"☃",0],[5,true,[255],true,[254],true]]"#)
        );
    })
    .join()
    .unwrap();
}
